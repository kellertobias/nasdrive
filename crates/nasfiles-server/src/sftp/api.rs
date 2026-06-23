use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use nasfiles_core::models::RootKind;
use serde::{Deserialize, Serialize};

use crate::auth::middleware::CurrentUser;
use crate::sftp::keys::normalize_public_key;
use crate::state::AppState;

const ALLOWED_TEMP_USER_EXPIRIES: &[i64] = &[
    60 * 60,
    12 * 60 * 60,
    24 * 60 * 60,
    7 * 24 * 60 * 60,
    14 * 24 * 60 * 60,
    31 * 24 * 60 * 60,
    90 * 24 * 60 * 60,
];

#[derive(Deserialize)]
pub struct AddUserKeyRequest {
    pub public_key: String,
    pub label: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateTempUserRequest {
    pub display_name: String,
    pub root_key: String,
    #[serde(default)]
    pub path: String,
    pub can_write: bool,
    pub expires_in: i64,
    pub public_key: String,
}

#[derive(Deserialize)]
pub struct ExtendTempUserRequest {
    pub expires_in: i64,
}

#[derive(Deserialize)]
pub struct AccessLogQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct UserKeyRow {
    id: String,
    key_fingerprint: String,
    label: Option<String>,
    created_at: i64,
    last_used_at: Option<i64>,
    revoked_at: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct TempUserRow {
    id: String,
    created_by_user_id: String,
    display_name: String,
    root_kind: String,
    root_key: String,
    relative_path: String,
    can_write: i64,
    expires_at: i64,
    revoked_at: Option<i64>,
    created_at: i64,
    restored_from_id: Option<String>,
    key_fingerprint: Option<String>,
    public_key: Option<String>,
    last_used_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct SftpAccessLogRow {
    id: String,
    principal_kind: String,
    principal_id: String,
    occurred_at: i64,
    action: String,
    root_key: Option<String>,
    path: Option<String>,
    ip: Option<String>,
    success: i64,
    error: Option<String>,
}

enum ApiValidationError {
    BadRequest(String),
    Forbidden(&'static str),
}

impl ApiValidationError {
    fn bad_request(msg: impl ToString) -> Self {
        Self::BadRequest(msg.to_string())
    }
}

impl IntoResponse for ApiValidationError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(msg) => (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": msg})),
            )
                .into_response(),
            Self::Forbidden(msg) => (
                axum::http::StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": msg})),
            )
                .into_response(),
        }
    }
}

pub async fn list_user_keys(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    let rows = sqlx::query_as::<_, UserKeyRow>(
        r#"
        SELECT id, key_fingerprint, label, created_at, last_used_at, revoked_at
        FROM user_public_keys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(&user.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    Ok(Json(serde_json::json!({ "keys": rows })))
}

pub async fn add_user_key(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<AddUserKeyRequest>,
) -> Result<impl IntoResponse, Response> {
    let normalized = normalize_public_key(&body.public_key).map_err(bad_request)?;
    let now = chrono::Utc::now().timestamp_millis();
    let id = uuid::Uuid::new_v4().to_string();
    let label = body
        .label
        .or(normalized.comment.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    sqlx::query(
        r#"
        INSERT INTO user_public_keys
            (id, user_id, key_fingerprint, public_key, label, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(&id)
    .bind(&user.user_id)
    .bind(&normalized.fingerprint)
    .bind(&normalized.public_key)
    .bind(&label)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            conflict("public key already exists")
        } else {
            db_error(e)
        }
    })?;

    Ok(Json(serde_json::json!({
        "id": id,
        "key_fingerprint": normalized.fingerprint,
        "label": label,
        "created_at": now,
        "last_used_at": null,
        "revoked_at": null,
    })))
}

pub async fn revoke_user_key(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(key_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        r#"
        UPDATE user_public_keys
        SET revoked_at = $1
        WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL
        "#,
    )
    .bind(now)
    .bind(&key_id)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(not_found("key not found"));
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn list_temp_users(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;

    let rows = sqlx::query_as::<_, TempUserRow>(
        r#"
        SELECT t.id, t.created_by_user_id, t.display_name, t.root_kind,
               t.root_key, t.relative_path,
               CASE WHEN t.can_write THEN 1 ELSE 0 END AS can_write,
               t.expires_at, t.revoked_at, t.created_at, t.restored_from_id,
               k.key_fingerprint, k.public_key, k.last_used_at
        FROM sftp_temp_users t
        LEFT JOIN sftp_temp_user_keys k
            ON k.temp_user_id = t.id AND k.revoked_at IS NULL
        ORDER BY t.created_at DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let users: Vec<_> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "created_by_user_id": row.created_by_user_id,
                "display_name": row.display_name,
                "root_kind": row.root_kind,
                "root_key": row.root_key,
                "relative_path": row.relative_path,
                "can_write": row.can_write != 0,
                "expires_at": row.expires_at,
                "revoked_at": row.revoked_at,
                "created_at": row.created_at,
                "restored_from_id": row.restored_from_id,
                "key_fingerprint": row.key_fingerprint,
                "public_key": row.public_key,
                "last_used_at": row.last_used_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "users": users })))
}

pub async fn list_access_log(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<AccessLogQuery>,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;

    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);

    let rows = sqlx::query_as::<_, SftpAccessLogRow>(
        r#"
        SELECT id, principal_kind, principal_id, occurred_at, action, root_key,
               path, ip,
               CASE WHEN success THEN 1 ELSE 0 END AS success,
               error
        FROM sftp_access_log
        ORDER BY occurred_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sftp_access_log")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let entries: Vec<_> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "principal_kind": row.principal_kind,
                "principal_id": row.principal_id,
                "occurred_at": row.occurred_at,
                "action": row.action,
                "root_key": row.root_key,
                "path": row.path,
                "ip": row.ip,
                "success": row.success != 0,
                "error": row.error,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "entries": entries,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn create_temp_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CreateTempUserRequest>,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;
    validate_expiry(body.expires_in).map_err(api_error)?;

    let display_name = body.display_name.trim();
    if display_name.is_empty() {
        return Err(bad_request("display_name is required"));
    }

    let root_kind =
        validate_admin_target(&state, &user, &body.root_key, &body.path).map_err(api_error)?;
    let normalized = normalize_public_key(&body.public_key).map_err(bad_request)?;

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = now + body.expires_in * 1000;
    let temp_user_id = uuid::Uuid::new_v4().to_string();
    let key_id = uuid::Uuid::new_v4().to_string();

    let mut tx = state.pool.begin().await.map_err(db_error)?;
    sqlx::query(
        r#"
        INSERT INTO sftp_temp_users
            (id, created_by_user_id, display_name, root_kind, root_key,
             relative_path, can_write, expires_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(&temp_user_id)
    .bind(&user.user_id)
    .bind(display_name)
    .bind(root_kind)
    .bind(&body.root_key)
    .bind(&body.path)
    .bind(body.can_write)
    .bind(expires_at)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    sqlx::query(
        r#"
        INSERT INTO sftp_temp_user_keys
            (id, temp_user_id, key_fingerprint, public_key, created_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(&key_id)
    .bind(&temp_user_id)
    .bind(&normalized.fingerprint)
    .bind(&normalized.public_key)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            conflict("public key already exists")
        } else {
            db_error(e)
        }
    })?;

    tx.commit().await.map_err(db_error)?;

    Ok(Json(serde_json::json!({
        "id": temp_user_id,
        "display_name": display_name,
        "root_kind": root_kind,
        "root_key": body.root_key,
        "relative_path": body.path,
        "can_write": body.can_write,
        "expires_at": expires_at,
        "created_at": now,
        "key_fingerprint": normalized.fingerprint,
        "login": "guest",
    })))
}

pub async fn extend_temp_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(temp_user_id): Path<String>,
    Json(body): Json<ExtendTempUserRequest>,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;
    validate_expiry(body.expires_in).map_err(api_error)?;

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = now + body.expires_in * 1000;

    let result = sqlx::query(
        r#"
        UPDATE sftp_temp_users
        SET expires_at = $1, revoked_at = NULL
        WHERE id = $2
        "#,
    )
    .bind(expires_at)
    .bind(&temp_user_id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(not_found("temporary user not found"));
    }

    Ok(Json(
        serde_json::json!({ "ok": true, "expires_at": expires_at }),
    ))
}

pub async fn revoke_temp_user(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(temp_user_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;
    let now = chrono::Utc::now().timestamp_millis();

    let result = sqlx::query(
        r#"
        UPDATE sftp_temp_users
        SET revoked_at = $1
        WHERE id = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(now)
    .bind(&temp_user_id)
    .execute(&state.pool)
    .await
    .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(not_found("temporary user not found"));
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

fn validate_admin_target(
    state: &AppState,
    _user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    path: &str,
) -> Result<&'static str, ApiValidationError> {
    let root_kind = if root_key == "~" {
        return Err(ApiValidationError::bad_request(
            "temporary SFTP users can only be created for common folders",
        ));
    } else {
        let root_path = state
            .config
            .common_folders
            .get(root_key)
            .ok_or_else(|| ApiValidationError::bad_request("root not found"))?;
        nasfiles_core::safe_path::resolve(root_path, path)
            .map_err(|_| ApiValidationError::bad_request("invalid target path"))?;
        RootKind::Common
    };

    Ok(match root_kind {
        RootKind::Common => "common",
        RootKind::Home => "home",
    })
}

fn validate_expiry(expires_in: i64) -> Result<(), ApiValidationError> {
    if ALLOWED_TEMP_USER_EXPIRIES.contains(&expires_in) {
        Ok(())
    } else {
        Err(ApiValidationError::bad_request(
            "unsupported expiration duration",
        ))
    }
}

fn require_admin(user: &nasfiles_core::models::AuthUser) -> Result<(), ApiValidationError> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiValidationError::Forbidden("admin access required"))
    }
}

fn api_error(err: ApiValidationError) -> Response {
    err.into_response()
}

fn bad_request(msg: impl ToString) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": msg.to_string()})),
    )
        .into_response()
}

fn conflict(msg: impl ToString) -> Response {
    (
        axum::http::StatusCode::CONFLICT,
        Json(serde_json::json!({"error": msg.to_string()})),
    )
        .into_response()
}

fn not_found(msg: impl ToString) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": msg.to_string()})),
    )
        .into_response()
}

fn db_error(e: sqlx::Error) -> Response {
    tracing::error!("sftp api db error: {e}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "database error"})),
    )
        .into_response()
}

pub async fn list_active_sessions(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    require_admin(&user).map_err(api_error)?;
    let sessions = state.sftp_sessions.list_guests().await;
    Ok(Json(serde_json::json!({ "sessions": sessions })))
}
