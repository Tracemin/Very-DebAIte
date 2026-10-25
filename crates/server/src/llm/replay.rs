//! `DEMO_MODE=replay` provider. Reads recorded raw model outputs from
//! `fixtures/` instead of calling the network.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use shared::AgentRole;

use crate::prompts::PROMPT_VERSION;

use super::{text_stream, LlmChunkStream, LlmClient, LlmError, LlmRequest};

#[derive(Debug, Clone)]
pub struct ReplayLlm {
    directory: PathBuf,
}

impl ReplayLlm {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn path(&self, fixture_key: &str, role: AgentRole) -> PathBuf {
        fixture_path(&self.directory, fixture_key, role)
    }
}

#[async_trait]
impl LlmClient for ReplayLlm {
    async fn complete(&self, request: LlmRequest) -> Result<LlmChunkStream, LlmError> {
        let path = self.path(&request.fixture_key, request.role);
        let raw = tokio::fs::read_to_string(&path).await.map_err(|_| {
            LlmError::Unavailable(format!("no replay fixture for {}", request.role))
        })?;
        tracing::info!(
            check_id = %request.check_id,
            agent = request.role.as_str(),
            "playing recorded LLM response"
        );
        Ok(text_stream(raw))
    }

    async fn probe(&self) -> Result<(), LlmError> {
        if tokio::fs::metadata(&self.directory).await.is_ok() {
            Ok(())
        } else {
            Err(LlmError::Unavailable(
                "fixtures directory does not exist".to_owned(),
            ))
        }
    }

    fn supports_fixture(&self, fixture_key: &str) -> bool {
        AgentRole::ALL
            .iter()
            .all(|role| self.path(fixture_key, *role).is_file())
    }
}

pub fn fixture_path(directory: &Path, fixture_key: &str, role: AgentRole) -> PathBuf {
    let safe_key: String = fixture_key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(64)
        .collect();
    directory.join(format!(
        "{PROMPT_VERSION}-{safe_key}-{}.json",
        role.as_str()
    ))
}

/// Persist one raw model output. Recording failures never break a live check:
/// they are logged without the payload.
pub async fn record_fixture(
    directory: &Path,
    fixture_key: &str,
    role: AgentRole,
    raw: &str,
) -> std::io::Result<()> {
    tokio::fs::create_dir_all(directory).await?;
    let path = fixture_path(directory, fixture_key, role);
    tokio::fs::write(path, raw).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replay_reads_recorded_files() {
        let directory = std::env::temp_dir().join(format!("vd-replay-{}", uuid::Uuid::new_v4()));
        let key = "abc123";
        for role in AgentRole::ALL {
            record_fixture(&directory, key, role, &format!("{{\"role\":\"{role}\"}}"))
                .await
                .unwrap();
        }

        let client = ReplayLlm::new(&directory);
        assert!(client.supports_fixture(key));
        let request = LlmRequest {
            role: AgentRole::Proponent,
            check_id: uuid::Uuid::nil(),
            fixture_key: key.to_owned(),
            model: "replay".to_owned(),
            system: String::new(),
            user: String::new(),
            temperature: 0.0,
            json_schema: None,
        };
        let mut stream = client.complete(request.clone()).await.unwrap();
        use futures_util::StreamExt;
        let chunk = stream.next().await.unwrap().unwrap();
        assert!(chunk.contains("proponent"));

        let missing = LlmRequest {
            role: AgentRole::Judge,
            fixture_key: "missing".to_owned(),
            ..request
        };
        assert!(client.complete(missing).await.is_err());

        let _ = tokio::fs::remove_dir_all(&directory).await;
    }
}
