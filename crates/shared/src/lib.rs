//! Wire contract shared by `crates/server` and the Leptos client.
//!
//! Everything that crosses the server ↔ browser boundary lives here:
//! WebSocket envelopes, snapshots, agent results, verdicts and error codes.

pub mod error;
pub mod events;
pub mod model;
pub mod validate;

pub use error::{ApiErrorBody, ApiErrorDetail, ErrorCode, ErrorInfo};
pub use events::{
    AgentCompletedData, AgentDeltaData, AgentStartedData, CheckCompletedData, CheckErrorData,
    CheckStartedData, CreateCheckResponse, HealthResponse, LlmReady, ReadyResponse, ReadyStatus,
    VerdictReadyData, WsEnvelope, WsEvent, WS_PROTOCOL_VERSION,
};
pub use model::warnings;
pub use model::{
    AgentResult, AgentRole, AgentState, AgentStates, AgentStatus, Argument, CheckSnapshot,
    CheckStatus, DemoMode, Source, Stance, Strength, Verdict, VerdictKind, Verification, CANARY,
    DISCLAIMER,
};
pub use validate::{
    agent_result_schema, apply_verdict_post_validation, dedupe_warnings, sanitize_agent_result,
    sanitize_sources, truncate_chars, verdict_schema,
};
