use leptos::*;
use shared::DISCLAIMER;

#[component]
pub fn Disclaimer() -> impl IntoView {
    view! {
        <footer class="disclaimer">
            <p>{DISCLAIMER}</p>
            <p class="muted">
                "Поиск в интернете в этой версии выключен. Источники помечены по способу проверки: HTTP, сниппет или знание модели."
            </p>
        </footer>
    }
}
