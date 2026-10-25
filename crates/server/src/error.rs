use axum::response::{IntoResponse, Response};
use axum::Json;
use http::header::RETRY_AFTER;
use http::StatusCode;
use shared::{ApiErrorBody, ApiErrorDetail, ErrorCode, ErrorInfo};
use uuid::Uuid;

/// Application error rendered with the unified HTTP error body from §5.
#[derive(Debug, Clone)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub request_id: String,
    pub retry_after: Option<u64>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            request_id: Uuid::new_v4().to_string(),
            retry_after: None,
        }
    }

    pub fn code(code: ErrorCode) -> Self {
        Self::new(code, code.default_message())
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds.max(1));
        self
    }

    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    pub fn info(&self) -> ErrorInfo {
        ErrorInfo::new(self.code, self.message.clone())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ApiErrorBody {
            error: ApiErrorDetail {
                code: self.code,
                message: self.message,
                retryable: self.code.retryable(),
                request_id: self.request_id,
            },
        };
        let mut response = (status, Json(body)).into_response();
        if let Some(retry_after) = self.retry_after {
            if let Ok(value) = http::HeaderValue::from_str(&retry_after.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
        }
        response
    }
}

impl From<shared::ErrorInfo> for AppError {
    fn from(info: shared::ErrorInfo) -> Self {
        Self::new(info.code, info.message)
    }
}

pub type AppResult<T> = Result<T, AppError>;
