use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::auth::middleware::CurrentUser;
use crate::fs::listing;
use crate::shares::model::UpdateShareRequest;
use crate::state::AppState;

/// Middleware check: require admin.
#[allow(clippy::result_large_err)]
fn require_admin(user: &nasfiles_core::models::AuthUser) -> Result<(), axum::response::Response> {
    if !user.is_admin {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "admin access required"})),
        )
            .into_response());
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct PaginationQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    /// Filter by status: "active", "expired", "revoked", or "all"
    #[serde(default = "default_all")]
    pub status: String,
}

fn default_limit() -> i64 {
    50
}
fn default_all() -> String {
    "all".to_string()
}

/// GET /api/admin/shares — list all shares across all users (admin only).
pub async fn list_all_shares(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<PaginationQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let limit = query.limit.clamp(1, 200);
    let offset = query.offset.max(0);

    let now_ms = chrono::Utc::now().timestamp_millis();

    let rows = sqlx::query_as::<_, ShareRow>(
        r#"
        SELECT s.id, s.owner_user_id, u.display_name AS owner_name,
               s.root_key, s.relative_path,
               CASE WHEN s.is_directory THEN 1 ELSE 0 END AS is_directory,
               s.target_kind, s.share_type, s.display_token,
               CASE WHEN s.password_hash IS NOT NULL THEN 1 ELSE 0 END AS has_password,
               CASE WHEN s.allow_upload THEN 1 ELSE 0 END AS allow_upload,
               CASE WHEN s.allow_download THEN 1 ELSE 0 END AS allow_download,
               s.expires_at, s.created_at, s.revoked_at, s.revoke_reason, s.revoke_source,
               (SELECT COUNT(*) FROM share_access_log sal WHERE sal.share_id = s.id) as access_count,
               (SELECT MAX(occurred_at) FROM share_access_log sal WHERE sal.share_id = s.id) as last_accessed_at
        FROM shares s
        JOIN users u ON u.id = s.owner_user_id
        ORDER BY s.created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("admin shares query: {e}");
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "database error"})),
        )
            .into_response()
    })?;

    // Filter in app layer (simpler than dynamic SQL with sqlx::Any)
    let filtered: Vec<_> = rows
        .into_iter()
        .filter(|s| match query.status.as_str() {
            "active" => s.revoked_at.is_none() && s.expires_at.map(|e| e > now_ms).unwrap_or(true),
            "expired" => s.expires_at.map(|e| e <= now_ms).unwrap_or(false),
            "revoked" => s.revoked_at.is_some(),
            _ => true, // "all"
        })
        .collect();

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM shares")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    let shares: Vec<serde_json::Value> = filtered
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "owner_user_id": s.owner_user_id,
                "owner_name": s.owner_name,
                "root_key": s.root_key,
                "relative_path": s.relative_path,
                "is_directory": s.is_directory != 0,
                "target_kind": s.target_kind,
                "share_type": s.share_type,
                "url": s.display_token.map(|token| format!("{}/s/{}", state.config.base_url.trim_end_matches('/'), token)),
                "has_password": s.has_password != 0,
                "allow_upload": s.allow_upload != 0,
                "allow_download": s.allow_download != 0,
                "expires_at": s.expires_at,
                "created_at": s.created_at,
                "revoked_at": s.revoked_at,
                "revoke_reason": s.revoke_reason,
                "revoke_source": s.revoke_source,
                "access_count": s.access_count,
                "last_accessed_at": s.last_accessed_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "shares": shares,
        "total": total,
        "page": (offset / limit) + 1,
        "pages": (total as f64 / limit as f64).ceil() as i64,
    })))
}

pub async fn update_share(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    Json(body): Json<UpdateShareRequest>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;
    Ok(crate::api::shares::update_share_for(&state, &share_id, None, body).await)
}

/// GET /api/admin/shares/{id} — share details, directory listing, and recent access log.
pub async fn get_share_details(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let share = fetch_share_details(&state, &share_id).await?;
    let entries = list_share_directory(&state, &share)
        .await
        .unwrap_or_else(|error| serde_json::json!({ "error": error }));
    let access_log = fetch_access_log(&state, 100, 0, Some(&share_id)).await?;

    Ok(Json(serde_json::json!({
        "share": share.to_json(&state.config.base_url),
        "listing": entries,
        "access_log": access_log,
    })))
}

/// GET /api/admin/access-log — global access log (admin only).
pub async fn list_access_log(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<AccessLogQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let limit = query.limit.clamp(1, 200);
    let offset = query.offset.max(0);

    let rows = fetch_access_log(&state, limit, offset, query.share_id.as_deref()).await?;

    let total = if let Some(share_id) = query.share_id.as_deref() {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM share_access_log WHERE share_id = $1")
            .bind(share_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0)
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM share_access_log")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0)
    };

    Ok(Json(serde_json::json!({
        "entries": rows,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

async fn fetch_access_log(
    state: &AppState,
    limit: i64,
    offset: i64,
    share_id: Option<&str>,
) -> Result<Vec<AccessLogRow>, axum::response::Response> {
    if let Some(share_id) = share_id {
        sqlx::query_as::<_, AccessLogRow>(
            r#"
            SELECT l.id, l.share_id, l.occurred_at, l.ip, l.user_agent, l.action, l.path,
                   s.root_key AS share_root_key, s.relative_path AS share_relative_path,
                   u.display_name AS share_owner_name
            FROM share_access_log l
            JOIN shares s ON s.id = l.share_id
            JOIN users u ON u.id = s.owner_user_id
            WHERE l.share_id = $1
            ORDER BY l.occurred_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(share_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("admin share access log query: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "database error"})),
            )
                .into_response()
        })
    } else {
        sqlx::query_as::<_, AccessLogRow>(
            r#"
            SELECT l.id, l.share_id, l.occurred_at, l.ip, l.user_agent, l.action, l.path,
                   s.root_key AS share_root_key, s.relative_path AS share_relative_path,
                   u.display_name AS share_owner_name
            FROM share_access_log l
            JOIN shares s ON s.id = l.share_id
            JOIN users u ON u.id = s.owner_user_id
            ORDER BY l.occurred_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("admin access log query: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "database error"})),
            )
                .into_response()
        })
    }
}

async fn fetch_share_details(
    state: &AppState,
    share_id: &str,
) -> Result<ShareDetailRow, axum::response::Response> {
    sqlx::query_as::<_, ShareDetailRow>(
        r#"
        SELECT s.id, s.owner_user_id, u.username AS owner_username,
               u.display_name AS owner_name, s.root_kind, s.root_key,
               s.relative_path,
               CASE WHEN s.is_directory THEN 1 ELSE 0 END AS is_directory,
               s.target_kind, s.share_type, s.display_token,
               CASE WHEN s.password_hash IS NOT NULL THEN 1 ELSE 0 END AS has_password,
               CASE WHEN s.allow_upload THEN 1 ELSE 0 END AS allow_upload,
               CASE WHEN s.allow_download THEN 1 ELSE 0 END AS allow_download,
               s.expires_at, s.created_at, s.revoked_at, s.revoke_reason, s.revoke_source,
               (SELECT COUNT(*) FROM share_access_log sal WHERE sal.share_id = s.id) as access_count,
               (SELECT MAX(occurred_at) FROM share_access_log sal WHERE sal.share_id = s.id) as last_accessed_at
        FROM shares s
        JOIN users u ON u.id = s.owner_user_id
        WHERE s.id = $1
        "#,
    )
    .bind(share_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("admin share details query: {e}");
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "database error"})),
        )
            .into_response()
    })?
    .ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "share not found"})),
        )
            .into_response()
    })
}

async fn list_share_directory(
    state: &AppState,
    share: &ShareDetailRow,
) -> Result<serde_json::Value, String> {
    if !share.is_directory() {
        return Ok(serde_json::json!({
            "path": share.relative_path,
            "entries": [],
            "file": true,
        }));
    }

    let root_path = if share.root_key == "~" {
        let home_root = state
            .config
            .home_folder_root
            .as_ref()
            .ok_or_else(|| "home folder root is not configured".to_string())?;
        home_root.join(nasfiles_core::models::AuthUser::sanitize_username(
            &share.owner_username,
        ))
    } else {
        state
            .config
            .common_folders
            .get(&share.root_key)
            .cloned()
            .ok_or_else(|| format!("root {} is not configured", share.root_key))?
    };

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &share.relative_path)
        .map_err(|e| e.to_string())?;
    let entries = listing::list_directory(&resolved, !state.config.no_server_side_execution)
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "path": share.relative_path,
        "entries": entries,
    }))
}

/// GET /api/admin/users — list all provisioned users (admin only).
pub async fn list_users(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let rows = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT id, username, display_name, picture_url,
               CASE WHEN is_admin THEN 1 ELSE 0 END AS is_admin,
               CASE WHEN has_home THEN 1 ELSE 0 END AS has_home,
               auth_provider, folder_permissions_json,
               created_at, last_login_at,
               (SELECT COUNT(*) FROM local_passkeys p WHERE p.user_id = users.id AND p.revoked_at IS NULL) AS passkey_count,
               (SELECT COUNT(*) FROM local_totp t WHERE t.user_id = users.id) AS totp_count,
               (SELECT COUNT(*) FROM local_totp_trusted_devices d WHERE d.user_id = users.id AND d.revoked_at IS NULL) AS trusted_device_count
        FROM users
        ORDER BY last_login_at DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("admin users query: {e}");
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "database error"})),
        )
            .into_response()
    })?;

    let users: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "username": u.username,
                "display_name": u.display_name,
                "picture_url": u.picture_url,
                "is_admin": u.is_admin != 0,
                "has_home": u.has_home != 0,
                "auth_provider": u.auth_provider,
                "folder_permissions": u.folder_permissions_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                    .unwrap_or_else(|| serde_json::json!({})),
                "passkey_count": u.passkey_count,
                "totp_enabled": u.totp_count > 0,
                "trusted_device_count": u.trusted_device_count,
                "created_at": u.created_at,
                "last_login_at": u.last_login_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "users": users })))
}

/// GET /api/admin/ip-blocklist — list blocklisted IPs (admin only).
pub async fn list_blocklist(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let entries = crate::blocklist::list(&state.pool).await.map_err(|e| {
        tracing::error!("admin blocklist query: {e}");
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "database error"})),
        )
            .into_response()
    })?;

    Ok(Json(serde_json::json!({ "entries": entries })))
}

/// POST /api/admin/ip-blocklist — manually block an IP (admin only).
pub async fn add_blocklist_entry(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<BlockIpRequest>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    // Parse + normalize to canonical form so manual entries match the
    // representation stored for automatic (SFTP) blocks.
    let ip: std::net::IpAddr = body.ip.trim().parse().map_err(|_| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid IP address"})),
        )
            .into_response()
    })?;
    let canonical = ip.to_string();

    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("manually blocked by admin");

    crate::blocklist::block(&state.pool, &canonical, reason).await;

    Ok(Json(serde_json::json!({ "ok": true, "ip": canonical })))
}

/// DELETE /api/admin/ip-blocklist/{ip} — remove an IP from the blocklist (admin only).
pub async fn remove_blocklist_entry(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(ip): Path<String>,
) -> Result<impl IntoResponse, axum::response::Response> {
    require_admin(&user)?;

    let removed = crate::blocklist::unblock(&state.pool, &ip)
        .await
        .map_err(|e| {
            tracing::error!("admin blocklist delete: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "database error"})),
            )
                .into_response()
        })?;

    if !removed {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "IP not found on blocklist"})),
        )
            .into_response());
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Query / row types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AccessLogQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub share_id: Option<String>,
}

#[derive(Deserialize)]
pub struct BlockIpRequest {
    pub ip: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct ShareRow {
    id: String,
    owner_user_id: String,
    owner_name: String,
    root_key: String,
    relative_path: String,
    is_directory: i64,
    target_kind: String,
    share_type: String,
    display_token: Option<String>,
    has_password: i64,
    allow_upload: i64,
    allow_download: i64,
    expires_at: Option<i64>,
    created_at: i64,
    revoked_at: Option<i64>,
    revoke_reason: Option<String>,
    revoke_source: Option<String>,
    access_count: i64,
    last_accessed_at: Option<i64>,
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct AccessLogRow {
    id: String,
    share_id: String,
    occurred_at: i64,
    ip: Option<String>,
    user_agent: Option<String>,
    action: String,
    path: Option<String>,
    share_root_key: String,
    share_relative_path: String,
    share_owner_name: String,
}

#[derive(sqlx::FromRow)]
struct ShareDetailRow {
    id: String,
    owner_user_id: String,
    owner_username: String,
    owner_name: String,
    root_kind: String,
    root_key: String,
    relative_path: String,
    is_directory: i64,
    target_kind: String,
    share_type: String,
    display_token: Option<String>,
    has_password: i64,
    allow_upload: i64,
    allow_download: i64,
    expires_at: Option<i64>,
    created_at: i64,
    revoked_at: Option<i64>,
    revoke_reason: Option<String>,
    revoke_source: Option<String>,
    access_count: i64,
    last_accessed_at: Option<i64>,
}

impl ShareDetailRow {
    fn is_directory(&self) -> bool {
        self.is_directory != 0
    }

    fn to_json(&self, base_url: &str) -> serde_json::Value {
        serde_json::json!({
            "id": &self.id,
            "owner_user_id": &self.owner_user_id,
            "owner_name": &self.owner_name,
            "root_kind": &self.root_kind,
            "root_key": &self.root_key,
            "relative_path": &self.relative_path,
            "is_directory": self.is_directory(),
            "target_kind": &self.target_kind,
            "share_type": &self.share_type,
            "url": self.display_token.as_ref().map(|token| format!("{}/s/{}", base_url.trim_end_matches('/'), token)),
            "has_password": self.has_password != 0,
            "allow_upload": self.allow_upload != 0,
            "allow_download": self.allow_download != 0,
            "expires_at": self.expires_at,
            "created_at": self.created_at,
            "revoked_at": self.revoked_at,
            "revoke_reason": &self.revoke_reason,
            "revoke_source": &self.revoke_source,
            "access_count": self.access_count,
            "last_accessed_at": self.last_accessed_at,
        })
    }
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct UserRow {
    id: String,
    username: String,
    display_name: String,
    picture_url: Option<String>,
    is_admin: i64,
    has_home: i64,
    auth_provider: String,
    folder_permissions_json: Option<String>,
    created_at: i64,
    last_login_at: i64,
    passkey_count: i64,
    totp_count: i64,
    trusted_device_count: i64,
}
