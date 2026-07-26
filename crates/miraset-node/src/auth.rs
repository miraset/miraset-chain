use miraset_core::ObjectId;
use rand::rand_core::TryRng;
use rand::rngs::SysRng;

/// Shared-secret authentication for node->worker job dispatch.
///
/// Uses BLAKE3 keyed hashing for a compact, deterministic authentication tag
/// over the dispatch payload. Both the node and worker must be configured
/// with the same 32-byte secret (e.g. via `MIRASET_DISPATCH_SECRET`).
///
/// This is a devnet-level remediation (H4/M2). Production deployments should
/// replace it with mTLS or per-worker Ed25519 signatures.
pub struct DispatchAuth;

impl DispatchAuth {
    /// Generate a new random 32-byte shared secret.
    ///
    /// # Errors
    /// Returns an error if the system RNG cannot provide entropy.
    pub fn generate_secret() -> anyhow::Result<[u8; 32]> {
        let mut secret = [0u8; 32];
        SysRng
            .try_fill_bytes(&mut secret)
            .map_err(|e| anyhow::anyhow!("system RNG failed: {}", e))?;
        Ok(secret)
    }

    /// Canonical input bytes for the dispatch authentication tag.
    fn tag_input(
        job_id: &ObjectId,
        worker_id: &ObjectId,
        epoch_id: u64,
        model_id: &str,
        max_tokens: u64,
    ) -> Vec<u8> {
        let mut input = Vec::new();
        input.extend_from_slice(job_id.as_ref());
        input.extend_from_slice(worker_id.as_ref());
        input.extend_from_slice(&epoch_id.to_le_bytes());
        input.extend_from_slice(model_id.as_bytes());
        input.extend_from_slice(&max_tokens.to_le_bytes());
        input
    }

    /// Produce a 32-byte authentication tag for a dispatch request.
    pub fn sign_dispatch(
        secret: &[u8; 32],
        job_id: &ObjectId,
        worker_id: &ObjectId,
        epoch_id: u64,
        model_id: &str,
        max_tokens: u64,
    ) -> [u8; 32] {
        let input = Self::tag_input(job_id, worker_id, epoch_id, model_id, max_tokens);
        blake3::keyed_hash(secret, &input).into()
    }

    /// Verify an authentication tag.
    pub fn verify_dispatch(
        secret: &[u8; 32],
        job_id: &ObjectId,
        worker_id: &ObjectId,
        epoch_id: u64,
        model_id: &str,
        max_tokens: u64,
        tag: &[u8; 32],
    ) -> bool {
        let expected =
            Self::sign_dispatch(secret, job_id, worker_id, epoch_id, model_id, max_tokens);
        crate::util::constant_time_eq(&expected, tag)
    }
}

/// Parse the scheme and host from an endpoint string without pulling in `url`.
fn parse_endpoint(endpoint: &str) -> Result<(&str, &str, u16), crate::error::StateError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(crate::error::StateError::Other(
            "worker endpoint must not be empty".to_string(),
        ));
    }

    let rest = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .ok_or_else(|| {
            crate::error::StateError::Other(
                "worker endpoint scheme must be http or https".to_string(),
            )
        })?;

    let scheme = if endpoint.starts_with("https://") {
        "https"
    } else {
        "http"
    };

    // Split off path/query if present.
    let host_port = rest.split_once('/').map(|(hp, _)| hp).unwrap_or(rest);
    let host_port = host_port
        .split_once('?')
        .map(|(hp, _)| hp)
        .unwrap_or(host_port);

    // Handle IPv6 bracketed literals, e.g. `[::1]:8080`.
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let stripped = h.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
        if let Some(ipv6) = stripped {
            let port = p.parse::<u16>().map_err(|_| {
                crate::error::StateError::Other(format!("invalid endpoint port: {}", endpoint))
            })?;
            (ipv6, port)
        } else {
            let port = p.parse::<u16>().map_err(|_| {
                crate::error::StateError::Other(format!("invalid endpoint port: {}", endpoint))
            })?;
            (h, port)
        }
    } else {
        let host = host_port
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host_port);
        (host, if scheme == "https" { 443 } else { 80 })
    };

    if host.is_empty() {
        return Err(crate::error::StateError::Other(
            "worker endpoint must have a host".to_string(),
        ));
    }

    Ok((scheme, host, port))
}

/// Validate a worker endpoint URL at registration time.
///
/// Remediation for M2: reject obviously dangerous endpoints (non-HTTP(S)
/// schemes, loopback, link-local, and private ranges) unless explicitly
/// allowed. Returns the validated URL on success.
pub fn validate_worker_endpoint(
    endpoint: &str,
    allow_private: bool,
) -> Result<String, crate::error::StateError> {
    use std::net::IpAddr;

    let (scheme, host, _port) = parse_endpoint(endpoint)?;

    if scheme != "http" && scheme != "https" {
        return Err(crate::error::StateError::Other(format!(
            "worker endpoint scheme must be http or https, got {}",
            scheme
        )));
    }

    if !allow_private {
        let lower = host.to_ascii_lowercase();
        if lower == "localhost"
            || lower == "127.0.0.1"
            || lower == "::1"
            || lower.ends_with(".local")
            || lower.ends_with(".localhost")
        {
            return Err(crate::error::StateError::Other(format!(
                "worker endpoint {} resolves to a loopback/local address; use --allow-private-endpoints or bind to a public interface",
                endpoint
            )));
        }

        // Try to parse as an IP address.
        if let Ok(ip) = host.parse::<IpAddr>() {
            let is_link_local = match ip {
                IpAddr::V4(v4) => {
                    let o = v4.octets();
                    o[0] == 169 && o[1] == 254
                }
                IpAddr::V6(v6) => {
                    let s = v6.segments();
                    (s[0] & 0xFFC0) == 0xFE80
                }
            };
            let is_documentation_v4 = match ip {
                IpAddr::V4(v4) => {
                    let o = v4.octets();
                    // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
                    (o[0] == 192 && o[1] == 0 && o[2] == 2)
                        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
                }
                IpAddr::V6(_) => false,
            };
            if ip.is_loopback()
                || is_link_local
                || ip.is_multicast()
                || ip.is_unspecified()
                || (ip.is_ipv4() && crate::util::is_private_ipv4(ip))
                || is_documentation_v4
            {
                return Err(crate::error::StateError::Other(format!(
                    "worker endpoint {} uses a loopback/link-local/private/multicast address",
                    endpoint
                )));
            }
        }
    }

    Ok(endpoint.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_dispatch_auth_roundtrip() {
        let secret = DispatchAuth::generate_secret().unwrap();
        let job_id = [1u8; 32];
        let worker_id = [2u8; 32];
        let tag = DispatchAuth::sign_dispatch(&secret, &job_id, &worker_id, 7, "model", 128);
        assert!(DispatchAuth::verify_dispatch(
            &secret, &job_id, &worker_id, 7, "model", 128, &tag
        ));
        assert!(!DispatchAuth::verify_dispatch(
            &secret, &job_id, &worker_id, 8, "model", 128, &tag
        ));
    }

    #[test]
    fn test_endpoint_validation_rejects_localhost() {
        assert!(validate_worker_endpoint("http://localhost:8080/jobs", false).is_err());
        assert!(validate_worker_endpoint("http://127.0.0.1:8080/jobs", false).is_err());
        assert!(validate_worker_endpoint("http://[::1]:8080/jobs", false).is_err());
    }

    #[test]
    fn test_endpoint_validation_accepts_public() {
        assert!(validate_worker_endpoint("https://worker.example.com/jobs", false).is_ok());
        assert!(validate_worker_endpoint("http://8.8.8.8:8080/jobs", false).is_ok());
    }

    #[test]
    fn test_endpoint_validation_allow_private() {
        assert!(validate_worker_endpoint("http://127.0.0.1:8080/jobs", true).is_ok());
    }
}
