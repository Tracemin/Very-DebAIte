use std::collections::HashMap;

use chrono::{DateTime, Utc};
use leptos::*;
use shared::{
    AgentResult, AgentRole, AgentState, AgentStatus, Argument, CheckSnapshot, CheckStartedData,
    CheckStatus, ErrorInfo, WsEvent,
};

use crate::text;

pub const MAX_LOG_ENTRIES: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Connecting,
    Running,
    Done,
    Error,
}

impl Phase {
    pub fn is_busy(self) -> bool {
        matches!(self, Phase::Connecting | Phase::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    Idle,
    Connecting,
    Open,
    Reconnecting(u32),
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
    Delta,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub id: u64,
    pub ts: String,
    pub level: LogLevel,
    pub agent: Option<AgentRole>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct UiError {
    pub code: Option<String>,
    pub message: String,
    pub retryable: bool,
}

impl UiError {
    pub fn new(code: Option<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LastSubmission {
    pub claim: String,
    pub file: Option<web_sys::File>,
}

/// Reactive application state shared through Leptos context.
///
/// `RwSignal` handles are `Copy`, so the whole store is `Copy` and can be moved
/// into event handlers and async tasks without cloning large payloads.
#[derive(Clone, Copy)]
pub struct AppStore {
    pub phase: RwSignal<Phase>,
    pub check_id: RwSignal<Option<String>>,
    pub claim: RwSignal<String>,
    pub file: RwSignal<Option<web_sys::File>>,
    pub file_name: RwSignal<Option<String>>,
    pub snapshot: RwSignal<Option<CheckSnapshot>>,
    pub logs: RwSignal<Vec<LogEntry>>,
    pub error: RwSignal<Option<UiError>>,
    pub ws_status: RwSignal<WsStatus>,
    pub last_seq: RwSignal<u64>,
    pub ws_generation: RwSignal<u64>,
    pub last_submission: RwSignal<Option<LastSubmission>>,
    next_log_id: RwSignal<u64>,
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AppStore {
    pub fn new() -> Self {
        Self {
            phase: RwSignal::new(Phase::Idle),
            check_id: RwSignal::new(None),
            claim: RwSignal::new(String::new()),
            file: RwSignal::new(None),
            file_name: RwSignal::new(None),
            snapshot: RwSignal::new(None),
            logs: RwSignal::new(Vec::new()),
            error: RwSignal::new(None),
            ws_status: RwSignal::new(WsStatus::Idle),
            last_seq: RwSignal::new(0),
            ws_generation: RwSignal::new(0),
            last_submission: RwSignal::new(None),
            next_log_id: RwSignal::new(1),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.phase.get().is_busy()
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.phase.get_untracked(), Phase::Done | Phase::Error)
    }

    /// Invalidate the current WebSocket task. Any in-flight reconnect loop will
    /// notice the generation mismatch and stop.
    pub fn cancel_ws(&self) {
        self.ws_generation.update(|generation| *generation += 1);
        self.ws_status.set(WsStatus::Closed);
    }

    /// Clear transient state before starting a new check. Keeps the input and
    /// the last submission so the user can retry on a transport error.
    pub fn begin_check(&self) {
        self.cancel_ws();
        self.check_id.set(None);
        self.snapshot.set(None);
        self.logs.set(Vec::new());
        self.error.set(None);
        self.last_seq.set(0);
        self.phase.set(Phase::Connecting);
        self.ws_status.set(WsStatus::Connecting);
    }

    pub fn set_http_error(&self, code: Option<String>, message: String, retryable: bool) {
        self.phase.set(Phase::Error);
        self.error.set(Some(UiError::new(code, message, retryable)));
        self.ws_status.set(WsStatus::Closed);
    }

    pub fn log(&self, level: LogLevel, agent: Option<AgentRole>, message: impl Into<String>) {
        let id = self.next_log_id.get_untracked();
        self.next_log_id.set(id.saturating_add(1));

        let entry = LogEntry {
            id,
            ts: now_hms(),
            level,
            agent,
            message: message.into(),
        };

        self.logs.update(|logs| {
            logs.push(entry);
            if logs.len() > MAX_LOG_ENTRIES {
                let excess = logs.len() - MAX_LOG_ENTRIES;
                logs.drain(0..excess);
            }
        });
    }

    pub fn agent_state(&self, role: AgentRole) -> AgentState {
        self.snapshot
            .get()
            .as_ref()
            .map(|snapshot| snapshot.agents.get(role).clone())
            .unwrap_or_else(|| AgentState::queued(role))
    }

    pub fn all_arguments(&self) -> Vec<Argument> {
        let Some(snapshot) = self.snapshot.get() else {
            return Vec::new();
        };

        let mut arguments = Vec::new();
        for role in AgentRole::ALL {
            if let Some(result) = snapshot.agents.get(role).result.as_ref() {
                arguments.extend(result.arguments.iter().cloned());
            }
        }
        arguments
    }

    fn with_agent<F>(&self, role: AgentRole, update: F)
    where
        F: FnOnce(&mut AgentState),
    {
        self.snapshot.update(|slot| {
            if let Some(snapshot) = slot {
                update(snapshot.agents.get_mut(role));
            }
        });
    }

    /// Replace the local state with the server snapshot. Called for the first WS
    /// message and after a reconnect, so it must be idempotent.
    pub fn apply_snapshot(&self, snapshot: CheckSnapshot) {
        let snapshot_id = snapshot.check_id.to_string();
        let current_id = self.check_id.get_untracked();
        if let Some(current_id) = current_id {
            if current_id != snapshot_id {
                return;
            }
        } else {
            self.check_id.set(Some(snapshot_id));
        }

        self.last_seq.set(snapshot.last_seq);
        if !snapshot.claim.is_empty() {
            self.claim.set(snapshot.claim.clone());
        }
        self.phase.set(match snapshot.status {
            CheckStatus::Queued | CheckStatus::Running => Phase::Running,
            CheckStatus::Done => Phase::Done,
            CheckStatus::Error | CheckStatus::Cancelled => Phase::Error,
        });

        match snapshot.status {
            CheckStatus::Error => {
                if self.error.get_untracked().is_none() {
                    self.error.set(Some(UiError::new(
                        Some("internal".to_string()),
                        "Проверка завершилась с ошибкой. Подробности — в карточках агентов.",
                        false,
                    )));
                }
            }
            CheckStatus::Cancelled => {
                self.error.set(Some(UiError::new(
                    Some("cancelled".to_string()),
                    text::error_message("cancelled", "Проверка отменена."),
                    false,
                )));
            }
            _ => {
                self.error.set(None);
            }
        }

        let agent_summary = AgentRole::ALL
            .iter()
            .map(|role| {
                let state = snapshot.agents.get(*role);
                format!(
                    "{}: {}",
                    text::agent_short(*role),
                    text::status_label(state.status)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        self.log(
            LogLevel::Info,
            None,
            format!(
                "Снимок состояния получен (seq {}, статус «{}»; {agent_summary})",
                snapshot.last_seq,
                text::check_status_label(snapshot.status)
            ),
        );

        self.snapshot.set(Some(snapshot));
    }

    /// Apply one non-snapshot event. Returns `true` if the event is terminal.
    pub fn apply_event(
        &self,
        seq: u64,
        ts: &DateTime<Utc>,
        agent: Option<AgentRole>,
        event: WsEvent,
    ) -> bool {
        self.last_seq.set(seq);

        match event {
            WsEvent::Snapshot(data) => {
                // Snapshots are normally handled before this method is called.
                self.apply_snapshot(*data);
                false
            }
            WsEvent::CheckStarted(data) => {
                self.apply_check_started(&data);
                false
            }
            WsEvent::AgentStarted(data) => {
                if let Some(role) = agent {
                    self.with_agent(role, |state| {
                        state.status = AgentStatus::Running;
                        if state.started_at.is_none() {
                            state.started_at = Some(*ts);
                        }
                    });
                    self.log(
                        LogLevel::Info,
                        Some(role),
                        format!(
                            "Агент «{}» начал работу{}",
                            text::agent_label(role),
                            if data.model.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", data.model)
                            }
                        ),
                    );
                }
                false
            }
            WsEvent::AgentDelta(data) => {
                if let Some(role) = agent {
                    if !data.text.is_empty() {
                        self.with_agent(role, |state| state.text.push_str(&data.text));
                        self.log(LogLevel::Delta, Some(role), text::truncate(&data.text, 140));
                    }
                }
                false
            }
            WsEvent::AgentCompleted(data) => {
                let result = data.result;
                let role = agent.unwrap_or(result.agent);
                self.apply_agent_result(role, ts, result);
                false
            }
            WsEvent::AgentError(data) => {
                if let Some(role) = agent {
                    self.apply_agent_error(role, ts, data);
                }
                false
            }
            WsEvent::VerdictReady(data) => {
                let verdict = data.verdict;
                self.snapshot.update(|slot| {
                    if let Some(snapshot) = slot {
                        snapshot.verdict = Some(verdict);
                    }
                });
                self.phase.set(Phase::Running);
                let label = self
                    .snapshot
                    .get_untracked()
                    .and_then(|snapshot| snapshot.verdict)
                    .map(|verdict| text::verdict_label(verdict.verdict))
                    .unwrap_or("готов");
                self.log(LogLevel::Success, None, format!("Вердикт готов: {label}"));
                false
            }
            WsEvent::CheckCompleted(data) => {
                let warnings = data.warnings;
                self.snapshot.update(|slot| {
                    if let Some(snapshot) = slot {
                        snapshot.status = CheckStatus::Done;
                        snapshot.warnings = warnings;
                    }
                });
                self.phase.set(Phase::Done);
                self.error.set(None);
                self.log(
                    LogLevel::Success,
                    None,
                    format!(
                        "Проверка завершена за {:.1} с{}",
                        data.duration_ms as f64 / 1000.0,
                        if data.degraded {
                            " (деградированный режим)"
                        } else {
                            ""
                        }
                    ),
                );
                true
            }
            WsEvent::CheckError(data) => {
                let partial = data.partial;
                let code = data.code.as_str().to_string();
                let message = text::error_message(&code, &data.message);
                self.snapshot.update(|slot| {
                    if let Some(snapshot) = slot {
                        snapshot.status = CheckStatus::Error;
                        snapshot.partial = partial;
                    }
                });
                self.phase.set(Phase::Error);
                self.error.set(Some(UiError::new(
                    Some(code.clone()),
                    message.clone(),
                    data.retryable,
                )));
                self.log(
                    LogLevel::Error,
                    None,
                    format!("Проверка завершилась ошибкой «{code}»: {message}"),
                );
                true
            }
        }
    }

    fn apply_check_started(&self, data: &CheckStartedData) {
        self.snapshot.update(|slot| {
            if let Some(snapshot) = slot {
                snapshot.status = CheckStatus::Running;
                snapshot.claim = data.claim.clone();
                snapshot.has_attachment = data.has_attachment;
                snapshot.demo = data.demo;
                snapshot.deadline_at = data.deadline_at;
            }
        });
        self.phase.set(Phase::Running);
        self.log(
            LogLevel::Info,
            None,
            format!(
                "Проверка запущена{}",
                if data.has_attachment {
                    " (с вложением)"
                } else {
                    ""
                }
            ),
        );
    }

    fn apply_agent_result(&self, role: AgentRole, ts: &DateTime<Utc>, result: AgentResult) {
        let message = result.message.clone();
        self.with_agent(role, |state| {
            state.status = AgentStatus::Done;
            state.result = Some(result);
            // The final result is authoritative; the streamed text may have
            // been coalesced or recovered from a snapshot. Keep the streamed
            // text if the provider returned an empty message.
            if !message.is_empty() {
                state.text = message;
            }
            state.finished_at = Some(*ts);
        });
        self.log(
            LogLevel::Success,
            Some(role),
            format!("Агент «{}» завершил работу", text::agent_label(role)),
        );
    }

    fn apply_agent_error(&self, role: AgentRole, ts: &DateTime<Utc>, error: ErrorInfo) {
        let message = text::error_message(error.code.as_str(), &error.message);
        self.with_agent(role, |state| {
            state.status = AgentStatus::Error;
            state.error = Some(error);
            state.finished_at = Some(*ts);
        });
        self.log(
            LogLevel::Error,
            Some(role),
            format!("Ошибка агента «{}»: {message}", text::agent_label(role)),
        );
    }

    /// Resolve argument ids from the final verdict to full argument objects.
    pub fn argument_index(&self) -> HashMap<String, Argument> {
        self.all_arguments()
            .into_iter()
            .map(|argument| (argument.id.clone(), argument))
            .collect()
    }
}

fn now_hms() -> String {
    let date = js_sys::Date::new_0();
    format!(
        "{:02}:{:02}:{:02}",
        date.get_hours(),
        date.get_minutes(),
        date.get_seconds()
    )
}
