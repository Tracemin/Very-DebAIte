use futures_util::StreamExt;
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;
use leptos::*;
use serde_json::Value;
use shared::{WsEnvelope, WsEvent};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::store::{AppStore, LogLevel, WsStatus};

/// Event types understood by this client (`docs/SPEC.md` §6.3/§6.4).
const KNOWN_EVENT_TYPES: &[&str] = &[
    "snapshot",
    "check_started",
    "agent_started",
    "agent_delta",
    "agent_completed",
    "agent_error",
    "verdict_ready",
    "check_completed",
    "check_error",
];

fn is_known_event_type(event_type: &str) -> bool {
    KNOWN_EVENT_TYPES.contains(&event_type)
}

fn ws_url(check_id: &str) -> String {
    let base = api::api_base();
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base
    };
    format!("{base}/ws/{check_id}")
}

fn backoff_delay(attempt: usize) -> u32 {
    match attempt {
        0 => 500,
        1 => 1_000,
        2 => 2_000,
        3 => 5_000,
        _ => 60_000,
    }
}

/// Open (and keep reopening) the WS stream for a check.
///
/// Reconnect backoff follows SPEC §11: 0.5 / 1 / 2 / 5 s, capped at 60 s.
/// Every reconnect starts with a fresh `snapshot`, so the UI state is rebuilt
/// from the server rather than from a half-merged local state.
pub fn connect(store: AppStore, check_id: String) {
    let generation = store.ws_generation.get_untracked().wrapping_add(1);
    store.ws_generation.set(generation);

    spawn_local(async move {
        let mut attempt = 0usize;

        loop {
            if store.ws_generation.get_untracked() != generation {
                return;
            }

            store.ws_status.set(WsStatus::Connecting);
            match WebSocket::open(&ws_url(&check_id)) {
                Ok(mut socket) => {
                    store.ws_status.set(WsStatus::Open);
                    store.log(LogLevel::Info, None, "WebSocket подключён");

                    while let Some(message) = socket.next().await {
                        if store.ws_generation.get_untracked() != generation {
                            return;
                        }

                        match message {
                            Ok(Message::Text(text)) => {
                                // A real message means the upgrade was accepted;
                                // reset backoff for the next possible drop.
                                attempt = 0;
                                if handle_message(&store, &text) {
                                    store.ws_status.set(WsStatus::Closed);
                                    return;
                                }
                            }
                            Ok(Message::Bytes(_)) => {
                                // The contract is text JSON, but ignore binary
                                // frames instead of killing the stream.
                            }
                            Err(error) => {
                                store.log(
                                    LogLevel::Warning,
                                    None,
                                    format!("Ошибка WebSocket: {error:?}"),
                                );
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    store.log(
                        LogLevel::Warning,
                        None,
                        format!("Не удалось открыть WebSocket: {error:?}"),
                    );
                }
            }

            if store.ws_generation.get_untracked() != generation {
                return;
            }

            if store.is_terminal() {
                store.ws_status.set(WsStatus::Closed);
                return;
            }

            let delay = backoff_delay(attempt);
            attempt = attempt.saturating_add(1);
            store.ws_status.set(WsStatus::Reconnecting(delay));
            store.log(
                LogLevel::Warning,
                None,
                format!(
                    "Соединение с сервером потеряно. Переподключение через {:.1} с…",
                    delay as f64 / 1000.0
                ),
            );

            TimeoutFuture::new(delay).await;
        }
    });
}

/// Returns `true` when the event is terminal and the stream must be closed.
fn handle_message(store: &AppStore, text: &str) -> bool {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => {
            store.log(
                LogLevel::Warning,
                None,
                format!("Некорректное сообщение WebSocket: {error}"),
            );
            return false;
        }
    };

    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if !is_known_event_type(&event_type) {
        store.log(
            LogLevel::Warning,
            None,
            format!("Неизвестный тип события «{event_type}» проигнорирован"),
        );
        return false;
    }

    let envelope: WsEnvelope = match serde_json::from_value(value) {
        Ok(envelope) => envelope,
        Err(error) => {
            store.log(
                LogLevel::Warning,
                None,
                format!("Не удалось разобрать событие «{event_type}»: {error}"),
            );
            return false;
        }
    };

    if let Some(current_id) = store.check_id.get_untracked() {
        if current_id != envelope.check_id.to_string() {
            return false;
        }
    }

    if let WsEvent::Snapshot(data) = &envelope.event {
        store.apply_snapshot((**data).clone());
        return false;
    }

    // SPEC §6.2: skip duplicates that are already part of the snapshot.
    if envelope.seq <= store.last_seq.get_untracked() {
        return false;
    }

    let WsEnvelope {
        seq,
        ts,
        agent,
        event,
        ..
    } = envelope;
    store.apply_event(seq, &ts, agent, event)
}
