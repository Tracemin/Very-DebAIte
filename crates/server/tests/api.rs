mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use reqwest::StatusCode;
use server::llm::replay::{record_fixture, ReplayLlm};
use server::llm::UnavailableLlm;
use server::AppState;
use shared::{
    ApiErrorBody, CheckSnapshot, CheckStatus, CreateCheckResponse, ErrorCode, VerdictKind,
};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
}

fn claim(text: &str) -> &str {
    text
}

async fn wait_terminal(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    id: &str,
    timeout: Duration,
) -> CheckSnapshot {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let response = client
            .get(format!("http://{addr}/api/check/{id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: CheckSnapshot = response.json().await.unwrap();
        if snapshot.status.is_terminal() {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "check did not finish"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn post_without_claim_returns_unified_400() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let response = post_check(&client(), addr, None, Some((b"hello", "note.txt"))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: ApiErrorBody = response.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::InvalidRequest);
    assert!(!body.error.retryable);
    assert!(!body.error.request_id.is_empty());
}

#[tokio::test]
async fn post_with_blank_claim_returns_400() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let response = post_check(&client(), addr, Some("   "), None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: ApiErrorBody = response.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::InvalidRequest);
}

#[tokio::test]
async fn post_png_is_rejected_with_ocr_hint() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0u8; 32]);
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C")),
        Some((&png, "image.png")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let body: ApiErrorBody = response.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::UnsupportedFileType);
    assert!(body.error.message.contains("OCR"));
}

#[tokio::test]
async fn post_oversized_file_returns_413() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let oversized = vec![b'a'; 5 * 1024 * 1024 + 1];
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C")),
        Some((&oversized, "big.txt")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body: ApiErrorBody = response.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::FileTooLarge);
}

#[tokio::test]
async fn valid_check_completes_and_applies_post_validation() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let created: CreateCheckResponse = response.json().await.unwrap();
    assert_eq!(created.ws_url, format!("/ws/{}", created.check_id));

    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snapshot.status, CheckStatus::Done);
    assert_eq!(snapshot.agents.proponent.status, shared::AgentStatus::Done);
    assert_eq!(snapshot.agents.skeptic.status, shared::AgentStatus::Done);
    assert_eq!(snapshot.agents.judge.status, shared::AgentStatus::Done);
    let verdict = snapshot.verdict.expect("verdict");
    // Model said `confirmed`, but there is no checked_http source in v1.
    assert_eq!(verdict.verdict, VerdictKind::MostlyConfirmed);
    assert!(verdict
        .warnings
        .contains(&shared::warnings::NO_VERIFIED_SOURCES.to_owned()));
    assert!(snapshot
        .warnings
        .contains(&shared::warnings::NO_VERIFIED_SOURCES.to_owned()));
    assert_eq!(verdict.for_arguments, vec!["a1"]);
    assert_eq!(verdict.against, vec!["a2"]);
    assert_eq!(verdict.disclaimer, shared::DISCLAIMER);
}

#[tokio::test]
async fn websocket_streams_snapshot_first_then_ordered_events() {
    let mock = MockLlm::builder().delay(Duration::from_millis(25)).build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let mut socket = connect_ws(addr, &created.check_id.to_string(), None)
        .await
        .unwrap();
    let first = next_event(&mut socket).await.expect("snapshot");
    assert_eq!(first.event.event_type(), "snapshot");
    let mut collected = vec![first];
    let rest = collect_events(socket, Duration::from_secs(20)).await;
    collected.extend(rest.events);

    let terminal = collected.last().expect("events");
    assert_eq!(terminal.event.event_type(), "check_completed");
    assert_eq!(rest.close_code, Some(1000));

    let mut last_seq = 0;
    let mut completed_roles = Vec::new();
    let mut verdict_position = None;
    for (index, event) in collected.iter().enumerate() {
        if event.event.event_type() != "snapshot" {
            assert!(event.seq > last_seq, "seq must be monotonic");
            last_seq = event.seq;
        }
        match &event.event {
            shared::WsEvent::AgentCompleted(data) => completed_roles.push(data.result.agent),
            shared::WsEvent::VerdictReady(_) => verdict_position = Some(index),
            _ => {}
        }
    }

    assert!(completed_roles.contains(&shared::AgentRole::Proponent));
    assert!(completed_roles.contains(&shared::AgentRole::Skeptic));
    let verdict_position = verdict_position.expect("verdict_ready");
    let before = &collected[..verdict_position];
    assert!(before.iter().any(|event| matches!(
        &event.event,
        shared::WsEvent::AgentCompleted(data) if data.result.agent == shared::AgentRole::Proponent
    )));
    assert!(before.iter().any(|event| matches!(
        &event.event,
        shared::WsEvent::AgentCompleted(data) if data.result.agent == shared::AgentRole::Skeptic
    )));

    // No agent_delta after that agent's terminal event.
    for role in [shared::AgentRole::Proponent, shared::AgentRole::Skeptic] {
        let finished = collected.iter().position(|event| {
            matches!(
                &event.event,
                shared::WsEvent::AgentCompleted(data) if data.result.agent == role
            )
        });
        if let Some(finished) = finished {
            assert!(!collected[finished + 1..].iter().any(|event| {
                event.agent == Some(role) && event.event.event_type() == "agent_delta"
            }));
        }
    }
}

#[tokio::test]
async fn reconnect_gets_snapshot_with_accumulated_state() {
    let mock = MockLlm::builder().delay(Duration::from_millis(20)).build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let mut first_socket = connect_ws(addr, &created.check_id.to_string(), None)
        .await
        .unwrap();
    let first = next_event(&mut first_socket).await.expect("snapshot");
    assert_eq!(first.event.event_type(), "snapshot");

    let mut last_seen = first.seq;
    let mut saw_delta = false;
    for _ in 0..80 {
        let Some(event) = next_event(&mut first_socket).await else {
            break;
        };
        last_seen = last_seen.max(event.seq);
        if event.event.event_type() == "agent_delta" {
            saw_delta = true;
            break;
        }
    }
    assert!(saw_delta, "expected at least one delta before reconnect");
    drop(first_socket);

    let mut second_socket = connect_ws(addr, &created.check_id.to_string(), None)
        .await
        .unwrap();
    let snapshot_event = next_event(&mut second_socket).await.expect("snapshot");
    let shared::WsEvent::Snapshot(snapshot) = &snapshot_event.event else {
        panic!("first message must be snapshot");
    };
    assert!(snapshot.last_seq >= last_seen);
    assert!(!snapshot.agents.proponent.text.is_empty() || !snapshot.agents.skeptic.text.is_empty());

    let rest = collect_events(second_socket, Duration::from_secs(20)).await;
    if snapshot.status.is_terminal() {
        if let Some(last) = rest.events.last() {
            assert!(last.event.is_terminal());
        }
    } else {
        let last = rest.events.last().expect("terminal event");
        assert_eq!(last.event.event_type(), "check_completed");
    }
}

#[tokio::test]
async fn delete_cancels_running_check() {
    let mock = MockLlm::builder()
        .hang(shared::AgentRole::Proponent)
        .hang(shared::AgentRole::Skeptic)
        .build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let mut socket = connect_ws(addr, &created.check_id.to_string(), None)
        .await
        .unwrap();
    let snapshot = next_event(&mut socket).await.expect("snapshot");
    assert_eq!(snapshot.event.event_type(), "snapshot");

    let delete = client()
        .delete(format!("http://{addr}/api/check/{}", created.check_id))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::ACCEPTED);

    let rest = collect_events(socket, Duration::from_secs(10)).await;
    let terminal = rest.events.last().expect("terminal event");
    let shared::WsEvent::CheckError(error) = &terminal.event else {
        panic!("expected check_error, got {}", terminal.event.event_type());
    };
    assert_eq!(error.code, ErrorCode::Cancelled);

    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(snapshot.status, CheckStatus::Cancelled);

    let second_delete = client()
        .delete(format!("http://{addr}/api/check/{}", created.check_id))
        .send()
        .await
        .unwrap();
    assert_eq!(second_delete.status(), StatusCode::CONFLICT);
    let body: ApiErrorBody = second_delete.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::AlreadyFinished);
}

#[tokio::test]
async fn judge_failure_keeps_partial_results() {
    let mock = MockLlm::builder()
        .invalid_json(shared::AgentRole::Judge, 100)
        .build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snapshot.status, CheckStatus::Error);
    assert!(snapshot.partial);
    assert!(snapshot.verdict.is_none());
    assert_eq!(snapshot.agents.proponent.status, shared::AgentStatus::Done);
    assert_eq!(snapshot.agents.skeptic.status, shared::AgentStatus::Done);
    assert_eq!(snapshot.agents.judge.status, shared::AgentStatus::Error);
    assert_eq!(
        snapshot.agents.judge.error.as_ref().unwrap().code,
        ErrorCode::BadModelOutput
    );
}

#[tokio::test]
async fn both_debaters_failing_forces_insufficient() {
    let mock = MockLlm::builder()
        .fail(shared::AgentRole::Proponent)
        .fail(shared::AgentRole::Skeptic)
        .build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snapshot.status, CheckStatus::Done);
    assert_eq!(snapshot.agents.proponent.status, shared::AgentStatus::Error);
    assert_eq!(snapshot.agents.skeptic.status, shared::AgentStatus::Error);
    let verdict = snapshot.verdict.expect("verdict");
    assert_eq!(verdict.verdict, VerdictKind::Insufficient);
    assert!(verdict
        .warnings
        .contains(&shared::warnings::NO_AGENT_RESULTS.to_owned()));
}

#[tokio::test]
async fn invalid_json_is_repaired_once() {
    let mock = MockLlm::builder()
        .invalid_json(shared::AgentRole::Proponent, 1)
        .invalid_json(shared::AgentRole::Skeptic, 1)
        .build();
    let (addr, _handle) = start_server(test_state(mock)).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();
    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snapshot.agents.proponent.status, shared::AgentStatus::Done);
    assert_eq!(snapshot.agents.skeptic.status, shared::AgentStatus::Done);
}

#[tokio::test]
async fn websocket_origin_is_checked_before_upgrade() {
    let (addr, _handle) = start_server(test_state(MockLlm::fast())).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    let created: CreateCheckResponse = response.json().await.unwrap();

    let rejected = connect_ws(
        addr,
        &created.check_id.to_string(),
        Some("http://evil.example"),
    )
    .await;
    match rejected {
        Err(error) => match *error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
            }
            other => panic!("expected HTTP 403, got {other:?}"),
        },
        Ok(_) => panic!("expected the upgrade to be rejected"),
    }

    let allowed = connect_ws(
        addr,
        &created.check_id.to_string(),
        Some("http://localhost:3000"),
    )
    .await;
    assert!(allowed.is_ok());
}

#[tokio::test]
async fn rate_limit_returns_429_with_retry_after() {
    let mut config = test_config();
    config.rate_limit.per_ip_requests = 1;
    config.rate_limit.per_ip_concurrent = 2;
    let state = AppState::new(config, Arc::new(MockLlm::fast()));
    let (addr, _handle) = start_server(state).await;

    let first = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);

    let second = post_check(&client(), addr, Some(claim("Земля плоская")), None).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(second.headers().get("retry-after").is_some());
    let body: ApiErrorBody = second.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::RateLimited);
}

#[tokio::test]
async fn ready_reports_provider_state() {
    let state = test_state(MockLlm::builder().probe(true).build());
    let (addr, _handle) = start_server(state).await;
    let response = client()
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: shared::ReadyResponse = response.json().await.unwrap();
    assert_eq!(body.status, shared::ReadyStatus::Ready);
    assert!(!body.demo_mode);
    assert!(body.llm.reachable);

    let state = test_state(MockLlm::builder().probe(false).build());
    let (addr, _handle) = start_server(state).await;
    let response = client()
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: shared::ReadyResponse = response.json().await.unwrap();
    assert_eq!(body.status, shared::ReadyStatus::Degraded);
    assert_eq!(body.llm.error.as_deref(), Some("provider_unavailable"));
}

#[tokio::test]
async fn unconfigured_provider_rejects_new_checks() {
    let mut config = test_config();
    config.llm_base_url = None;
    config.llm_api_key = None;
    let state = AppState::new(config, Arc::new(UnavailableLlm));
    let (addr, _handle) = start_server(state).await;
    let response = post_check(
        &client(),
        addr,
        Some(claim("Вода кипит при 100 °C на уровне моря")),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: ApiErrorBody = response.json().await.unwrap();
    assert_eq!(body.error.code, ErrorCode::ProviderUnavailable);
}

#[tokio::test]
async fn replay_mode_uses_recorded_fixtures() {
    let directory = std::env::temp_dir().join(format!("vd-replay-it-{}", uuid::Uuid::new_v4()));
    let claim_text = "Вода кипит при 100 °C на уровне моря";
    let key = fixture_key(claim_text);

    let raw = serde_json::json!({
        "agent": "proponent",
        "status": "done",
        "message": "Оптимист",
        "arguments": [{"id":"a1","stance":"for","claim":"c","evidence":"e","source_ids":[],"strength":"moderate"}],
        "sources": [],
        "confidence": 0.5
    });
    record_fixture(
        &directory,
        &key,
        shared::AgentRole::Proponent,
        &raw.to_string(),
    )
    .await
    .unwrap();
    let raw = serde_json::json!({
        "agent": "skeptic",
        "status": "done",
        "message": "Скептик",
        "arguments": [{"id":"a2","stance":"against","claim":"c","evidence":"e","source_ids":[],"strength":"weak"}],
        "sources": [],
        "confidence": 0.4
    });
    record_fixture(
        &directory,
        &key,
        shared::AgentRole::Skeptic,
        &raw.to_string(),
    )
    .await
    .unwrap();
    let raw = serde_json::json!({
        "verdict": "refuted",
        "confidence": 0.7,
        "explanation": "Записанный вердикт",
        "is_falsifiable": true,
        "for": ["a1"],
        "against": ["a2"],
        "sources": [],
        "search_used": false
    });
    record_fixture(&directory, &key, shared::AgentRole::Judge, &raw.to_string())
        .await
        .unwrap();

    let mut config = test_config();
    config.demo_mode = shared::DemoMode::Replay;
    config.fixtures_dir = directory.clone();
    let state = AppState::new(config, Arc::new(ReplayLlm::new(directory.clone())));
    let (addr, _handle) = start_server(state).await;

    let ready: shared::ReadyResponse = client()
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(ready.demo_mode);
    assert_eq!(ready.status, shared::ReadyStatus::Ready);

    let missing = post_check(
        &client(),
        addr,
        Some(claim("Совсем другое утверждение")),
        None,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);

    let response = post_check(&client(), addr, Some(claim(claim_text)), None).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let created: CreateCheckResponse = response.json().await.unwrap();
    let snapshot = wait_terminal(
        &client(),
        addr,
        &created.check_id.to_string(),
        Duration::from_secs(10),
    )
    .await;
    assert_eq!(snapshot.status, CheckStatus::Done);
    assert!(snapshot.demo);
    let verdict = snapshot.verdict.expect("verdict");
    assert_eq!(verdict.verdict, VerdictKind::MostlyRefuted);

    let _ = tokio::fs::remove_dir_all(&directory).await;
}

fn fixture_key(claim: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(claim.as_bytes());
    hasher.update([0u8]);
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
