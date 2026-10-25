//! Error codes shared by the HTTP API and the WebSocket protocol.

use serde::{Deserialize, Serialize};

/// Machine-readable error codes from `docs/SPEC.md` §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    FileTooLarge,
    UnsupportedFileType,
    InvalidEncoding,
    TextExtractionFailed,
    NotFound,
    AlreadyFinished,
    RateLimited,
    Overloaded,
    ProviderUnavailable,
    Internal,
    ProviderTimeout,
    ProviderRateLimited,
    BadModelOutput,
    CheckTimeout,
    Cancelled,
    JudgeFailed,
    ReplayUnavailable,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::FileTooLarge => "file_too_large",
            Self::UnsupportedFileType => "unsupported_file_type",
            Self::InvalidEncoding => "invalid_encoding",
            Self::TextExtractionFailed => "text_extraction_failed",
            Self::NotFound => "not_found",
            Self::AlreadyFinished => "already_finished",
            Self::RateLimited => "rate_limited",
            Self::Overloaded => "overloaded",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Internal => "internal",
            Self::ProviderTimeout => "provider_timeout",
            Self::ProviderRateLimited => "provider_rate_limited",
            Self::BadModelOutput => "bad_model_output",
            Self::CheckTimeout => "check_timeout",
            Self::Cancelled => "cancelled",
            Self::JudgeFailed => "judge_failed",
            Self::ReplayUnavailable => "replay_unavailable",
        }
    }

    /// HTTP status used for the corresponding API error.
    pub fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest => 400,
            Self::NotFound => 404,
            Self::AlreadyFinished => 409,
            Self::FileTooLarge => 413,
            Self::UnsupportedFileType => 415,
            Self::InvalidEncoding | Self::TextExtractionFailed => 422,
            Self::RateLimited => 429,
            Self::Overloaded | Self::ProviderUnavailable | Self::ReplayUnavailable => 503,
            Self::ProviderTimeout | Self::CheckTimeout => 504,
            Self::ProviderRateLimited => 429,
            Self::BadModelOutput | Self::JudgeFailed => 502,
            Self::Cancelled => 409,
            Self::Internal => 500,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::Overloaded
                | Self::ProviderUnavailable
                | Self::Internal
                | Self::ProviderTimeout
                | Self::ProviderRateLimited
        )
    }

    pub fn default_message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "Некорректный запрос",
            Self::FileTooLarge => "Файл больше 5 МиБ",
            Self::UnsupportedFileType => {
                "Неподдерживаемый тип файла. Поддерживаются TXT и текстовые PDF; OCR в этой версии не поддерживается."
            }
            Self::InvalidEncoding => "Файл TXT должен быть в кодировке UTF-8",
            Self::TextExtractionFailed => {
                "Не удалось извлечь текст из PDF. Похоже, это скан; OCR в этой версии не поддерживается."
            }
            Self::NotFound => "Проверка не найдена",
            Self::AlreadyFinished => "Проверка уже завершена",
            Self::RateLimited => "Слишком много запросов. Попробуйте позже",
            Self::Overloaded => "Сервис перегружен. Попробуйте позже",
            Self::ProviderUnavailable => "LLM-провайдер недоступен",
            Self::Internal => "Внутренняя ошибка",
            Self::ProviderTimeout => "Провайдер не ответил вовремя",
            Self::ProviderRateLimited => "Провайдер ограничил частоту запросов",
            Self::BadModelOutput => "Модель вернула невалидный ответ",
            Self::CheckTimeout => "Проверка не уложилась в отведённое время",
            Self::Cancelled => "Проверка отменена",
            Self::JudgeFailed => "Судья не смог вынести вердикт",
            Self::ReplayUnavailable => "Нет записанного ответа для этого утверждения",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error payload used by `agent_error` / `check_error` WebSocket events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl ErrorInfo {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code,
            message: if message.is_empty() {
                code.default_message().to_owned()
            } else {
                message
            },
            retryable: code.retryable(),
        }
    }

    pub fn from_code(code: ErrorCode) -> Self {
        Self::new(code, code.default_message())
    }
}

/// Detail object of the unified HTTP error body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorDetail {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub request_id: String,
}

/// Unified HTTP error body from `docs/SPEC.md` §5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}
