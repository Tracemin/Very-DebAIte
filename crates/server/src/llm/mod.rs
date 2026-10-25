//! LLM provider abstraction and OpenAI-compatible implementation.

pub mod openai;
pub mod replay;
pub mod stream_text;

use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use serde_json::Value;
use shared::AgentRole;
use uuid::Uuid;

use crate::limits;

pub type LlmChunkStream = Pin<Box<dyn Stream<Item = Result<String, LlmError>> + Send>>;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub role: AgentRole,
    pub check_id: Uuid,
    /// Stable key of the claim + attachment, used by demo fixtures.
    pub fixture_key: String,
    pub model: String,
    pub system: String,
    pub user: String,
    pub temperature: f32,
    pub json_schema: Option<Value>,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Start a completion. The returned stream must be dropped to abort the
    /// upstream request.
    async fn complete(&self, request: LlmRequest) -> Result<LlmChunkStream, LlmError>;

    /// Cheap reachability probe for `GET /ready` (cached by the caller).
    async fn probe(&self) -> Result<(), LlmError>;

    /// Replay clients check that all three fixture files exist before a check
    /// is accepted.
    fn supports_fixture(&self, _fixture_key: &str) -> bool {
        true
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    #[error("provider timeout")]
    Timeout,
    #[error("provider rate limited")]
    RateLimited,
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("invalid provider response: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    pub fn code(&self) -> shared::ErrorCode {
        match self {
            Self::Timeout => shared::ErrorCode::ProviderTimeout,
            Self::RateLimited => shared::ErrorCode::ProviderRateLimited,
            Self::Unavailable(_) => shared::ErrorCode::ProviderUnavailable,
            Self::Protocol(_) => shared::ErrorCode::BadModelOutput,
            Self::Cancelled => shared::ErrorCode::Cancelled,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::RateLimited | Self::Unavailable(_)
        )
    }
}

/// A provider that is not configured. The server still starts so `/ready` can
/// report a useful status, but checks fail with `provider_unavailable`.
#[derive(Debug, Default)]
pub struct UnavailableLlm;

#[async_trait]
impl LlmClient for UnavailableLlm {
    async fn complete(&self, _request: LlmRequest) -> Result<LlmChunkStream, LlmError> {
        Err(LlmError::Unavailable("LLM is not configured".to_owned()))
    }

    async fn probe(&self) -> Result<(), LlmError> {
        Err(LlmError::Unavailable("LLM is not configured".to_owned()))
    }

    fn supports_fixture(&self, _fixture_key: &str) -> bool {
        false
    }
}

pub fn text_stream(text: impl Into<String>) -> LlmChunkStream {
    Box::pin(futures_util::stream::iter(vec![Ok(text.into())]))
}

/// Retry/backoff settings shared by provider implementations.
pub fn retry_delay(attempt: usize) -> std::time::Duration {
    let _ = attempt;
    limits::LLM_RETRY_BACKOFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_errors_map_to_expected_codes() {
        assert_eq!(LlmError::Timeout.code(), shared::ErrorCode::ProviderTimeout);
        assert_eq!(
            LlmError::RateLimited.code(),
            shared::ErrorCode::ProviderRateLimited
        );
        assert_eq!(
            LlmError::Unavailable("x".into()).code(),
            shared::ErrorCode::ProviderUnavailable
        );
        assert_eq!(
            LlmError::Protocol("x".into()).code(),
            shared::ErrorCode::BadModelOutput
        );
    }

    #[tokio::test]
    async fn unavailable_client_is_not_configured() {
        let client = UnavailableLlm;
        assert!(client.probe().await.is_err());
        assert!(!client.supports_fixture("key"));
    }
}
