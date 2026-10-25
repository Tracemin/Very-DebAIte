use gloo_net::http::Request;
use shared::{ApiErrorBody, CheckSnapshot, CreateCheckResponse};
use web_sys::{File, FormData};

use crate::text;

/// Base URL of the Axum server.
///
/// In production/demo the UI is served by Axum from `:3000`, so requests are
/// same-origin. During `trunk serve` on `:8080` we target the backend on
/// `:3000` explicitly (CORS is configured for that case by the server).
pub fn api_base() -> String {
    let Some(window) = web_sys::window() else {
        return String::new();
    };
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_default();

    if location.port().unwrap_or_default() == "8080" {
        let hostname = location
            .hostname()
            .unwrap_or_else(|_| "127.0.0.1".to_string());
        format!("{protocol}//{hostname}:3000")
    } else {
        format!("{protocol}//{host}")
    }
}

#[derive(Debug, Clone)]
pub struct SubmitError {
    pub code: Option<String>,
    pub message: String,
    pub retryable: bool,
    pub status: Option<u16>,
}

impl SubmitError {
    pub fn user_message(&self) -> String {
        match self.code.as_deref() {
            Some(code) => text::error_message(code, &self.message),
            None if !self.message.trim().is_empty() => self.message.clone(),
            None => "Не удалось выполнить запрос. Попробуйте ещё раз.".to_string(),
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.status == Some(404) || self.code.as_deref() == Some("not_found")
    }

    fn network(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            retryable: true,
            status: None,
        }
    }

    fn from_response(status: u16, body: &str) -> Self {
        match serde_json::from_str::<ApiErrorBody>(body) {
            Ok(parsed) => Self {
                code: Some(parsed.error.code.as_str().to_string()),
                message: parsed.error.message,
                retryable: parsed.error.retryable || status == 429 || status >= 500,
                status: Some(status),
            },
            Err(_) => Self {
                code: None,
                message: format!(
                    "Сервер вернул ошибку HTTP {status}{}",
                    if body.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", text::truncate(body.trim(), 240))
                    }
                ),
                retryable: status == 429 || status >= 500,
                status: Some(status),
            },
        }
    }
}

pub async fn create_check(
    claim: &str,
    file: Option<File>,
) -> Result<CreateCheckResponse, SubmitError> {
    let form = FormData::new()
        .map_err(|_| SubmitError::network("Браузер не позволил подготовить данные формы."))?;

    form.append_with_str("claim", claim)
        .map_err(|_| SubmitError::network("Не удалось добавить утверждение в форму."))?;

    if let Some(file) = file.as_ref() {
        form.append_with_blob("file", file)
            .map_err(|_| SubmitError::network("Не удалось приложить файл к форме."))?;
    }

    let response = Request::post(&format!("{}/api/check", api_base()))
        .body(form)
        .map_err(|error| SubmitError::network(format!("Ошибка подготовки запроса: {error}")))?
        .send()
        .await
        .map_err(|error| {
            SubmitError::network(format!(
                "Не удалось связаться с сервером: {error}. Проверьте, что backend запущен."
            ))
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| SubmitError::network(format!("Не удалось прочитать ответ: {error}")))?;

    if (200..300).contains(&status) {
        serde_json::from_str::<CreateCheckResponse>(&body).map_err(|error| SubmitError {
            code: Some("internal".to_string()),
            message: format!("Сервер вернул неожиданный ответ: {error}"),
            retryable: true,
            status: Some(status),
        })
    } else {
        Err(SubmitError::from_response(status, &body))
    }
}

pub async fn get_check(check_id: &str) -> Result<CheckSnapshot, SubmitError> {
    let response = Request::get(&format!("{}/api/check/{check_id}", api_base()))
        .send()
        .await
        .map_err(|error| {
            SubmitError::network(format!(
                "Не удалось связаться с сервером: {error}. Проверьте, что backend запущен."
            ))
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| SubmitError::network(format!("Не удалось прочитать ответ: {error}")))?;

    if status == 200 {
        serde_json::from_str::<CheckSnapshot>(&body).map_err(|error| SubmitError {
            code: Some("internal".to_string()),
            message: format!("Сервер вернул неожиданный снимок состояния: {error}"),
            retryable: true,
            status: Some(status),
        })
    } else {
        Err(SubmitError::from_response(status, &body))
    }
}
