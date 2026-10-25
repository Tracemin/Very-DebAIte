//! OpenAI-compatible `/chat/completions` provider with SSE streaming.

use std::time::Duration;

use async_stream::try_stream;
use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde_json::{json, Value};
use shared::AgentRole;
use uuid::Uuid;

use crate::limits;

use super::{retry_delay, text_stream, LlmChunkStream, LlmClient, LlmError, LlmRequest};

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(limits::PROVIDER_CONNECT_TIMEOUT)
            .user_agent("Very-DebAIte/0.1")
            .build()
            .map_err(|_| LlmError::Unavailable("failed to build HTTP client".to_owned()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
        })
    }

    fn completion_url(&self) -> String {
        if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }

    fn models_url(&self) -> String {
        let base = self
            .base_url
            .strip_suffix("/chat/completions")
            .unwrap_or(&self.base_url);
        format!("{base}/models")
    }

    fn build_body(&self, request: &LlmRequest) -> Value {
        let mut body = json!({
            "model": request.model,
            "messages": [
                { "role": "system", "content": request.system },
                { "role": "user", "content": request.user }
            ],
            "temperature": request.temperature,
            "stream": true
        });

        if let Some(schema) = &request.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": format!("{}_result", request.role.as_str()),
                    "strict": true,
                    "schema": schema
                }
            });
        }

        body
    }

    async fn send_with_retry(&self, body: &mut Value) -> Result<Response, LlmError> {
        let mut attempt = 0usize;
        let mut schema_fallback_used = false;

        loop {
            let sent = self
                .client
                .post(self.completion_url())
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await;

            match sent {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();

                    // Some OpenAI-compatible providers reject `response_format`
                    // with 400. The specification allows using it only when the
                    // provider supports it, so retry once without the schema.
                    if status == StatusCode::BAD_REQUEST
                        && body.get("response_format").is_some()
                        && !schema_fallback_used
                    {
                        schema_fallback_used = true;
                        if let Some(object) = body.as_object_mut() {
                            object.remove("response_format");
                        }
                        continue;
                    }

                    if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                        && attempt < limits::LLM_RETRIES
                    {
                        let delay = retry_after(&response);
                        attempt += 1;
                        tracing::warn!(
                            status = status.as_u16(),
                            attempt,
                            "LLM provider returned a retryable status"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    if status == StatusCode::TOO_MANY_REQUESTS {
                        return Err(LlmError::RateLimited);
                    }
                    return Err(LlmError::Unavailable(format!("HTTP {}", status.as_u16())));
                }
                Err(error) => {
                    let timed_out = error.is_timeout();
                    if attempt < limits::LLM_RETRIES {
                        attempt += 1;
                        tokio::time::sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(if timed_out {
                        LlmError::Timeout
                    } else {
                        LlmError::Unavailable("provider request failed".to_owned())
                    });
                }
            }
        }
    }

    fn open_stream(&self, response: Response, check_id: Uuid, role: AgentRole) -> LlmChunkStream {
        let mut bytes = Box::pin(response.bytes_stream());

        let stream = try_stream! {
            let mut buffer: Vec<u8> = Vec::new();
            let mut event_data = String::new();
            let mut saw_token = false;

            loop {
                let next = if !saw_token {
                    tokio::time::timeout(limits::PROVIDER_FIRST_TOKEN_TIMEOUT, bytes.next())
                        .await
                        .map_err(|_| {
                            tracing::warn!(
                                %check_id,
                                agent = role.as_str(),
                                "LLM provider did not send the first token in time"
                            );
                            LlmError::Timeout
                        })?
                } else {
                    bytes.next().await
                };

                let Some(chunk) = next else { break; };
                let chunk = chunk.map_err(|_| {
                    LlmError::Unavailable("provider stream failed".to_owned())
                })?;
                buffer.extend_from_slice(&chunk);

                while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=position).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim_end_matches(['\r', '\n']);

                    if line.is_empty() {
                        if !event_data.is_empty() {
                            if event_data.trim() == "[DONE]" {
                                return;
                            }
                            if let Some(text) = parse_sse_payload(&event_data)? {
                                if !text.is_empty() {
                                    saw_token = true;
                                    yield text;
                                }
                            }
                            event_data.clear();
                        }
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data:") {
                        if !event_data.is_empty() {
                            event_data.push('\n');
                        }
                        event_data.push_str(data.trim_start());
                    }
                    // `event:`, `id:`, `retry:` and comments are ignored.
                }
            }

            if !event_data.is_empty() {
                if event_data.trim() == "[DONE]" {
                    return;
                } else if let Some(text) = parse_sse_payload(&event_data)? {
                    if !text.is_empty() {
                        yield text;
                    }
                }
            }

            Err(LlmError::Protocol(
                "provider stream ended before [DONE]".to_owned(),
            ))?;
        };

        Box::pin(stream)
    }
}

#[async_trait::async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmChunkStream, LlmError> {
        let started = std::time::Instant::now();
        let mut body = self.build_body(&request);
        let response = self.send_with_retry(&mut body).await?;

        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));

        if is_sse {
            tracing::info!(
                check_id = %request.check_id,
                agent = request.role.as_str(),
                model = %request.model,
                connect_ms = started.elapsed().as_millis() as u64,
                "LLM stream started"
            );
            Ok(self.open_stream(response, request.check_id, request.role))
        } else {
            let body = response.text().await.map_err(|_| {
                LlmError::Unavailable("failed to read provider response".to_owned())
            })?;
            let content = parse_non_stream_response(&body)?;
            tracing::info!(
                check_id = %request.check_id,
                agent = request.role.as_str(),
                model = %request.model,
                duration_ms = started.elapsed().as_millis() as u64,
                "LLM completion finished (non-streaming)"
            );
            Ok(text_stream(content))
        }
    }

    async fn probe(&self) -> Result<(), LlmError> {
        let response = self
            .client
            .get(self.models_url())
            .bearer_auth(&self.api_key)
            .timeout(limits::PROVIDER_CONNECT_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    LlmError::Timeout
                } else {
                    LlmError::Unavailable("provider probe failed".to_owned())
                }
            })?;

        match response.status() {
            status if status.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Ok(()),
            StatusCode::TOO_MANY_REQUESTS => Err(LlmError::RateLimited),
            status => Err(LlmError::Unavailable(format!("HTTP {}", status.as_u16()))),
        }
    }
}

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|value| value.min(limits::LLM_MAX_RETRY_AFTER))
        .unwrap_or(limits::LLM_RETRY_BACKOFF)
}

fn parse_sse_payload(data: &str) -> Result<Option<String>, LlmError> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|_| LlmError::Protocol("provider sent invalid SSE JSON".to_owned()))?;

    if value.get("error").is_some() {
        return Err(LlmError::Protocol(
            "provider sent an error chunk".to_owned(),
        ));
    }

    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return Ok(None);
    };

    if let Some(delta) = choice.get("delta") {
        return content_to_text(delta.get("content"));
    }
    if let Some(text) = choice.get("text").and_then(Value::as_str) {
        return Ok(Some(text.to_owned()));
    }
    if let Some(message) = choice.get("message") {
        return content_to_text(message.get("content"));
    }

    Ok(None)
}

fn parse_non_stream_response(body: &str) -> Result<String, LlmError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| LlmError::Protocol("provider sent invalid JSON".to_owned()))?;
    if value.get("error").is_some() {
        return Err(LlmError::Protocol(
            "provider returned an error object".to_owned(),
        ));
    }
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"));

    content_to_text(content)?
        .ok_or_else(|| LlmError::Protocol("provider response has no message content".to_owned()))
}

fn content_to_text(content: Option<&Value>) -> Result<Option<String>, LlmError> {
    match content {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Array(parts)) => {
            let mut text = String::new();
            for part in parts {
                if let Some(value) = part.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                }
            }
            Ok(Some(text))
        }
        Some(_) => Err(LlmError::Protocol(
            "provider message content has an unexpected type".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sse_delta_content() {
        let payload = r#"{"choices":[{"delta":{"content":"привет"},"finish_reason":null}]}"#;
        assert_eq!(
            parse_sse_payload(payload).unwrap(),
            Some("привет".to_owned())
        );
    }

    #[test]
    fn ignores_usage_only_chunks() {
        assert_eq!(
            parse_sse_payload(r#"{"choices":[],"usage":{"total_tokens":1}}"#).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_invalid_sse_json() {
        assert!(parse_sse_payload("{not json").is_err());
    }

    #[test]
    fn parses_non_stream_response() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
        assert_eq!(parse_non_stream_response(body).unwrap(), "ok");
    }

    #[test]
    fn constructs_openai_compatible_urls() {
        let client = OpenAiClient::new("https://example.org/v1/", "key").unwrap();
        assert_eq!(
            client.completion_url(),
            "https://example.org/v1/chat/completions"
        );
        assert_eq!(client.models_url(), "https://example.org/v1/models");

        let client = OpenAiClient::new("https://example.org/v1/chat/completions", "key").unwrap();
        assert_eq!(
            client.completion_url(),
            "https://example.org/v1/chat/completions"
        );
    }
}
