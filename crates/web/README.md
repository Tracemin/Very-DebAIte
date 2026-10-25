# Very-DebAIte — web

Leptos CSR frontend for the fact-checker. No SSR, no hydration, no
`cargo-leptos`.

## Run

```bash
# dev server on http://127.0.0.1:8080
trunk serve --open

# production bundle in crates/web/dist/
trunk build --release
```

During `trunk serve` the client talks to the Axum backend on port `3000`
(`http://127.0.0.1:3000`). When `dist/` is served by Axum itself, all API and
WebSocket requests are same-origin.

## Backend contract

- `POST /api/check` — `multipart/form-data` with `claim` and optional `file`.
- `GET /api/check/{id}` — restores the last check after a page reload.
- `GET /ws/{id}` — text frames carrying `WsEnvelope` from `crates/shared`.

The contract lives in `docs/SPEC.md` §4 and §6. Types are shared with the
server through `crates/shared`; do not duplicate them in this crate.

## UI state

`localStorage` key `very_debate_last_check_id` stores the last check id. On
startup the client calls `GET /api/check/{id}` and then reconnects to the WS.
A stale id is cleared instead of starting an endless reconnect loop.

## Styles

The MVP uses a plain-CSS fallback (`styles.css`) as allowed by `docs/SPEC.md`
§11. Markup keeps semantic class names so Tailwind can be introduced later
without changing components.
