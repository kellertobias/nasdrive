use nasfiles_core::tokens;

/// Issue a stateless HMAC bearer token for authenticated share access.
///
/// The bearer contains `{share_id, iat, exp}` and is signed with the session secret.
/// TTL is 30 minutes by default.
pub fn issue_bearer(session_secret: &[u8], share_id: &str) -> Result<String, BearerError> {
    tokens::create_bearer_token(session_secret, share_id, 1800)
        .map_err(|e| BearerError::Issue(e.to_string()))
}

// @tour comment Bearer verification does not check revocation
// The doc comment is explicit: this validates signature, expiry and share-id match, but not
// revocation — the caller must check that separately against the database.
//
// Every public handler satisfies that by calling `access::resolve_share` *before* verifying
// the bearer, so a revoked share fails at token resolution. A new endpoint that verified a
// bearer without first re-resolving the share would honour it for up to 30 minutes past
// revocation.

/// Verify a bearer token and ensure it belongs to the expected share.
///
/// Checks: HMAC signature, expiry, share_id match.
/// Does NOT check share revocation — the caller must check that separately against the DB.
pub fn verify_bearer(
    session_secret: &[u8],
    token: &str,
    expected_share_id: &str,
) -> Result<(), BearerError> {
    let (share_id, _iat, _exp) = tokens::verify_bearer_token(session_secret, token)
        .map_err(|e| BearerError::Invalid(e.to_string()))?;

    if share_id != expected_share_id {
        return Err(BearerError::WrongShare);
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum BearerError {
    #[error("failed to issue bearer: {0}")]
    Issue(String),
    #[error("invalid bearer token: {0}")]
    Invalid(String),
    #[error("bearer token is for a different share")]
    WrongShare,
}

impl axum::response::IntoResponse for BearerError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}
