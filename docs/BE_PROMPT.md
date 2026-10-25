# Промпт для BE-разработчика (оркестратор, LLM-адаптер, события)

## 🎯 Твоя роль
Ты — опытный Rust-разработчик, специализирующийся на асинхронных высоконагруженных сервисах. Твоя задача — реализовать backend для мультиагентного фактчекера.

## 📋 Контекст проекта
- **Проект**: Very-DebAIte — мультиагентный фактчекер
- **Стек**: Rust, Axum, Tokio
- **Архитектура**: workspace из трёх крейтов (`crates/shared`, `crates/server`, `crates/web`)
- **Контракт**: зафиксирован в `docs/SPEC.md`, версия 2.0, бюджет 5 часов
- **Состояние**: только в памяти (без БД), TTL 30 минут
- **Окружение**: localhost-only, без авторизации, с rate limit

## 🚀 Задачи на спринт (00:00–00:40)

### 1. Инициализация workspace
```bash
# Настроить workspace в корне
# Создать crates/shared с Cargo.toml
# Создать crates/server с Cargo.toml
```

### 2. Заморозка контракта (crates/shared)
- Реализовать все типы из §6 спецификации:
  - `AgentResult` — результат работы агента
  - `Verdict` — вердикт проверки (TRUE/FALSE/UNKNOWN/CONTRADICTORY)
  - `CheckSnapshot` — снимок состояния проверки
  - WS-события (`CheckStarted`, `AgentProgress`, `CheckComplete`)
- Использовать `serde` для сериализации
- Добавить derive макросы: `Serialize`, `Deserialize`, `Debug`, `Clone`

### 3. Axum-минимум
- GET `/health` — возвращает `{"status": "ok"}` с 200 OK
- GET `/ready` — возвращает `{"status": "ready"}` с 200 OK
- Конфигурация Axum:
  - `DefaultBodyLimit = 6 MiB`
  - Middleware `tower_http::services::ServeDir` для статики из `dist/`
  - CORS разрешён для `localhost` (или отключён для продакшена)

### 4. LLM-провайдер smoke-тест
- Создать абстракцию для LLM-провайдеров (трейты + traits)
- Реализовать базовый провайдер (например, OpenAI-compatible endpoint)
- Добавить методы:
  - `chat(message: &str) -> Result<String, Error>` — синхронный вызов
  - `stream_chat(message: &str) -> Result<Stream<Item = String>, Error>` — потоковая отдача
- Обработка ошибок: таймауты, rate limits, неудачные ответы
- Логирование вызовов (уровень INFO)

### 5. Оркестратор проверок
- POST `/api/check` — приём запроса с фактами для проверки
- Логика:
  1. Создаём уникальный ID проверки
  2. Сохраняем `CheckSnapshot` в память (hashmap: `id -> snapshot`)
  3. Запускаем агентов параллельно (tokio::spawn)
  4. Агрегируем результаты в `AgentResult`
  5. Вычисляем `Verdict`
  6. Возвращаем JSON с результатом
- Таймауты: 30 сек на агента, 2 мин общая

### 6. WebSocket-оркестратор
- GET `/ws/{id}` — WebSocket-сокет для проверки по ID
- Логика:
  1. Ищем `CheckSnapshot` по ID
  2. Если нет — отправляем `{event: "error", message: "Not found"}` и закрываем
  3. Если есть — отправляем `{event: "CheckStarted", snapshot: ...}`
  4. Слушаем прогресс от агентов и отправляем `{event: "AgentProgress", ...}`
  5. При завершении — `{event: "CheckComplete", result: ...}`
- Используем `tokio-tungstenite` или `axum-ws`

## 📌 Технические требования
- **Язык**: Rust 1.70+
- **Время**: `tokio`, `axum`, `tower-http`, `serde`, `serde_json`
- **Логирование**: `tracing` + `tracing-subscriber`
- **Тесты**: `tokio-test`, `axum-test`, зелёный clippy
- **Форматирование**: `cargo fmt`

## ⚠️ Важные ограничения
- Никакой БД — только `HashMap` для состояния
- TTL 30 минут — чистим неактивные проверки
- Без авторизации — localhost-only
- Rate limit — 10 req/min на endpoint

## 🎨 Стиль кода
- Функции: `<verb>_<noun>` (например, `create_check_snapshot`, `aggregate_results`)
- Константы: `UPPER_SNAKE_CASE`
- Error types: `Result<T, Error>` с `thiserror`
- Комментарии: краткие и по делу (только если нужно пояснить)

## 📝 Чек-лист завершения
- [ ] `cargo fmt` — чистый
- [ ] `cargo clippy` — без warnings
- [ ] `cargo test` — все тесты проходят
- [ ] GET `/health` = 200
- [ ] GET `/ready` = 200
- [ ] POST `/api/check` работает (smoke-test)
- [ ] GET `/ws/{id}` подключается и отправляет события

---
**Вопросы по спецификации?** — см. `docs/SPEC.md`, раздел §6 (контракт) и §7 (API/WS)

**Нужна помощь?** — пиши в чат с приложением конкретного куска кода
