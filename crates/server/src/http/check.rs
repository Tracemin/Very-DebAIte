//! `POST /api/check`, `GET /api/check/{id}`, `DELETE /api/check/{id}`.

use std::sync::Arc;

use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use sha2::{Digest, Sha256};
use shared::{CheckSnapshot, CreateCheckResponse};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::files::extract_attachment;
use crate::http::ClientIp;
use crate::limits;
use crate::orchestrator::{run_check, CheckRecord};
use crate::state::AppState;

pub async fn create_check(
    State(state): State<Arc<AppState>>,
    ClientIp(ip): ClientIp,
    request: Request,
) -> AppResult<Response> {
    let permit = state.limiter.try_acquire(ip)?;

    let multipart = Multipart::from_request(request, &state)
        .await
        .map_err(multipart_rejection)?;

    let form = parse_form(multipart).await?;
    let claim = validate_claim(form.claim)?;

    let attachment = match form.file {
        Some(bytes) => Some(extract_attachment(bytes).await?),
        None => None,
    };

    if !state.llm_configured() {
        return Err(AppError::code(shared::ErrorCode::ProviderUnavailable));
    }

    let attachment_text = attachment.as_ref().map(|file| file.text.clone());
    let fixture_key = fixture_key(&claim, attachment_text.as_deref());
    if state.config.demo_mode == shared::DemoMode::Replay
        && !state.llm.supports_fixture(&fixture_key)
    {
        return Err(AppError::new(
            shared::ErrorCode::ProviderUnavailable,
            "Для этого утверждения нет записанного ответа",
        ));
    }

    let check_id = Uuid::new_v4();
    let now = Utc::now();
    let deadline_at = now + chrono::Duration::from_std(limits::CHECK_TIMEOUT).unwrap();
    let has_attachment = attachment_text.is_some();
    let demo = state.config.demo_mode == shared::DemoMode::Replay;
    let warnings = attachment
        .as_ref()
        .map(|file| file.warnings.clone())
        .unwrap_or_default();

    let snapshot = CheckSnapshot::new(
        check_id,
        claim.clone(),
        has_attachment,
        demo,
        now,
        deadline_at,
        warnings,
    );
    let record = CheckRecord::new(snapshot, attachment_text, fixture_key);
    state.insert_check(Arc::clone(&record)).await?;

    tokio::spawn(run_check(
        Arc::clone(&record),
        Arc::clone(&state.config),
        Arc::clone(&state.llm),
        permit,
    ));

    let expires_at = now + chrono::Duration::from_std(limits::CHECK_TTL).unwrap();
    let response = CreateCheckResponse {
        check_id,
        ws_url: format!("/ws/{check_id}"),
        expires_at,
    };

    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

pub async fn get_check(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let record = find_check(&state, &id).await?;
    Ok(Json(record.snapshot().await).into_response())
}

pub async fn delete_check(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let record = find_check(&state, &id).await?;
    if record.is_finished() {
        return Err(AppError::code(shared::ErrorCode::AlreadyFinished));
    }
    if !record.request_cancel() {
        return Err(AppError::code(shared::ErrorCode::AlreadyFinished));
    }
    tracing::info!(check_id = %record.id, "check cancellation requested");
    Ok(StatusCode::ACCEPTED.into_response())
}

async fn find_check(state: &AppState, id: &str) -> AppResult<Arc<CheckRecord>> {
    let id = Uuid::parse_str(id).map_err(|_| AppError::code(shared::ErrorCode::NotFound))?;
    state
        .get_check(id)
        .await
        .ok_or_else(|| AppError::code(shared::ErrorCode::NotFound))
}

struct CheckForm {
    claim: Option<String>,
    file: Option<Vec<u8>>,
}

async fn parse_form(mut multipart: Multipart) -> AppResult<CheckForm> {
    let mut claim = None;
    let mut file = None;

    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "claim" => {
                if claim.is_some() {
                    return Err(AppError::new(
                        shared::ErrorCode::InvalidRequest,
                        "Поле claim указано более одного раза",
                    ));
                }
                let text = field.text().await.map_err(multipart_error)?;
                claim = Some(text);
            }
            "file" => {
                if file.is_some() {
                    return Err(AppError::new(
                        shared::ErrorCode::InvalidRequest,
                        "Можно приложить только один файл",
                    ));
                }
                let bytes = field.bytes().await.map_err(multipart_error)?;
                if bytes.len() > limits::MAX_FILE_BYTES {
                    return Err(AppError::code(shared::ErrorCode::FileTooLarge));
                }
                file = Some(bytes.to_vec());
            }
            _ => {
                // Unknown multipart fields are ignored for forward compatibility.
            }
        }
    }

    Ok(CheckForm { claim, file })
}

fn validate_claim(value: Option<String>) -> AppResult<String> {
    let Some(value) = value else {
        return Err(AppError::new(
            shared::ErrorCode::InvalidRequest,
            "Поле claim обязательно",
        ));
    };
    let claim = value.trim().to_owned();
    let length = claim.chars().count();
    if !(limits::CLAIM_MIN_CHARS..=limits::CLAIM_MAX_CHARS).contains(&length) {
        return Err(AppError::new(
            shared::ErrorCode::InvalidRequest,
            format!(
                "Утверждение должно содержать от {} до {} символов",
                limits::CLAIM_MIN_CHARS,
                limits::CLAIM_MAX_CHARS
            ),
        ));
    }
    if !claim.chars().any(char::is_alphanumeric) {
        return Err(AppError::new(
            shared::ErrorCode::InvalidRequest,
            "Утверждение должно содержать хотя бы одну букву или цифру",
        ));
    }
    Ok(claim)
}

fn fixture_key(claim: &str, document: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(claim.as_bytes());
    hasher.update([0u8]);
    if let Some(document) = document {
        hasher.update(document.as_bytes());
    }
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn multipart_rejection(rejection: axum::extract::multipart::MultipartRejection) -> AppError {
    multipart_status_error(rejection.status())
}

fn multipart_error(error: axum::extract::multipart::MultipartError) -> AppError {
    multipart_status_error(error.status())
}

fn multipart_status_error(status: StatusCode) -> AppError {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::new(
            shared::ErrorCode::FileTooLarge,
            "Тело запроса больше допустимого размера",
        )
    } else {
        AppError::new(
            shared::ErrorCode::InvalidRequest,
            "Некорректный multipart-запрос",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_validation_trims_and_accepts_unicode() {
        assert_eq!(
            validate_claim(Some("  Вода кипит  ".to_owned())).unwrap(),
            "Вода кипит"
        );
    }

    #[test]
    fn claim_validation_rejects_short_and_symbol_only_values() {
        assert_eq!(
            validate_claim(Some(" ab ".to_owned())).unwrap_err().code,
            shared::ErrorCode::InvalidRequest
        );
        assert_eq!(
            validate_claim(Some("...".to_owned())).unwrap_err().code,
            shared::ErrorCode::InvalidRequest
        );
        assert!(validate_claim(None).is_err());
    }

    #[test]
    fn fixture_key_is_stable_and_exists_sensitive() {
        assert_eq!(fixture_key("claim", None), fixture_key("claim", None));
        assert_ne!(
            fixture_key("claim", None),
            fixture_key("claim", Some("document"))
        );
        assert_eq!(fixture_key("claim", None).len(), 16);
    }
}
