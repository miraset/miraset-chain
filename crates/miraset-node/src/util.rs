/// Small utility helpers used across the node crate.

/// Constant-time equality for byte arrays.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// True if the IPv4 address is in a private (RFC1918) range.
pub fn is_private_ipv4(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            // 10.0.0.0/8
            o[0] == 10
                // 172.16.0.0/12
                || (o[0] == 172 && (o[1] & 0xF0) == 16)
                // 192.168.0.0/16
                || (o[0] == 192 && o[1] == 168)
                // 169.254.0.0/16 (link-local, also handled by IpAddr::is_link_local)
                || (o[0] == 169 && o[1] == 254)
        }
        std::net::IpAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn test_private_ipv4() {
        assert!(is_private_ipv4("10.0.0.1".parse().unwrap()));
        assert!(is_private_ipv4("172.16.0.1".parse().unwrap()));
        assert!(is_private_ipv4("192.168.1.1".parse().unwrap()));
        assert!(!is_private_ipv4("8.8.8.8".parse().unwrap()));
    }
}
