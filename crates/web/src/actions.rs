use leptos::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::storage;
use crate::store::{AppStore, LastSubmission, LogLevel, Phase, UiError, WsStatus};
use crate::ws;

pub const MIN_CLAIM_CHARS: usize = 3;
pub const MAX_CLAIM_CHARS: usize = 4_000;
pub const MAX_FILE_BYTES: f64 = 5.0 * 1024.0 * 1024.0;

pub fn claim_char_count(claim: &str) -> usize {
    claim.chars().count()
}

/// Client-side mirror of the server rule from SPEC §4.2. The server remains
/// authoritative; this only provides immediate feedback.
pub fn claim_error(claim: &str) -> Option<&'static str> {
    let trimmed = claim.trim();
    let count = claim_char_count(trimmed);

    if count < MIN_CLAIM_CHARS {
        return Some("Утверждение должно содержать минимум 3 символа.");
    }
    if count > MAX_CLAIM_CHARS {
        return Some("Утверждение должно быть не длиннее 4000 символов.");
    }
    if !trimmed.chars().any(char::is_alphanumeric) {
        return Some("Утверждение должно содержать хотя бы одну букву или цифру.");
    }
    None
}

pub fn start_check(store: AppStore) {
    if store.is_busy() {
        return;
    }

    let claim = store.claim.get_untracked().trim().to_string();
    if let Some(message) = claim_error(&claim) {
        store.error.set(Some(UiError::new(
            Some("invalid_request".to_string()),
            message,
            false,
        )));
        return;
    }

    let file = store.file.get_untracked();
    store.last_submission.set(Some(LastSubmission {
        claim: claim.clone(),
        file: file.clone(),
    }));
    store.begin_check();
    storage::clear_last_check_id();

    spawn_local(async move {
        match api::create_check(&claim, file).await {
            Ok(response) => {
                let check_id = response.check_id.to_string();
                store.check_id.set(Some(check_id.clone()));
                storage::save_last_check_id(&check_id);
                store.phase.set(Phase::Connecting);
                store.log(
                    LogLevel::Info,
                    None,
                    "Проверка создана, подключаемся к потоку событий",
                );
                ws::connect(store, check_id);
            }
            Err(error) => {
                let message = error.user_message();
                store.set_http_error(error.code.clone(), message.clone(), error.retryable);
                store.log(LogLevel::Error, None, message);
            }
        }
    });
}

pub fn retry(store: AppStore) {
    if store.is_busy() {
        return;
    }

    let Some(last) = store.last_submission.get_untracked() else {
        return;
    };

    store.claim.set(last.claim);
    store
        .file_name
        .set(last.file.as_ref().map(|file| file.name()));
    store.file.set(last.file);
    store.error.set(None);
    start_check(store);
}

pub fn new_check(store: AppStore) {
    store.cancel_ws();
    store.check_id.set(None);
    store.snapshot.set(None);
    store.logs.set(Vec::new());
    store.error.set(None);
    store.last_seq.set(0);
    store.phase.set(Phase::Idle);
    store.ws_status.set(WsStatus::Idle);
    store.claim.set(String::new());
    store.file.set(None);
    store.file_name.set(None);
    store.last_submission.set(None);
    storage::clear_last_check_id();
}

/// Restore the last check from `localStorage` after a page reload.
///
/// A `GET /api/check/{id}` is used first so that a stale/expired id is cleared
/// instead of triggering an endless WS reconnect loop.
pub fn restore_last_check(store: AppStore) {
    if store.check_id.get_untracked().is_some() {
        return;
    }

    let Some(check_id) = storage::load_last_check_id() else {
        return;
    };

    let generation = store.ws_generation.get_untracked();
    store.check_id.set(Some(check_id.clone()));
    store.phase.set(Phase::Connecting);
    store.ws_status.set(WsStatus::Connecting);

    spawn_local(async move {
        match api::get_check(&check_id).await {
            Ok(snapshot) => {
                if store.ws_generation.get_untracked() != generation {
                    return;
                }
                store.apply_snapshot(snapshot);
                if store.is_terminal() {
                    store.ws_status.set(WsStatus::Closed);
                } else {
                    ws::connect(store, check_id);
                }
            }
            Err(error) if error.is_not_found() => {
                if store.check_id.get_untracked().as_deref() == Some(check_id.as_str()) {
                    storage::clear_last_check_id();
                    store.check_id.set(None);
                    store.phase.set(Phase::Idle);
                    store.ws_status.set(WsStatus::Idle);
                    store.log(
                        LogLevel::Warning,
                        None,
                        "Сохранённая проверка не найдена — возможно, истёк срок хранения.",
                    );
                }
            }
            Err(error) => {
                if store.check_id.get_untracked().as_deref() == Some(check_id.as_str()) {
                    store.check_id.set(None);
                    store.phase.set(Phase::Idle);
                    store.ws_status.set(WsStatus::Idle);
                    store.log(
                        LogLevel::Warning,
                        None,
                        format!(
                            "Не удалось восстановить предыдущую проверку: {}",
                            error.user_message()
                        ),
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_normal_claim() {
        assert!(claim_error("Вода кипит при 100 °C на уровне моря").is_none());
        assert!(claim_error("abc").is_none());
    }

    #[test]
    fn rejects_short_and_blank_claims() {
        assert_eq!(
            claim_error(""),
            Some("Утверждение должно содержать минимум 3 символа.")
        );
        assert_eq!(
            claim_error("  ab  "),
            Some("Утверждение должно содержать минимум 3 символа.")
        );
    }

    #[test]
    fn rejects_too_long_claims() {
        let claim = "а".repeat(MAX_CLAIM_CHARS + 1);
        assert_eq!(
            claim_error(&claim),
            Some("Утверждение должно быть не длиннее 4000 символов.")
        );
    }

    #[test]
    fn rejects_claims_without_letters_or_digits() {
        assert_eq!(
            claim_error("!!!"),
            Some("Утверждение должно содержать хотя бы одну букву или цифру.")
        );
    }

    #[test]
    fn counts_unicode_characters_not_bytes() {
        let claim = "Привет";
        assert_eq!(claim_char_count(claim), 6);
        assert!(claim_error(claim).is_none());
    }
}
