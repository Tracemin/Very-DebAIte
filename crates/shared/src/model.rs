//! Domain model of a fact check: agents, arguments, sources and the verdict.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ErrorInfo;

pub const MAX_AGENT_MESSAGE_CHARS: usize = 8_000;
pub const MAX_VERDICT_EXPLANATION_CHARS: usize = 2_000;
pub const MAX_SOURCE_URL_CHARS: usize = 2_048;
pub const MAX_SOURCE_TITLE_CHARS: usize = 300;
pub const MAX_SOURCE_DESCRIPTION_CHARS: usize = 1_000;
pub const MAX_ARGUMENT_CLAIM_CHARS: usize = 2_000;
pub const MAX_ARGUMENT_EVIDENCE_CHARS: usize = 4_000;

pub const DISCLAIMER: &str =
    "Анализ основан на знаниях модели и загруженном материале; это не независимая проверка фактов.";

pub const CANARY: &str = "VD-CANARY-7f3a";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Proponent,
    Skeptic,
    Judge,
}

impl AgentRole {
    pub const ALL: [Self; 3] = [Self::Proponent, Self::Skeptic, Self::Judge];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proponent => "proponent",
            Self::Skeptic => "skeptic",
            Self::Judge => "judge",
        }
    }

    pub fn is_debater(self) -> bool {
        matches!(self, Self::Proponent | Self::Skeptic)
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    #[default]
    Queued,
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    For,
    Against,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Weak,
    Moderate,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    CheckedHttp,
    SnippetOnly,
    ModelOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Confirmed,
    MostlyConfirmed,
    Insufficient,
    MostlyRefuted,
    Refuted,
}

impl VerdictKind {
    pub fn downgrade(self) -> Self {
        match self {
            Self::Confirmed => Self::MostlyConfirmed,
            Self::Refuted => Self::MostlyRefuted,
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
}

impl CheckStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DemoMode {
    #[default]
    Off,
    Record,
    Replay,
}

impl DemoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Record => "record",
            Self::Replay => "replay",
        }
    }
}

impl std::fmt::Display for DemoMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DemoMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "off" => Ok(Self::Off),
            "record" => Ok(Self::Record),
            "replay" => Ok(Self::Replay),
            other => Err(format!(
                "DEMO_MODE must be one of off/record/replay, got {other}"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Argument {
    pub id: String,
    pub stance: Stance,
    pub claim: String,
    pub evidence: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub strength: Strength,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub published_at: Option<String>,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    pub agent: AgentRole,
    pub status: AgentStatus,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub arguments: Vec<Argument>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub error: Option<String>,
}

impl AgentResult {
    pub fn empty(role: AgentRole, model: impl Into<String>) -> Self {
        Self {
            agent: role,
            status: AgentStatus::Done,
            message: String::new(),
            arguments: Vec::new(),
            sources: Vec::new(),
            confidence: None,
            model: model.into(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub verdict: VerdictKind,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub explanation: String,
    pub is_falsifiable: bool,
    #[serde(rename = "for", default)]
    pub for_arguments: Vec<String>,
    #[serde(default)]
    pub against: Vec<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub search_used: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub disclaimer: String,
}

impl Verdict {
    pub fn empty() -> Self {
        Self {
            verdict: VerdictKind::Insufficient,
            confidence: None,
            explanation: String::new(),
            is_falsifiable: true,
            for_arguments: Vec::new(),
            against: Vec::new(),
            sources: Vec::new(),
            search_used: false,
            warnings: Vec::new(),
            disclaimer: DISCLAIMER.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub agent: AgentRole,
    pub status: AgentStatus,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub result: Option<AgentResult>,
    #[serde(default)]
    pub error: Option<ErrorInfo>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
}

impl AgentState {
    pub fn queued(agent: AgentRole) -> Self {
        Self {
            agent,
            status: AgentStatus::Queued,
            text: String::new(),
            result: None,
            error: None,
            started_at: None,
            finished_at: None,
        }
    }
}

/// Fixed set of the three agent cards. Serialized as named fields, as required
/// by `docs/SPEC.md` §6.4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStates {
    pub proponent: AgentState,
    pub skeptic: AgentState,
    pub judge: AgentState,
}

impl Default for AgentStates {
    fn default() -> Self {
        Self {
            proponent: AgentState::queued(AgentRole::Proponent),
            skeptic: AgentState::queued(AgentRole::Skeptic),
            judge: AgentState::queued(AgentRole::Judge),
        }
    }
}

impl AgentStates {
    pub fn get(&self, role: AgentRole) -> &AgentState {
        match role {
            AgentRole::Proponent => &self.proponent,
            AgentRole::Skeptic => &self.skeptic,
            AgentRole::Judge => &self.judge,
        }
    }

    pub fn get_mut(&mut self, role: AgentRole) -> &mut AgentState {
        match role {
            AgentRole::Proponent => &mut self.proponent,
            AgentRole::Skeptic => &mut self.skeptic,
            AgentRole::Judge => &mut self.judge,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckSnapshot {
    pub check_id: Uuid,
    pub status: CheckStatus,
    pub claim: String,
    pub has_attachment: bool,
    pub demo: bool,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub last_seq: u64,
    pub agents: AgentStates,
    pub verdict: Option<Verdict>,
    pub warnings: Vec<String>,
    pub partial: bool,
}

impl CheckSnapshot {
    pub fn new(
        check_id: Uuid,
        claim: String,
        has_attachment: bool,
        demo: bool,
        created_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            check_id,
            status: CheckStatus::Queued,
            claim,
            has_attachment,
            demo,
            created_at,
            deadline_at,
            last_seq: 0,
            agents: AgentStates::default(),
            verdict: None,
            warnings,
            partial: false,
        }
    }
}

/// Warning codes used by the server. They are plain strings in the wire
/// contract, because warnings may be added additively.
pub mod warnings {
    pub const NOT_FALSIFIABLE: &str = "not_falsifiable";
    pub const NO_VERIFIED_SOURCES: &str = "no_verified_sources";
    pub const NO_AGENT_RESULTS: &str = "no_agent_results";
    pub const INVALID_SOURCES_DROPPED: &str = "invalid_sources_dropped";
    pub const DOCUMENT_TRUNCATED: &str = "document_truncated";
}
