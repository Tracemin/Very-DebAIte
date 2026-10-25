use std::time::Duration;

use futures_util::StreamExt;
use server::llm::openai::OpenAiClient;
use server::llm::{LlmChunkStream, LlmClient, LlmError, LlmRequest};
use shared::{verdict_schema, AgentRole};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(role: AgentRole) -> LlmRequest {
    LlmRequest {
        role,
        check_id: uuid::Uuid::nil(),
        fixture_key: "test".to_owned(),
        model: "test/model".to_owned(),
        system: "system".to_owned(),
        user: "user".to_owned(),
        temperature: 0.0,
        json_schema: Some(verdict_schema()),
    }
}

async fn collect(mut stream: LlmChunkStream) -> Result<Vec<String>, LlmError> {
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item?);
    }
    Ok(chunks)
}

#[tokio::test]
async fn streams_sse_until_done() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"При\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"вет\"}}]}\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let stream = client
        .complete(request(AgentRole::Proponent))
        .await
        .unwrap();
    let chunks = collect(stream).await.unwrap();
    assert_eq!(chunks.concat(), "Привет");
}

#[tokio::test]
async fn supports_non_streaming_json_response() {
    let server = MockServer::start().await;
    let body = r#"{"choices":[{"message":{"role":"assistant","content":"целиком"}}]}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let stream = client.complete(request(AgentRole::Skeptic)).await.unwrap();
    assert_eq!(collect(stream).await.unwrap(), vec!["целиком"]);
}

#[tokio::test]
async fn maps_rate_limit_after_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0")
                .set_body_string("{}"),
        )
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let error = match client.complete(request(AgentRole::Proponent)).await {
        Ok(_) => panic!("expected the rate limited request to fail"),
        Err(error) => error,
    };
    assert!(matches!(error, LlmError::RateLimited));
}

#[tokio::test]
async fn maps_server_error_after_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(500)
                .insert_header("retry-after", "0")
                .set_body_string("{}"),
        )
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let error = match client.complete(request(AgentRole::Skeptic)).await {
        Ok(_) => panic!("expected the server error to surface"),
        Err(error) => error,
    };
    assert!(matches!(error, LlmError::Unavailable(_)));
}

#[tokio::test]
async fn rejects_invalid_sse_json() {
    let server = MockServer::start().await;
    let body = "data: {not json}\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let stream = client
        .complete(request(AgentRole::Proponent))
        .await
        .unwrap();
    let error = collect(stream).await.unwrap_err();
    assert!(matches!(error, LlmError::Protocol(_)));
}

#[tokio::test]
async fn rejects_stream_that_ends_without_done() {
    let server = MockServer::start().await;
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    let stream = client
        .complete(request(AgentRole::Proponent))
        .await
        .unwrap();
    let error = collect(stream).await.unwrap_err();
    assert!(matches!(error, LlmError::Protocol(_)));
}

#[tokio::test]
async fn probe_uses_models_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;

    let client = OpenAiClient::new(server.uri(), "key").unwrap();
    tokio::time::timeout(Duration::from_secs(5), client.probe())
        .await
        .unwrap()
        .unwrap();
}
