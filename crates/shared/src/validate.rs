//! Validation and post-processing shared by the server and, if needed, by the UI.
//!
//! The model output is never "repaired" with regular expressions: it is either
//! valid JSON matching the contract, or the agent fails with
//! `bad_model_output`. Sanitization below only removes values that are allowed
//! to be dropped by the spec (invalid sources, dangling argument references)
//! and records a warning.

use serde_json::{json, Value};
use url::Url;

use crate::model::{
    warnings, AgentResult, AgentRole, AgentStatus, Source, Verdict, VerdictKind, Verification,
    MAX_AGENT_MESSAGE_CHARS, MAX_ARGUMENT_CLAIM_CHARS, MAX_ARGUMENT_EVIDENCE_CHARS,
    MAX_SOURCE_DESCRIPTION_CHARS, MAX_SOURCE_TITLE_CHARS, MAX_SOURCE_URL_CHARS,
    MAX_VERDICT_EXPLANATION_CHARS,
};

/// Remove the prompt canary from any model-controlled text.
pub fn redact_canary(value: &str) -> String {
    value.replace(crate::model::CANARY, "[redacted]")
}

/// Truncate a string to at most `max` characters without splitting a code point.
pub fn truncate_chars(value: &str, max: usize) -> (String, bool) {
    if value.chars().count() <= max {
        return (value.to_owned(), false);
    }
    (value.chars().take(max).collect(), true)
}

/// Validate an agent result and sanitize fields that may be dropped.
///
/// Returns warnings to add to the check. A hard schema violation (empty
/// message, empty arguments for a debater, invalid confidence) is an `Err` and
/// must become `bad_model_output`.
pub fn sanitize_agent_result(
    result: &mut AgentResult,
    role: AgentRole,
) -> Result<Vec<String>, String> {
    if !matches!(result.status, AgentStatus::Done) {
        return Err("status must be \"done\"".to_owned());
    }
    if result.agent != role {
        return Err(format!(
            "agent must be \"{}\", got \"{}\"",
            role.as_str(),
            result.agent.as_str()
        ));
    }

    let (message, _) = truncate_chars(&result.message, MAX_AGENT_MESSAGE_CHARS);
    result.message = redact_canary(&message);
    if result.message.trim().is_empty() {
        return Err("message must not be empty".to_owned());
    }

    if let Some(confidence) = result.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            return Err("confidence must be between 0 and 1".to_owned());
        }
    }

    let mut warnings = Vec::new();
    if sanitize_sources(&mut result.sources) {
        warnings.push(warnings::INVALID_SOURCES_DROPPED.to_owned());
    }

    let valid_source_ids: std::collections::HashSet<&str> = result
        .sources
        .iter()
        .map(|source| source.id.as_str())
        .collect();

    let mut seen_argument_ids = std::collections::HashSet::new();
    result.arguments.retain_mut(|argument| {
        if argument.id.trim().is_empty() {
            return false;
        }
        if !seen_argument_ids.insert(argument.id.clone()) {
            return false;
        }
        argument
            .source_ids
            .retain(|id| valid_source_ids.contains(id.as_str()));
        argument.source_ids.sort();
        argument.source_ids.dedup();
        argument.claim =
            redact_canary(&truncate_chars(&argument.claim, MAX_ARGUMENT_CLAIM_CHARS).0);
        argument.evidence =
            redact_canary(&truncate_chars(&argument.evidence, MAX_ARGUMENT_EVIDENCE_CHARS).0);
        !argument.claim.trim().is_empty()
    });

    if role.is_debater() && result.arguments.is_empty() {
        return Err("arguments must not be empty".to_owned());
    }

    Ok(warnings)
}

/// Drop sources that violate the URL rules, deduplicate ids and truncate long
/// text fields. Returns `true` when at least one source was dropped as invalid.
pub fn sanitize_sources(sources: &mut Vec<Source>) -> bool {
    let mut dropped_invalid = false;
    let mut seen = std::collections::HashSet::new();

    sources.retain_mut(|source| {
        if source.id.trim().is_empty() || !seen.insert(source.id.clone()) {
            dropped_invalid = true;
            return false;
        }
        if !is_valid_source_url(&source.url) {
            dropped_invalid = true;
            return false;
        }
        source.url = source.url.trim().to_owned();
        source.title = redact_canary(&truncate_chars(&source.title, MAX_SOURCE_TITLE_CHARS).0);
        source.description =
            redact_canary(&truncate_chars(&source.description, MAX_SOURCE_DESCRIPTION_CHARS).0);
        if let Some(published_at) = &source.published_at {
            if !is_valid_date(published_at) {
                source.published_at = None;
            }
        }
        true
    });

    dropped_invalid
}

pub fn is_valid_source_url(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_SOURCE_URL_CHARS {
        return false;
    }
    match Url::parse(value) {
        Ok(url) => {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| !host.is_empty())
        }
        Err(_) => false,
    }
}

fn is_valid_date(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// Apply the server-side verdict rules from `docs/SPEC.md` §2.2.
///
/// `debaters` contains both sides in any order; `None` and results with
/// `status = error` are treated as failed.
pub fn apply_verdict_post_validation(
    verdict: &mut Verdict,
    debaters: &[Option<&AgentResult>],
) -> Vec<String> {
    verdict.search_used = false;
    verdict.disclaimer = crate::model::DISCLAIMER.to_owned();
    verdict.explanation =
        redact_canary(&truncate_chars(&verdict.explanation, MAX_VERDICT_EXPLANATION_CHARS).0);

    if let Some(confidence) = verdict.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            verdict.confidence = None;
        }
    }

    let mut warnings = Vec::new();
    if sanitize_sources(&mut verdict.sources) {
        warnings.push(warnings::INVALID_SOURCES_DROPPED.to_owned());
    }

    let argument_ids: std::collections::HashSet<&str> = debaters
        .iter()
        .flatten()
        .flat_map(|result| result.arguments.iter())
        .map(|argument| argument.id.as_str())
        .collect();

    verdict
        .for_arguments
        .retain(|id| argument_ids.contains(id.as_str()));
    verdict
        .against
        .retain(|id| argument_ids.contains(id.as_str()));
    verdict.for_arguments.sort();
    verdict.for_arguments.dedup();
    verdict.against.sort();
    verdict.against.dedup();

    let both_debaters_failed = !debaters.is_empty()
        && debaters.iter().all(|result| match result {
            Some(result) => matches!(result.status, AgentStatus::Error),
            None => true,
        });

    if both_debaters_failed {
        verdict.verdict = VerdictKind::Insufficient;
        warnings.push(warnings::NO_AGENT_RESULTS.to_owned());
    }

    if !verdict.is_falsifiable {
        verdict.verdict = VerdictKind::Insufficient;
        warnings.push(warnings::NOT_FALSIFIABLE.to_owned());
    }

    let has_verified_source = verdict
        .sources
        .iter()
        .any(|source| matches!(source.verification, Verification::CheckedHttp));
    if !has_verified_source {
        verdict.verdict = verdict.verdict.downgrade();
        warnings.push(warnings::NO_VERIFIED_SOURCES.to_owned());
    }

    if verdict.explanation.trim().is_empty() {
        verdict.explanation = "Недостаточно надёжных данных для категоричного вывода.".to_owned();
    }

    dedupe_warnings(&mut warnings);
    verdict.warnings = warnings.clone();
    warnings
}

pub fn dedupe_warnings(warnings: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    warnings.retain(|warning| seen.insert(warning.clone()));
}

/// JSON schema sent in `response_format` and embedded in the prompt.
pub fn agent_result_schema(role: AgentRole) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["agent", "status", "message", "arguments", "sources", "confidence"],
        "properties": {
            "agent": { "type": "string", "enum": [role.as_str()] },
            "status": { "type": "string", "enum": ["done"] },
            "message": { "type": "string", "maxLength": MAX_AGENT_MESSAGE_CHARS },
            "arguments": {
                "type": "array",
                "minItems": if role.is_debater() { 1 } else { 0 },
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "stance", "claim", "evidence", "source_ids", "strength"],
                    "properties": {
                        "id": { "type": "string" },
                        "stance": { "type": "string", "enum": ["for", "against", "neutral"] },
                        "claim": { "type": "string" },
                        "evidence": { "type": "string" },
                        "source_ids": { "type": "array", "items": { "type": "string" } },
                        "strength": { "type": "string", "enum": ["weak", "moderate", "strong"] }
                    }
                }
            },
            "sources": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "url", "title", "description", "published_at", "verification"],
                    "properties": {
                        "id": { "type": "string" },
                        "url": { "type": "string", "maxLength": MAX_SOURCE_URL_CHARS },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "published_at": { "type": ["string", "null"] },
                        "verification": {
                            "type": "string",
                            "enum": ["checked_http", "snippet_only", "model_only"]
                        }
                    }
                }
            },
            "confidence": { "type": ["number", "null"], "minimum": 0, "maximum": 1 }
        }
    })
}

pub fn verdict_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "verdict", "confidence", "explanation", "is_falsifiable",
            "for", "against", "sources", "search_used"
        ],
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["confirmed", "mostly_confirmed", "insufficient", "mostly_refuted", "refuted"]
            },
            "confidence": { "type": ["number", "null"], "minimum": 0, "maximum": 1 },
            "explanation": { "type": "string", "maxLength": MAX_VERDICT_EXPLANATION_CHARS },
            "is_falsifiable": { "type": "boolean" },
            "for": { "type": "array", "items": { "type": "string" } },
            "against": { "type": "array", "items": { "type": "string" } },
            "sources": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "url", "title", "description", "published_at", "verification"],
                    "properties": {
                        "id": { "type": "string" },
                        "url": { "type": "string", "maxLength": MAX_SOURCE_URL_CHARS },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "published_at": { "type": ["string", "null"] },
                        "verification": {
                            "type": "string",
                            "enum": ["checked_http", "snippet_only", "model_only"]
                        }
                    }
                }
            },
            "search_used": { "type": "boolean" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Argument, Stance, Strength};

    fn source(id: &str, url: &str, verification: Verification) -> Source {
        Source {
            id: id.to_owned(),
            url: url.to_owned(),
            title: "title".to_owned(),
            description: "description".to_owned(),
            published_at: None,
            verification,
        }
    }

    fn debater_result(role: AgentRole, argument_id: &str) -> AgentResult {
        AgentResult {
            agent: role,
            status: AgentStatus::Done,
            message: "message".to_owned(),
            arguments: vec![Argument {
                id: argument_id.to_owned(),
                stance: Stance::For,
                claim: "claim".to_owned(),
                evidence: "evidence".to_owned(),
                source_ids: Vec::new(),
                strength: Strength::Moderate,
            }],
            sources: Vec::new(),
            confidence: Some(0.5),
            model: "test/model".to_owned(),
            error: None,
        }
    }

    #[test]
    fn truncate_keeps_unicode_boundaries() {
        let (value, truncated) = truncate_chars("привет", 3);
        assert_eq!(value, "при");
        assert!(truncated);
        assert!(!truncate_chars("ok", 3).1);
    }

    #[test]
    fn invalid_sources_are_dropped_with_warning() {
        let mut sources = vec![
            source("s1", "https://example.org/report", Verification::ModelOnly),
            source("s2", "javascript:alert(1)", Verification::ModelOnly),
            source("s3", "ftp://example.org/file", Verification::ModelOnly),
            source(
                "s1",
                "https://duplicate.example.org",
                Verification::ModelOnly,
            ),
        ];
        assert!(sanitize_sources(&mut sources));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "s1");
    }

    #[test]
    fn no_verified_sources_downgrades_confirmed_and_refuted() {
        let mut verdict = Verdict {
            verdict: VerdictKind::Confirmed,
            explanation: "ok".to_owned(),
            is_falsifiable: true,
            sources: vec![source("s1", "https://example.org", Verification::ModelOnly)],
            ..Verdict::empty()
        };
        let warnings = apply_verdict_post_validation(&mut verdict, &[]);
        assert_eq!(verdict.verdict, VerdictKind::MostlyConfirmed);
        assert!(warnings.contains(&warnings::NO_VERIFIED_SOURCES.to_owned()));

        let mut verdict = Verdict {
            verdict: VerdictKind::Refuted,
            explanation: "ok".to_owned(),
            is_falsifiable: true,
            ..Verdict::empty()
        };
        apply_verdict_post_validation(&mut verdict, &[]);
        assert_eq!(verdict.verdict, VerdictKind::MostlyRefuted);
    }

    #[test]
    fn checked_http_keeps_confirmed() {
        let mut verdict = Verdict {
            verdict: VerdictKind::Confirmed,
            explanation: "ok".to_owned(),
            is_falsifiable: true,
            sources: vec![source(
                "s1",
                "https://example.org",
                Verification::CheckedHttp,
            )],
            ..Verdict::empty()
        };
        let warnings = apply_verdict_post_validation(&mut verdict, &[]);
        assert_eq!(verdict.verdict, VerdictKind::Confirmed);
        assert!(!warnings.contains(&warnings::NO_VERIFIED_SOURCES.to_owned()));
    }

    #[test]
    fn not_falsifiable_forces_insufficient() {
        let mut verdict = Verdict {
            verdict: VerdictKind::Confirmed,
            explanation: "ok".to_owned(),
            is_falsifiable: false,
            ..Verdict::empty()
        };
        let warnings = apply_verdict_post_validation(&mut verdict, &[]);
        assert_eq!(verdict.verdict, VerdictKind::Insufficient);
        assert!(warnings.contains(&warnings::NOT_FALSIFIABLE.to_owned()));
    }

    #[test]
    fn both_debaters_failed_forces_insufficient() {
        let mut verdict = Verdict {
            verdict: VerdictKind::Confirmed,
            explanation: "ok".to_owned(),
            is_falsifiable: true,
            ..Verdict::empty()
        };
        let warnings = apply_verdict_post_validation(&mut verdict, &[None, None]);
        assert_eq!(verdict.verdict, VerdictKind::Insufficient);
        assert!(warnings.contains(&warnings::NO_AGENT_RESULTS.to_owned()));
    }

    #[test]
    fn dangling_argument_ids_are_removed() {
        let proponent = debater_result(AgentRole::Proponent, "a1");
        let mut verdict = Verdict {
            verdict: VerdictKind::Insufficient,
            explanation: "ok".to_owned(),
            is_falsifiable: true,
            for_arguments: vec!["a1".to_owned(), "missing".to_owned()],
            against: vec!["also-missing".to_owned()],
            ..Verdict::empty()
        };
        apply_verdict_post_validation(&mut verdict, &[Some(&proponent)]);
        assert_eq!(verdict.for_arguments, vec!["a1"]);
        assert!(verdict.against.is_empty());
    }

    #[test]
    fn agent_result_rejects_empty_debater_arguments() {
        let mut result = debater_result(AgentRole::Proponent, "a1");
        result.arguments.clear();
        assert!(sanitize_agent_result(&mut result, AgentRole::Proponent).is_err());
    }

    #[test]
    fn agent_result_accepts_judge_without_arguments() {
        let mut result = AgentResult::empty(AgentRole::Judge, "test/model");
        result.message = "verdict".to_owned();
        assert!(sanitize_agent_result(&mut result, AgentRole::Judge).is_ok());
    }

    #[test]
    fn canary_is_redacted_from_model_text() {
        let mut result = debater_result(AgentRole::Proponent, "a1");
        result.message = format!("секрет {} конец", crate::model::CANARY);
        result.arguments[0].claim = crate::model::CANARY.to_owned();
        sanitize_agent_result(&mut result, AgentRole::Proponent).unwrap();
        assert!(!result.message.contains(crate::model::CANARY));
        assert!(!result.arguments[0].claim.contains(crate::model::CANARY));
    }
}
