use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::CurrentUser;
use crate::state::AppState;
use nasfiles_core::tokens;

#[derive(Serialize)]
pub struct ApiTokenInfo {
    pub id: String,
    pub label: String,
    pub access_key: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub label: String,
    /// Expiry in seconds from now. None = no expiry.
    pub expires_in: Option<i64>,
}

#[derive(Serialize)]
pub struct CreatedTokenResponse {
    pub id: String,
    pub label: String,
    pub access_key: String,
    /// Secret key — returned only at creation time.
    pub secret_key: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct RenewTokenRequest {
    /// Additional seconds to extend the token's lifetime.
    pub extend_by: i64,
}

/// GET /api/profile/api-tokens
pub async fn list_tokens(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        label: String,
        access_key: String,
        created_at: i64,
        expires_at: Option<i64>,
        last_used_at: Option<i64>,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, label, access_key, created_at, expires_at, last_used_at \
         FROM user_api_tokens \
         WHERE user_id = $1 AND revoked_at IS NULL \
         ORDER BY created_at DESC",
    )
    .bind(&user.user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let tokens: Vec<ApiTokenInfo> = rows
        .into_iter()
        .map(|r| ApiTokenInfo {
            id: r.id,
            label: r.label,
            access_key: r.access_key,
            created_at: r.created_at,
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
        })
        .collect();

    Json(serde_json::json!({ "tokens": tokens }))
}

/// POST /api/profile/api-tokens
pub async fn create_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    if body.label.trim().is_empty() || body.label.len() > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "label must be 1–100 characters"})),
        )
            .into_response();
    }

    let id = uuid::Uuid::new_v4().to_string();
    // access_key: "NASDRIVE" prefix + 16 random chars (total ~24 chars, URL-safe)
    let access_key = format!("NASDRIVE{}", tokens::generate_share_token(12));
    // secret_key: 32 random bytes base64url-encoded
    let secret_key = tokens::generate_share_token(32);

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = body.expires_in.map(|s| now + s * 1000);

    let encrypted_secret =
        match crate::crypto::encrypt_secret(&state.config.session_secret, &secret_key) {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("failed to store token: {e}")})),
                )
                    .into_response();
            }
        };

    if let Err(e) = sqlx::query(
        "INSERT INTO user_api_tokens (id, user_id, label, access_key, secret_key, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&id)
    .bind(&user.user_id)
    .bind(&body.label)
    .bind(&access_key)
    .bind(&encrypted_secret)
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }

    Json(serde_json::json!(CreatedTokenResponse {
        id,
        label: body.label,
        access_key,
        secret_key,
        created_at: now,
        expires_at,
    }))
    .into_response()
}

/// PATCH /api/profile/api-tokens/:id — renew expiry
pub async fn renew_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(token_id): Path<String>,
    Json(body): Json<RenewTokenRequest>,
) -> impl IntoResponse {
    if body.extend_by <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "extend_by must be positive"})),
        )
            .into_response();
    }

    let now_ms = chrono::Utc::now().timestamp_millis();

    // Fetch current expiry to add on top of it (or now if expired)
    let current_expires: Option<i64> = sqlx::query_scalar(
        "SELECT expires_at FROM user_api_tokens WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(&token_id)
    .bind(&user.user_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .flatten();

    let new_expires = match current_expires {
        Some(exp) if exp > now_ms => exp + body.extend_by * 1000,
        _ => now_ms + body.extend_by * 1000,
    };

    let rows_affected = sqlx::query(
        "UPDATE user_api_tokens SET expires_at = $1 \
         WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(new_expires)
    .bind(&token_id)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await
    .map(|r| r.rows_affected())
    .unwrap_or(0);

    if rows_affected == 0 {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "token not found"})),
        )
            .into_response();
    }

    Json(serde_json::json!({ "ok": true, "expires_at": new_expires })).into_response()
}

/// DELETE /api/profile/api-tokens/:id
pub async fn revoke_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().timestamp_millis();

    let rows_affected = sqlx::query(
        "UPDATE user_api_tokens SET revoked_at = $1 \
         WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&token_id)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await
    .map(|r| r.rows_affected())
    .unwrap_or(0);

    if rows_affected == 0 {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "token not found"})),
        )
            .into_response();
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}
