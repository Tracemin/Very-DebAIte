//! Single-writer event sink for one check.
//!
//! Every WebSocket event goes through this task. It assigns `seq`, updates the
//! authoritative snapshot and coalesces `agent_delta` events in a 100 ms
//! window, which guarantees ordering and a gap-free sequence.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use shared::{
    dedupe_warnings, AgentCompletedData, AgentDeltaData, AgentRole, AgentStartedData, AgentStatus,
    CheckCompletedData, CheckErrorData, CheckSnapshot, CheckStartedData, CheckStatus, ErrorInfo,
    VerdictReadyData, WsEnvelope, WsEvent,
};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::limits;

use super::CheckRecord;

#[derive(Debug)]
pub enum InternalEvent {
    CheckStarted {
        claim: String,
        has_attachment: bool,
        demo: bool,
        deadline_at: chrono::DateTime<Utc>,
    },
    AgentStarted {
        role: AgentRole,
        model: String,
    },
    AgentDelta {
        role: AgentRole,
        text: String,
    },
    AgentCompleted {
        role: AgentRole,
        result: shared::AgentResult,
        warnings: Vec<String>,
    },
    AgentError {
        role: AgentRole,
        error: ErrorInfo,
    },
    Verdict {
        result: shared::AgentResult,
        verdict: shared::Verdict,
        warnings: Vec<String>,
    },
    Warning(String),
    Completed {
        duration_ms: u64,
        degraded: bool,
    },
    Failed {
        status: CheckStatus,
        error: ErrorInfo,
        partial: bool,
    },
}

/// Cloneable handle used by the orchestrator and agent tasks.
#[derive(Debug, Clone)]
pub struct EventSink {
    tx: mpsc::UnboundedSender<InternalEvent>,
}

impl EventSink {
    pub fn spawn(record: Arc<CheckRecord>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run_sink(record, rx));
        Self { tx }
    }

    pub fn send(&self, event: InternalEvent) {
        let _ = self.tx.send(event);
    }

    pub fn delta(&self, role: AgentRole, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.send(InternalEvent::AgentDelta { role, text });
        }
    }

    pub fn warning(&self, warning: impl Into<String>) {
        self.send(InternalEvent::Warning(warning.into()));
    }
}

async fn run_sink(record: Arc<CheckRecord>, mut rx: mpsc::UnboundedReceiver<InternalEvent>) {
    let mut seq = record.snapshot.read().await.last_seq;
    let mut pending: HashMap<AgentRole, String> = HashMap::new();
    let mut ticker = tokio::time::interval(limits::DELTA_COALESCE_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // the first tick fires immediately

    loop {
        let maybe_event = tokio::select! {
            event = rx.recv() => event,
            _ = ticker.tick() => {
                for role in AgentRole::ALL {
                    flush_delta(&record, &mut seq, &mut pending, role).await;
                }
                continue;
            }
        };

        let Some(event) = maybe_event else {
            // The orchestrator dropped every sender without a terminal event.
            let is_terminal = record.snapshot.read().await.status.is_terminal();
            if !is_terminal {
                finish_failed(
                    &record,
                    &mut seq,
                    CheckStatus::Error,
                    ErrorInfo::from_code(shared::ErrorCode::Internal),
                    false,
                )
                .await;
            }
            break;
        };

        match event {
            InternalEvent::CheckStarted {
                claim,
                has_attachment,
                demo,
                deadline_at,
            } => {
                let event = WsEvent::CheckStarted(CheckStartedData {
                    claim: claim.clone(),
                    has_attachment,
                    demo,
                    deadline_at,
                });
                commit(&record, &mut seq, |snapshot| {
                    snapshot.status = CheckStatus::Running;
                    snapshot.claim = claim;
                    snapshot.has_attachment = has_attachment;
                    snapshot.demo = demo;
                    snapshot.deadline_at = deadline_at;
                })
                .await;
                send_event(&record, seq, None, event);
            }
            InternalEvent::AgentStarted { role, model } => {
                commit(&record, &mut seq, |snapshot| {
                    let state = snapshot.agents.get_mut(role);
                    state.status = AgentStatus::Running;
                    state.started_at = Some(Utc::now());
                    state.error = None;
                })
                .await;
                send_event(
                    &record,
                    seq,
                    Some(role),
                    WsEvent::AgentStarted(AgentStartedData { model }),
                );
            }
            InternalEvent::AgentDelta { role, text } => {
                pending.entry(role).or_default().push_str(&text);
            }
            InternalEvent::AgentCompleted {
                role,
                result,
                warnings,
            } => {
                flush_delta(&record, &mut seq, &mut pending, role).await;
                commit(&record, &mut seq, |snapshot| {
                    add_warnings_locked(snapshot, &warnings);
                    let state = snapshot.agents.get_mut(role);
                    state.status = AgentStatus::Done;
                    state.result = Some(result.clone());
                    state.text = result.message.clone();
                    state.finished_at = Some(Utc::now());
                })
                .await;
                send_event(
                    &record,
                    seq,
                    Some(role),
                    WsEvent::AgentCompleted(AgentCompletedData { result }),
                );
            }
            InternalEvent::AgentError { role, error } => {
                flush_delta(&record, &mut seq, &mut pending, role).await;
                commit(&record, &mut seq, |snapshot| {
                    let state = snapshot.agents.get_mut(role);
                    state.status = AgentStatus::Error;
                    state.error = Some(error.clone());
                    state.finished_at = Some(Utc::now());
                })
                .await;
                send_event(&record, seq, Some(role), WsEvent::AgentError(error));
            }
            InternalEvent::Verdict {
                result,
                verdict,
                warnings,
            } => {
                flush_delta(&record, &mut seq, &mut pending, AgentRole::Judge).await;
                commit(&record, &mut seq, |snapshot| {
                    add_warnings_locked(snapshot, &warnings);
                    add_warnings_locked(snapshot, &verdict.warnings);
                    let state = snapshot.agents.get_mut(AgentRole::Judge);
                    state.status = AgentStatus::Done;
                    state.result = Some(result.clone());
                    state.text = result.message.clone();
                    state.finished_at = Some(Utc::now());
                    snapshot.verdict = Some(verdict.clone());
                })
                .await;
                send_event(
                    &record,
                    seq,
                    Some(AgentRole::Judge),
                    WsEvent::AgentCompleted(AgentCompletedData {
                        result: result.clone(),
                    }),
                );
                // `verdict_ready` is a separate event and therefore has its
                // own sequence number; no state changes on this step.
                commit(&record, &mut seq, |_| {}).await;
                send_event(
                    &record,
                    seq,
                    None,
                    WsEvent::VerdictReady(VerdictReadyData { verdict }),
                );
            }
            InternalEvent::Warning(warning) => {
                let mut snapshot = record.snapshot.write().await;
                add_warnings_locked(&mut snapshot, &[warning]);
            }
            InternalEvent::Completed {
                duration_ms,
                degraded,
            } => {
                for role in AgentRole::ALL {
                    flush_delta(&record, &mut seq, &mut pending, role).await;
                }
                let warnings = commit(&record, &mut seq, |snapshot| {
                    snapshot.status = CheckStatus::Done;
                    snapshot.partial = false;
                    snapshot.warnings.clone()
                })
                .await;
                record.mark_finished();
                send_event(
                    &record,
                    seq,
                    None,
                    WsEvent::CheckCompleted(CheckCompletedData {
                        duration_ms,
                        degraded,
                        warnings,
                    }),
                );
                break;
            }
            InternalEvent::Failed {
                status,
                error,
                partial,
            } => {
                finish_failed(&record, &mut seq, status, error, partial).await;
                break;
            }
        }
    }
}

async fn finish_failed(
    record: &Arc<CheckRecord>,
    seq: &mut u64,
    status: CheckStatus,
    error: ErrorInfo,
    partial: bool,
) {
    commit(record, seq, |snapshot| {
        snapshot.status = status;
        snapshot.partial = partial;
        for role in AgentRole::ALL {
            let state = snapshot.agents.get_mut(role);
            if matches!(state.status, AgentStatus::Queued | AgentStatus::Running) {
                state.status = AgentStatus::Error;
                state.error = Some(error.clone());
                state.finished_at = Some(Utc::now());
            }
        }
    })
    .await;
    record.mark_finished();
    send_event(
        record,
        *seq,
        None,
        WsEvent::CheckError(CheckErrorData {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            partial,
        }),
    );
}

async fn flush_delta(
    record: &Arc<CheckRecord>,
    seq: &mut u64,
    pending: &mut HashMap<AgentRole, String>,
    role: AgentRole,
) {
    let Some(text) = pending.remove(&role).filter(|text| !text.is_empty()) else {
        return;
    };
    commit(record, seq, |snapshot| {
        snapshot.agents.get_mut(role).text.push_str(&text);
    })
    .await;
    send_event(
        record,
        *seq,
        Some(role),
        WsEvent::AgentDelta(AgentDeltaData { text }),
    );
}

/// Apply a snapshot mutation and allocate the next sequence number in one
/// critical section. This is what makes `snapshot.last_seq` exactly match the
/// state the client receives.
async fn commit<F, R>(record: &Arc<CheckRecord>, seq: &mut u64, mutate: F) -> R
where
    F: FnOnce(&mut CheckSnapshot) -> R,
{
    let mut snapshot = record.snapshot.write().await;
    *seq += 1;
    snapshot.last_seq = *seq;
    mutate(&mut snapshot)
}

fn send_event(record: &Arc<CheckRecord>, seq: u64, agent: Option<AgentRole>, event: WsEvent) {
    let _ = record
        .events
        .send(WsEnvelope::new(record.id, seq, agent, event));
}

fn add_warnings_locked(snapshot: &mut CheckSnapshot, warnings: &[String]) {
    let mut merged = snapshot.warnings.clone();
    merged.extend(warnings.iter().cloned());
    dedupe_warnings(&mut merged);
    snapshot.warnings = merged;
}
