use std::collections::HashMap;

use leptos::*;
use shared::{Argument, DISCLAIMER};

use crate::components::common::{ArgumentContent, SourceList};
use crate::store::AppStore;
use crate::text;

#[component]
pub fn VerdictPanel() -> impl IntoView {
    let store = expect_context::<AppStore>();
    let verdict =
        Signal::derive(move || store.snapshot.get().and_then(|snapshot| snapshot.verdict));
    let argument_index = Signal::derive(move || store.argument_index());

    view! {
        {move || {
            let Some(verdict) = verdict.get() else {
                return ().into_view();
            };

            let kind = verdict.verdict;
            let class = text::verdict_class(kind);
            let explanation = verdict.explanation;
            let confidence = text::percent(verdict.confidence);
            let is_falsifiable = verdict.is_falsifiable;
            let for_arguments = verdict.for_arguments;
            let against = verdict.against;
            let sources = verdict.sources;
            let warnings = verdict.warnings;
            let search_used = verdict.search_used;
            let disclaimer = if verdict.disclaimer.trim().is_empty() {
                DISCLAIMER.to_string()
            } else {
                verdict.disclaimer
            };
            let index = argument_index.get();

            view! {
                <section class=format!("card verdict-panel {class}")>
                    <div class="verdict-head">
                        <div>
                            <div class="verdict-kicker">"Итоговый вердикт"</div>
                            <h2 class=format!("verdict-title {class}")>
                                {text::verdict_label(kind)}
                            </h2>
                        </div>
                        <div class="confidence">
                            <span class="confidence-value">{confidence}</span>
                            <span class="confidence-note">
                                "уверенность модели, не вероятность истинности"
                            </span>
                        </div>
                    </div>

                    {if is_falsifiable {
                        None
                    } else {
                        Some(view! {
                            <p class="note note-warn">
                                "Утверждение не является проверяемым фактом — вердикт ограничен значением «Недостаточно данных»."
                            </p>
                        })
                    }}

                    {if explanation.is_empty() {
                        None
                    } else {
                        Some(view! { <p class="verdict-explanation">{explanation}</p> })
                    }}

                    <div class="verdict-sides">
                        {render_side("Аргументы за", for_arguments, index.clone())}
                        {render_side("Аргументы против", against, index)}
                    </div>

                    <SourceList sources=sources/>

                    {if warnings.is_empty() {
                        None
                    } else {
                        Some(view! {
                            <div class="warnings">
                                <h4>"Предупреждения"</h4>
                                <ul>
                                    {warnings
                                        .iter()
                                        .map(|warning| view! { <li>{text::warning_label(warning)}</li> })
                                        .collect_view()}
                                </ul>
                            </div>
                        })
                    }}

                    {if search_used {
                        None
                    } else {
                        Some(view! {
                            <p class="note">
                                "Поиск в интернете не использовался: анализ основан на знаниях модели и загруженном материале."
                            </p>
                        })
                    }}

                    {if disclaimer.is_empty() {
                        None
                    } else {
                        Some(view! { <p class="verdict-disclaimer">{disclaimer}</p> })
                    }}
                </section>
            }
            .into_view()
        }}
    }
}

fn render_side(
    title: &'static str,
    ids: Vec<String>,
    index: HashMap<String, Argument>,
) -> impl IntoView {
    if ids.is_empty() {
        return view! {
            <div class="verdict-side">
                <h4>{title}</h4>
                <p class="muted">"Нет аргументов."</p>
            </div>
        }
        .into_view();
    }

    view! {
        <div class="verdict-side">
            <h4>{title}</h4>
            <ul class="verdict-arguments">
                <For
                    each=move || ids.clone()
                    key=|id| id.clone()
                    children=move |id| {
                        let argument = index.get(&id).cloned();
                        view! {
                            <li class="argument">
                                {match argument {
                                    Some(argument) => {
                                        view! { <ArgumentContent argument=argument/> }.into_view()
                                    }
                                    None => {
                                        view! { <span class="muted">{id.clone()}</span> }.into_view()
                                    }
                                }}
                            </li>
                        }
                    }
                />
            </ul>
        </div>
    }
    .into_view()
}
