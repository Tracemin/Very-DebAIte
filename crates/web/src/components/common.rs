use leptos::*;
use shared::{Argument, Source};

use crate::text;

#[component]
pub fn SourceList(sources: Vec<Source>) -> impl IntoView {
    if sources.is_empty() {
        return view! { <p class="muted">"Источники не указаны."</p> }.into_view();
    }

    view! {
        <div class="sources">
            <h4 class="sources-title">"Источники"</h4>
            <ul class="source-list">
                <For
                    each=move || sources.clone()
                    key=|source| source.id.clone()
                    children=move |source| view! { <SourceItem source=source/> }
                />
            </ul>
        </div>
    }
    .into_view()
}

#[component]
fn SourceItem(source: Source) -> impl IntoView {
    let title = text::source_title(&source.url, &source.title);
    let safe = text::is_safe_url(&source.url);
    let url = source.url;
    let verification = source.verification;
    let description = source.description;
    let published_at = source.published_at;
    let id = source.id;

    let source_link = if safe {
        view! {
            <a href=url target="_blank" rel="noopener noreferrer">{title}</a>
        }
        .into_view()
    } else {
        view! { <span>{title}</span> }.into_view()
    };

    view! {
        <li class="source">
            <div class="source-head">
                {source_link}
                <span class=format!("verification {}", text::verification_class(verification))>
                    {text::verification_label(verification)}
                </span>
            </div>
            {if description.is_empty() {
                None
            } else {
                Some(view! { <p class="source-desc">{description}</p> })
            }}
            <div class="source-meta">
                {published_at.map(|date| view! { <span>"Опубликовано: " {date}</span> })}
                <span class="source-id">{id}</span>
            </div>
        </li>
    }
}

#[component]
pub fn ArgumentList(arguments: Vec<Argument>) -> impl IntoView {
    if arguments.is_empty() {
        return view! { <p class="muted">"Аргументы не приведены."</p> }.into_view();
    }

    view! {
        <ul class="arguments">
            <For
                each=move || arguments.clone()
                key=|argument| argument.id.clone()
                children=move |argument| view! { <ArgumentItem argument=argument/> }
            />
        </ul>
    }
    .into_view()
}

#[component]
pub fn ArgumentItem(argument: Argument) -> impl IntoView {
    view! {
        <li class="argument">
            <ArgumentContent argument=argument/>
        </li>
    }
}

#[component]
pub fn ArgumentContent(argument: Argument) -> impl IntoView {
    let stance = argument.stance;
    let strength = argument.strength;
    let claim = argument.claim;
    let evidence = argument.evidence;
    let source_ids = argument.source_ids;

    view! {
        <div class="argument-content">
            <div class="argument-head">
                <span class=format!("stance {}", text::stance_class(stance))>
                    {text::stance_label(stance)}
                </span>
                <span class=format!("strength {}", text::strength_class(strength))>
                    "сила: " {text::strength_label(strength)}
                </span>
            </div>
            <p class="argument-claim">{claim}</p>
            {if evidence.is_empty() {
                None
            } else {
                Some(view! { <p class="argument-evidence">{evidence}</p> })
            }}
            {if source_ids.is_empty() {
                None
            } else {
                Some(view! {
                    <p class="argument-sources">"Источники: " {source_ids.join(", ")}</p>
                })
            }}
        </div>
    }
}
