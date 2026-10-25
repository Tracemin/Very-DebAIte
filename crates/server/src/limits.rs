//! Numeric limits from `docs/SPEC.md` §8. Changing a value here requires a
//! corresponding change in the specification.

use std::time::Duration;

pub const CLAIM_MIN_CHARS: usize = 3;
pub const CLAIM_MAX_CHARS: usize = 4_000;

pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 6 * 1024 * 1024;

pub const MAX_EXTRACTED_TEXT_CHARS: usize = 24_000;
pub const MAX_DEBATER_DOCUMENT_CHARS: usize = 12_000;
pub const MAX_JUDGE_MESSAGE_CHARS: usize = 6_000;

pub const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const PROVIDER_FIRST_TOKEN_TIMEOUT: Duration = Duration::from_secs(20);
pub const DEBATER_TIMEOUT: Duration = Duration::from_secs(45);
pub const JUDGE_TIMEOUT: Duration = Duration::from_secs(45);
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(120);

pub const LLM_RETRIES: usize = 1;
pub const LLM_RETRY_BACKOFF: Duration = Duration::from_millis(1_500);
pub const LLM_MAX_RETRY_AFTER: Duration = Duration::from_secs(5);

pub const CHECK_TTL: Duration = Duration::from_secs(30 * 60);
pub const CHECK_GC_INTERVAL: Duration = Duration::from_secs(60);
pub const MAX_CHECKS_STORED: usize = 50;

pub const MAX_CONCURRENT_CHECKS_DEFAULT: usize = 20;
pub const GLOBAL_CHECKS_PER_HOUR: usize = 60;

pub const WS_PING_INTERVAL: Duration = Duration::from_secs(20);
pub const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
pub const WS_CHANNEL_CAPACITY: usize = 256;
pub const WS_CLOSE_AFTER_TERMINAL: Duration = Duration::from_millis(500);

pub const DELTA_COALESCE_INTERVAL: Duration = Duration::from_millis(100);

pub const PDF_PARSE_TIMEOUT: Duration = Duration::from_secs(10);
pub const READY_CACHE_TTL: Duration = Duration::from_secs(30);
