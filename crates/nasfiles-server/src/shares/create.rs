use nasfiles_core::models::AuthUser;
use nasfiles_core::tokens;
use sqlx::AnyPool;

use super::model::{CreateShareRequest, Share, ShareType, TargetKind};
use crate::config::AppConfig;
use crate::fs::roots;

// @tour share-management:60 Validation and permission derivation
// Before anything is written, `roots::resolve_root(..., RequiredCap::Share)` proves the
// caller may share that root and `safe_path::resolve` proves the path is inside it.
//
// `permissions_for_share_type` is the real source of the permission pair. Upload and
// gallery shares are rejected unless the target is a directory, and guest shares require a
// password of at least 4 characters, hashed with Argon2id.

/// Create a new share for a file or directory.
///
/// Validates that the user has access to the target path, generates a cryptographic
/// share token, optionally hashes a password (for guest shares), and inserts the
/// share into the database.
///
/// Returns `(share, raw_token)` — the raw token is only available at creation time.
pub async fn create_share(
    pool: &AnyPool,
    config: &AppConfig,
    user: &AuthUser,
    request: CreateShareRequest,
) -> Result<(Share, String), ShareCreateError> {
    // Validate user has access to the root
    let root_path = roots::resolve_root(config, user, &request.root_key, roots::RequiredCap::Share)
        .map_err(|e| ShareCreateError::AccessDenied(e.to_string()))?;

    // Validate the path exists
    let resolved = nasfiles_core::safe_path::resolve(&root_path, &request.path)
        .map_err(|e| ShareCreateError::InvalidPath(e.to_string()))?;

    let is_directory = resolved.is_dir();
    // @tour comment The client's permission flags are decorative
    // `ShareDialog` computes and sends `allow_download`/`allow_upload`, and
    // `api.createShare` types them as required fields — but `CreateShareRequest` has no
    // such fields, so they are dropped and permissions are derived purely from
    // [`share_type`](glossary:share-type).
    //
    // That is the safe direction: a client cannot escalate a Dropbox share into a
    // downloadable one. The dead fields just invite the opposite assumption when reading
    // the frontend.

    let (allow_download, allow_upload) = permissions_for_share_type(&request.share_type);

    // For upload shares, target must be a directory
    if allow_upload && !is_directory {
        return Err(ShareCreateError::InvalidPath(
            "upload is only allowed for directories".into(),
        ));
    }

    if request.share_type == ShareType::Gallery && !is_directory {
        return Err(ShareCreateError::InvalidPath(
            "gallery shares are only supported for folders".into(),
        ));
    }

    // Generate token
    let raw_token = tokens::generate_share_token(config.share_token_bytes);
    let token_hash = tokens::hash_token(&raw_token);

    // Hash password if guest share
    let password_hash = if request.target_kind == TargetKind::Guest {
        let password = request
            .password
            .as_deref()
            .ok_or(ShareCreateError::PasswordRequired)?;

        if password.len() < 4 {
            return Err(ShareCreateError::WeakPassword);
        }

        Some(hash_password(password)?)
    } else {
        None
    };

    // Determine root_kind
    let root_kind = if request.root_key == "~" {
        "home"
    } else {
        "common"
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = request.expires_in.map(|secs| now + secs * 1000);

    // Insert into database
    sqlx::query(
        r#"INSERT INTO shares
           (id, token_hash, owner_user_id, root_kind, root_key, relative_path,
            is_directory, target_kind, target_user_id, password_hash,
            allow_upload, allow_download, share_type, expires_at, created_at, revoked_at,
            display_token)
           // @tour comment Raw tokens ARE stored, in display_token
           // `tokens::hash_token`'s doc comment promises the raw token is never stored, and
           // the `token_hash` column honours that — but the last bound parameter writes
           // `raw_token` straight into `display_token`, which `list_shares` reads back to
           // rebuild the link.
           //
           // So the hashing buys nothing against a database read; it only prevents a
           // lookup-by-plaintext query pattern. Worth knowing before assuming a database
           // compromise leaves share tokens safe.

           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NULL, $16)"#,
    )
    .bind(&id)
    .bind(&token_hash)
    .bind(&user.user_id)
    .bind(root_kind)
    .bind(&request.root_key)
    .bind(&request.path)
    .bind(is_directory)
    .bind(request.target_kind.as_str())
    .bind(&request.target_user_id)
    .bind(&password_hash)
    .bind(allow_upload)
    .bind(allow_download)
    .bind(request.share_type.as_str())
    .bind(expires_at)
    .bind(now)
    .bind(&raw_token)
    .execute(pool)
    .await
    .map_err(|e| ShareCreateError::Database(e.to_string()))?;

    let share = Share {
        id,
        token_hash,
        owner_user_id: user.user_id.clone(),
        root_kind: root_kind.to_string(),
        root_key: request.root_key,
        relative_path: request.path,
        is_directory,
        target_kind: request.target_kind,
        target_user_id: request.target_user_id,
        password_hash,
        allow_upload,
        allow_download,
        share_type: request.share_type,
        expires_at,
        created_at: now,
        revoked_at: None,
    };

    Ok((share, raw_token))
}

pub(crate) fn permissions_for_share_type(share_type: &ShareType) -> (bool, bool) {
    match share_type {
        ShareType::Typical | ShareType::Gallery => (true, false),
        ShareType::Dropbox => (false, true),
        ShareType::Collaboration => (true, true),
    }
}

/// Hash a guest-share password using Argon2id.
pub(crate) fn hash_password(password: &str) -> Result<String, ShareCreateError> {
    use argon2::{
        Argon2,
        password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
    };

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| ShareCreateError::PasswordHash(e.to_string()))?;

    Ok(hash.to_string())
}

// -----------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ShareCreateError {
    #[error("access denied: {0}")]
    AccessDenied(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("password is required for guest shares")]
    PasswordRequired,
    #[error("password is too weak (minimum 4 characters)")]
    WeakPassword,
    #[error("password hashing error: {0}")]
    PasswordHash(String),
    #[error("database error: {0}")]
    Database(String),
}

impl axum::response::IntoResponse for ShareCreateError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let (status, msg) = match &self {
            ShareCreateError::AccessDenied(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ShareCreateError::InvalidPath(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ShareCreateError::PasswordRequired => (StatusCode::BAD_REQUEST, self.to_string()),
            ShareCreateError::WeakPassword => (StatusCode::BAD_REQUEST, self.to_string()),
            ShareCreateError::PasswordHash(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
            ShareCreateError::Database(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}
