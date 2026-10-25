use leptos::*;

use crate::actions;
use crate::store::AppStore;

#[component]
pub fn DemoBanner() -> impl IntoView {
    let store = expect_context::<AppStore>();
    let demo = Signal::derive(move || {
        store
            .snapshot
            .get()
            .map(|snapshot| snapshot.demo)
            .unwrap_or(false)
    });

    view! {
        {move || {
            demo.get().then(|| {
                view! {
                    <div class="banner banner-demo" role="status">
                        <strong>"Демонстрация записанного ответа, не реальная проверка."</strong>
                        <span>"Данные получены из заранее записанного прогона."</span>
                    </div>
                }
            })
        }}
    }
}

#[component]
pub fn ErrorBanner() -> impl IntoView {
    let store = expect_context::<AppStore>();

    view! {
        {move || {
            store.error.get().map(|error| {
                let retryable = error.retryable;
                let code = error.code.clone();
                let message = error.message;
                view! {
                    <div class="banner banner-error" role="alert">
                        <div class="banner-error-text">
                            <strong>"Ошибка"</strong>
                            <p>{message}</p>
                            {code.map(|code| view! { <code class="error-code">{code}</code> })}
                        </div>
                        {if retryable {
                            Some(view! {
                                <button
                                    type="button"
                                    class="button secondary"
                                    disabled=move || store.is_busy()
                                    on:click=move |_| actions::retry(store)
                                >
                                    "Повторить"
                                </button>
                            })
                        } else {
                            None
                        }}
                    </div>
                }
            })
        }}
    }
}
