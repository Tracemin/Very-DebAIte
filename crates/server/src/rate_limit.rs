//! In-memory per-IP and global limits (`docs/SPEC.md` §8).

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shared::ErrorCode;

use crate::config::RateLimitConfig;
use crate::error::{AppError, AppResult};

#[derive(Debug, Default)]
struct IpBucket {
    requests: VecDeque<Instant>,
    active: usize,
}

#[derive(Debug, Default)]
struct Inner {
    per_ip: HashMap<IpAddr, IpBucket>,
    global_requests: VecDeque<Instant>,
    global_active: usize,
}

#[derive(Debug)]
pub struct RateLimiter {
    inner: Mutex<Inner>,
    config: RateLimitConfig,
    max_concurrent: usize,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig, max_concurrent: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            config,
            max_concurrent,
        })
    }

    /// Reserve a slot for a new check. All limits are checked before any
    /// counter is incremented, so a rejected request leaves no trace.
    pub fn try_acquire(self: &Arc<Self>, ip: IpAddr) -> AppResult<RatePermit> {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");

        inner
            .per_ip
            .retain(|_, bucket| !bucket.requests.is_empty() || bucket.active > 0);
        inner
            .global_requests
            .retain(|timestamp| now.duration_since(*timestamp) < Duration::from_secs(3600));

        if inner.global_active >= self.max_concurrent {
            drop(inner);
            return Err(AppError::new(
                ErrorCode::Overloaded,
                "Сервер уже выполняет максимум одновременных проверок",
            )
            .with_retry_after(5));
        }
        if inner.global_requests.len() >= self.config.global_per_hour {
            drop(inner);
            return Err(AppError::new(
                ErrorCode::Overloaded,
                "Превышен глобальный лимит проверок за час",
            )
            .with_retry_after(60));
        }

        let bucket = inner.per_ip.entry(ip).or_default();
        bucket
            .requests
            .retain(|timestamp| now.duration_since(*timestamp) < self.config.per_ip_window);

        if bucket.active >= self.config.per_ip_concurrent {
            drop(inner);
            return Err(AppError::new(
                ErrorCode::RateLimited,
                "Слишком много одновременных проверок с этого адреса",
            )
            .with_retry_after(5));
        }
        if bucket.requests.len() >= self.config.per_ip_requests {
            let oldest = *bucket.requests.front().expect("checked non-empty");
            let retry_after = self
                .config
                .per_ip_window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            drop(inner);
            return Err(AppError::new(
                ErrorCode::RateLimited,
                "Превышен лимит проверок с этого адреса",
            )
            .with_retry_after(retry_after));
        }

        bucket.requests.push_back(now);
        bucket.active += 1;
        inner.global_requests.push_back(now);
        inner.global_active += 1;

        Ok(RatePermit {
            limiter: Arc::clone(self),
            ip,
        })
    }

    fn release(&self, ip: IpAddr) {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.global_active = inner.global_active.saturating_sub(1);
        if let Some(bucket) = inner.per_ip.get_mut(&ip) {
            bucket.active = bucket.active.saturating_sub(1);
        }
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.inner.lock().unwrap().global_active
    }
}

/// Lives as long as the check is running; dropping it returns the slot.
#[derive(Debug)]
pub struct RatePermit {
    limiter: Arc<RateLimiter>,
    ip: IpAddr,
}

impl Drop for RatePermit {
    fn drop(&mut self) {
        self.limiter.release(self.ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn limiter(requests: usize, concurrent: usize) -> Arc<RateLimiter> {
        RateLimiter::new(
            RateLimitConfig {
                per_ip_requests: requests,
                per_ip_window: Duration::from_secs(600),
                per_ip_concurrent: concurrent,
                global_per_hour: 60,
            },
            20,
        )
    }

    #[test]
    fn limits_requests_per_ip() {
        let limiter = limiter(2, 2);
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let first = limiter.try_acquire(ip).unwrap();
        let second = limiter.try_acquire(ip).unwrap();
        let error = limiter.try_acquire(ip).unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert!(error.retry_after.is_some());
        drop(first);
        drop(second);
    }

    #[test]
    fn limits_concurrent_checks_per_ip() {
        let limiter = limiter(5, 1);
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let permit = limiter.try_acquire(ip).unwrap();
        let error = limiter.try_acquire(ip).unwrap_err();
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert_eq!(limiter.active(), 1);
        drop(permit);
        assert_eq!(limiter.active(), 0);
    }

    #[test]
    fn different_ips_do_not_share_the_per_ip_bucket() {
        let limiter = limiter(1, 1);
        let first = limiter
            .try_acquire(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap();
        let second = limiter
            .try_acquire(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)))
            .unwrap();
        drop(first);
        drop(second);
    }
}
