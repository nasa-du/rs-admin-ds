//! Argon2id password hashing and the offline password policy
//! (docs/specs/login-system.md §8.1–§8.2).
//!
//! Hashing is CPU- and memory-bound, so every operation runs on a blocking
//! thread behind a permit-limited semaphore. Callers that wait longer than the
//! queue budget receive [`PasswordError::Busy`], which the API maps to 503
//! `service_unavailable`.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use unicode_normalization::UnicodeNormalization;

/// Argon2id memory cost in KiB (64 MiB).
pub const ARGON2_MEMORY_KIB: u32 = 65_536;
/// Argon2id time cost.
pub const ARGON2_TIME_COST: u32 = 3;
/// Argon2id parallelism (lanes).
pub const ARGON2_PARALLELISM: u32 = 4;
/// Hash output length in bytes.
pub const ARGON2_OUTPUT_BYTES: usize = 32;

/// Password length bounds in Unicode code points after NFC normalization.
pub const PASSWORD_MIN_CHARS: usize = 15;
pub const PASSWORD_MAX_CHARS: usize = 128;
/// Password byte cap applied to authentication inputs.
pub const PASSWORD_MAX_BYTES: usize = 512;

/// Maximum time a request waits for a hashing permit before receiving 503.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(2);

/// Password used only to keep unknown-identifier verification as expensive as
/// a real one. It is never accepted by any account.
const DUMMY_PASSWORD: &str = "rs-admin-dummy-password-not-a-credential";

const BLOCKLIST: &str = include_str!("../data/pwdb-top-100000.txt");

static PARSED_BLOCKLIST: OnceLock<HashSet<String>> = OnceLock::new();

/// Why a candidate password does not satisfy the policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyViolation {
    TooShort,
    TooLong,
    Blocklisted,
    EqualsIdentifier,
}

impl PolicyViolation {
    /// Stable reason string exposed in `password_policy_violation.params`.
    pub const fn as_reason(self) -> &'static str {
        match self {
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::Blocklisted => "blocklisted",
            Self::EqualsIdentifier => "equals_identifier",
        }
    }
}

/// Offline password policy: length, blocklist and identifier equality.
///
/// Passwords are never trimmed. NFC normalization happens before every check,
/// and the blocklist/identifier comparisons are case-insensitive.
#[derive(Clone, Debug, Default)]
pub struct PasswordPolicy;

impl PasswordPolicy {
    /// Construct the policy. The embedded blocklist is parsed on first use.
    pub fn new() -> Self {
        Self
    }

    pub fn validate(&self, password: &str, identifiers: &[&str]) -> Result<(), PolicyViolation> {
        let normalized: String = password.nfc().collect();
        let characters = normalized.chars().count();

        if characters < PASSWORD_MIN_CHARS {
            return Err(PolicyViolation::TooShort);
        }
        if characters > PASSWORD_MAX_CHARS || normalized.len() > PASSWORD_MAX_BYTES {
            return Err(PolicyViolation::TooLong);
        }

        let comparison_key = normalize_for_comparison(&normalized);
        if identifiers
            .iter()
            .any(|identifier| normalize_for_comparison(identifier) == comparison_key)
        {
            return Err(PolicyViolation::EqualsIdentifier);
        }
        if blocklist().contains(&comparison_key) {
            return Err(PolicyViolation::Blocklisted);
        }

        Ok(())
    }
}

/// Returns true when an authentication input exceeds the hashing input caps.
/// Callers still perform a dummy verification before rejecting it.
pub fn exceeds_authentication_limit(password: &str) -> bool {
    let normalized: String = password.nfc().collect();
    normalized.chars().count() > PASSWORD_MAX_CHARS || normalized.len() > PASSWORD_MAX_BYTES
}

fn normalize_for_comparison(value: &str) -> String {
    value.nfc().collect::<String>().to_lowercase()
}

/// Passwords are never trimmed or truncated; NFC normalization is the only
/// transformation, applied identically when hashing and verifying.
fn normalize_password(password: &str) -> String {
    password.nfc().collect()
}

fn blocklist() -> &'static HashSet<String> {
    PARSED_BLOCKLIST.get_or_init(|| {
        BLOCKLIST
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .map(normalize_for_comparison)
            .collect()
    })
}

/// Result of checking a password against a stored PHC string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyOutcome {
    /// Whether the password matched the stored hash.
    pub valid: bool,
    /// Whether the stored hash uses parameters below the current policy.
    pub needs_rehash: bool,
    /// A fresh hash to persist, present only when verification succeeded and
    /// the stored hash needs upgrading.
    pub replacement_hash: Option<String>,
}

impl VerifyOutcome {
    fn invalid() -> Self {
        Self {
            valid: false,
            needs_rehash: false,
            replacement_hash: None,
        }
    }
}

/// Errors from the password hashing service. Messages never contain passwords
/// or hash values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    #[error("password hash concurrency must be at least one")]
    InvalidConfiguration,
    #[error("password hashing queue is saturated")]
    Busy,
    #[error("password hashing failed")]
    HashFailed,
    #[error("password hashing task failed")]
    TaskFailed,
}

/// Argon2id hashing behind a bounded concurrency gate.
#[derive(Clone)]
pub struct PasswordService {
    inner: Arc<Inner>,
}

struct Inner {
    argon2: Argon2<'static>,
    dummy_hash: String,
    permits: Semaphore,
    queue_timeout: Duration,
}

impl PasswordService {
    /// Create the service and precompute the dummy hash used for unknown
    /// identifiers. `concurrency` is the maximum number of simultaneous
    /// hashing operations.
    pub fn new(concurrency: u32) -> Result<Self, PasswordError> {
        Self::with_queue_timeout(concurrency, QUEUE_TIMEOUT)
    }

    /// Same as [`PasswordService::new`] with an explicit queue budget. The
    /// budget bounds how long a request waits for a hashing permit before the
    /// API returns 503.
    pub fn with_queue_timeout(
        concurrency: u32,
        queue_timeout: Duration,
    ) -> Result<Self, PasswordError> {
        if concurrency == 0 || queue_timeout.is_zero() {
            return Err(PasswordError::InvalidConfiguration);
        }

        let argon2 = argon2();
        let dummy_hash = argon2
            .hash_password(DUMMY_PASSWORD.as_bytes())
            .map_err(|_| PasswordError::HashFailed)?
            .to_string();

        Ok(Self {
            inner: Arc::new(Inner {
                argon2,
                dummy_hash,
                permits: Semaphore::new(concurrency as usize),
                queue_timeout,
            }),
        })
    }

    /// Hash a new password with the current parameters.
    pub async fn hash(&self, password: &str) -> Result<String, PasswordError> {
        let _permit = self.acquire().await?;
        let inner = Arc::clone(&self.inner);
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || inner.hash_blocking(&password))
            .await
            .map_err(|_| PasswordError::TaskFailed)?
    }

    /// Verify a password and, on success, produce a replacement hash when the
    /// stored parameters are below the current policy.
    pub async fn verify(
        &self,
        password: &str,
        stored_hash: &str,
    ) -> Result<VerifyOutcome, PasswordError> {
        let _permit = self.acquire().await?;
        let inner = Arc::clone(&self.inner);
        let password = password.to_owned();
        let stored_hash = stored_hash.to_owned();
        tokio::task::spawn_blocking(move || inner.verify_blocking(&password, &stored_hash))
            .await
            .map_err(|_| PasswordError::TaskFailed)?
    }

    /// Run a verification against the dummy hash. Used for unknown identifiers
    /// and over-long login inputs so response timing does not reveal whether an
    /// account exists.
    pub async fn verify_dummy(&self) -> Result<(), PasswordError> {
        let _permit = self.acquire().await?;
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || inner.verify_dummy_blocking())
            .await
            .map_err(|_| PasswordError::TaskFailed)?
    }

    async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, PasswordError> {
        match tokio::time::timeout(self.inner.queue_timeout, self.inner.permits.acquire()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(PasswordError::TaskFailed),
            Err(_) => Err(PasswordError::Busy),
        }
    }
}

impl Inner {
    fn verify_blocking(
        &self,
        password: &str,
        stored_hash: &str,
    ) -> Result<VerifyOutcome, PasswordError> {
        let Ok(parsed) = PasswordHash::new(stored_hash) else {
            return Ok(VerifyOutcome::invalid());
        };
        let normalized = normalize_password(password);
        if self
            .argon2
            .verify_password(normalized.as_bytes(), &parsed)
            .is_err()
        {
            return Ok(VerifyOutcome::invalid());
        }

        let needs_rehash = needs_rehash(&parsed);
        let replacement_hash = if needs_rehash {
            Some(self.hash_blocking(password)?)
        } else {
            None
        };

        Ok(VerifyOutcome {
            valid: true,
            needs_rehash,
            replacement_hash,
        })
    }

    fn hash_blocking(&self, password: &str) -> Result<String, PasswordError> {
        let normalized = normalize_password(password);
        self.argon2
            .hash_password(normalized.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|_| PasswordError::HashFailed)
    }

    fn verify_dummy_blocking(&self) -> Result<(), PasswordError> {
        let parsed = PasswordHash::new(&self.dummy_hash).map_err(|_| PasswordError::HashFailed)?;
        // The result is intentionally ignored: this call exists only to spend
        // the same time as a real verification.
        let _ = self
            .argon2
            .verify_password(DUMMY_PASSWORD.as_bytes(), &parsed);
        Ok(())
    }
}

fn argon2() -> Argon2<'static> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(ARGON2_OUTPUT_BYTES),
    )
    .expect("password policy parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// A stored hash needs upgrading when it is not Argon2id v19 with at least the
/// configured cost, parallelism and output size.
fn needs_rehash(parsed: &PasswordHash) -> bool {
    if parsed.algorithm.as_str() != "argon2id" || parsed.version != Some(19) {
        return true;
    }

    let parameter = |name: &str| {
        parsed
            .params
            .get(name)
            .and_then(|value| value.decimal().ok())
            .unwrap_or(0)
    };
    let output_bytes = parsed.hash.as_ref().map(|hash| hash.len()).unwrap_or(0);

    parameter("m") < ARGON2_MEMORY_KIB
        || parameter("t") < ARGON2_TIME_COST
        || parameter("p") < ARGON2_PARALLELISM
        || output_bytes < ARGON2_OUTPUT_BYTES
}
