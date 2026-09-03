use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use nasfiles_core::{models::AuthUser, sigv4};
use sqlx::AnyPool;
use std::collections::HashMap;
use subtle::ConstantTimeEq;

use crate::{config::AppConfig, shares::model::Share, state::AppState};

pub const S3_SERVICE: &str = "s3";

/// The verified identity behind an S3 request.
#[derive(Clone)]
pub enum S3Principal {
    /// A user-generated API token — permissions match the live user record.
    UserToken { user_id: String, user: AuthUser },
    /// A temporary credential issued via share + optional password exchange.
    ShareCredential { share: Share, cred_id: String },
}

#[derive(Debug)]
pub enum S3AuthError {
    MissingCredentials,
    InvalidCredentials,
    SignatureDoesNotMatch,
    ExpiredCredential,
    RequestTimeTooSkewed,
    AccessDenied,
    NoSuchBucket,
    Internal(String),
}

impl S3AuthError {
    pub fn xml_code(&self) -> &str {
        match self {
            S3AuthError::MissingCredentials => "InvalidRequest",
            S3AuthError::InvalidCredentials | S3AuthError::SignatureDoesNotMatch => {
                "SignatureDoesNotMatch"
            }
            S3AuthError::ExpiredCredential => "ExpiredToken",
            S3AuthError::RequestTimeTooSkewed => "RequestTimeTooSkewed",
            S3AuthError::AccessDenied => "AccessDenied",
            S3AuthError::NoSuchBucket => "NoSuchBucket",
            S3AuthError::Internal(_) => "InternalError",
        }
    }

    pub fn http_status(&self) -> StatusCode {
        match self {
            S3AuthError::MissingCredentials
            | S3AuthError::InvalidCredentials
            | S3AuthError::SignatureDoesNotMatch
            | S3AuthError::RequestTimeTooSkewed
            | S3AuthError::ExpiredCredential
            | S3AuthError::AccessDenied => StatusCode::FORBIDDEN,
            S3AuthError::NoSuchBucket => StatusCode::NOT_FOUND,
            S3AuthError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for S3AuthError {
    fn into_response(self) -> Response {
        let code = self.xml_code();
        let message = match &self {
            S3AuthError::Internal(msg) => msg.clone(),
            _ => code.to_string(),
        };
        let body = super::xml::error_xml(code, &message);
        (
            self.http_status(),
            [("content-type", "application/xml")],
            body,
        )
            .into_response()
    }
}

/// Axum extractor that verifies SigV4 and returns the S3 principal.
pub struct S3Auth(pub S3Principal);

impl FromRequestParts<AppState> for S3Auth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve_s3_auth(parts, state)
            .await
            .map(S3Auth)
            .map_err(|e| e.into_response())
    }
}

// @tour s3-api:40 The auth extractor
// `S3Auth` implements `FromRequestParts`, so axum runs this before any handler body and
// converts a failure into a proper XML error response.
//
// It prefers the `authorization` header and falls back to presigned-URL mode only if the
// raw query contains a signature parameter. The result is one of two variants — a user
// token or a share-scoped credential — and the entire rest of the API is written against
// that enum. See [S3 principal](glossary:s3-principal).

async fn resolve_s3_auth(parts: &mut Parts, state: &AppState) -> Result<S3Principal, S3AuthError> {
    // Try header-based auth first, then presigned URL
    if let Some(auth_header) = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
    {
        return verify_header_auth(parts, state, &auth_header).await;
    }

    let query = parts.uri.query().unwrap_or("");
    if query.contains("X-Amz-Signature=") || query.contains("x-amz-signature=") {
        return verify_presigned_auth(parts, state).await;
    }

    Err(S3AuthError::MissingCredentials)
}

// @tour s3-api:50 Reconstructing what the client signed
// Parsing the authorization header yields the access key, scope date, region, service,
// signed-header list and signature. The server then re-reads each named header off the live
// request, sorts them, and enforces a 900-second clock-skew window *before* doing any
// crypto.
//
// The payload hash is taken verbatim from `x-amz-content-sha256`, defaulting to
// `UNSIGNED-PAYLOAD` when absent. Everything is bundled into a request context and handed
// to the framework-free verifier in `nasfiles-core`.

async fn verify_header_auth(
    parts: &mut Parts,
    state: &AppState,
    auth_header: &str,
) -> Result<S3Principal, S3AuthError> {
    let (access_key, date, region, service, signed_header_names, signature) =
        sigv4::parse_authorization(auth_header).ok_or(S3AuthError::MissingCredentials)?;

    let (secret_key, principal) =
        lookup_credential(&state.pool, &access_key, &state.config).await?;

    // Collect signed headers from the actual request, sorted
    let mut signed_headers: Vec<(String, String)> = signed_header_names
        .iter()
        .map(|name| {
            let value = parts
                .headers
                .get(name.as_str())
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            (name.clone(), value)
        })
        .collect();
    signed_headers.sort_by(|a, b| a.0.cmp(&b.0));

    let datetime = parts
        .headers
        .get("x-amz-date")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Reject requests with a timestamp more than 15 minutes off (replay attack prevention).
    let issued_secs = parse_datetime_secs(datetime).ok_or(S3AuthError::MissingCredentials)?;
    if (chrono::Utc::now().timestamp() - issued_secs).unsigned_abs() > 900 {
        return Err(S3AuthError::RequestTimeTooSkewed);
    }

    let payload_hash = parts
        .headers
        .get("x-amz-content-sha256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("UNSIGNED-PAYLOAD");

    let ctx = sigv4::SigV4RequestContext {
        secret_key: &secret_key,
        method: parts.method.as_str(),
        path: parts.uri.path(),
        raw_query: parts.uri.query().unwrap_or(""),
        datetime,
        date: &date,
        region: &region,
        service: &service,
        signature: &signature,
    };
    if !sigv4::verify_header_auth(&ctx, &signed_headers, payload_hash) {
        return Err(S3AuthError::SignatureDoesNotMatch);
    }

    update_last_used(&state.pool, &access_key).await;
    Ok(principal)
}

// @tour s3-api:70 Presigned URLs: expiry instead of skew
// The access key and region are pulled out of the credential parameter by splitting on `/`,
// and the scope date is the first eight characters of the date parameter.
//
// Instead of a skew window this path checks the issue time plus an expiry defaulting to 900
// seconds. The query is rebuilt without the signature parameter before canonicalization,
// since a signature cannot sign itself.

async fn verify_presigned_auth(
    parts: &mut Parts,
    state: &AppState,
) -> Result<S3Principal, S3AuthError> {
    let query = parts.uri.query().unwrap_or("").to_string();
    let params = parse_query_params(&query);

    let access_key = extract_presigned_access_key(&params)?.to_string();
    let datetime = params
        .get("X-Amz-Date")
        .or_else(|| params.get("x-amz-date"))
        .ok_or(S3AuthError::MissingCredentials)?
        .clone();
    let expires = params
        .get("X-Amz-Expires")
        .or_else(|| params.get("x-amz-expires"))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(900);
    let signature = params
        .get("X-Amz-Signature")
        .or_else(|| params.get("x-amz-signature"))
        .ok_or(S3AuthError::MissingCredentials)?
        .clone();
    let region = extract_presigned_region(&params)?;
    let date = &datetime[..8.min(datetime.len())];

    let issued_secs = parse_datetime_secs(&datetime).ok_or(S3AuthError::MissingCredentials)?;
    if chrono::Utc::now().timestamp() > issued_secs + expires {
        return Err(S3AuthError::ExpiredCredential);
    }

    let (secret_key, principal) =
        lookup_credential(&state.pool, &access_key, &state.config).await?;

    let host = parts
        .headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let query_without_sig = rebuild_query_without_sig(&query);

    let ctx = sigv4::SigV4RequestContext {
        secret_key: &secret_key,
        method: parts.method.as_str(),
        path: parts.uri.path(),
        raw_query: &query_without_sig,
        datetime: &datetime,
        date,
        region: &region,
        service: S3_SERVICE,
        signature: &signature,
    };
    if !sigv4::verify_presigned(&ctx, host) {
        return Err(S3AuthError::SignatureDoesNotMatch);
    }

    update_last_used(&state.pool, &access_key).await;
    Ok(principal)
}

// @tour s3-api:80 Two credential tables, one access-key namespace
// The access key is looked up first among user API tokens and then among share credentials.
// Either way the stored secret is encrypted at rest and decrypted with the configured
// session secret.
//
// User tokens honour revocation and an optional expiry; share credentials have a mandatory
// expiry and additionally re-check the parent share's own revocation state. For user tokens
// `load_user` reloads the live user row, so permissions can never go stale relative to a
// long-lived token.

/// Look up a credential by access_key and return (secret_key, principal).
pub async fn lookup_credential(
    pool: &AnyPool,
    access_key: &str,
    config: &AppConfig,
) -> Result<(String, S3Principal), S3AuthError> {
    let now = chrono::Utc::now().timestamp_millis();

    // User API token
    #[derive(sqlx::FromRow)]
    struct TokenRow {
        user_id: String,
        secret_key: String,
        expires_at: Option<i64>,
        revoked_at: Option<i64>,
    }

    if let Some(row) = sqlx::query_as::<_, TokenRow>(
        "SELECT user_id, secret_key, expires_at, revoked_at FROM user_api_tokens WHERE access_key = $1",
    )
    .bind(access_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| S3AuthError::Internal(e.to_string()))?
    {
        if row.revoked_at.is_some() {
            return Err(S3AuthError::InvalidCredentials);
        }
        if row.expires_at.is_some_and(|exp| now > exp) {
            return Err(S3AuthError::ExpiredCredential);
        }
        let secret_key = crate::crypto::decrypt_secret(&config.session_secret, &row.secret_key)
            .map_err(|e| S3AuthError::Internal(format!("failed to decrypt token: {e}")))?;
        let user = load_user(pool, config, &row.user_id).await?;
        return Ok((
            secret_key,
            S3Principal::UserToken {
                user_id: row.user_id,
                user,
            },
        ));
    }

    // Share credential
    #[derive(sqlx::FromRow)]
    struct CredRow {
        id: String,
        share_id: String,
        secret_key: String,
        expires_at: i64,
    }

    if let Some(row) = sqlx::query_as::<_, CredRow>(
        "SELECT id, share_id, secret_key, expires_at FROM s3_share_credentials WHERE access_key = $1",
    )
    .bind(access_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| S3AuthError::Internal(e.to_string()))?
    {
        if now > row.expires_at {
            return Err(S3AuthError::ExpiredCredential);
        }
        let share = crate::shares::access::resolve_share_by_id(pool, &row.share_id)
            .await
            .map_err(|_| S3AuthError::InvalidCredentials)?;
        if share.revoked_at.is_some() {
            return Err(S3AuthError::InvalidCredentials);
        }
        if share.expires_at.is_some_and(|exp| now > exp) {
            return Err(S3AuthError::ExpiredCredential);
        }
        let secret_key = crate::crypto::decrypt_secret(&config.session_secret, &row.secret_key)
            .map_err(|e| S3AuthError::Internal(format!("failed to decrypt credential: {e}")))?;
        return Ok((
            secret_key,
            S3Principal::ShareCredential {
                share,
                cred_id: row.id,
            },
        ));
    }

    Err(S3AuthError::InvalidCredentials)
}

/// Verify a personal API token as an access-key/secret pair.
///
/// S3 needs the decrypted secret in order to reconstruct a SigV4 signature.
/// Other device protocols use this narrower helper so share-scoped S3
/// credentials are never accepted as user credentials and the secret is
/// compared in constant time before the live user permissions are returned.
pub async fn verify_user_api_credential(
    pool: &AnyPool,
    config: &AppConfig,
    access_key: &str,
    supplied_secret: &str,
) -> Result<AuthUser, S3AuthError> {
    let (stored_secret, principal) = lookup_credential(pool, access_key, config).await?;
    let valid = stored_secret.len() == supplied_secret.len()
        && stored_secret
            .as_bytes()
            .ct_eq(supplied_secret.as_bytes())
            .into();
    if !valid {
        return Err(S3AuthError::InvalidCredentials);
    }

    let S3Principal::UserToken { user, .. } = principal else {
        return Err(S3AuthError::InvalidCredentials);
    };
    update_last_used(pool, access_key).await;
    Ok(user)
}

pub async fn load_user(
    pool: &AnyPool,
    config: &AppConfig,
    user_id: &str,
) -> Result<AuthUser, S3AuthError> {
    #[derive(sqlx::FromRow)]
    struct UserRow {
        id: String,
        username: String,
        display_name: String,
        picture_url: Option<String>,
        folder_permissions_json: Option<String>,
        has_home: i64,
        is_admin: i64,
    }

    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, display_name, picture_url, folder_permissions_json, \
         CASE WHEN has_home THEN 1 ELSE 0 END AS has_home, \
         CASE WHEN is_admin THEN 1 ELSE 0 END AS is_admin \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| S3AuthError::Internal(e.to_string()))?
    .ok_or(S3AuthError::InvalidCredentials)?;

    let folder_permissions = row
        .folder_permissions_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| config.default_folder_caps.clone());

    Ok(AuthUser {
        user_id: row.id.clone(),
        external_id: row.id,
        username: row.username,
        display_name: row.display_name,
        picture_url: row.picture_url,
        folder_permissions,
        has_home: row.has_home != 0,
        is_admin: row.is_admin != 0,
    })
}

async fn update_last_used(pool: &AnyPool, access_key: &str) {
    let now = chrono::Utc::now().timestamp_millis();
    let _ = sqlx::query("UPDATE user_api_tokens SET last_used_at = $1 WHERE access_key = $2")
        .bind(now)
        .bind(access_key)
        .execute(pool)
        .await;
    let _ = sqlx::query("UPDATE s3_share_credentials SET last_used_at = $1 WHERE access_key = $2")
        .bind(now)
        .bind(access_key)
        .execute(pool)
        .await;
}

// @tour comment Presigned query parsing does not decode
// This splits on `&` and `=` and keeps raw, still-percent-encoded values. That is harmless
// for the signature (hex) and the date, but most SDKs URL-encode the credential parameter,
// writing `/` as `%2F`.
//
// The extractors above split on a literal `/`, so an encoded credential will not decompose
// into its scope components. Anyone extending presigned support should test against a
// client that encodes it.

fn parse_query_params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn extract_presigned_access_key(params: &HashMap<String, String>) -> Result<&str, S3AuthError> {
    let cred = params
        .get("X-Amz-Credential")
        .or_else(|| params.get("x-amz-credential"))
        .ok_or(S3AuthError::MissingCredentials)?;
    cred.split('/')
        .next()
        .ok_or(S3AuthError::MissingCredentials)
}

fn extract_presigned_region(params: &HashMap<String, String>) -> Result<String, S3AuthError> {
    let cred = params
        .get("X-Amz-Credential")
        .or_else(|| params.get("x-amz-credential"))
        .ok_or(S3AuthError::MissingCredentials)?;
    let parts: Vec<&str> = cred.split('/').collect();
    Ok(parts
        .get(2)
        .ok_or(S3AuthError::MissingCredentials)?
        .to_string())
}

fn rebuild_query_without_sig(query: &str) -> String {
    query
        .split('&')
        .filter(|p| {
            let key = p.split('=').next().unwrap_or("");
            !key.eq_ignore_ascii_case("X-Amz-Signature")
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn parse_datetime_secs(datetime: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(datetime, "%Y%m%dT%H%M%SZ")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_config;
    use sqlx::any::AnyPoolOptions;

    async fn pool_with_user_token(revoked: bool) -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        for statement in [
            "CREATE TABLE users (
                id TEXT PRIMARY KEY, external_id TEXT NOT NULL UNIQUE, username TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL, picture_url TEXT, is_admin BOOLEAN NOT NULL DEFAULT FALSE,
                folder_permissions_json TEXT, has_home BOOLEAN NOT NULL DEFAULT FALSE,
                created_at BIGINT NOT NULL, last_login_at BIGINT NOT NULL)",
            "CREATE TABLE user_api_tokens (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, label TEXT NOT NULL,
                access_key TEXT NOT NULL UNIQUE, secret_key TEXT NOT NULL, created_at BIGINT NOT NULL,
                expires_at BIGINT, last_used_at BIGINT, revoked_at BIGINT)",
            "CREATE TABLE s3_share_credentials (
                id TEXT PRIMARY KEY, share_id TEXT NOT NULL, access_key TEXT NOT NULL UNIQUE,
                secret_key TEXT NOT NULL, created_at BIGINT NOT NULL, expires_at BIGINT NOT NULL,
                last_used_at BIGINT)",
            "INSERT INTO users (id, external_id, username, display_name, is_admin, folder_permissions_json, has_home, created_at, last_login_at)
             VALUES ('u1', 'ext', 'alice', 'Alice', TRUE, '{\"docs\":{\"read\":true,\"write\":false,\"share\":false}}', TRUE, 0, 0)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let encrypted =
            crate::crypto::encrypt_secret(&test_config().session_secret, "s3cret").unwrap();
        sqlx::query(
            "INSERT INTO user_api_tokens (id, user_id, label, access_key, secret_key, created_at, revoked_at)
             VALUES ('t1', 'u1', 'l', 'AK', $1, 0, $2)",
        )
        .bind(encrypted)
        .bind(if revoked { Some(1_i64) } else { None })
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    /// Regression: `load_user` used to filter on a `disabled_at` column that no
    /// migration ever created, so every personal-token request failed at the
    /// database. It also has to map SQLite's integer booleans correctly.
    #[tokio::test]
    async fn load_user_reads_live_row_with_boolean_flags() {
        let pool = pool_with_user_token(false).await;
        let user = load_user(&pool, &test_config(), "u1").await.unwrap();
        assert_eq!(user.username, "alice");
        assert!(user.is_admin);
        assert!(user.has_home);
        assert!(user.can_read("docs"));
        assert!(!user.can_write("docs"));
        assert!(matches!(
            load_user(&pool, &test_config(), "missing").await,
            Err(S3AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn verify_user_api_credential_checks_secret_and_revocation() {
        let pool = pool_with_user_token(false).await;
        let config = test_config();
        let user = verify_user_api_credential(&pool, &config, "AK", "s3cret")
            .await
            .expect("valid credential");
        assert_eq!(user.user_id, "u1");
        assert!(matches!(
            verify_user_api_credential(&pool, &config, "AK", "s3cre").await,
            Err(S3AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            verify_user_api_credential(&pool, &config, "AK", "s3cret!").await,
            Err(S3AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            verify_user_api_credential(&pool, &config, "nope", "s3cret").await,
            Err(S3AuthError::InvalidCredentials)
        ));
        let last_used: Option<i64> =
            sqlx::query_scalar("SELECT last_used_at FROM user_api_tokens WHERE id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            last_used.is_some(),
            "successful verification records last use"
        );

        let revoked = pool_with_user_token(true).await;
        assert!(matches!(
            verify_user_api_credential(&revoked, &config, "AK", "s3cret").await,
            Err(S3AuthError::InvalidCredentials)
        ));
    }
}
