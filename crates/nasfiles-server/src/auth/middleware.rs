use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use nasfiles_core::models::AuthUser;

use crate::config::{self, AuthMode};
use crate::state::AppState;

// @tour authentication:130 The gate every protected request passes
// Applied once in `main.rs` via `middleware::from_fn_with_state` over the whole `/api`
// router. It resolves the user three ways depending on mode:
// `refresh::maybe_refresh_groups` for SSO, `local::current_session_user` for local auth,
// and a plain session read otherwise.
//
// It then re-checks the `X-NasFiles-Request` header for state-changing methods, and finally
// inserts the resolved `AuthUser` into `request.extensions_mut()` so handlers can extract
// it.

/// Extract `AuthUser` from session. Returns 401 if not authenticated.
pub async fn require_auth(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    // @tour comment Dev bypass skips every revalidation
    // With `dev_mode` on and `dev_user` configured, the middleware fabricates an `AuthUser`
    // with `user_id: "dev-user-id"` and *persists it into the session*. It also skips
    // `maybe_refresh_groups` and `current_session_user` entirely, so none of the
    // per-request revalidation runs.
    //
    // This is what `auth_config`'s `dev_auth_bypass` flag surfaces, and what
    // `DevModeBanner` renders as a red bar in the UI. It must never be enabled in
    // production.

    // Dev bypass mode: inject a fake user
    if state.config.dev_mode
        && let Some(ref dev_user_config) = state.config.dev_user
    {
        // Check if there's already a real session
        let has_session: bool = session
            .get::<AuthUser>("user")
            .await
            .ok()
            .flatten()
            .is_some();

        if !has_session {
            let folder_permissions =
                config::compute_folder_permissions(&state.config, &dev_user_config.groups);
            let is_admin = config::is_admin(&state.config, &dev_user_config.groups);
            let has_home = config::personal_folder_allowed(&state.config, &dev_user_config.groups)
                && state.config.home_folder_root.is_some();

            let dev_user = AuthUser {
                user_id: "dev-user-id".to_string(),
                external_id: "dev:dev-user".to_string(),
                username: dev_user_config.username.clone(),
                display_name: dev_user_config.display_name.clone(),
                picture_url: None,
                folder_permissions,
                has_home,
                is_admin,
            };

            // Store in session so downstream extractors can read it
            session.insert("user", &dev_user).await.map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session error: {e}"),
                )
                    .into_response()
            })?;
        }
    }

    // Call maybe_refresh_groups unless in dev bypass mode
    let user = if !state.config.dev_mode && matches!(state.config.auth_mode, AuthMode::Sso) {
        match super::refresh::maybe_refresh_groups(&state, &session).await {
            Ok(u) => u,
            Err(super::refresh::RefreshOutcome::NoAccess) => {
                return Err((
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({"error": "access revoked"})),
                )
                    .into_response());
            }
            Err(super::refresh::RefreshOutcome::Expired) => {
                return Err((
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "session expired"})),
                )
                    .into_response());
            }
        }
    } else if !state.config.dev_mode && matches!(state.config.auth_mode, AuthMode::Local) {
        super::local::current_session_user(&state, &session).await?
    } else {
        session
            .get::<AuthUser>("user")
            .await
            .map_err(|_| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "session error"})),
                )
                    .into_response()
            })?
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "not authenticated"})),
                )
                    .into_response()
            })?
    };

    // CSRF protection for state-changing methods
    let method = request.method().clone();
    if matches!(
        method,
        axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::DELETE
    ) {
        let has_csrf_header = request
            .headers()
            .get("X-NasFiles-Request")
            .is_some_and(|v| v == "1");

        if !has_csrf_header {
            return Err((
                axum::http::StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({"error": "CSRF header missing"})),
            )
                .into_response());
        }
    }

    // Inject user into request extensions so handlers can access it
    let mut request = request;
    request.extensions_mut().insert(user);

    Ok(next.run(request).await)
}

/// Axum extractor for getting the authenticated user from request extensions.
/// Must be used on routes behind the `require_auth` middleware.
pub struct CurrentUser(pub AuthUser);

impl<S> axum::extract::FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .map(CurrentUser)
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "not authenticated"})),
                )
                    .into_response()
            })
    }
}
