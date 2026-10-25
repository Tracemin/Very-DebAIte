use leptos::*;
use shared::{AgentResult, AgentRole, AgentState, AgentStatus};

use crate::components::common::{ArgumentList, SourceList};
use crate::store::AppStore;
use crate::text;

#[component]
pub fn AgentCard(role: AgentRole) -> impl IntoView {
    let store = expect_context::<AppStore>();
    let state = Signal::derive(move || store.agent_state(role));
    let status = Signal::derive(move || state.get().status);
    let is_busy = Signal::derive(move || store.is_busy());

    view! {
        <article class=format!("card agent-card {}", text::agent_class(role))>
            <header class="agent-head">
                <div class="agent-title">
                    <span class=format!("agent-dot {}", text::agent_class(role))></span>
                    <div>
                        <h3 class="agent-name">{text::agent_label(role)}</h3>
                        <p class="agent-desc">{text::agent_description(role)}</p>
                    </div>
                </div>
                <span class=move || {
                    format!("status-badge {}", text::status_class(status.get()))
                }>
                    {move || text::status_label(status.get())}
                </span>
            </header>
            <div class="agent-body-wrap">
                {move || render_body(state.get(), status.get(), is_busy.get())}
            </div>
        </article>
    }
}

fn render_body(state: AgentState, status: AgentStatus, is_busy: bool) -> impl IntoView {
    let text = state.text;
    let error = state.error;
    let result = state.result;

    let text_block = if text.is_empty() {
        match status {
            AgentStatus::Queued => {
                view! { <p class="agent-placeholder">"Ожидает запуска"</p> }.into_view()
            }
            AgentStatus::Running if is_busy => view! {
                <p class="agent-placeholder working">
                    <span class="typing-dot"></span>
                    "Ожидание ответа провайдера…"
                </p>
            }
            .into_view(),
            AgentStatus::Running => {
                view! { <p class="agent-placeholder">"Ожидание ответа провайдера…"</p> }.into_view()
            }
            AgentStatus::Done => {
                view! { <p class="agent-placeholder">"Агент завершил работу без текста."</p> }
                    .into_view()
            }
            AgentStatus::Error => {
                view! { <p class="agent-placeholder error">"Текст ответа недоступен."</p> }
                    .into_view()
            }
        }
    } else {
        view! { <div class="agent-text">{text}</div> }.into_view()
    };

    view! {
        <div class="agent-body">
            {text_block}
            {error.map(|error| {
                view! {
                    <div class="agent-error">
                        <div class="agent-error-message">
                            {text::error_message(error.code.as_str(), &error.message)}
                        </div>
                        <code class="error-code">{error.code.as_str()}</code>
                    </div>
                }
            })}
            {result.map(render_result)}
        </div>
    }
    .into_view()
}

fn render_result(result: AgentResult) -> impl IntoView {
    let confidence = text::percent(result.confidence);
    let model = if result.model.trim().is_empty() {
        "модель не указана".to_string()
    } else {
        result.model
    };
    let arguments = result.arguments;
    let sources = result.sources;

    view! {
        <div class="result-block">
            <div class="result-meta">
                <span>"Уверенность: " <strong>{confidence}</strong></span>
                <span class="model">{model}</span>
            </div>
            <ArgumentList arguments=arguments/>
            <SourceList sources=sources/>
        </div>
    }
    .into_view()
}
