#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use server::config::{Config, RateLimitConfig};
use server::llm::{LlmChunkStream, LlmClient, LlmError, LlmRequest};
use server::{build_router, AppState};
use shared::{AgentRole, WsEnvelope};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub type WsSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone)]
struct MockOptions {
    fail_roles: HashSet<AgentRole>,
    hang_roles: HashSet<AgentRole>,
    invalid_json_calls: HashMap<AgentRole, usize>,
    delay: Duration,
    chunk_size: usize,
    judge_output: Option<serde_json::Value>,
    probe_ok: bool,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self {
            fail_roles: HashSet::new(),
            hang_roles: HashSet::new(),
            invalid_json_calls: HashMap::new(),
            delay: Duration::from_millis(5),
            chunk_size: 9,
            judge_output: None,
            probe_ok: true,
        }
    }
}

#[derive(Clone)]
pub struct MockLlm {
    options: Arc<MockOptions>,
    calls: Arc<Mutex<HashMap<AgentRole, usize>>>,
}

impl MockLlm {
    pub fn fast() -> Self {
        Self::from_options(MockOptions::default())
    }

    fn from_options(options: MockOptions) -> Self {
        Self {
            options: Arc::new(options),
            calls: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn builder() -> MockBuilder {
        MockBuilder(MockOptions::default())
    }

    pub fn calls(&self, role: AgentRole) -> usize {
        self.calls.lock().unwrap().get(&role).copied().unwrap_or(0)
    }
}

pub struct MockBuilder(MockOptions);

impl MockBuilder {
    pub fn delay(mut self, delay: Duration) -> Self {
        self.0.delay = delay;
        self
    }

    pub fn fail(mut self, role: AgentRole) -> Self {
        self.0.fail_roles.insert(role);
        self
    }

    pub fn hang(mut self, role: AgentRole) -> Self {
        self.0.hang_roles.insert(role);
        self
    }

    pub fn invalid_json(mut self, role: AgentRole, calls: usize) -> Self {
        self.0.invalid_json_calls.insert(role, calls);
        self
    }

    pub fn judge_output(mut self, output: serde_json::Value) -> Self {
        self.0.judge_output = Some(output);
        self
    }

    pub fn probe(mut self, ok: bool) -> Self {
        self.0.probe_ok = ok;
        self
    }

    pub fn build(self) -> MockLlm {
        MockLlm::from_options(self.0)
    }
}

fn role_output(role: AgentRole, model: &str) -> serde_json::Value {
    match role {
        AgentRole::Proponent => serde_json::json!({
            "agent": "proponent",
            "status": "done",
            "message": "Оптимист: утверждение подтверждается известными данными.",
            "arguments": [{
                "id": "a1",
                "stance": "for",
                "claim": "Утверждение согласуется с известными фактами",
                "evidence": "Модель знает подтверждающие сведения",
                "source_ids": [],
                "strength": "moderate"
            }],
            "sources": [],
            "confidence": 0.7,
            "model": model
        }),
        AgentRole::Skeptic => serde_json::json!({
            "agent": "skeptic",
            "status": "done",
            "message": "Скептик: контекст может быть неполным.",
            "arguments": [{
                "id": "a2",
                "stance": "against",
                "claim": "Нужен более точный контекст",
                "evidence": "Формулировка допускает исключения",
                "source_ids": [],
                "strength": "weak"
            }],
            "sources": [],
            "confidence": 0.4,
            "model": model
        }),
        AgentRole::Judge => serde_json::json!({
            "verdict": "confirmed",
            "confidence": 0.6,
            "explanation": "Аргументы за сильнее, но независимой проверки источников не было.",
            "is_falsifiable": true,
            "for": ["a1"],
            "against": ["a2"],
            "sources": [],
            "search_used": false
        }),
    }
}

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(&self, request: LlmRequest) -> Result<LlmChunkStream, LlmError> {
        {
            let mut calls = self.calls.lock().unwrap();
            *calls.entry(request.role).or_insert(0) += 1;
        }

        if self.options.hang_roles.contains(&request.role) {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        if self.options.fail_roles.contains(&request.role) {
            return Err(LlmError::Unavailable("mock provider down".to_owned()));
        }

        let call_number = self.calls(request.role);
        let invalid_limit = self
            .options
            .invalid_json_calls
            .get(&request.role)
            .copied()
            .unwrap_or(0);
        let raw = if call_number <= invalid_limit {
            "this is not JSON".to_owned()
        } else if request.role == AgentRole::Judge {
            self.options
                .judge_output
                .clone()
                .unwrap_or_else(|| role_output(AgentRole::Judge, &request.model))
                .to_string()
        } else {
            role_output(request.role, &request.model).to_string()
        };

        let chunks: Vec<String> = raw
            .chars()
            .collect::<Vec<_>>()
            .chunks(self.options.chunk_size.max(1))
            .map(|chunk| chunk.iter().collect())
            .collect();
        let delay = self.options.delay;

        let stream = async_stream::stream! {
            for chunk in chunks {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                yield Ok::<String, LlmError>(chunk);
            }
        };
        Ok(Box::pin(stream))
    }

    async fn probe(&self) -> Result<(), LlmError> {
        if self.options.probe_ok {
            Ok(())
        } else {
            Err(LlmError::Unavailable("mock probe failed".to_owned()))
        }
    }
}

pub fn test_config() -> Config {
    Config {
        bind_addr: "127.0.0.1:3000".parse().unwrap(),
        llm_base_url: Some("http://127.0.0.1:1/v1".to_owned()),
        llm_api_key: Some("test-key".to_owned()),
        llm_model_debater: "test/debater".to_owned(),
        llm_model_judge: "test/judge".to_owned(),
        allowed_origins: vec!["http://localhost:3000".to_owned()],
        demo_mode: shared::DemoMode::Off,
        max_concurrent_checks: 20,
        rate_limit: RateLimitConfig::default(),
        log_level: "info".to_owned(),
        dist_dir: std::path::PathBuf::from("dist-not-present"),
        fixtures_dir: std::env::temp_dir().join("vd-fixtures-test"),
    }
}

pub fn test_state(llm: MockLlm) -> Arc<AppState> {
    AppState::new(test_config(), Arc::new(llm))
}

pub async fn start_server(state: Arc<AppState>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = build_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, handle)
}

pub const BOUNDARY: &str = "----VeryDebAIteBoundary";

pub fn multipart_body(claim: Option<&str>, file: Option<(&[u8], &str)>) -> (String, Vec<u8>) {
    let mut body = Vec::new();
    if let Some(claim) = claim {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"claim\"\r\n\r\n{claim}\r\n"
            )
            .as_bytes(),
        );
    }
    if let Some((bytes, filename)) = file {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

pub async fn post_check(
    client: &reqwest::Client,
    addr: SocketAddr,
    claim: Option<&str>,
    file: Option<(&[u8], &str)>,
) -> reqwest::Response {
    let (content_type, body) = multipart_body(claim, file);
    client
        .post(format!("http://{addr}/api/check"))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .unwrap()
}

pub fn ws_url(addr: SocketAddr, check_id: &str) -> String {
    format!("ws://{addr}/ws/{check_id}")
}

pub async fn connect_ws(
    addr: SocketAddr,
    check_id: &str,
    origin: Option<&str>,
) -> Result<WsSocket, Box<tokio_tungstenite::tungstenite::Error>> {
    let mut request = ws_url(addr, check_id).into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_str(origin).unwrap());
    }
    connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(Box::new)
}

pub struct WsCollect {
    pub events: Vec<WsEnvelope>,
    pub close_code: Option<u16>,
}

pub async fn next_event(socket: &mut WsSocket) -> Option<WsEnvelope> {
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if let Ok(envelope) = serde_json::from_str::<WsEnvelope>(&text) {
                    return Some(envelope);
                }
            }
            Ok(Message::Ping(_)) => {
                let _ = socket.send(Message::Pong(Vec::new())).await;
            }
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
    None
}

pub async fn collect_events(mut socket: WsSocket, timeout: Duration) -> WsCollect {
    let mut events = Vec::new();
    let mut close_code = None;

    let _ = tokio::time::timeout(timeout, async {
        while let Some(message) = socket.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    if let Ok(envelope) = serde_json::from_str::<WsEnvelope>(&text) {
                        events.push(envelope);
                    }
                }
                Ok(Message::Close(frame)) => {
                    close_code = frame.map(|frame| u16::from(frame.code));
                    break;
                }
                Ok(Message::Ping(_)) => {
                    let _ = socket.send(Message::Pong(Vec::new())).await;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
    .await;

    WsCollect { events, close_code }
}
