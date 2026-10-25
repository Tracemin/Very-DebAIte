use leptos::html;
use leptos::*;

use crate::actions;
use crate::store::{AppStore, UiError};

#[component]
pub fn ClaimForm() -> impl IntoView {
    let store = expect_context::<AppStore>();
    let file_input = NodeRef::<html::Input>::new();

    let char_count = Signal::derive(move || actions::claim_char_count(store.claim.get().trim()));
    let busy = Signal::derive(move || store.is_busy());

    // Clear the native file input when the store no longer holds a file
    // (after "Убрать", "Новая проверка" or a rejected file).
    create_effect(move |_| {
        if store.file_name.get().is_none() {
            if let Some(input) = file_input.get() {
                input.set_value("");
            }
        }
    });

    let on_submit = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        actions::start_check(store);
    };

    let on_input = move |event| {
        store.claim.set(event_target_value(&event));
        let has_input_error = store
            .error
            .get_untracked()
            .and_then(|error| error.code)
            .as_deref()
            == Some("invalid_request");
        if has_input_error {
            store.error.set(None);
        }
    };

    let on_file_change = move |event| {
        let input = event_target::<web_sys::HtmlInputElement>(&event);

        let Some(file) = input.files().and_then(|files| files.get(0)) else {
            store.file.set(None);
            store.file_name.set(None);
            return;
        };

        if file.size() > actions::MAX_FILE_BYTES {
            store.file.set(None);
            store.file_name.set(None);
            input.set_value("");
            store.error.set(Some(UiError::new(
                Some("file_too_large".to_string()),
                crate::text::error_message("file_too_large", "Файл больше 5 МиБ."),
                false,
            )));
            return;
        }

        if store
            .error
            .get_untracked()
            .and_then(|error| error.code)
            .as_deref()
            == Some("file_too_large")
        {
            store.error.set(None);
        }

        store.file_name.set(Some(file.name()));
        store.file.set(Some(file));
    };

    view! {
        <form class="card claim-form" on:submit=on_submit>
            <div class="field">
                <label class="field-label" for="claim">"Утверждение для проверки"</label>
                <textarea
                    id="claim"
                    class="textarea"
                    rows="5"
                    maxlength=actions::MAX_CLAIM_CHARS.to_string()
                    placeholder="Например: Вода кипит при 100 °C на уровне моря"
                    prop:value=move || store.claim.get()
                    on:input=on_input
                    disabled=move || busy.get()
                ></textarea>
                <div class="form-meta">
                    <span class=move || {
                        let count = char_count.get();
                        if count > actions::MAX_CLAIM_CHARS {
                            "counter invalid".to_string()
                        } else if count > 0 && count < actions::MIN_CLAIM_CHARS {
                            "counter warn".to_string()
                        } else {
                            "counter".to_string()
                        }
                    }>
                        {move || format!("{} / {}", char_count.get(), actions::MAX_CLAIM_CHARS)}
                    </span>
                    {move || {
                        let claim = store.claim.get();
                        if claim.trim().is_empty() {
                            None
                        } else {
                            actions::claim_error(&claim)
                                .map(|message| view! { <span class="hint-error">{message}</span> })
                        }
                    }}
                </div>
            </div>

            <div class="field">
                <label class="field-label" for="attachment">"Вложение (необязательно)"</label>
                <div class="file-row">
                    <input
                        node_ref=file_input
                        id="attachment"
                        class="file-input"
                        type="file"
                        accept=".txt,.pdf,text/plain,application/pdf"
                        on:change=on_file_change
                        disabled=move || busy.get()
                    />
                    {move || {
                        store.file_name.get().map(|name| view! {
                            <span class="file-chip" title=name.clone()>{name}</span>
                        })
                    }}
                    {move || {
                        store.file_name.get().map(|_| view! {
                            <button
                                type="button"
                                class="link-button"
                                disabled=move || busy.get()
                                on:click=move |_| {
                                    store.file.set(None);
                                    store.file_name.set(None);
                                }
                            >
                                "Убрать"
                            </button>
                        })
                    }}
                </div>
                <p class="hint">
                    "TXT или текстовый PDF, до 5 МиБ. OCR изображений в этой версии не поддерживается."
                </p>
            </div>

            <div class="form-actions">
                <button class="button primary" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { "Проверяем…" } else { "Проверить факт" }}
                </button>
                <span class="hint">
                    {move || {
                        if busy.get() {
                            "Идёт проверка: события появятся в карточках и журнале ниже."
                        } else {
                            "Проверка занимает до 2 минут."
                        }
                    }}
                </span>
            </div>
        </form>
    }
}
