use leptos::*;

use crate::store::{AppStore, LogLevel};
use crate::text;

#[component]
pub fn LogsPanel() -> impl IntoView {
    let store = expect_context::<AppStore>();

    view! {
        <section class="card logs-panel">
            <div class="logs-head">
                <h2>"Журнал событий"</h2>
                <span class="muted">"WebSocket"</span>
            </div>
            <ul class="log-list">
                <For
                    each=move || store.logs.get()
                    key=|entry| entry.id
                    children=move |entry| {
                        let (level, level_label) = match entry.level {
                            LogLevel::Info => ("info", "INFO"),
                            LogLevel::Success => ("success", "OK"),
                            LogLevel::Warning => ("warning", "WARN"),
                            LogLevel::Error => ("error", "ERR"),
                            LogLevel::Delta => ("delta", "TEXT"),
                        };
                        view! {
                            <li class=format!("log-entry {level}")>
                                <span class="log-time">{entry.ts}</span>
                                <span class="log-level">{level_label}</span>
                                {entry.agent.map(|role| {
                                    view! {
                                        <span class="log-agent">{text::agent_short(role)}</span>
                                    }
                                })}
                                <span class="log-message">{entry.message}</span>
                            </li>
                        }
                    }
                />
            </ul>
            {move || {
                store.logs.get().is_empty().then(|| {
                    view! {
                        <p class="muted logs-empty">
                            "Событий пока нет. Отправьте утверждение на проверку."
                        </p>
                    }
                })
            }}
        </section>
    }
}
