use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64ct::{Base64UrlUnpadded, Encoding};

// @tour comment Two encrypt_secret functions with inverted arguments
// This one takes the key *first* and a `&str`. The private `local::encrypt_secret` takes
// the key *second* and raw bytes. Both produce base64url(nonce‖ciphertext) with AES-256-GCM
// keyed on `session_secret[..32]`, so ciphertexts are interchangeable but the call
// signatures are mirror images.
//
// Both also index `session_secret[..32]` directly, so a shorter secret panics in `local.rs`
// rather than returning the friendly error this module produces.

/// Encrypt a short plaintext (e.g., an S3 secret key) using AES-256-GCM.
/// The encryption key is derived from the first 32 bytes of `session_secret`.
/// Returns base64url(nonce[12] || ciphertext).
pub fn encrypt_secret(session_secret: &[u8], plaintext: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(&session_secret[..32])
        .map_err(|_| "session_secret must be >= 32 bytes".to_string())?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| format!("encryption error: {e}"))?;

    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(Base64UrlUnpadded::encode_string(&combined))
}

/// Decrypt a value produced by [`encrypt_secret`].
pub fn decrypt_secret(session_secret: &[u8], ciphertext_b64: &str) -> Result<String, String> {
    let combined = Base64UrlUnpadded::decode_vec(ciphertext_b64)
        .map_err(|e| format!("base64 decode error: {e}"))?;

    if combined.len() < 12 {
        return Err("invalid encrypted secret".into());
    }

    let cipher = Aes256Gcm::new_from_slice(&session_secret[..32])
        .map_err(|_| "session_secret must be >= 32 bytes".to_string())?;

    let nonce_arr: [u8; 12] = combined[..12].try_into().expect("nonce must be 12 bytes");
    let plaintext = cipher
        .decrypt(&nonce_arr.into(), &combined[12..])
        .map_err(|e| format!("decryption error: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("utf8 error: {e}"))
}
