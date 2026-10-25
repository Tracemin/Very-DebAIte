use std::net::SocketAddr;
use std::sync::Arc;

use server::config::Config;
use server::llm::openai::OpenAiClient;
use server::llm::replay::ReplayLlm;
use server::llm::{LlmClient, UnavailableLlm};
use server::{build_router, AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration error: {error}");
            std::process::exit(2);
        }
    };

    let filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let llm: Arc<dyn LlmClient> = if config.demo_mode == shared::DemoMode::Replay {
        Arc::new(ReplayLlm::new(config.fixtures_dir.clone()))
    } else if config.llm_configured() {
        Arc::new(OpenAiClient::new(
            config
                .llm_base_url
                .clone()
                .expect("checked by llm_configured"),
            config
                .llm_api_key
                .clone()
                .expect("checked by llm_configured"),
        )?)
    } else {
        tracing::warn!("LLM is not configured; /ready will report degraded");
        Arc::new(UnavailableLlm)
    };

    let bind_addr = config.bind_addr;
    let demo_mode = config.demo_mode;
    let llm_configured = config.llm_configured();
    let state = AppState::new(config, llm);
    state.spawn_gc();

    let router = build_router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    tracing::info!(
        %bind_addr,
        demo_mode = demo_mode.as_str(),
        llm_configured,
        "Very-DebAIte server listening"
    );

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
