use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use sqlx::Row;

use crate::api::gallery;
use crate::auth::middleware::CurrentUser;
use crate::shares::{audit, create, model::CreateShareRequest};
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
                  s.share_type,
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
