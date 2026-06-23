use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};

use crate::auth::middleware::CurrentUser;
use crate::state::AppState;

const BUILD_COMMIT: &str = env!("NASFILES_BUILD_COMMIT");
const BUILD_DATE: &str = env!("NASFILES_BUILD_DATE");

/// GET /api/me — return current authenticated user info.
pub async fn me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    let roots = crate::fs::roots::visible_roots(&state.config, &user);
    let server_side_enabled = !state.config.no_server_side_execution;

    let sftp_hostname = state.config.public_hostname();
    let sftp_port = state.config.effective_sftp_port();

    Json(serde_json::json!({
        "user_id": user.user_id,
        "username": user.username,
        "display_name": user.display_name,
        "picture_url": user.picture_url,
        "is_admin": user.is_admin,
        "roots": roots,
        "custom_links": state.config.custom_links,
        "auth": {
            "mode": state.config.auth_mode.as_str(),
            "passkeys_enabled": matches!(state.config.auth_mode, crate::config::AuthMode::Local) && !state.config.disable_passkeys && state.webauthn.is_some(),
            "totp_enabled": matches!(state.config.auth_mode, crate::config::AuthMode::Local) && !state.config.disable_totp,
        },
        "capabilities": {
            "archive_extraction": server_side_enabled,
            "thumbnails": server_side_enabled,
            "media_preview_transcoding": server_side_enabled,
            "media_metadata_probe": server_side_enabled,
            "sftp_enabled": state.config.sftp_enabled,
            "sftp_hostname": sftp_hostname,
            "sftp_port": sftp_port,
        },
        "build": {
            "commit": BUILD_COMMIT,
            "date": BUILD_DATE,
        },
    }))
}

/// POST /auth/logout — destroy session and redirect to login.
pub async fn logout(session: tower_sessions::Session, headers: HeaderMap) -> Response {
    if headers
        .get("X-NasFiles-Request")
        .is_none_or(|value| value != "1")
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "CSRF header missing"})),
        )
            .into_response();
    }
    if let Err(e) = session.delete().await {
        tracing::warn!("Failed to delete session on logout: {e}");
    }
    Redirect::temporary("/login").into_response()
}
