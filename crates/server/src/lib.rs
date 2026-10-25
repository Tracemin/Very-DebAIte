//! Very-DebAIte backend: Axum HTTP API, WebSocket events, orchestrator and
//! OpenAI-compatible LLM adapter.

pub mod config;
pub mod error;
pub mod files;
pub mod http;
pub mod limits;
pub mod llm;
pub mod orchestrator;
pub mod prompts;
pub mod rate_limit;
pub mod state;

pub use config::Config;
pub use http::build_router;
pub use state::AppState;
