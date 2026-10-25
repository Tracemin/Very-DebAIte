use shared::{AgentRole, AgentStatus, CheckStatus, Stance, Strength, VerdictKind, Verification};

pub fn agent_label(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Proponent => "Оптимист",
        AgentRole::Skeptic => "Скептик",
        AgentRole::Judge => "Судья",
    }
}

pub fn agent_short(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Proponent => "оптимист",
        AgentRole::Skeptic => "скептик",
        AgentRole::Judge => "судья",
    }
}

pub fn agent_class(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Proponent => "proponent",
        AgentRole::Skeptic => "skeptic",
        AgentRole::Judge => "judge",
    }
}

pub fn agent_description(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Proponent => "Ищет аргументы и факты в пользу утверждения",
        AgentRole::Skeptic => "Ищет контраргументы, ошибки и нехватку контекста",
        AgentRole::Judge => "Сравнивает стороны и выносит итоговый вердикт",
    }
}

pub fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Queued => "Ожидание",
        AgentStatus::Running => "Работает",
        AgentStatus::Done => "Готово",
        AgentStatus::Error => "Ошибка",
    }
}

pub fn status_class(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Queued => "queued",
        AgentStatus::Running => "running",
        AgentStatus::Done => "done",
        AgentStatus::Error => "error",
    }
}

pub fn check_status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Queued => "в очереди",
        CheckStatus::Running => "выполняется",
        CheckStatus::Done => "завершена",
        CheckStatus::Error => "ошибка",
        CheckStatus::Cancelled => "отменена",
    }
}

pub fn verdict_label(verdict: VerdictKind) -> &'static str {
    match verdict {
        VerdictKind::Confirmed => "Подтверждено",
        VerdictKind::MostlyConfirmed => "Скорее подтверждено",
        VerdictKind::Insufficient => "Недостаточно данных",
        VerdictKind::MostlyRefuted => "Скорее опровергнуто",
        VerdictKind::Refuted => "Опровергнуто",
    }
}

pub fn verdict_class(verdict: VerdictKind) -> &'static str {
    match verdict {
        VerdictKind::Confirmed => "confirmed",
        VerdictKind::MostlyConfirmed => "mostly-confirmed",
        VerdictKind::Insufficient => "insufficient",
        VerdictKind::MostlyRefuted => "mostly-refuted",
        VerdictKind::Refuted => "refuted",
    }
}

pub fn stance_label(stance: Stance) -> &'static str {
    match stance {
        Stance::For => "за",
        Stance::Against => "против",
        Stance::Neutral => "нейтрально",
    }
}

pub fn stance_class(stance: Stance) -> &'static str {
    match stance {
        Stance::For => "for",
        Stance::Against => "against",
        Stance::Neutral => "neutral",
    }
}

pub fn strength_label(strength: Strength) -> &'static str {
    match strength {
        Strength::Weak => "слабая",
        Strength::Moderate => "умеренная",
        Strength::Strong => "сильная",
    }
}

pub fn strength_class(strength: Strength) -> &'static str {
    match strength {
        Strength::Weak => "weak",
        Strength::Moderate => "moderate",
        Strength::Strong => "strong",
    }
}

pub fn verification_label(verification: Verification) -> &'static str {
    match verification {
        Verification::CheckedHttp => "проверено по HTTP",
        Verification::SnippetOnly => "из сниппета",
        Verification::ModelOnly => "знание модели",
    }
}

pub fn verification_class(verification: Verification) -> &'static str {
    match verification {
        Verification::CheckedHttp => "checked-http",
        Verification::SnippetOnly => "snippet-only",
        Verification::ModelOnly => "model-only",
    }
}

pub fn warning_label(warning: &str) -> String {
    match warning {
        "no_verified_sources" => {
            "Нет источников, проверенных по HTTP: вердикт понижен на одну ступень.".to_string()
        }
        "not_falsifiable" => "Утверждение не является проверяемым фактом.".to_string(),
        "no_agent_results" => "Оба дебатера завершились ошибкой.".to_string(),
        "document_truncated" => "Текст документа был усечён по лимиту.".to_string(),
        "invalid_sources_dropped" => {
            "Часть источников отброшена из-за некорректных ссылок.".to_string()
        }
        "judge_failed" => "Судья не смог вынести вердикт.".to_string(),
        other => other.to_string(),
    }
}

pub fn error_message(code: &str, fallback: &str) -> String {
    let message = match code {
        "invalid_request" => {
            "Некорректный запрос. Проверьте утверждение и приложенный файл."
        }
        "file_too_large" => "Файл больше 5 МиБ.",
        "unsupported_file_type" => {
            "Неподдерживаемый тип файла. Принимаются TXT и текстовые PDF; OCR изображений не поддерживается."
        }
        "invalid_encoding" => "Текстовый файл должен быть в кодировке UTF-8.",
        "text_extraction_failed" => {
            "Не удалось извлечь текст из файла. Возможно, это скан; OCR в этой версии не поддерживается."
        }
        "not_found" => "Проверка не найдена или срок её хранения истёк.",
        "already_finished" => "Проверка уже завершена.",
        "rate_limited" => "Слишком много запросов. Попробуйте позже.",
        "overloaded" => "Сервер перегружен. Попробуйте ещё раз через минуту.",
        "provider_unavailable" => "LLM-провайдер недоступен. Попробуйте ещё раз.",
        "provider_timeout" => "Провайдер не ответил за отведённое время.",
        "provider_rate_limited" => "Провайдер ограничил частоту запросов.",
        "bad_model_output" => "Модель вернула ответ, не соответствующий схеме.",
        "check_timeout" => "Проверка превысила общий лимит 120 секунд.",
        "cancelled" => "Проверка отменена.",
        "judge_failed" => "Судья не смог вынести вердикт. Карточки сторон сохранены.",
        "internal" => "Внутренняя ошибка сервера.",
        "replay_unavailable" => "Нет записанного ответа для этого утверждения.",
        _ => {
            let fallback = fallback.trim();
            if fallback.is_empty() {
                "Неизвестная ошибка."
            } else {
                fallback
            }
        }
    };
    message.to_string()
}

pub fn truncate(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

pub fn percent(confidence: Option<f64>) -> String {
    match confidence {
        Some(value) => format!("{:.0}%", (value.clamp(0.0, 1.0)) * 100.0),
        None => "—".to_string(),
    }
}

pub fn is_safe_url(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() <= 2048
}

pub fn source_title(url: &str, title: &str) -> String {
    if title.trim().is_empty() {
        url.to_string()
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_handles_none_and_clamps() {
        assert_eq!(percent(None), "—");
        assert_eq!(percent(Some(0.623)), "62%");
        assert_eq!(percent(Some(1.5)), "100%");
        assert_eq!(percent(Some(-0.2)), "0%");
    }

    #[test]
    fn truncate_is_unicode_safe() {
        assert_eq!(truncate("Привет", 3), "При…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn only_http_urls_are_links() {
        assert!(is_safe_url("https://example.org/report"));
        assert!(is_safe_url("http://example.org"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("ftp://example.org"));
        assert!(!is_safe_url(&format!(
            "https://example.org/{}",
            "a".repeat(2048)
        )));
    }

    #[test]
    fn known_warning_has_a_human_message() {
        assert!(warning_label("no_verified_sources").contains("HTTP"));
        assert_eq!(warning_label("custom"), "custom");
    }

    #[test]
    fn error_codes_are_translated_and_fallback_is_used() {
        assert!(error_message("rate_limited", "").contains("Слишком много"));
        assert_eq!(error_message("custom_code", "server text"), "server text");
    }

    #[test]
    fn source_title_falls_back_to_url() {
        assert_eq!(
            source_title("https://example.org", "  "),
            "https://example.org"
        );
        assert_eq!(source_title("https://example.org", "Report"), "Report");
    }
}
