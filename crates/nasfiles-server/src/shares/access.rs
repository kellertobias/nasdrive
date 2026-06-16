use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use sqlx::AnyPool;

use super::model::{Share, TargetKind};
use crate::config::AppConfig;
use nasfiles_core::tokens;

/// State for share access rate limiting.
///
/// The map is bounded via periodic eviction: entries whose maximum backoff has
/// long elapsed are pruned every 100 `record_failure` calls, keeping memory use
/// proportional to the number of *recent* failed tokens.
#[derive(Clone)]
pub struct ShareRateLimiter {
    /// Map from token_hash → (failed_attempts, last_attempt_timestamp_ms)
    attempts: Arc<DashMap<String, (u32, i64)>>,
    call_count: Arc<AtomicU64>,
}

impl Default for ShareRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ShareRateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(DashMap::new()),
            call_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Check if this token is rate limited. Returns true if the caller should be blocked.
    pub fn is_rate_limited(&self, token_hash: &str) -> bool {
        if let Some(entry) = self.attempts.get(token_hash) {
            let (count, last) = *entry;
            if count >= 5 {
                // Exponential backoff: 2^(count-5) seconds, max ~4.3 minutes
                let backoff_ms: i64 = (1i64 << (count - 5).min(8)) * 1000;
                let now = chrono::Utc::now().timestamp_millis();
                return now - last < backoff_ms;
            }
        }
        false
    }

    /// Record a failed authentication attempt.
    pub fn record_failure(&self, token_hash: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        self.attempts
            .entry(token_hash.to_string())
            .and_modify(|e| {
                e.0 += 1;
                e.1 = now;
            })
            .or_insert((1, now));

        // Evict stale entries every 100 calls to bound map growth.
        let n = self.call_count.fetch_add(1, Ordering::Relaxed);
        if n % 100 == 0 {
            self.evict_expired(now);
        }
    }

    /// Reset on successful auth.
    pub fn reset(&self, token_hash: &str) {
        self.attempts.remove(token_hash);
    }

    /// Remove entries whose backoff period has expired (with a 5-minute grace window).
    fn evict_expired(&self, now_ms: i64) {
        let grace_ms: i64 = 5 * 60 * 1000;
        self.attempts.retain(|_, (count, last)| {
            let max_backoff_ms: i64 = (1i64 << (*count).saturating_sub(5).min(8)) * 1000;
            *last + max_backoff_ms + grace_ms > now_ms
        });
    }
}

/// Resolve a raw share token to a Share record.
///
/// Hashes the token with SHA-256 and looks it up in the database.
/// Checks that the share is not expired and not revoked.
pub async fn resolve_share(pool: &AnyPool, raw_token: &str) -> Result<Share, ShareAccessError> {
    let token_hash = tokens::hash_token(raw_token);

    let row = sqlx::query_as::<_, ShareRow>(
        r#"SELECT id, token_hash, owner_user_id, root_kind, root_key, relative_path,
                  CASE WHEN is_directory THEN 1 ELSE 0 END AS is_directory,
                  target_kind, target_user_id, password_hash,
                  CASE WHEN allow_upload THEN 1 ELSE 0 END AS allow_upload,
                  CASE WHEN allow_download THEN 1 ELSE 0 END AS allow_download,
                  expires_at, created_at, revoked_at
           FROM shares
           WHERE token_hash = $1"#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| ShareAccessError::Database(e.to_string()))?
    .ok_or(ShareAccessError::NotFound)?;

    let share = row.into_share()?;

    // Check revoked
    if share.revoked_at.is_some() {
        return Err(ShareAccessError::Revoked);
    }

    // Check expired
    if let Some(expires_at) = share.expires_at {
        let now = chrono::Utc::now().timestamp_millis();
        if now > expires_at {
            return Err(ShareAccessError::Expired);
        }
    }

    Ok(share)
}

/// Verify a guest share password using Argon2id (constant-time).
pub fn verify_password(share: &Share, password: &str) -> Result<bool, ShareAccessError> {
    use argon2::{
        Argon2,
        password_hash::{PasswordHash, PasswordVerifier},
    };

    let stored_hash = share
        .password_hash
        .as_deref()
        .ok_or(ShareAccessError::NoPassword)?;

    let parsed_hash = PasswordHash::new(stored_hash)
        .map_err(|e| ShareAccessError::Internal(format!("invalid password hash: {e}")))?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Resolve the filesystem path for a share.
pub async fn resolve_share_path(
    pool: &sqlx::AnyPool,
    config: &AppConfig,
    share: &Share,
    relative: &str,
) -> Result<PathBuf, ShareAccessError> {
    // Get the root path
    let root_path = if share.root_kind == "home" {
        let home_root = config
            .home_folder_root
            .as_ref()
            .ok_or(ShareAccessError::NotFound)?;

        let owner_username =
            sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
                .bind(&share.owner_user_id)
                .fetch_optional(pool)
                .await
                .map_err(|_| ShareAccessError::Internal("db error".into()))?
                .ok_or(ShareAccessError::NotFound)?;

        let safe_username = nasfiles_core::models::AuthUser::sanitize_username(&owner_username);

        home_root.join(safe_username)
    } else {
        config
            .common_folders
            .get(&share.root_key)
            .ok_or(ShareAccessError::NotFound)?
            .clone()
    };

    // Resolve to the share's base path
    let share_base = if share.relative_path.is_empty() {
        root_path.clone()
    } else {
        nasfiles_core::safe_path::resolve(&root_path, &share.relative_path)
            .map_err(|e| ShareAccessError::Internal(format!("share base path error: {e}")))?
    };

    // Now resolve the requested relative path within the share scope
    if relative.is_empty() {
        Ok(share_base)
    } else {
        nasfiles_core::safe_path::resolve(&share_base, relative)
            .map_err(|_| ShareAccessError::NotFound)
    }
}

// -----------------------------------------------------------------------
// Internal query helper
// -----------------------------------------------------------------------

/// Row type for SQLx query — maps to the shares table columns.
#[derive(sqlx::FromRow)]
struct ShareRow {
    id: String,
    token_hash: String,
    owner_user_id: String,
    root_kind: String,
    root_key: String,
    relative_path: String,
    is_directory: i64,
    target_kind: String,
    target_user_id: Option<String>,
    password_hash: Option<String>,
    allow_upload: i64,
    allow_download: i64,
    expires_at: Option<i64>,
    created_at: i64,
    revoked_at: Option<i64>,
}

impl ShareRow {
    fn into_share(self) -> Result<Share, ShareAccessError> {
        let target_kind = TargetKind::from_str(&self.target_kind).ok_or_else(|| {
            ShareAccessError::Internal(format!("invalid target_kind: {}", self.target_kind))
        })?;

        Ok(Share {
            id: self.id,
            token_hash: self.token_hash,
            owner_user_id: self.owner_user_id,
            root_kind: self.root_kind,
            root_key: self.root_key,
            relative_path: self.relative_path,
            is_directory: self.is_directory != 0,
            target_kind,
            target_user_id: self.target_user_id,
            password_hash: self.password_hash,
            allow_upload: self.allow_upload != 0,
            allow_download: self.allow_download != 0,
            expires_at: self.expires_at,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
        })
    }
}

// -----------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ShareAccessError {
    #[error("share not found")]
    NotFound,
    #[error("share has expired")]
    Expired,
    #[error("share has been revoked")]
    Revoked,
    #[error("no password set on share")]
    NoPassword,
    #[error("rate limited")]
    RateLimited,
    #[error("database error: {0}")]
    Database(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for ShareAccessError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        // Intentionally return 404 for not found, expired, and revoked
        // to avoid leaking information about share existence
        let status = match &self {
            ShareAccessError::NotFound | ShareAccessError::Expired | ShareAccessError::Revoked => {
                StatusCode::NOT_FOUND
            }
            ShareAccessError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ShareAccessError::NoPassword => StatusCode::BAD_REQUEST,
            ShareAccessError::Database(_) | ShareAccessError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            axum::Json(serde_json::json!({"error": "share not found"})),
        )
            .into_response()
    }
}
