//! Access control for submissions (TODO 8.4.3; owner decision 2026-11-11).
//!
//! - **API keys.** With a key file configured, `POST /v1/transactions` and `POST /v1/receipts`
//!   require `Authorization: Bearer <token>`. The file stores only SHA-256 hashes of tokens, one
//!   `<name> <sha256-hex>` per line, so it is not itself a secret. Without a key file
//!   submissions are open (devnet) and the node logs a warning at startup.
//! - **Rate limits.** Each client gets a token bucket for submissions: per key name when keys are
//!   configured, otherwise per client IP. The number of tracked clients is bounded.
//!
//! Reads stay open. Payload validity (signatures, nonces, replay) is checked separately by
//! admission; an API key authorizes a client to submit, it does not make its items valid.

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::server::ApiResponse;
use crate::observability::{NodeMetrics, RpcReject};

/// Clients whose buckets are tracked at once; idle (full) buckets are dropped first.
pub const MAX_TRACKED_CLIENTS: usize = 10_000;
const MAX_KEY_NAME_LEN: usize = 64;

/// Hashes of the API tokens a node accepts, by key name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeys {
    entries: Vec<(String, [u8; 32])>,
}

/// Why a key file was refused.
#[derive(Debug, thiserror::Error)]
pub enum KeyFileError {
    #[error("cannot read API key file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("API key file line {line}: {reason}")]
    Line { line: usize, reason: &'static str },
    #[error("API key file has no keys")]
    Empty,
    #[error(
        "API key file {path} is writable by group or others (mode {mode:o}); anyone who can \
         write it can grant themselves access (chmod 600 or 644)"
    )]
    Writable { path: String, mode: u32 },
}

impl ApiKeys {
    /// Load `<name> <sha256-hex>` lines; `#` starts a comment.
    /// The file decides who may submit, so it must not be writable by group or others.
    pub fn load(path: &Path) -> Result<Self, KeyFileError> {
        check_permissions(path)?;
        let text = std::fs::read_to_string(path).map_err(|source| KeyFileError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, KeyFileError> {
        let mut entries: Vec<(String, [u8; 32])> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |reason| KeyFileError::Line {
                line: line_no,
                reason,
            };
            let mut fields = line.split_whitespace();
            let (Some(name), Some(hash), None) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(err("expected `<name> <sha256-hex>`"));
            };
            if !valid_name(name) {
                return Err(err("name must be 1-64 characters of [A-Za-z0-9_.-]"));
            }
            let bytes = hex::decode(hash).map_err(|_| err("hash is not hex"))?;
            let hash: [u8; 32] = bytes
                .try_into()
                .map_err(|_| err("hash must be 32 bytes (SHA-256)"))?;
            if entries.iter().any(|(existing, _)| existing == name) {
                return Err(err("duplicate key name"));
            }
            if entries.iter().any(|(_, existing)| *existing == hash) {
                return Err(err("duplicate key hash (one token, two names)"));
            }
            entries.push((name.to_string(), hash));
        }
        if entries.is_empty() {
            return Err(KeyFileError::Empty);
        }
        Ok(Self { entries })
    }

    /// The key name a presented token belongs to. Every entry is compared in constant time.
    pub fn authenticate(&self, token: &str) -> Option<&str> {
        let presented: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut found = None;
        for (name, hash) in &self.entries {
            if constant_time_eq(hash, &presented) {
                found = Some(name.as_str());
            }
        }
        found
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), KeyFileError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|source| KeyFileError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(KeyFileError::Writable {
            path: path.display().to_string(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<(), KeyFileError> {
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_KEY_NAME_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A new random API token (hex, 256 bits) and the key-file line that accepts it.
pub fn generate_key(name: &str) -> Option<(String, String)> {
    if !valid_name(name) {
        return None;
    }
    let token = hex::encode(rand::random::<[u8; 32]>());
    let line = format!("{name} {}", hex::encode(Sha256::digest(token.as_bytes())));
    Some((token, line))
}

/// Submission rate per client: `rate_per_sec` sustained, `burst` at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateLimit {
    pub rate_per_sec: f64,
    pub burst: u32,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Token buckets keyed by client identity, at most `MAX_TRACKED_CLIENTS` at once. When every
/// tracked bucket is busy, further clients share one overflow bucket: they are limited
/// together rather than locked out, and memory stays bounded.
#[derive(Debug)]
pub struct RateLimiter {
    limit: RateLimit,
    state: Mutex<LimiterState>,
    max_clients: usize,
}

#[derive(Debug, Default)]
struct LimiterState {
    buckets: HashMap<String, Bucket>,
    overflow: Option<Bucket>,
    last_sweep: Option<Instant>,
}

/// Shortest interval between sweeps for idle buckets (each sweep walks the whole map).
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

impl RateLimiter {
    pub fn new(limit: RateLimit) -> Self {
        Self::with_capacity(limit, MAX_TRACKED_CLIENTS)
    }

    fn with_capacity(limit: RateLimit, max_clients: usize) -> Self {
        Self {
            limit,
            state: Mutex::new(LimiterState::default()),
            max_clients,
        }
    }

    /// Take one token for `client`; on refusal, how long until one is available.
    pub fn check(&self, client: &str, now: Instant) -> Result<(), Duration> {
        let RateLimit {
            rate_per_sec,
            burst,
        } = self.limit;
        let capacity = f64::from(burst.max(1));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = &mut *state;
        if !state.buckets.contains_key(client) && state.buckets.len() >= self.max_clients {
            let due = state
                .last_sweep
                .is_none_or(|last| now.saturating_duration_since(last) >= SWEEP_INTERVAL);
            if due {
                // Forget clients whose buckets have refilled: they are idle.
                state.last_sweep = Some(now);
                state
                    .buckets
                    .retain(|_, bucket| refilled(bucket, now, rate_per_sec) < capacity);
            }
        }
        let bucket = if state.buckets.contains_key(client) || state.buckets.len() < self.max_clients
        {
            state.buckets.entry(client.to_string()).or_insert(Bucket {
                tokens: capacity,
                updated: now,
            })
        } else {
            state.overflow.get_or_insert(Bucket {
                tokens: capacity,
                updated: now,
            })
        };
        bucket.tokens = refilled(bucket, now, rate_per_sec).min(capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let wait = (1.0 - bucket.tokens) / rate_per_sec;
            Err(Duration::from_secs_f64(wait.max(0.0)))
        }
    }
}

fn refilled(bucket: &Bucket, now: Instant, rate_per_sec: f64) -> f64 {
    bucket.tokens + now.saturating_duration_since(bucket.updated).as_secs_f64() * rate_per_sec
}

/// Access policy for submission endpoints.
#[derive(Debug, Default)]
pub struct RpcAccess {
    api_keys: Option<ApiKeys>,
    limiter: Option<RateLimiter>,
    metrics: Option<Arc<NodeMetrics>>,
}

impl RpcAccess {
    /// No key and no rate limit (tests, the in-process demo).
    pub fn open() -> Self {
        Self::default()
    }

    pub fn new(api_keys: Option<ApiKeys>, limit: Option<RateLimit>) -> Self {
        Self {
            api_keys,
            limiter: limit.map(RateLimiter::new),
            metrics: None,
        }
    }

    /// Count refusals in the node's metrics.
    pub fn with_metrics(mut self, metrics: Arc<NodeMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn requires_api_key(&self) -> bool {
        self.api_keys.is_some()
    }

    fn reject(&self, reason: RpcReject, response: Response) -> Response {
        if let Some(metrics) = &self.metrics {
            metrics.record_rpc_rejected(reason);
        }
        response
    }
}

fn bearer_token(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

/// Rate-limit identity of a connection without API keys: the IPv4 address, or the IPv6 /64
/// (one host usually controls a whole /64, so per-address buckets would be free to multiply).
/// Behind a reverse proxy every client shares the proxy's address; deployments with a proxy
/// must configure API keys so limits apply per key.
fn client_ip(request: &Request) -> String {
    match request.extensions().get::<ConnectInfo<SocketAddr>>() {
        Some(ConnectInfo(addr)) => match addr.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => v4.to_string(),
                None => {
                    let s = v6.segments();
                    format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
                }
            },
        },
        None => "unknown".to_string(),
    }
}

/// Middleware for submission routes: authenticate (when keys are configured), then rate limit.
pub async fn guard_submission(
    State(access): State<Arc<RpcAccess>>,
    request: Request,
    next: Next,
) -> Response {
    let client = match &access.api_keys {
        Some(keys) => match bearer_token(&request).and_then(|token| keys.authenticate(token)) {
            Some(name) => format!("key:{name}"),
            None => {
                let mut response = (
                    StatusCode::UNAUTHORIZED,
                    Json(ApiResponse::<()>::err("missing or unknown API key")),
                )
                    .into_response();
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                return access.reject(RpcReject::Unauthorized, response);
            }
        },
        None => format!("ip:{}", client_ip(&request)),
    };
    if let Some(Err(wait)) = access
        .limiter
        .as_ref()
        .map(|limiter| limiter.check(&client, Instant::now()))
    {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::<()>::err("submission rate limit exceeded")),
        )
            .into_response();
        let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
        if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return access.reject(RpcReject::RateLimited, response);
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(name: &str, token: &str) -> String {
        format!("{name} {}", hex::encode(Sha256::digest(token.as_bytes())))
    }

    #[test]
    fn test_key_file_accepts_known_tokens_only() {
        let text = format!(
            "# control plane\n{}\n\n{}  # wallet\n",
            line("control-plane", "secret-a"),
            line("wallet_1", "secret-b")
        );
        let keys = ApiKeys::parse(&text).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys.authenticate("secret-a"), Some("control-plane"));
        assert_eq!(keys.authenticate("secret-b"), Some("wallet_1"));
        assert_eq!(keys.authenticate("secret-c"), None);
        assert_eq!(keys.authenticate(""), None);
    }

    #[test]
    fn test_malformed_key_files_are_refused() {
        let hash = hex::encode([0u8; 32]);
        for bad in [
            String::new(),
            "# only comments\n".to_string(),
            "name-only\n".to_string(),
            format!("bad name {hash}\n"),
            format!("a {hash} extra\n"),
            "a deadbeef\n".to_string(),
            "a not-hex\n".to_string(),
            format!("a {hash}\na {hash}\n"),
            format!("a {hash}\nb {hash}\n"),
            format!("{} {hash}\n", "x".repeat(65)),
        ] {
            assert!(ApiKeys::parse(&bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn test_generated_key_round_trips_through_the_key_file() {
        let (token, file_line) = generate_key("ops").unwrap();
        let keys = ApiKeys::parse(&file_line).unwrap();
        assert_eq!(keys.authenticate(&token), Some("ops"));
        assert!(!file_line.contains(&token), "the file holds only the hash");
        assert!(generate_key("bad name").is_none());
    }

    #[test]
    fn test_bucket_allows_burst_then_refills_at_the_rate() {
        let limiter = RateLimiter::new(RateLimit {
            rate_per_sec: 2.0,
            burst: 3,
        });
        let start = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check("a", start).is_ok());
        }
        let wait = limiter.check("a", start).unwrap_err();
        assert!(wait > Duration::ZERO && wait <= Duration::from_millis(500));
        // Other clients have their own bucket.
        assert!(limiter.check("b", start).is_ok());
        // Half a second refills one token at 2/s.
        assert!(
            limiter
                .check("a", start + Duration::from_millis(500))
                .is_ok()
        );
        assert!(
            limiter
                .check("a", start + Duration::from_millis(500))
                .is_err()
        );
    }

    #[test]
    fn test_tracked_clients_are_bounded_and_extra_ones_share_an_overflow_bucket() {
        let limit = RateLimit {
            rate_per_sec: 1.0,
            burst: 2,
        };
        let limiter = RateLimiter::with_capacity(limit, 2);
        let start = Instant::now();
        assert!(limiter.check("a", start).is_ok());
        assert!(limiter.check("b", start).is_ok());
        // Both buckets are busy: newcomers are not locked out but share one overflow bucket.
        assert!(limiter.check("c", start).is_ok());
        assert!(limiter.check("d", start).is_ok());
        assert!(limiter.check("e", start).is_err());
        let tracked = limiter.state.lock().unwrap().buckets.len();
        assert_eq!(tracked, 2);
        // Once tracked buckets have refilled they are idle and make room for a new client.
        assert!(limiter.check("f", start + Duration::from_secs(5)).is_ok());
        assert!(limiter.state.lock().unwrap().buckets.contains_key("f"));
    }

    #[test]
    fn test_key_file_writable_by_others_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        std::fs::write(&path, line("ops", "t")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(matches!(
            ApiKeys::load(&path),
            Err(KeyFileError::Writable { .. })
        ));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(ApiKeys::load(&path).unwrap().authenticate("t"), Some("ops"));
    }

    #[test]
    fn test_ipv6_clients_are_keyed_by_their_slash_64() {
        let request = |ip: &str| {
            let mut req = Request::new(axum::body::Body::empty());
            let addr: SocketAddr = format!("[{ip}]:1").parse().unwrap();
            req.extensions_mut().insert(ConnectInfo(addr));
            req
        };
        assert_eq!(
            client_ip(&request("2001:db8:1:2:aaaa::1")),
            client_ip(&request("2001:db8:1:2:bbbb::9"))
        );
        assert_ne!(
            client_ip(&request("2001:db8:1:2::1")),
            client_ip(&request("2001:db8:1:3::1"))
        );
        assert_eq!(client_ip(&request("::ffff:10.0.0.1")), "10.0.0.1");
    }
}
