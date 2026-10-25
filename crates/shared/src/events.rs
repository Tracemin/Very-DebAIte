//! WebSocket envelope and event payloads (`docs/SPEC.md` §6).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ErrorCode;
use crate::model::{AgentResult, AgentRole, CheckSnapshot, Verdict};

pub const WS_PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckStartedData {
    pub claim: String,
    pub has_attachment: bool,
    pub demo: bool,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStartedData {
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDeltaData {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCompletedData {
    pub result: AgentResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictReadyData {
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCompletedData {
    pub duration_ms: u64,
    pub degraded: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckErrorData {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum WsEvent {
    Snapshot(Box<CheckSnapshot>),
    CheckStarted(CheckStartedData),
    AgentStarted(AgentStartedData),
    AgentDelta(AgentDeltaData),
    AgentCompleted(AgentCompletedData),
    AgentError(crate::error::ErrorInfo),
    VerdictReady(VerdictReadyData),
    CheckCompleted(CheckCompletedData),
    CheckError(CheckErrorData),
}

impl WsEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Snapshot(_) => "snapshot",
            Self::CheckStarted(_) => "check_started",
            Self::AgentStarted(_) => "agent_started",
            Self::AgentDelta(_) => "agent_delta",
            Self::AgentCompleted(_) => "agent_completed",
            Self::AgentError(_) => "agent_error",
            Self::VerdictReady(_) => "verdict_ready",
            Self::CheckCompleted(_) => "check_completed",
            Self::CheckError(_) => "check_error",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::CheckCompleted(_) | Self::CheckError(_))
    }
}

/// A single message on the check WebSocket.
///
/// `type`/`data` come from the flattened [`WsEvent`], `agent` is always present
/// (null for events that are not attached to a specific agent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsEnvelope {
    pub v: u8,
    pub seq: u64,
    pub check_id: Uuid,
    pub ts: DateTime<Utc>,
    pub agent: Option<AgentRole>,
    #[serde(flatten)]
    pub event: WsEvent,
}

impl WsEnvelope {
    pub fn new(check_id: Uuid, seq: u64, agent: Option<AgentRole>, event: WsEvent) -> Self {
        Self {
            v: WS_PROTOCOL_VERSION,
            seq,
            check_id,
            ts: Utc::now(),
            agent,
            event,
        }
    }

    pub fn snapshot(check_id: Uuid, snapshot: CheckSnapshot) -> Self {
        let seq = snapshot.last_seq;
        Self::new(check_id, seq, None, WsEvent::Snapshot(Box::new(snapshot)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCheckResponse {
    pub check_id: Uuid,
    pub ws_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

impl Default for HealthResponse {
    fn default() -> Self {
        Self { status: "ok" }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyStatus {
    Ready,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmReady {
    pub configured: bool,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub status: ReadyStatus,
    /// True when the server is serving recorded demo output (`DEMO_MODE=replay`).
    pub demo_mode: bool,
    pub llm: LlmReady,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentStatus, CheckStatus};
    use chrono::TimeZone;

    fn sample_snapshot() -> CheckSnapshot {
        let created = Utc.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).unwrap();
        let mut snapshot = CheckSnapshot::new(
            Uuid::nil(),
            "Вода кипит при 100 °C".to_owned(),
            false,
            false,
            created,
            created + chrono::Duration::seconds(120),
            Vec::new(),
        );
        snapshot.status = CheckStatus::Running;
        snapshot.last_seq = 7;
        snapshot.agents.proponent.status = AgentStatus::Done;
        snapshot
    }

    #[test]
    fn envelope_has_type_data_and_agent_fields() {
        let snapshot = sample_snapshot();
        let envelope = WsEnvelope::snapshot(snapshot.check_id, snapshot);
        let json = serde_json::to_value(&envelope).unwrap();

        assert_eq!(json["v"], 1);
        assert_eq!(json["seq"], 7);
        assert_eq!(json["type"], "snapshot");
        assert!(json["agent"].is_null());
        assert_eq!(json["data"]["status"], "running");
        assert_eq!(json["data"]["last_seq"], 7);
    }

    #[test]
    fn envelope_roundtrips_with_agent() {
        let envelope = WsEnvelope::new(
            Uuid::nil(),
            3,
            Some(AgentRole::Skeptic),
            WsEvent::AgentDelta(AgentDeltaData {
                text: "фрагмент".to_owned(),
            }),
        );
        let json = serde_json::to_string(&envelope).unwrap();
        let decoded: WsEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, envelope);
        assert_eq!(decoded.agent, Some(AgentRole::Skeptic));
        assert_eq!(decoded.event.event_type(), "agent_delta");
    }

    #[test]
    fn check_error_serializes_flat_data() {
        let envelope = WsEnvelope::new(
            Uuid::nil(),
            9,
            None,
            WsEvent::CheckError(CheckErrorData {
                code: ErrorCode::JudgeFailed,
                message: "нет вердикта".to_owned(),
                retryable: false,
                partial: true,
            }),
        );
        let json = serde_json::to_value(&envelope).unwrap();

        assert_eq!(json["type"], "check_error");
        assert_eq!(json["data"]["code"], "judge_failed");
        assert_eq!(json["data"]["partial"], true);
        assert!(json["data"].get("v").is_none());
    }

    #[test]
    fn terminal_events_are_detected() {
        let completed = WsEnvelope::new(
            Uuid::nil(),
            1,
            None,
            WsEvent::CheckCompleted(CheckCompletedData {
                duration_ms: 10,
                degraded: false,
                warnings: Vec::new(),
            }),
        );
        assert!(completed.event.is_terminal());
        assert!(
            !completed.event.is_terminal() || completed.event.event_type() == "check_completed"
        );
    }
}
