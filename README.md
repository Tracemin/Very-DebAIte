# Very-DebAIte

Multi-agent fact checker: two debating LLM agents (proponent and skeptic) collect
arguments, a judge produces a verdict, and the whole run is streamed to the
browser over WebSocket. The backend is a Rust workspace with an in-memory store
(30-minute TTL, no database, no authentication) intended to run on localhost.

The authoritative contract is [`docs/SPEC.md`](docs/SPEC.md).

## Layout

```
crates/shared   Wire types, validation, verdict post-processing, canary redaction
crates/server   HTTP API, WebSocket streaming, orchestration, LLM providers
```

`crates/web` is owned by the frontend and is intentionally not part of this
workspace. The server serves a built frontend from `dist/` when it exists.

## Requirements

- Stable Rust (edition 2021)
- An OpenAI-compatible chat-completions provider for real checks

## Run

```bash
cp .env.example .env
# fill LLM_BASE_URL, LLM_API_KEY, LLM_MODEL_DEBATER, LLM_MODEL_JUDGE
cargo run -p server
```

The server binds to `127.0.0.1:3000` by default. Without a configured provider
it still starts; `GET /ready` reports the degradation and new checks are
rejected with a structured error.

### Demo replay

`DEMO_MODE=replay` serves recorded model outputs from `fixtures/`, keyed by
`PROMPT_VERSION` and a hash of the claim and attachment text. `DEMO_MODE=record`
writes fixtures for successful runs. Recordings are provider/model output and
must be reviewed before committing.

## API

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness, always `200` |
| `GET` | `/ready` | Provider and demo-mode readiness |
| `POST` | `/api/check` | Start a check (`multipart/form-data`: `claim`, optional `file`) |
| `GET` | `/api/check/{id}` | Current check result |
| `DELETE` | `/api/check/{id}` | Cancel and delete a check |
| `GET` | `/ws/{id}` | WebSocket: snapshot first, then ordered events |

Checks are in-memory only and expire after 30 minutes. Rate limits are applied
per IP (`RATE_LIMIT_PER_IP`, default `5/10min,2`) plus a global hourly cap.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Configuration

See [`.env.example`](.env.example). Required variables are `LLM_BASE_URL`,
`LLM_API_KEY`, `LLM_MODEL_DEBATER`, and `LLM_MODEL_JUDGE`; the rest have safe
defaults. Secrets belong in the environment only — never in `dist/`, git, or
logs.
