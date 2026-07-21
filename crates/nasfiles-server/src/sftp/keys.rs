use russh::keys::ssh_key::{HashAlg, PublicKey};

#[derive(Debug, Clone)]
pub struct NormalizedPublicKey {
    pub public_key: String,
    pub fingerprint: String,
    pub comment: Option<String>,
}

pub fn normalize_public_key(input: &str) -> anyhow::Result<NormalizedPublicKey> {
    let key = PublicKey::from_openssh(input.trim())
        .map_err(|e| anyhow::anyhow!("invalid OpenSSH public key: {e}"))?;
    normalize_russh_public_key(&key)
}

// @tour sftp-server:70 One normalization function, two callers
// `NormalizedPublicKey` carries the re-encoded OpenSSH blob, a SHA-256 fingerprint, and the
// optional key comment.
//
// The same function backs the HTTP API used when a user pastes a key into the profile page.
// That is the point: the fingerprint stored by the web UI and the fingerprint computed
// during SSH auth are byte-identical by construction, rather than by two implementations
// agreeing.

pub fn normalize_russh_public_key(key: &PublicKey) -> anyhow::Result<NormalizedPublicKey> {
    let public_key = key
        .to_openssh()
        .map_err(|e| anyhow::anyhow!("failed to encode public key: {e}"))?;
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let comment = key.comment();
    let comment = if comment.is_empty() {
        None
    } else {
        Some(comment.to_string())
    };

    Ok(NormalizedPublicKey {
        public_key,
        fingerprint,
        comment,
    })
}
