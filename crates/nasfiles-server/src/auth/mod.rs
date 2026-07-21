pub mod local;
pub mod middleware;
pub mod oidc;
pub mod permission_grace;
pub mod refresh;
pub mod session;
pub mod share_audit;
pub mod share_reconcile;

use axum::http::HeaderMap;

// @tour comment The rate limiter's input is attacker-influenced
// `X-Forwarded-For` is client-appendable, so this takes the entry at `len -
// trusted_proxy_depth` rather than the leftmost one, and returns `None` entirely when the
// depth is `0`.
//
// Both misconfigurations weaken login throttling: too low and an attacker's forged leading
// entries count as distinct IPs; `0` behind a real proxy disables per-IP throttling
// completely, since `is_login_rate_limited_by_ip` treats `None` as "not limited". The unit
// tests below encode the intended semantics.

/// Extract the real client IP from proxy headers, honoring the configured
/// trusted-proxy depth.
///
/// `X-Forwarded-For` is a client-appendable list — each proxy appends the
/// address it received the connection from, so a direct client can forge
/// arbitrary leading entries. Only the rightmost `trusted_proxy_depth` entries
/// are written by infrastructure we control, so the real client sits at
/// `len - depth`. With `depth == 0` no proxy is trusted and the headers are
/// ignored entirely (`None`), so a client that can reach the server directly
/// cannot spoof its address. `X-Real-IP` is used as a fallback (set by nginx or
/// an explicit Traefik middleware) and is likewise only consulted when a proxy
/// is trusted.
pub fn client_ip(headers: &HeaderMap, trusted_proxy_depth: u8) -> Option<String> {
    if trusted_proxy_depth == 0 {
        return None;
    }
    let depth = trusted_proxy_depth as usize;
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let parts: Vec<&str> = xff.split(',').collect();
        let idx = parts.len().saturating_sub(depth);
        if let Some(ip) = parts.get(idx).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return Some(ip.to_string());
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod client_ip_tests {
    use super::client_ip;
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        use axum::http::{HeaderName, HeaderValue};
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn depth_zero_ignores_headers() {
        let h = headers(&[("x-forwarded-for", "1.2.3.4"), ("x-real-ip", "1.2.3.4")]);
        assert_eq!(client_ip(&h, 0), None);
    }

    #[test]
    fn depth_one_takes_rightmost_xff_entry() {
        // Client-spoofed "9.9.9.9" is to the left of the entry our proxy appended.
        let h = headers(&[("x-forwarded-for", "9.9.9.9, 203.0.113.7")]);
        assert_eq!(client_ip(&h, 1).as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn depth_two_skips_two_trusted_hops() {
        let h = headers(&[("x-forwarded-for", "9.9.9.9, 203.0.113.7, 10.0.0.1")]);
        assert_eq!(client_ip(&h, 2).as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn falls_back_to_x_real_ip() {
        let h = headers(&[("x-real-ip", "198.51.100.5")]);
        assert_eq!(client_ip(&h, 1).as_deref(), Some("198.51.100.5"));
    }
}
