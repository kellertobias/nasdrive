use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64ct::{Base64UrlUnpadded, Encoding};

/// Encrypt a short plaintext (e.g., an S3 secret key) using AES-256-GCM.
/// The encryption key is derived from the first 32 bytes of `session_secret`.
/// Returns base64url(nonce[12] || ciphertext).
pub fn encrypt_secret(session_secret: &[u8], plaintext: &str) -> Result<String, String> {
    let key = Key::<Aes256Gcm>::from_slice(&session_secret[..32]);
    let cipher = Aes256Gcm::new(key);
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

    let key = Key::<Aes256Gcm>::from_slice(&session_secret[..32]);
    let cipher = Aes256Gcm::new(key);
    let nonce = aes_gcm::Nonce::from_slice(&combined[..12]);

    let plaintext = cipher
        .decrypt(nonce, &combined[12..])
        .map_err(|e| format!("decryption error: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("utf8 error: {e}"))
}
