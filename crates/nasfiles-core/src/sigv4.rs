use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Derive the SigV4 signing key from a secret key, date, region, and service.
/// `date` must be `YYYYMMDD` format.
fn derive_signing_key(secret_key: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let k_secret = format!("AWS4{secret_key}");
    let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Percent-encode a byte as `%XX` (uppercase hex).
fn percent_encode_byte(b: u8) -> String {
    format!("%{:02X}", b)
}

/// SigV4 URI encoding: encode everything except unreserved chars.
/// Unreserved: A-Z a-z 0-9 - _ . ~
/// Used for path segments (does NOT encode `/`) and for query key/value (DOES encode everything).
pub fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&percent_encode_byte(b)),
        }
    }
    out
}

/// Build the canonical query string from a raw query string.
/// Sorts by (encoded key, encoded value).
fn canonical_query_string(raw_query: &str) -> String {
    if raw_query.is_empty() {
        return String::new();
    }

    let mut pairs: Vec<(String, String)> = raw_query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            // Decode then re-encode to normalize percent encoding
            let k_decoded = percent_decode(k);
            let v_decoded = percent_decode(v);
            (uri_encode(&k_decoded, true), uri_encode(&v_decoded, true))
        })
        .collect();

    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Minimal percent-decoder: replaces %XX sequences with the corresponding byte.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Build the canonical headers string.
/// `headers` must already be lowercase; values are trimmed.
fn canonical_headers(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k, v.trim()))
        .collect()
}

/// Signed headers string (sorted lowercase header names joined with `;`).
fn signed_headers_str(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";")
}

/// Build the complete canonical request and return its SHA-256 hex digest.
fn canonical_request_hash(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
) -> String {
    let canon_headers = canonical_headers(headers);
    let signed_hdrs = signed_headers_str(headers);
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canon_headers}\n{signed_hdrs}\n{payload_hash}"
    );
    sha256_hex(canonical_request.as_bytes())
}

/// Verify an AWS SigV4 Authorization-header signature.
///
/// # Parameters
/// - `secret_key`: plaintext S3 secret key
/// - `method`: HTTP method (uppercase, e.g. "GET")
/// - `path`: full URI path as seen by the server (e.g. `/s3/home/file.txt`)
/// - `raw_query`: raw query string (without `?`)
/// - `headers`: **all** lowercase header (name, value) pairs the client claims to have signed,
///   in the order they appear in `SignedHeaders` (must be pre-sorted by caller)
/// - `payload_hash`: value of `x-amz-content-sha256` header (or `"UNSIGNED-PAYLOAD"`)
/// - `datetime`: value of `x-amz-date` header (`YYYYMMDDTHHmmssZ`)
/// - `date`: `YYYYMMDD` extracted from `datetime`
/// - `region`: S3 region (we use `"us-east-1"` as a fixed constant)
/// - `service`: always `"s3"`
/// - `signature`: hex signature from the Authorization header
pub fn verify_header_auth(
    secret_key: &str,
    method: &str,
    path: &str,
    raw_query: &str,
    headers: &[(String, String)],
    payload_hash: &str,
    datetime: &str,
    date: &str,
    region: &str,
    service: &str,
    signature: &str,
) -> bool {
    let canonical_uri = uri_encode(path, false);
    let canonical_query = canonical_query_string(raw_query);
    let cr_hash = canonical_request_hash(
        method,
        &canonical_uri,
        &canonical_query,
        headers,
        payload_hash,
    );

    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{cr_hash}");

    let signing_key = derive_signing_key(secret_key, date, region, service);
    let expected_sig = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    // Constant-time comparison
    subtle_eq(expected_sig.as_bytes(), signature.as_bytes())
}

/// Verify an AWS SigV4 presigned URL signature.
///
/// For presigned URLs the payload hash is always `UNSIGNED-PAYLOAD`, and the
/// `X-Amz-Signature` query parameter is excluded from the canonical query string.
/// The headers to sign are typically just `host`.
pub fn verify_presigned(
    secret_key: &str,
    method: &str,
    path: &str,
    raw_query_without_sig: &str,
    host_header: &str,
    datetime: &str,
    date: &str,
    region: &str,
    service: &str,
    signature: &str,
) -> bool {
    let canonical_uri = uri_encode(path, false);
    let canonical_query = canonical_query_string(raw_query_without_sig);
    let headers = vec![("host".to_string(), host_header.to_string())];
    let cr_hash = canonical_request_hash(
        method,
        &canonical_uri,
        &canonical_query,
        &headers,
        "UNSIGNED-PAYLOAD",
    );

    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{datetime}\n{scope}\n{cr_hash}");

    let signing_key = derive_signing_key(secret_key, date, region, service);
    let expected_sig = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    subtle_eq(expected_sig.as_bytes(), signature.as_bytes())
}

/// Constant-time byte slice comparison to prevent timing attacks.
fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Parse the `Authorization` header for SigV4.
/// Returns `(access_key, date, region, service, signed_header_names, signature)`.
pub fn parse_authorization(
    auth: &str,
) -> Option<(String, String, String, String, Vec<String>, String)> {
    let auth = auth.trim();
    let rest = auth.strip_prefix("AWS4-HMAC-SHA256 ")?;

    let mut access_key = None;
    let mut date = None;
    let mut region = None;
    let mut service = None;
    let mut signed_headers = None;
    let mut signature = None;

    for part in rest.split(',') {
        let part = part.trim();
        if let Some(cred) = part.strip_prefix("Credential=") {
            // Credential=<access_key>/<date>/<region>/<service>/aws4_request
            let parts: Vec<&str> = cred.splitn(5, '/').collect();
            if parts.len() >= 4 {
                access_key = Some(parts[0].to_string());
                date = Some(parts[1].to_string());
                region = Some(parts[2].to_string());
                service = Some(parts[3].to_string());
            }
        } else if let Some(sh) = part.strip_prefix("SignedHeaders=") {
            signed_headers = Some(sh.split(';').map(str::to_lowercase).collect::<Vec<_>>());
        } else if let Some(sig) = part.strip_prefix("Signature=") {
            signature = Some(sig.to_string());
        }
    }

    Some((
        access_key?,
        date?,
        region?,
        service?,
        signed_headers?,
        signature?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_encode_path() {
        assert_eq!(
            uri_encode("/home/my files/test.txt", false),
            "/home/my%20files/test.txt"
        );
        assert_eq!(uri_encode("hello world", true), "hello%20world");
        assert_eq!(uri_encode("a-z_0.~", true), "a-z_0.~");
        assert_eq!(uri_encode("/", false), "/");
    }

    #[test]
    fn test_canonical_query_string() {
        let q = "b=2&a=1&c=3";
        let canon = canonical_query_string(q);
        assert_eq!(canon, "a=1&b=2&c=3");
    }

    #[test]
    fn test_canonical_query_empty() {
        assert_eq!(canonical_query_string(""), "");
    }

    #[test]
    fn test_subtle_eq() {
        assert!(subtle_eq(b"abc", b"abc"));
        assert!(!subtle_eq(b"abc", b"abd"));
        assert!(!subtle_eq(b"ab", b"abc"));
    }
}
