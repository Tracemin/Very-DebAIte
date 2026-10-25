//! Debater and judge execution, streaming and structured output parsing.

use std::sync::Arc;

use futures_util::{FutureExt, StreamExt};
use shared::{
    agent_result_schema, sanitize_agent_result, truncate_chars, verdict_schema, AgentResult,
    AgentRole, AgentStatus, ErrorCode, ErrorInfo, Verdict,
};

use crate::config::Config;
use crate::limits;
use crate::llm::replay::record_fixture;
use crate::llm::stream_text::JsonStringFieldStreamer;
use crate::llm::{LlmClient, LlmError, LlmRequest};
use crate::prompts;

use super::sink::{EventSink, InternalEvent};
use super::CheckRecord;

const REPAIR_INSTRUCTION: &str = "Верни только валидный JSON по схеме из системного сообщения. \
Не добавляй markdown-ограждения, комментарии или текст до/после JSON.";

enum CompletionError {
    Llm(LlmError),
    BadOutput,
    Cancelled,
}

/// Shared inputs for one check's agents. Kept in one struct so the agent
/// functions stay readable and testable without long parameter lists.
#[derive(Clone)]
pub struct AgentContext {
    pub record: Arc<CheckRecord>,
    pub config: Arc<Config>,
    pub llm: Arc<dyn LlmClient>,
    pub sink: EventSink,
    pub claim: String,
    pub document: Option<String>,
}

pub async fn run_debater(ctx: AgentContext, role: AgentRole, model: String) -> Option<AgentResult> {
    if ctx.record.cancel.is_cancelled() {
        ctx.sink.send(InternalEvent::AgentError {
            role,
            error: ErrorInfo::from_code(ErrorCode::Cancelled),
        });
        return None;
    }

    ctx.sink.send(InternalEvent::AgentStarted {
        role,
        model: model.clone(),
    });

    let (system, user) = prompts::debater_messages(role, &ctx.claim, ctx.document.as_deref());
    let request = LlmRequest {
        role,
        check_id: ctx.record.id,
        fixture_key: ctx.record.fixture_key.clone(),
        model: model.clone(),
        system,
        user,
        temperature: 0.2,
        json_schema: Some(agent_result_schema(role)),
    };

    let outcome = tokio::time::timeout(
        limits::DEBATER_TIMEOUT,
        complete_debater(
            role,
            &ctx.record,
            &ctx.sink,
            Arc::clone(&ctx.llm),
            request,
            model.clone(),
        ),
    )
    .await;

    match outcome {
        Err(_) => {
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = role.as_str(),
                code = ErrorCode::ProviderTimeout.as_str(),
                "agent timed out"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role,
                error: ErrorInfo::from_code(ErrorCode::ProviderTimeout),
            });
            None
        }
        Ok(Ok((result, warnings, raw))) => {
            if ctx.config.demo_mode == shared::DemoMode::Record {
                if let Err(error) = record_fixture(
                    &ctx.config.fixtures_dir,
                    &ctx.record.fixture_key,
                    role,
                    &raw,
                )
                .await
                {
                    tracing::warn!(
                        check_id = %ctx.record.id,
                        agent = role.as_str(),
                        error = %error,
                        "failed to record demo fixture"
                    );
                }
            }
            ctx.sink.send(InternalEvent::AgentCompleted {
                role,
                result: result.clone(),
                warnings,
            });
            Some(result)
        }
        Ok(Err(CompletionError::Cancelled)) => {
            ctx.sink.send(InternalEvent::AgentError {
                role,
                error: ErrorInfo::from_code(ErrorCode::Cancelled),
            });
            None
        }
        Ok(Err(CompletionError::Llm(error))) => {
            let code = error.code();
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = role.as_str(),
                code = code.as_str(),
                "agent failed"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role,
                error: ErrorInfo::new(code, error.to_string()),
            });
            None
        }
        Ok(Err(CompletionError::BadOutput)) => {
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = role.as_str(),
                code = ErrorCode::BadModelOutput.as_str(),
                "agent returned invalid structured output"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role,
                error: ErrorInfo::from_code(ErrorCode::BadModelOutput),
            });
            None
        }
    }
}

async fn complete_debater(
    role: AgentRole,
    record: &Arc<CheckRecord>,
    sink: &EventSink,
    llm: Arc<dyn LlmClient>,
    request: LlmRequest,
    model: String,
) -> Result<(AgentResult, Vec<String>, String), CompletionError> {
    let stream = start_completion(&record.cancel, llm.as_ref(), request.clone()).await?;
    let (raw, _streamed) = collect_stream(
        record,
        sink,
        stream,
        role,
        JsonStringFieldStreamer::new("message"),
    )
    .await?;

    match parse_debater(role, &raw, &model) {
        Ok((result, warnings)) => Ok((result, warnings, raw)),
        Err(_) => {
            let repair = repair_request(&request, &raw);
            let repaired = collect_without_stream(record, llm.as_ref(), repair).await?;
            match parse_debater(role, &repaired, &model) {
                Ok((result, warnings)) => Ok((result, warnings, repaired)),
                Err(_) => Err(CompletionError::BadOutput),
            }
        }
    }
}

pub async fn run_judge(
    ctx: AgentContext,
    proponent: Option<&AgentResult>,
    skeptic: Option<&AgentResult>,
) -> Option<Verdict> {
    let model = ctx.config.llm_model_judge.clone();
    ctx.sink.send(InternalEvent::AgentStarted {
        role: AgentRole::Judge,
        model: model.clone(),
    });

    let (system, user) =
        prompts::judge_messages(&ctx.claim, ctx.document.as_deref(), proponent, skeptic);
    let request = LlmRequest {
        role: AgentRole::Judge,
        check_id: ctx.record.id,
        fixture_key: ctx.record.fixture_key.clone(),
        model: model.clone(),
        system,
        user,
        temperature: 0.0,
        json_schema: Some(verdict_schema()),
    };

    let outcome = tokio::time::timeout(
        limits::JUDGE_TIMEOUT,
        complete_judge(&ctx.record, &ctx.sink, Arc::clone(&ctx.llm), request),
    )
    .await;

    match outcome {
        Err(_) => {
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = AgentRole::Judge.as_str(),
                code = ErrorCode::ProviderTimeout.as_str(),
                "judge timed out"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::from_code(ErrorCode::ProviderTimeout),
            });
            None
        }
        Ok(Err(CompletionError::Cancelled)) => {
            ctx.sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::from_code(ErrorCode::Cancelled),
            });
            None
        }
        Ok(Err(CompletionError::Llm(error))) => {
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = AgentRole::Judge.as_str(),
                code = error.code().as_str(),
                "judge failed"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::new(error.code(), error.to_string()),
            });
            None
        }
        Ok(Err(CompletionError::BadOutput)) => {
            tracing::warn!(
                check_id = %ctx.record.id,
                agent = AgentRole::Judge.as_str(),
                code = ErrorCode::BadModelOutput.as_str(),
                "judge returned invalid structured output"
            );
            ctx.sink.send(InternalEvent::AgentError {
                role: AgentRole::Judge,
                error: ErrorInfo::from_code(ErrorCode::BadModelOutput),
            });
            None
        }
        Ok(Ok((verdict, raw))) => {
            if ctx.config.demo_mode == shared::DemoMode::Record {
                if let Err(error) = record_fixture(
                    &ctx.config.fixtures_dir,
                    &ctx.record.fixture_key,
                    AgentRole::Judge,
                    &raw,
                )
                .await
                {
                    tracing::warn!(
                        check_id = %ctx.record.id,
                        agent = AgentRole::Judge.as_str(),
                        error = %error,
                        "failed to record demo fixture"
                    );
                }
            }
            Some(verdict)
        }
    }
}

async fn complete_judge(
    record: &Arc<CheckRecord>,
    sink: &EventSink,
    llm: Arc<dyn LlmClient>,
    request: LlmRequest,
) -> Result<(Verdict, String), CompletionError> {
    let stream = start_completion(&record.cancel, llm.as_ref(), request.clone()).await?;
    let (raw, _streamed) = collect_stream(
        record,
        sink,
        stream,
        AgentRole::Judge,
        JsonStringFieldStreamer::new("explanation"),
    )
    .await?;

    match parse_verdict(&raw) {
        Ok(verdict) => Ok((verdict, raw)),
        Err(_) => {
            let repair = repair_request(&request, &raw);
            let repaired = collect_without_stream(record, llm.as_ref(), repair).await?;
            match parse_verdict(&repaired) {
                Ok(verdict) => Ok((verdict, repaired)),
                Err(_) => Err(CompletionError::BadOutput),
            }
        }
    }
}

async fn start_completion(
    cancel: &tokio_util::sync::CancellationToken,
    llm: &dyn LlmClient,
    request: LlmRequest,
) -> Result<crate::llm::LlmChunkStream, CompletionError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(CompletionError::Cancelled),
        result = llm.complete(request) => result.map_err(CompletionError::Llm),
    }
}

async fn collect_stream(
    record: &Arc<CheckRecord>,
    sink: &EventSink,
    mut stream: crate::llm::LlmChunkStream,
    role: AgentRole,
    mut streamer: JsonStringFieldStreamer,
) -> Result<(String, bool), CompletionError> {
    let mut raw = String::new();
    let mut streamed_any = false;

    loop {
        let item = tokio::select! {
            _ = record.cancel.cancelled() => return Err(CompletionError::Cancelled),
            item = stream.next() => item,
        };
        match item {
            Some(Ok(chunk)) => {
                raw.push_str(&chunk);
                let text = streamer.feed(&chunk);
                if !text.is_empty() {
                    streamed_any = true;
                    sink.delta(role, text);
                }
            }
            Some(Err(error)) => return Err(CompletionError::Llm(error)),
            None => break,
        }
    }

    Ok((raw, streamed_any))
}

async fn collect_without_stream(
    record: &Arc<CheckRecord>,
    llm: &dyn LlmClient,
    request: LlmRequest,
) -> Result<String, CompletionError> {
    let mut stream = start_completion(&record.cancel, llm, request).await?;
    let mut raw = String::new();
    loop {
        let item = tokio::select! {
            _ = record.cancel.cancelled() => return Err(CompletionError::Cancelled),
            item = stream.next() => item,
        };
        match item {
            Some(Ok(chunk)) => raw.push_str(&chunk),
            Some(Err(error)) => return Err(CompletionError::Llm(error)),
            None => break,
        }
    }
    Ok(raw)
}

fn repair_request(request: &LlmRequest, previous: &str) -> LlmRequest {
    let previous = truncate_chars(previous, 20_000).0;
    LlmRequest {
        role: request.role,
        check_id: request.check_id,
        fixture_key: request.fixture_key.clone(),
        model: request.model.clone(),
        system: request.system.clone(),
        user: format!(
            "{}\n\n<previous_answer>\n{}\n</previous_answer>\n\n{}",
            request.user, previous, REPAIR_INSTRUCTION
        ),
        temperature: 0.0,
        json_schema: request.json_schema.clone(),
    }
}

fn parse_debater(
    role: AgentRole,
    raw: &str,
    model: &str,
) -> Result<(AgentResult, Vec<String>), String> {
    let mut result: AgentResult =
        serde_json::from_str(raw.trim()).map_err(|error| format!("invalid JSON: {error}"))?;
    result.status = AgentStatus::Done;
    result.model = model.to_owned();
    result.error = None;
    let warnings = sanitize_agent_result(&mut result, role)?;
    Ok((result, warnings))
}

fn parse_verdict(raw: &str) -> Result<Verdict, String> {
    serde_json::from_str::<Verdict>(raw.trim()).map_err(|error| format!("invalid JSON: {error}"))
}

/// Panic-safe wrapper used by the orchestrator.
pub async fn spawn_debater(
    ctx: AgentContext,
    role: AgentRole,
    model: String,
) -> Option<AgentResult> {
    let fallback_sink = ctx.sink.clone();
    let check_id = ctx.record.id;
    let result = std::panic::AssertUnwindSafe(run_debater(ctx, role, model))
        .catch_unwind()
        .await;

    match result {
        Ok(result) => result,
        Err(_) => {
            tracing::error!(
                %check_id,
                agent = role.as_str(),
                code = ErrorCode::Internal.as_str(),
                "agent task panicked"
            );
            fallback_sink.send(InternalEvent::AgentError {
                role,
                error: ErrorInfo::from_code(ErrorCode::Internal),
            });
            None
        }
    }
}
