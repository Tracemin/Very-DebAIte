//! Axum router and HTTP handlers.

pub mod check;
pub mod ws;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::HeaderValue;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use shared::{HealthResponse, ReadyResponse, ReadyStatus};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::limits;
use crate::state::AppState;

/// Client address from the TCP connection. Deliberately does not trust
/// `X-Forwarded-For`: the demo is bound to loopback.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

#[async_trait::async_trait]
impl FromRequestParts<Arc<AppState>> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        Ok(Self(ip))
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route(
            "/api/check",
            post(check::create_check)
                .layer(axum::extract::DefaultBodyLimit::max(limits::MAX_BODY_BYTES)),
        )
        .route(
            "/api/check/:id",
            get(check::get_check).delete(check::delete_check),
        )
        .route("/ws/:id", get(ws::ws_handler));

    if !state.config.allowed_origins.is_empty() {
        let origins: Vec<HeaderValue> = state
            .config
            .allowed_origins
            .iter()
            .filter_map(|origin| HeaderValue::from_str(origin).ok())
            .collect();
        let cors = CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([CONTENT_TYPE]);
        router = router.layer(cors);
    }

    router
        .fallback_service(ServeDir::new(&state.config.dist_dir))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(HealthResponse::default())
}

async fn ready(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    let response = state.ready().await;
    let status = if response.status == ReadyStatus::Ready {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json::<ReadyResponse>(response))
}

/// Exposed for the integration tests to assert the CORS preflight.
pub fn cors_origin_header(origin: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(origin).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[test]
    fn cors_helper_accepts_valid_origin() {
        assert!(cors_origin_header("http://localhost:8080").is_some());
    }

    #[tokio::test]
    async fn health_is_always_ok() {
        let config = crate::config::Config {
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            llm_base_url: None,
            llm_api_key: None,
            llm_model_debater: String::new(),
            llm_model_judge: String::new(),
            allowed_origins: Vec::new(),
            demo_mode: shared::DemoMode::Off,
            max_concurrent_checks: 20,
            rate_limit: crate::config::RateLimitConfig::default(),
            log_level: "info".to_owned(),
            dist_dir: std::path::PathBuf::from("dist-not-present"),
            fixtures_dir: std::path::PathBuf::from("fixtures"),
        };
        let state = AppState::new(config, Arc::new(crate::llm::UnavailableLlm));
        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
