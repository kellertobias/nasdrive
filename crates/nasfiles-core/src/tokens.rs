use base64ct::{Base64UrlUnpadded, Encoding};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// @tour share-management:70 Token generation
// `generate_share_token` fills `byte_length` random bytes and base64url-encodes them
// without padding; `create.rs` passes `config.share_token_bytes`. `hash_token` just below
// SHA-256s the token and hex-encodes it, and `token_prefix` exists so logs can reference a
// token without printing it.
//
// Read `hash_token`'s doc comment carefully, then read the callout on the INSERT in
// `create.rs` — the two do not quite agree.

/// Generate a cryptographically random share token.
/// Returns the raw token bytes base64url-encoded.
pub fn generate_share_token(byte_length: usize) -> String {
    let mut bytes = vec![0u8; byte_length];
    rand::fill(&mut bytes[..]);
    Base64UrlUnpadded::encode_string(&bytes)
}

/// Hash a share token for storage. We never store the raw token in the DB.
pub fn hash_token(token: &str) -> String {
    use sha2::Digest;
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

/// Get a safe token prefix for logging (first 8 chars).
pub fn token_prefix(token: &str) -> &str {
    if token.len() >= 8 { &token[..8] } else { token }
}

/// Create an HMAC-signed bearer token for a guest/public share session.
///
/// Payload: `{share_id}:{iat}:{exp}`
/// The token is stateless — the server validates signature + expiry + share-not-revoked
/// on every request.
pub fn create_bearer_token(
    secret: &[u8],
    share_id: &str,
    ttl_seconds: i64,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp();
    let exp = now + ttl_seconds;
    let payload = format!("{share_id}:{now}:{exp}");

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| anyhow::anyhow!("HMAC error: {e}"))?;
    mac.update(payload.as_bytes());
    let signature = mac.finalize().into_bytes();

    let sig_b64 = Base64UrlUnpadded::encode_string(&signature);
    let payload_b64 = Base64UrlUnpadded::encode_string(payload.as_bytes());

    Ok(format!("{payload_b64}.{sig_b64}"))
}

/// Verify and decode a bearer token.
/// Returns `(share_id, issued_at, expires_at)` if valid.
pub fn verify_bearer_token(secret: &[u8], token: &str) -> anyhow::Result<(String, i64, i64)> {
    let parts: Vec<&str> = token.splitn(2, '.').collect();
    if parts.len() != 2 {
        anyhow::bail!("invalid bearer token format");
    }

    let payload_bytes = Base64UrlUnpadded::decode_vec(parts[0])
        .map_err(|_| anyhow::anyhow!("invalid bearer payload encoding"))?;
    let signature_bytes = Base64UrlUnpadded::decode_vec(parts[1])
        .map_err(|_| anyhow::anyhow!("invalid bearer signature encoding"))?;

    // Verify HMAC
    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| anyhow::anyhow!("HMAC error: {e}"))?;
    mac.update(&payload_bytes);
    mac.verify_slice(&signature_bytes)
        .map_err(|_| anyhow::anyhow!("invalid bearer signature"))?;

    // Parse payload
    let payload =
        String::from_utf8(payload_bytes).map_err(|_| anyhow::anyhow!("invalid bearer payload"))?;
    let parts: Vec<&str> = payload.splitn(3, ':').collect();
    if parts.len() != 3 {
        anyhow::bail!("invalid bearer payload format");
    }

    let share_id = parts[0].to_string();
    let iat: i64 = parts[1]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid iat in bearer"))?;
    let exp: i64 = parts[2]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid exp in bearer"))?;

    // Check expiry
    let now = chrono::Utc::now().timestamp();
    if now > exp {
        anyhow::bail!("bearer token expired");
    }

    Ok((share_id, iat, exp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token_length() {
        let token = generate_share_token(24);
        // 24 bytes → 32 chars in base64url without padding
        assert_eq!(token.len(), 32);
    }

    #[test]
    fn test_generate_tokens_unique() {
        let t1 = generate_share_token(24);
        let t2 = generate_share_token(24);
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_hash_token_deterministic() {
        let h1 = hash_token("test_token");
        let h2 = hash_token("test_token");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_token_different_input() {
        let h1 = hash_token("token_a");
        let h2 = hash_token("token_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_token_prefix() {
        assert_eq!(token_prefix("abcdefghijklmnop"), "abcdefgh");
        assert_eq!(token_prefix("short"), "short");
    }

    #[test]
    fn test_bearer_roundtrip() {
        let secret = b"test_secret_key_at_least_32_bytes_long!!";
        let share_id = "share-123";

        let token = create_bearer_token(secret, share_id, 300).unwrap();
        let (decoded_id, _iat, _exp) = verify_bearer_token(secret, &token).unwrap();

        assert_eq!(decoded_id, share_id);
    }

    #[test]
    fn test_bearer_wrong_secret() {
        let secret1 = b"secret_one_at_least_32_bytes_long!!!!!";
        let secret2 = b"secret_two_at_least_32_bytes_long!!!!!";

        let token = create_bearer_token(secret1, "share-123", 300).unwrap();
        let result = verify_bearer_token(secret2, &token);

        assert!(result.is_err());
    }

    #[test]
    fn test_bearer_expired() {
        let secret = b"test_secret_key_at_least_32_bytes_long!!";
        // Create with -100 second TTL → already expired
        let token = create_bearer_token(secret, "share-123", -100).unwrap();
        let result = verify_bearer_token(secret, &token);
        assert!(result.is_err());
    }
}
