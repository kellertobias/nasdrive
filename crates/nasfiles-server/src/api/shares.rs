use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use sqlx::Row;

use crate::api::gallery;
use crate::auth::middleware::CurrentUser;
use crate::shares::{
    audit, create,
    model::{CreateShareRequest, UpdateShareRequest},
};
use crate::state::AppState;

/// POST /api/shares — create a new share.
pub async fn create_share(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CreateShareRequest>,
) -> Result<impl IntoResponse, create::ShareCreateError> {
    let (share, raw_token) = create::create_share(&state.pool, &state.config, &user, body).await?;
    if share.share_type.as_str() == "gallery" {
        gallery::spawn_gallery_preparation(state.clone(), share.id.clone());
    }

    let share_url = format!(
        "{}/s/{}",
        state.config.base_url.trim_end_matches('/'),
        raw_token
    );

    Ok(Json(serde_json::json!({
        "id": share.id,
        "token": raw_token,
        "url": share_url,
        "created_at": share.created_at,
        "expires_at": share.expires_at,
        "target_kind": share.target_kind,
        "share_type": share.share_type,
        "allow_upload": share.allow_upload,
        "allow_download": share.allow_download,
    })))
}

/// GET /api/shares — list current user's shares.
pub async fn list_shares(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    let rows = sqlx::query(
        r#"SELECT s.id, s.root_kind, s.root_key, s.relative_path,
                  CASE WHEN s.is_directory THEN 1 ELSE 0 END AS is_directory,
                  s.target_kind,
                  s.share_type, s.display_token,
                  CASE WHEN s.password_hash IS NOT NULL THEN 1 ELSE 0 END AS has_password,
                  CASE WHEN s.allow_upload THEN 1 ELSE 0 END AS allow_upload,
                  CASE WHEN s.allow_download THEN 1 ELSE 0 END AS allow_download,
                  s.expires_at, s.created_at, s.revoked_at,
                  (SELECT COUNT(*) FROM share_access_log sal WHERE sal.share_id = s.id) as access_count,
                  (SELECT MAX(occurred_at) FROM share_access_log sal WHERE sal.share_id = s.id) as last_accessed_at
           FROM shares s
           WHERE s.owner_user_id = $1
           ORDER BY s.created_at DESC"#,
    )
    .bind(&user.user_id)
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(rows) => {
            let shares: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "root_key": r.get::<String, _>("root_key"),
                        "relative_path": r.get::<String, _>("relative_path"),
                        "is_directory": r.get::<i64, _>("is_directory") != 0,
                        "target_kind": r.get::<String, _>("target_kind"),
                        "share_type": r.get::<String, _>("share_type"),
                        "has_password": r.get::<i64, _>("has_password") != 0,
                        "url": r.get::<Option<String>, _>("display_token").map(|token| format!("{}/s/{}", state.config.base_url.trim_end_matches('/'), token)),
                        "allow_upload": r.get::<i64, _>("allow_upload") != 0,
                        "allow_download": r.get::<i64, _>("allow_download") != 0,
                        "expires_at": r.get::<Option<i64>, _>("expires_at"),
                        "created_at": r.get::<i64, _>("created_at"),
                        "revoked_at": r.get::<Option<i64>, _>("revoked_at"),
                        "access_count": r.get::<i64, _>("access_count"),
                        "last_accessed_at": r.get::<Option<i64>, _>("last_accessed_at"),
                    })
                })
                .collect();

            Json(serde_json::json!({ "shares": shares })).into_response()
        }
        Err(e) => {
            tracing::error!("list shares error: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
                .into_response()
        }
    }
}

pub async fn update_share(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    Json(body): Json<UpdateShareRequest>,
) -> impl IntoResponse {
    update_share_for(&state, &share_id, Some(&user.user_id), body).await
}

pub(crate) async fn update_share_for(
    state: &AppState,
    share_id: &str,
    owner_user_id: Option<&str>,
    body: UpdateShareRequest,
) -> axum::response::Response {
    let prepare_gallery = matches!(
        body.share_type,
        Some(crate::shares::model::ShareType::Gallery)
    );
    let current = sqlx::query(
        "SELECT CASE WHEN is_directory THEN 1 ELSE 0 END AS is_directory, target_kind FROM shares WHERE id = $1",
    )
        .bind(share_id)
        .fetch_optional(&state.pool)
        .await;
    let Ok(Some(current)) = current else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"share not found"})),
        )
            .into_response();
    };
    if let Some(owner) = owner_user_id {
        let owns = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM shares WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(share_id)
        .bind(owner)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0)
            > 0;
        if !owns {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error":"share not found"})),
            )
                .into_response();
        }
    }
    let is_directory = current.get::<i64, _>("is_directory") != 0;
    let target_kind = current.get::<String, _>("target_kind");
    if body.password.is_some() && target_kind != "guest" {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"passwords can only be changed on guest shares"})),
        )
            .into_response();
    }
    if let Some(ref password) = body.password
        && password.len() < 4
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"password is too weak (minimum 4 characters)"})),
        )
            .into_response();
    }
    if let Some(ref kind) = body.share_type
        && *kind != crate::shares::model::ShareType::Typical
        && !is_directory
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"this share type requires a folder"})),
        )
            .into_response();
    }
    let password_hash = match body.password.as_deref() {
        Some(password) => match create::hash_password(password) {
            Ok(hash) => Some(hash),
            Err(error) => return error.into_response(),
        },
        None => None,
    };
    let result = if let Some(kind) = body.share_type {
        let (allow_download, allow_upload) = create::permissions_for_share_type(&kind);
        sqlx::query("UPDATE shares SET share_type = $1, allow_download = $2, allow_upload = $3, password_hash = COALESCE($4, password_hash) WHERE id = $5")
            .bind(kind.as_str()).bind(allow_download).bind(allow_upload).bind(password_hash).bind(share_id).execute(&state.pool).await
    } else {
        sqlx::query("UPDATE shares SET password_hash = COALESCE($1, password_hash) WHERE id = $2")
            .bind(password_hash)
            .bind(share_id)
            .execute(&state.pool)
            .await
    };
    match result {
        Ok(_) => {
            if prepare_gallery {
                gallery::spawn_gallery_preparation(state.clone(), share_id.to_string());
            }
            Json(serde_json::json!({"ok":true})).into_response()
        }
        Err(error) => {
            tracing::error!("update share error: {error}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":"internal error"})),
            )
                .into_response()
        }
    }
}

/// GET /api/shares/:id — get share details + access log.
pub async fn get_share(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query(
        r#"SELECT s.id, s.root_kind, s.root_key, s.relative_path,
                  CASE WHEN s.is_directory THEN 1 ELSE 0 END AS is_directory,
                  s.target_kind,
                  s.share_type,
                  CASE WHEN s.allow_upload THEN 1 ELSE 0 END AS allow_upload,
                  CASE WHEN s.allow_download THEN 1 ELSE 0 END AS allow_download,
                  s.expires_at, s.created_at, s.revoked_at,
                  (SELECT COUNT(*) FROM share_access_log sal WHERE sal.share_id = s.id) as access_count,
                  (SELECT MAX(occurred_at) FROM share_access_log sal WHERE sal.share_id = s.id) as last_accessed_at
           FROM shares s
           WHERE s.id = $1 AND s.owner_user_id = $2"#,
    )
    .bind(&share_id)
    .bind(&user.user_id)
    .fetch_optional(&state.pool)
    .await;

    match row {
        Ok(Some(r)) => {
            let access_log = audit::get_access_log(&state.pool, &share_id, 50)
                .await
                .unwrap_or_default();

            Json(serde_json::json!({
                "id": r.get::<String, _>("id"),
                "root_key": r.get::<String, _>("root_key"),
                "relative_path": r.get::<String, _>("relative_path"),
                "is_directory": r.get::<i64, _>("is_directory") != 0,
                "target_kind": r.get::<String, _>("target_kind"),
                "share_type": r.get::<String, _>("share_type"),
                "allow_upload": r.get::<i64, _>("allow_upload") != 0,
                "allow_download": r.get::<i64, _>("allow_download") != 0,
                "expires_at": r.get::<Option<i64>, _>("expires_at"),
                "created_at": r.get::<i64, _>("created_at"),
                "revoked_at": r.get::<Option<i64>, _>("revoked_at"),
                "access_count": r.get::<i64, _>("access_count"),
                "last_accessed_at": r.get::<Option<i64>, _>("last_accessed_at"),
                "access_log": access_log,
            }))
            .into_response()
        }
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "share not found"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("get share error: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
                .into_response()
        }
    }
}

/// DELETE /api/shares/:id — revoke a share (sets revoked_at).
pub async fn revoke_share(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().timestamp_millis();

    let result = sqlx::query(
        r#"UPDATE shares SET revoked_at = $1, revoke_reason = 'manual', revoke_source = 'manual'
           WHERE id = $2 AND owner_user_id = $3 AND revoked_at IS NULL"#,
    )
    .bind(now)
    .bind(&share_id)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(_) => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "share not found or already revoked"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("revoke share error: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
                .into_response()
        }
    }
}
