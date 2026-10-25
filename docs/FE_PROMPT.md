# Промпт для FE-разработчика (Leptos UI, WS-клиент, статика)

## 🎯 Твоя роль
Ты — опытный фронтенд-разработчик на Rust-стеке. Специализируешься на реактивных UI-фреймворках и работе с WebSocket.

## 📋 Контекст проекта
- **Проект**: Very-DebAIte — мультиагентный фактчекер
- **Стек**: Leptos (CSR, не SSR), Trunk, TailwindCSS
- **Архитектура**: workspace из трёх крейтов (`crates/shared`, `crates/server`, `crates/web`)
- **Контракт**: зафиксирован в `docs/SPEC.md`, версия 2.0, бюджет 5 часов
- **Frontend-сервер**: Axum отдаёт статiku и API с порта :3000

## 🚀 Задачи на спринт (00:00–00:40)

### 1. Инициализация web-крейта
```bash
# Настроить crates/web с Cargo.toml
# Leptos 0.6+ (CSR-режим), не SSR
# Trunk для сборки, TailwindCSS через CDN или postcss
```

### 2. Leptos + Trunk setup
```bash
trunk serve --open --port 8080
```
- **Важно**: CSR-режим, без гидрации и SSR
- Сборка: `trunk build --release`
- Статика попадает в `dist/`

### 3. Hello-World страница
- Минимальная страница с:
  - `<h1>Very-DebAIte — Fact Checker</h1>`
  - `<p>Введите URL для проверки:</p>`
  - `<input type="url" placeholder="https://example.com/news">`
  - `<button>Проверить факт</button>`
- Стили: TailwindCSS (`class: "p-4 mb-4"`, и т.д.)

### 4. Сборка и деплой статики
- Команда: `trunk build --release`
- Статика попадает в `dist/`
- **Важно**: Axum будет отдавать `dist/` с порта :3000

### 5. WS-клиент-заглушка
- Подключение к `wss://localhost:3000/ws/{id}`
- Обработка событий:
  - `CheckStarted` — показать логи
  - `AgentProgress` — добавлять новые строки в лог
  - `CheckComplete` — показать вердикт
- UI-компоненты:
  - `<form>` для отправки `/api/check`
  - `<ul>` для логов событий
  - `<div>` для вердикта (зелёный/красный)

### 6. Интеграция с сервером
- POST `/api/check` — отправка запроса
- GET `/ws/{id}` — WebSocket-клиент
- Хранение состояния: `localStorage` для ID проверки
- Таймауты: 30 сек на агента, 2 мин общая

## 📌 Технические требования
- **Язык**: Rust, Leptos 0.6+
- **Компильция**: `trunk serve` (dev), `trunk build --release` (prod)
- **Стили**: TailwindCSS (классы в `class=""`)
- **Коммуникация**: WebSocket через `wasm-bindgen`
- **Структура**: компоненты + store (Rx или Context)

## ⚠️ Важные ограничения
- **Только CSR** — нет SSR, гидрации, `cargo-leptos`
- **Один origin** — localhost:3000 для демо
- **Без авторизации**
- **localStorage** для хранения ID проверок

## 🎨 UI-компоненты
```rust
// Пример структуры
pub mod components {
    pub mod button;
    pub mod input;
    pub mod verdict;
    pub mod logs;
}

pub mod store {
    pub use leptos::*;
    pub struct AppState {
        pub current_check_id: Option<String>,
        pub logs: Vec<LogEvent>,
        pub verdict: Option<Verdict>,
    }
}
```

## 📝 Чек-лист завершения
- [ ] `trunk serve` — сервер запущен на :8080
- [ ] `trunk build --release` — сборка без ошибок
- [ ] Страница открывается в браузере
- [ ] Форма отправки работает
- [ ] WebSocket подключается
- [ ] События отображаются в логах
- [ ] Вердикт показывается при завершении

---
**Вопросы по спецификации?** — см. `docs/SPEC.md`, раздел §6 (контракт) и §8 (Frontend)

**Нужна помощь?** — пиши в чат с приложением конкретного компонента
