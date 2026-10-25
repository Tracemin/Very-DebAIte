use leptos::*;
use shared::AgentRole;

use crate::actions;
use crate::components::{
    AgentCard, ClaimForm, DemoBanner, Disclaimer, ErrorBanner, LogsPanel, VerdictPanel,
};
use crate::store::{AppStore, Phase, WsStatus};

#[component]
pub fn App() -> impl IntoView {
    let store = AppStore::new();
    provide_context(store);
    actions::restore_last_check(store);

    let phase = Signal::derive(move || store.phase.get());
    let ws_status = Signal::derive(move || store.ws_status.get());
    let check_id = Signal::derive(move || store.check_id.get());

    view! {
        <div class="app">
            <header class="app-header">
                <div class="header-inner">
                    <div class="brand">
                        <span class="brand-mark">"VD"</span>
                        <div>
                            <h1>"Very-DebAIte — Fact Checker"</h1>
                            <p>"Мультиагентная проверка утверждений: Оптимист, Скептик и Судья"</p>
                        </div>
                    </div>
                    <div class="header-meta">
                        <span class="badge badge-soft">"MVP v2"</span>
                        <span class="muted">"Leptos CSR · WebSocket"</span>
                    </div>
                </div>
            </header>

            <main class="container">
                <DemoBanner/>
                <ClaimForm/>
                <ErrorBanner/>

                <div class="status-strip">
                    <span class=move || format!("phase-pill {}", phase_class(phase.get()))>
                        {move || phase_label(phase.get())}
                    </span>
                    <span class=move || format!("ws-pill {}", ws_class(ws_status.get()))>
                        {move || ws_label(ws_status.get())}
                    </span>
                    {move || {
                        check_id.get().map(|id| {
                            view! {
                                <span class="check-id" title=id.clone()>
                                    {format!("ID: {}", short_id(&id))}
                                </span>
                            }
                        })
                    }}
                    {move || terminal_actions(store)}
                </div>

                <section class="agents-section" aria-label="Агенты проверки">
                    <div class="section-head">
                        <div>
                            <h2>"Агенты"</h2>
                            <p class="muted">
                                "Два дебатера работают параллельно, затем Судья выносит вердикт."
                            </p>
                        </div>
                    </div>
                    <div class="agents-grid">
                        <AgentCard role=AgentRole::Proponent/>
                        <AgentCard role=AgentRole::Skeptic/>
                        <AgentCard role=AgentRole::Judge/>
                    </div>
                </section>

                <VerdictPanel/>
                <LogsPanel/>
                <Disclaimer/>
            </main>
        </div>
    }
}

fn terminal_actions(store: AppStore) -> Option<impl IntoView> {
    matches!(store.phase.get(), Phase::Done | Phase::Error).then(|| {
        view! {
            <button
                type="button"
                class="button secondary"
                disabled=move || store.is_busy()
                on:click=move |_| actions::new_check(store)
            >
                "Новая проверка"
            </button>
        }
        .into_view()
    })
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "Ожидание",
        Phase::Connecting => "Подключение",
        Phase::Running => "Проверка идёт",
        Phase::Done => "Готово",
        Phase::Error => "Ошибка",
    }
}

fn phase_class(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Connecting => "connecting",
        Phase::Running => "running",
        Phase::Done => "done",
        Phase::Error => "error",
    }
}

fn ws_label(status: WsStatus) -> String {
    match status {
        WsStatus::Idle => "WS: не подключён".to_string(),
        WsStatus::Connecting => "WS: подключение…".to_string(),
        WsStatus::Open => "WS: на связи".to_string(),
        WsStatus::Reconnecting(delay_ms) => {
            format!(
                "WS: переподключение через {:.1} с",
                delay_ms as f64 / 1000.0
            )
        }
        WsStatus::Closed => "WS: закрыт".to_string(),
    }
}

fn ws_class(status: WsStatus) -> &'static str {
    match status {
        WsStatus::Idle => "idle",
        WsStatus::Connecting => "connecting",
        WsStatus::Open => "open",
        WsStatus::Reconnecting(_) => "reconnecting",
        WsStatus::Closed => "closed",
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}
