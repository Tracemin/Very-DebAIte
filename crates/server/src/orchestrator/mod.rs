//! Check orchestration: parallel debaters, judge, cancellation and deadlines.

pub mod agent;
pub mod sink;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::FutureExt;
use shared::{
    apply_verdict_post_validation, AgentResult, AgentRole, CheckSnapshot, CheckStatus, ErrorCode,
    ErrorInfo, Verdict, WsEnvelope,
};
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::limits;
use crate::llm::LlmClient;
use crate::rate_limit::RatePermit;

use sink::{EventSink, InternalEvent};

pub struct CheckRecord {
    pub id: uuid::Uuid,
    pub snapshot: RwLock<CheckSnapshot>,
    pub events: broadcast::Sender<WsEnvelope>,
    pub cancel: CancellationToken,
    pub attachment_text: Option<String>,
    pub fixture_key: String,
    pub created_at: Instant,
    pub deadline: tokio::time::Instant,
    finished: AtomicBool,
    terminal_at: Mutex<Option<Instant>>,
}

impl CheckRecord {
    pub fn new(
        snapshot: CheckSnapshot,
        attachment_text: Option<String>,
        fixture_key: String,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(limits::WS_CHANNEL_CAPACITY);
        Arc::new(Self {
            id: snapshot.check_id,
            snapshot: RwLock::new(snapshot),
            events,
            cancel: CancellationToken::new(),
            attachment_text,
            fixture_key,
            created_at: Instant::now(),
            deadline: tokio::time::Instant::now() + limits::CHECK_TIMEOUT,
            finished: AtomicBool::new(false),
            terminal_at: Mutex::new(None),
        })
    }

    pub fn mark_finished(&self) {
        self.finished.store(true, Ordering::SeqCst);
        if let Ok(mut terminal_at) = self.terminal_at.lock() {
            terminal_at.get_or_insert_with(Instant::now);
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub fn terminal_at(&self) -> Option<Instant> {
        self.terminal_at.lock().ok().and_then(|value| *value)
    }

    /// Returns false when the check has already finished.
    pub fn request_cancel(&self) -> bool {
        if self.is_finished() {
            return false;
        }
        self.cancel.cancel();
        true
    }

    pub async fn snapshot(&self) -> CheckSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn status(&self) -> CheckStatus {
        self.snapshot.read().await.status
    }
}

/// Runs one check to a terminal event. The rate-limit permit is held for the
/// whole execution and released here, including on panic.
pub async fn run_check(
    record: Arc<CheckRecord>,
    config: Arc<Config>,
    llm: Arc<dyn LlmClient>,
    permit: RatePermit,
) {
    let _permit = permit;
    let sink = EventSink::spawn(Arc::clone(&record));
    let started_at = Instant::now();

    let snapshot = record.snapshot().await;
    sink.send(InternalEvent::CheckStarted {
        claim: snapshot.claim.clone(),
        has_attachment: snapshot.has_attachment,
        demo: snapshot.demo,
        deadline_at: snapshot.deadline_at,
    });

    let fallback_sink = sink.clone();
    let fallback_record = Arc::clone(&record);
    let result = std::panic::AssertUnwindSafe(run_check_inner(
        Arc::clone(&record),
        Arc::clone(&config),
        llm,
        sink,
        snapshot,
        started_at,
    ))
    .catch_unwind()
    .await;

    if result.is_err() {
        tracing::error!(
            check_id = %fallback_record.id,
            code = ErrorCode::Internal.as_str(),
            "orchestrator panicked"
        );
        fallback_sink.send(InternalEvent::Failed {
            status: CheckStatus::Error,
            error: ErrorInfo::from_code(ErrorCode::Internal),
            partial: false,
        });
    }
}

async fn run_check_inner(
    record: Arc<CheckRecord>,
    config: Arc<Config>,
    llm: Arc<dyn LlmClient>,
    sink: EventSink,
    snapshot: CheckSnapshot,
    started_at: Instant,
) {
    let debater_model = model_or(&config.llm_model_debater, "replay/debater");
    let judge_model = model_or(&config.llm_model_judge, "replay/judge");
    let ctx = agent::AgentContext {
        record: Arc::clone(&record),
        config: Arc::clone(&config),
        llm,
        sink: sink.clone(),
        claim: snapshot.claim.clone(),
        document: record.attachment_text.clone(),
    };

    let proponent = tokio::spawn(agent::spawn_debater(
        ctx.clone(),
        AgentRole::Proponent,
        debater_model.clone(),
    ));
    let skeptic = tokio::spawn(agent::spawn_debater(
        ctx.clone(),
        AgentRole::Skeptic,
        debater_model,
    ));

    let joined = async { tokio::join!(proponent, skeptic) };
    tokio::pin!(joined);

    #[derive(Clone, Copy)]
    enum Phase {
        Done,
        Cancelled,
        TimedOut,
    }

    let mut joined_pair = None;
    let phase = tokio::select! {
        _ = record.cancel.cancelled() => Phase::Cancelled,
        _ = tokio::time::sleep_until(record.deadline) => Phase::TimedOut,
        result = &mut joined => {
            joined_pair = Some(result);
            Phase::Done
        }
    };

    if matches!(phase, Phase::Cancelled | Phase::TimedOut) {
        record.cancel.cancel();
    }

    // Let agent tasks observe cancellation and emit their final agent_error
    // before the terminal check event. They are bounded by their own timeout.
    let (proponent, skeptic) = match joined_pair {
        Some(result) => unwrap_agent_pair(result),
        None => match tokio::time::timeout(std::time::Duration::from_secs(2), joined).await {
            Ok(result) => unwrap_agent_pair(result),
            Err(_) => (None, None),
        },
    };

    let partial = proponent.is_some() || skeptic.is_some();

    match phase {
        Phase::Cancelled => {
            sink.send(InternalEvent::Failed {
                status: CheckStatus::Cancelled,
                error: ErrorInfo::from_code(ErrorCode::Cancelled),
                partial,
            });
            return;
        }
        Phase::TimedOut => {
            sink.send(InternalEvent::Failed {
                status: CheckStatus::Error,
                error: ErrorInfo::from_code(ErrorCode::CheckTimeout),
                partial,
            });
            return;
        }
        Phase::Done => {}
    }

    if record.cancel.is_cancelled() {
        sink.send(InternalEvent::Failed {
            status: CheckStatus::Cancelled,
            error: ErrorInfo::from_code(ErrorCode::Cancelled),
            partial,
        });
        return;
    }
    if tokio::time::Instant::now() >= record.deadline {
        sink.send(InternalEvent::Failed {
            status: CheckStatus::Error,
            error: ErrorInfo::from_code(ErrorCode::CheckTimeout),
            partial,
        });
        return;
    }

    let judge_future = agent::run_judge(ctx, proponent.as_ref(), skeptic.as_ref());
    tokio::pin!(judge_future);

    let judge_result = tokio::select! {
        _ = record.cancel.cancelled() => {
            sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::from_code(ErrorCode::Cancelled),
            });
            sink.send(InternalEvent::Failed {
                status: CheckStatus::Cancelled,
                error: ErrorInfo::from_code(ErrorCode::Cancelled),
                partial,
            });
            return;
        }
        _ = tokio::time::sleep_until(record.deadline) => {
            record.cancel.cancel();
            sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::from_code(ErrorCode::ProviderTimeout),
            });
            sink.send(InternalEvent::Failed {
                status: CheckStatus::Error,
                error: ErrorInfo::from_code(ErrorCode::CheckTimeout),
                partial,
            });
            return;
        }
        result = &mut judge_future => result,
    };

    let mut verdict = match judge_result {
        Some(verdict) => verdict,
        None => {
            tracing::warn!(
                check_id = %record.id,
                code = ErrorCode::JudgeFailed.as_str(),
                "judge did not produce a valid verdict"
            );
            sink.send(InternalEvent::Failed {
                status: CheckStatus::Error,
                error: ErrorInfo::from_code(ErrorCode::JudgeFailed),
                partial: true,
            });
            return;
        }
    };

    let debaters = [proponent.as_ref(), skeptic.as_ref()];
    let warnings = apply_verdict_post_validation(&mut verdict, &debaters);

    let judge_result = judge_agent_result(&verdict, &judge_model);
    sink.send(InternalEvent::Verdict {
        result: judge_result,
        verdict: verdict.clone(),
        warnings: warnings.clone(),
    });

    let degraded = proponent.is_none() || skeptic.is_none();
    let duration_ms = started_at.elapsed().as_millis() as u64;
    sink.send(InternalEvent::Completed {
        duration_ms,
        degraded,
    });

    tracing::info!(
        check_id = %record.id,
        duration_ms,
        verdict = ?verdict.verdict,
        degraded,
        "check completed"
    );
}

fn unwrap_agent_pair(
    result: (
        Result<Option<AgentResult>, tokio::task::JoinError>,
        Result<Option<AgentResult>, tokio::task::JoinError>,
    ),
) -> (Option<AgentResult>, Option<AgentResult>) {
    (result.0.ok().flatten(), result.1.ok().flatten())
}

fn judge_agent_result(verdict: &Verdict, model: &str) -> AgentResult {
    let mut result = AgentResult::empty(AgentRole::Judge, model);
    result.message = verdict.explanation.clone();
    result.sources = verdict.sources.clone();
    result.confidence = verdict.confidence;
    result
}

fn model_or(configured: &str, fallback: &str) -> String {
    if configured.trim().is_empty() {
        fallback.to_owned()
    } else {
        configured.to_owned()
    }
}
