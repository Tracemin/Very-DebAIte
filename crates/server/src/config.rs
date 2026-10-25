use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use shared::DemoMode;

use crate::limits;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub per_ip_requests: usize,
    pub per_ip_window: Duration,
    pub per_ip_concurrent: usize,
    pub global_per_hour: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            per_ip_requests: 5,
            per_ip_window: Duration::from_secs(10 * 60),
            per_ip_concurrent: 2,
            global_per_hour: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model_debater: String,
    pub llm_model_judge: String,
    pub allowed_origins: Vec<String>,
    pub demo_mode: DemoMode,
    pub max_concurrent_checks: usize,
    pub rate_limit: RateLimitConfig,
    pub log_level: String,
    pub dist_dir: PathBuf,
    pub fixtures_dir: PathBuf,
}

impl Config {
    /// Read configuration from the environment. The server starts even when
    /// the LLM is not configured, so `/ready` can report the degradation.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr = match std::env::var("BIND_ADDR") {
            Ok(value) if !value.trim().is_empty() => value
                .parse()
                .map_err(|_| ConfigError::Invalid("BIND_ADDR must be a socket address".into()))?,
            _ => "127.0.0.1:3000".parse().expect("static socket address"),
        };

        let llm_base_url =
            non_empty_env("LLM_BASE_URL").map(|value| value.trim_end_matches('/').to_owned());
        let llm_api_key = non_empty_env("LLM_API_KEY");
        let llm_model_debater = non_empty_env("LLM_MODEL_DEBATER").unwrap_or_default();
        let llm_model_judge = non_empty_env("LLM_MODEL_JUDGE").unwrap_or_default();

        let allowed_origins = non_empty_env("ALLOWED_ORIGIN")
            .map(|value| {
                value
                    .split(',')
                    .map(|origin| origin.trim().trim_end_matches('/').to_ascii_lowercase())
                    .filter(|origin| !origin.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let demo_mode = non_empty_env("DEMO_MODE")
            .unwrap_or_else(|| "off".to_owned())
            .parse()
            .map_err(|error: String| ConfigError::Invalid(error))?;

        let max_concurrent_checks = non_empty_env("MAX_CONCURRENT_CHECKS")
            .map(|value| {
                value.parse::<usize>().map_err(|_| {
                    ConfigError::Invalid("MAX_CONCURRENT_CHECKS must be a positive integer".into())
                })
            })
            .transpose()?
            .unwrap_or(limits::MAX_CONCURRENT_CHECKS_DEFAULT);

        if max_concurrent_checks == 0 {
            return Err(ConfigError::Invalid(
                "MAX_CONCURRENT_CHECKS must be greater than zero".into(),
            ));
        }

        let rate_limit = non_empty_env("RATE_LIMIT_PER_IP")
            .map(|value| parse_rate_limit(&value))
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            bind_addr,
            llm_base_url,
            llm_api_key,
            llm_model_debater,
            llm_model_judge,
            allowed_origins,
            demo_mode,
            max_concurrent_checks,
            rate_limit,
            log_level: non_empty_env("LOG_LEVEL").unwrap_or_else(|| "info".to_owned()),
            dist_dir: non_empty_env("DIST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("dist")),
            fixtures_dir: non_empty_env("FIXTURES_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("fixtures")),
        })
    }

    pub fn llm_configured(&self) -> bool {
        self.llm_base_url.is_some()
            && self.llm_api_key.is_some()
            && !self.llm_model_debater.is_empty()
            && !self.llm_model_judge.is_empty()
    }

    /// Origins accepted for WebSocket upgrades. With `ALLOWED_ORIGIN` unset,
    /// only the demo origin derived from `BIND_ADDR` is accepted.
    pub fn origin_allowlist(&self) -> Vec<String> {
        if !self.allowed_origins.is_empty() {
            return self.allowed_origins.clone();
        }
        let port = self.bind_addr.port();
        vec![
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
            format!("http://[::1]:{port}"),
        ]
    }

    pub fn is_origin_allowed(&self, origin: &str) -> bool {
        let origin = origin.trim().trim_end_matches('/').to_ascii_lowercase();
        self.origin_allowlist()
            .iter()
            .any(|allowed| allowed == &origin)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Invalid(String),
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Parses `RATE_LIMIT_PER_IP` in the format `5/10min,2`.
fn parse_rate_limit(value: &str) -> Result<RateLimitConfig, ConfigError> {
    let (requests_window, concurrent) = value
        .split_once(',')
        .ok_or_else(|| ConfigError::Invalid("RATE_LIMIT_PER_IP must look like 5/10min,2".into()))?;
    let (requests, window) = requests_window
        .split_once('/')
        .ok_or_else(|| ConfigError::Invalid("RATE_LIMIT_PER_IP must look like 5/10min,2".into()))?;
    let requests = requests
        .trim()
        .parse::<usize>()
        .map_err(|_| ConfigError::Invalid("RATE_LIMIT_PER_IP request count is invalid".into()))?;
    let concurrent = concurrent
        .trim()
        .parse::<usize>()
        .map_err(|_| ConfigError::Invalid("RATE_LIMIT_PER_IP concurrency is invalid".into()))?;
    let window = parse_duration(window.trim())
        .ok_or_else(|| ConfigError::Invalid("RATE_LIMIT_PER_IP window is invalid".into()))?;

    if requests == 0 || concurrent == 0 {
        return Err(ConfigError::Invalid(
            "RATE_LIMIT_PER_IP values must be greater than zero".into(),
        ));
    }

    Ok(RateLimitConfig {
        per_ip_requests: requests,
        per_ip_window: window,
        per_ip_concurrent: concurrent,
        global_per_hour: 60,
    })
}

fn parse_duration(value: &str) -> Option<Duration> {
    let (number, unit) = value.split_at(value.find(|ch: char| !ch.is_ascii_digit())?);
    let number = number.parse::<u64>().ok()?;
    let seconds = match unit.trim().to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => number,
        "m" | "min" | "mins" | "minute" | "minutes" => number * 60,
        "h" | "hour" | "hours" => number * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_rate_limit_format() {
        let config = parse_rate_limit("5/10min,2").unwrap();
        assert_eq!(config.per_ip_requests, 5);
        assert_eq!(config.per_ip_window, Duration::from_secs(600));
        assert_eq!(config.per_ip_concurrent, 2);
    }

    #[test]
    fn rejects_malformed_rate_limit() {
        assert!(parse_rate_limit("5").is_err());
        assert!(parse_rate_limit("five/10min,2").is_err());
        assert!(parse_rate_limit("5/10parsecs,2").is_err());
    }

    #[test]
    fn origin_allowlist_is_derived_from_bind_addr() {
        let config = Config {
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            llm_base_url: None,
            llm_api_key: None,
            llm_model_debater: String::new(),
            llm_model_judge: String::new(),
            allowed_origins: Vec::new(),
            demo_mode: DemoMode::Off,
            max_concurrent_checks: 20,
            rate_limit: RateLimitConfig::default(),
            log_level: "info".to_owned(),
            dist_dir: PathBuf::from("dist"),
            fixtures_dir: PathBuf::from("fixtures"),
        };
        assert!(config.is_origin_allowed("http://127.0.0.1:3000"));
        assert!(config.is_origin_allowed("http://localhost:3000/"));
        assert!(!config.is_origin_allowed("http://evil.example"));
        assert!(!config.llm_configured());
    }
}
