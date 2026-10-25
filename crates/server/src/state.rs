//! Shared application state: check store, LLM client, readiness cache and
//! limits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use shared::{DemoMode, ErrorCode, LlmReady, ReadyResponse, ReadyStatus};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::limits;
use crate::llm::LlmClient;
use crate::orchestrator::CheckRecord;
use crate::rate_limit::RateLimiter;

#[derive(Debug, Clone)]
struct ReadyCache {
    checked_at: Option<Instant>,
    response: Option<ReadyResponse>,
}

pub struct AppState {
    pub config: Arc<Config>,
    pub llm: Arc<dyn LlmClient>,
    pub checks: Arc<RwLock<HashMap<Uuid, Arc<CheckRecord>>>>,
    pub limiter: Arc<RateLimiter>,
    ready_cache: Mutex<ReadyCache>,
}

impl AppState {
    pub fn new(config: Config, llm: Arc<dyn LlmClient>) -> Arc<Self> {
        let limiter = RateLimiter::new(config.rate_limit.clone(), config.max_concurrent_checks);
        Arc::new(Self {
            config: Arc::new(config),
            llm,
            checks: Arc::new(RwLock::new(HashMap::new())),
            limiter,
            ready_cache: Mutex::new(ReadyCache {
                checked_at: None,
                response: None,
            }),
        })
    }

    pub async fn insert_check(&self, record: Arc<CheckRecord>) -> AppResult<()> {
        let mut checks = self.checks.write().await;
        if checks.len() >= limits::MAX_CHECKS_STORED {
            let victim = checks
                .values()
                .filter_map(|record| record.terminal_at().map(|at| (at, record.id)))
                .min_by_key(|(at, _)| *at)
                .map(|(_, id)| id);

            match victim {
                Some(id) => {
                    checks.remove(&id);
                }
                None => {
                    return Err(AppError::new(
                        ErrorCode::Overloaded,
                        "Хранилище проверок заполнено",
                    )
                    .with_retry_after(30));
                }
            }
        }
        checks.insert(record.id, record);
        Ok(())
    }

    pub async fn get_check(&self, id: Uuid) -> Option<Arc<CheckRecord>> {
        self.checks.read().await.get(&id).cloned()
    }

    pub async fn remove_check(&self, id: Uuid) {
        self.checks.write().await.remove(&id);
    }

    /// Remove checks whose TTL after the terminal event has expired.
    pub async fn gc(&self) {
        let now = Instant::now();
        let mut checks = self.checks.write().await;
        checks.retain(|_, record| match record.terminal_at() {
            Some(terminal_at) => now.duration_since(terminal_at) < limits::CHECK_TTL,
            None => {
                now.duration_since(record.created_at) < limits::CHECK_TTL + limits::CHECK_TIMEOUT
            }
        });
    }

    pub fn spawn_gc(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(limits::CHECK_GC_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                interval.tick().await;
                state.gc().await;
            }
        });
    }

    pub fn llm_configured(&self) -> bool {
        self.config.demo_mode == DemoMode::Replay || self.config.llm_configured()
    }

    pub async fn ready(&self) -> ReadyResponse {
        if let Ok(cache) = self.ready_cache.lock() {
            if let (Some(checked_at), Some(response)) = (cache.checked_at, &cache.response) {
                if checked_at.elapsed() < limits::READY_CACHE_TTL {
                    return response.clone();
                }
            }
        }

        let configured = self.llm_configured();
        let llm = if !configured {
            LlmReady {
                configured: false,
                reachable: false,
                error: Some("not_configured".to_owned()),
            }
        } else {
            match tokio::time::timeout(limits::PROVIDER_CONNECT_TIMEOUT, self.llm.probe()).await {
                Ok(Ok(())) => LlmReady {
                    configured: true,
                    reachable: true,
                    error: None,
                },
                Ok(Err(error)) => LlmReady {
                    configured: true,
                    reachable: false,
                    error: Some(error.code().as_str().to_owned()),
                },
                Err(_) => LlmReady {
                    configured: true,
                    reachable: false,
                    error: Some(ErrorCode::ProviderTimeout.as_str().to_owned()),
                },
            }
        };

        let response = ReadyResponse {
            status: if configured && llm.reachable {
                ReadyStatus::Ready
            } else {
                ReadyStatus::Degraded
            },
            demo_mode: self.config.demo_mode == DemoMode::Replay,
            llm,
        };

        if let Ok(mut cache) = self.ready_cache.lock() {
            cache.checked_at = Some(Instant::now());
            cache.response = Some(response.clone());
        }

        response
    }
}
