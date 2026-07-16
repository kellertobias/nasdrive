use axum::{
    Json,
    extract::{Multipart, OriginalUri, Path, Query, State},
    http::Uri,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::fs::{image_info, listing, media_info, ops, stream};
use crate::shares::{access, audit, bearer, model::ShareAuthRequest};
use crate::state::AppState;
use crate::thumb::kind;
use nasfiles_core::tokens;

/// GET /api/public/shares/:token — get share metadata (no auth required).
pub async fn share_metadata(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
) -> impl IntoResponse {
    match access::resolve_share(&state.pool, &raw_token).await {
        Ok(share) => {
            // Get owner display name
            let owner_name =
                sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id = $1")
                    .bind(&share.owner_user_id)
                    .fetch_optional(&state.pool)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "Unknown".to_string());

            Json(serde_json::json!({
                "name": share.display_name(),
                "is_directory": share.is_directory,
                "requires_password": share.password_hash.is_some(),
                "owner_display_name": owner_name,
                "allow_upload": share.allow_upload,
                "allow_download": share.allow_download,
                "share_type": share.share_type,
                "expires_at": share.expires_at,
            }))
            .into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// POST /api/public/shares/:token/auth — authenticate for a share.
/// For guest shares: verifies password → returns bearer.
/// For public shares: returns bearer immediately.
pub async fn share_auth(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ShareAuthRequest>,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let token_hash = tokens::hash_token(&raw_token);

    // Check rate limiting
    if state.rate_limiter.is_rate_limited(&token_hash) {
        let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
        let ua = extract_user_agent(&headers);
        let _ =
            audit::log_access(&state.pool, &share.id, ip.as_deref(), ua, "auth_fail", None).await;
        return access::ShareAccessError::RateLimited.into_response();
    }

    // If guest share, verify password
    if share.password_hash.is_some() {
        let password = match body.password.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "password required"})),
                )
                    .into_response();
            }
        };

        match access::verify_password(&share, password) {
            Ok(true) => {
                state.rate_limiter.reset(&token_hash);
            }
            Ok(false) => {
                state.rate_limiter.record_failure(&token_hash);
                let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
                let ua = extract_user_agent(&headers);
                let _ =
                    audit::log_access(&state.pool, &share.id, ip.as_deref(), ua, "auth_fail", None)
                        .await;
                return (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "incorrect password"})),
                )
                    .into_response();
            }
            Err(e) => return e.into_response(),
        }
    }

    // Issue bearer token
    match bearer::issue_bearer(&state.config.session_secret, &share.id) {
        Ok(bearer) => {
            let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
            let ua = extract_user_agent(&headers);
            let _ =
                audit::log_access(&state.pool, &share.id, ip.as_deref(), ua, "open", None).await;

            Json(serde_json::json!({
                "bearer": bearer,
                "expires_in": 1800,
            }))
            .into_response()
        }
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
pub struct ShareListQuery {
    #[serde(default)]
    pub path: String,
}

/// GET /api/public/shares/:token/list?path= — list directory within share scope.
/// Requires bearer token in Authorization header or ?t= query param.
pub async fn share_list(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    Query(query): Query<ShareListQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    // Verify bearer
    if let Err(e) = verify_request_bearer(&state, &headers, &share.id) {
        return e.into_response();
    }

    // A share with downloads disabled (e.g. an upload-only drop box) must not
    // expose its file tree: listing leaks every name/size/mtime in scope even
    // though the bytes are withheld. Gate it like the other content endpoints.
    if let Some(resp) = reject_if_download_disallowed(share.allow_download) {
        return resp;
    }

    // Resolve the path within the share
    let resolved =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    match listing::list_directory(&resolved, !state.config.no_server_side_execution) {
        Ok(entries) => {
            let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
            let ua = extract_user_agent(&headers);
            let _ = audit::log_access(
                &state.pool,
                &share.id,
                ip.as_deref(),
                ua,
                "list",
                Some(&query.path),
            )
            .await;

            Json(serde_json::json!({
                "path": query.path,
                "entries": entries,
            }))
            .into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// GET /api/public/shares/:token/download?path= — download a file within share scope.
pub async fn share_download(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    Query(query): Query<ShareDownloadQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    // Check download permission
    if !share.allow_download {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "download not allowed"})),
        )
            .into_response();
    }

    // Verify bearer — accept from header or ?t= query param
    let bearer_token = extract_bearer(&headers).or(query.t.as_deref());
    match bearer_token {
        Some(token) => {
            if let Err(e) = bearer::verify_bearer(&state.config.session_secret, token, &share.id) {
                return e.into_response();
            }
        }
        None => {
            return bearer::BearerError::Invalid("missing bearer token".into()).into_response();
        }
    }

    let resolved =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
    let ua = extract_user_agent(&headers);
    let _ = audit::log_access(
        &state.pool,
        &share.id,
        ip.as_deref(),
        ua,
        "download",
        Some(&query.path),
    )
    .await;

    match stream::serve_file(&resolved, &headers).await {
        Ok(resp) => resp.into_response(),
        Err(e) => e.into_response(),
    }
}

/// GET /api/public/shares/:token/info?path= — get file info within share scope.
pub async fn share_info(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    Query(query): Query<ShareDownloadQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    if let Some(response) = reject_if_download_disallowed(share.allow_download) {
        return response;
    }

    if let Err(e) =
        verify_bearer_from_header_or_query(&state, &headers, &share.id, query.t.as_deref())
    {
        return e.into_response();
    }

    let resolved =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    let metadata = match std::fs::metadata(&resolved) {
        Ok(metadata) => metadata,
        Err(_) => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not found"})),
            )
                .into_response();
        }
    };

    let name = resolved
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let is_dir = metadata.is_dir();
    let size = if is_dir { 0 } else { metadata.len() };
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mime_type = if is_dir {
        None
    } else {
        mime_guess::from_path(&name).first().map(|m| m.to_string())
    };
    let has_thumbnail =
        !is_dir && kind::supports_thumbnail_path(&resolved, !state.config.no_server_side_execution);

    let media_info = if !state.config.no_server_side_execution
        && !is_dir
        && mime_type
            .as_ref()
            .is_some_and(|m| m.starts_with("video/") || m.starts_with("audio/"))
    {
        let cache_relative_path = if share.relative_path.is_empty() {
            query.path.clone()
        } else if query.path.is_empty() {
            share.relative_path.clone()
        } else {
            format!("{}/{}", share.relative_path, query.path)
        };
        match media_info::get_or_probe(
            &state.config.thumbnail_cache_dir,
            &resolved,
            &share.root_kind,
            &share.root_key,
            &cache_relative_path,
        )
        .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    "failed to read shared media info for {}: {e}",
                    resolved.display()
                );
                None
            }
        }
    } else {
        None
    };

    let image_info = if !state.config.no_server_side_execution
        && !is_dir
        && mime_type.as_ref().is_some_and(|m| m.starts_with("image/"))
    {
        let cache_relative_path = if share.relative_path.is_empty() {
            query.path.clone()
        } else if query.path.is_empty() {
            share.relative_path.clone()
        } else {
            format!("{}/{}", share.relative_path, query.path)
        };
        match image_info::get_or_probe(image_info::ImageInfoProbeRequest {
            cache_dir: &state.config.thumbnail_cache_dir,
            source_path: &resolved,
            root_kind: &share.root_kind,
            root_key: &share.root_key,
            relative_path: &cache_relative_path,
            max_image_width: state.config.thumbnail_max_image_width,
            max_image_height: state.config.thumbnail_max_image_height,
            max_alloc: state.config.thumbnail_max_image_alloc,
        })
        .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    "failed to read shared image info for {}: {e}",
                    resolved.display()
                );
                None
            }
        }
    } else {
        None
    };

    Json(serde_json::json!({
        "name": name,
        "size": size,
        "modified_at": modified_at,
        "is_dir": is_dir,
        "mime_type": mime_type,
        "has_thumbnail": has_thumbnail,
        "media_info": media_info,
        "image_info": image_info,
        "path": query.path,
    }))
    .into_response()
}

/// GET /api/public/shares/:token/preview?path= — stream a transcoded media preview.
pub async fn share_preview(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<SharePreviewQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    if let Some(response) = reject_if_download_disallowed(share.allow_download) {
        return response;
    }

    if let Err(e) =
        verify_bearer_from_header_or_query(&state, &headers, &share.id, query.t.as_deref())
    {
        return e.into_response();
    }

    let resolved =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    match state
        .media_preview
        .serve_media_preview(
            &resolved,
            query.session.as_deref(),
            query.segment.as_deref(),
            share_preview_segment_url_prefix(&uri).as_deref(),
            !state.config.no_server_side_execution,
        )
        .await
    {
        Ok(resp) => resp.into_response(),
        Err(e) => e.into_response(),
    }
}

/// GET /api/public/shares/:token/preview-status?path=...&session=...
pub async fn share_preview_status(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    Query(query): Query<SharePreviewQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    if let Some(response) = reject_if_download_disallowed(share.allow_download) {
        return response;
    }

    if let Err(e) =
        verify_bearer_from_header_or_query(&state, &headers, &share.id, query.t.as_deref())
    {
        return e.into_response();
    }

    let resolved =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    let Some(session) = query.session.as_deref() else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session is required"})),
        )
            .into_response();
    };

    match state.media_preview.status(session, &resolved) {
        Some(status) => Json(status).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "preview session not found"})),
        )
            .into_response(),
    }
}

/// POST /api/public/shares/:token/upload?path= — upload files to a share that allows uploads.
pub async fn share_upload(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    Query(query): Query<ShareListQuery>,
    headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    if !share.allow_upload {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "upload not allowed on this share"})),
        )
            .into_response();
    }

    if let Err(e) = verify_request_bearer(&state, &headers, &share.id) {
        return e.into_response();
    }

    let dest_dir =
        match access::resolve_share_path(&state.pool, &state.config, &share, &query.path).await {
            Ok(p) => p,
            Err(e) => return e.into_response(),
        };

    if !dest_dir.is_dir() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is not a directory"})),
        )
            .into_response();
    }

    let max_size = state.config.max_upload_file_size;
    let mut count = 0u32;

    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("multipart error: {e}")})),
                )
                    .into_response();
            }
        };
        let filename = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| format!("upload-{count}"));
        if let Err(e) = ops::receive_upload_raw(&dest_dir, &filename, &mut field, max_size).await {
            return e.into_response();
        }
        count += 1;
    }

    let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
    let ua = extract_user_agent(&headers);
    let _ = audit::log_access(
        &state.pool,
        &share.id,
        ip.as_deref(),
        ua,
        "upload",
        Some(&query.path),
    )
    .await;

    Json(serde_json::json!({"ok": true, "files_uploaded": count})).into_response()
}

/// POST /api/public/shares/:token/zip — download selected paths as a ZIP archive within share scope.
pub async fn share_zip(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<crate::api::files::ZipDownloadRequest>,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    // Check download permission
    if !share.allow_download {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "download not allowed"})),
        )
            .into_response();
    }

    // Verify bearer
    if let Err(e) = verify_request_bearer(&state, &headers, &share.id) {
        return e.into_response();
    }

    let mut resolved_paths = Vec::new();
    for rel_path in &body.paths {
        let resolved =
            match access::resolve_share_path(&state.pool, &state.config, &share, rel_path).await {
                Ok(p) => p,
                Err(e) => return e.into_response(),
            };
        resolved_paths.push(resolved);
    }

    let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
    let ua = extract_user_agent(&headers);
    let _ = audit::log_access(
        &state.pool,
        &share.id,
        ip.as_deref(),
        ua,
        "download_zip",
        Some(&format!("{:?}", body.paths)),
    )
    .await;

    let archive_name = if body.paths.len() == 1 {
        let name = std::path::Path::new(&body.paths[0])
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&share.root_key);
        let name = if name.is_empty() {
            &share.root_key
        } else {
            name
        };
        format!("{name}.zip")
    } else {
        "download.zip".to_string()
    };

    crate::fs::zip::stream_zip(resolved_paths, &archive_name)
        .await
        .map_err(|e| e.into_response())
        .into_response()
}

#[derive(Deserialize)]
pub struct ShareDownloadQuery {
    #[serde(default)]
    pub path: String,
    /// Bearer token via query parameter (for direct download links).
    pub t: Option<String>,
}

#[derive(Deserialize)]
pub struct SharePreviewQuery {
    #[serde(default)]
    pub path: String,
    /// Bearer token via query parameter (for browser media element URLs).
    pub t: Option<String>,
    pub session: Option<String>,
    pub segment: Option<String>,
}

/// POST /api/public/shares/:token/s3-credentials — exchange share + optional password for S3 credentials.
/// Returns a temporary access_key + secret_key pair usable with any S3 client.
pub async fn share_s3_credentials(
    State(state): State<AppState>,
    Path(raw_token): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ShareAuthRequest>,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let token_hash = tokens::hash_token(&raw_token);

    if state.rate_limiter.is_rate_limited(&token_hash) {
        let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
        let ua = extract_user_agent(&headers);
        let _ =
            audit::log_access(&state.pool, &share.id, ip.as_deref(), ua, "auth_fail", None).await;
        return access::ShareAccessError::RateLimited.into_response();
    }

    if share.password_hash.is_some() {
        let password = match body.password.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "password required"})),
                )
                    .into_response();
            }
        };
        match access::verify_password(&share, password) {
            Ok(true) => {
                state.rate_limiter.reset(&token_hash);
            }
            Ok(false) => {
                state.rate_limiter.record_failure(&token_hash);
                let ip = extract_ip(&headers, state.config.trusted_proxy_depth);
                let ua = extract_user_agent(&headers);
                let _ =
                    audit::log_access(&state.pool, &share.id, ip.as_deref(), ua, "auth_fail", None)
                        .await;
                return (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "incorrect password"})),
                )
                    .into_response();
            }
            Err(e) => return e.into_response(),
        }
    }

    // Issue temporary S3 credentials
    let id = uuid::Uuid::new_v4().to_string();
    let access_key = format!("NASDRIVE{}", tokens::generate_share_token(12));
    let secret_key = tokens::generate_share_token(32);
    let now = chrono::Utc::now().timestamp_millis();

    // Credentials expire at share expiry or in 1 hour, whichever is sooner
    let one_hour = now + 3600 * 1000;
    let expires_at = match share.expires_at {
        Some(share_exp) if share_exp < one_hour => share_exp,
        _ => one_hour,
    };

    let encrypted_secret =
        match crate::crypto::encrypt_secret(&state.config.session_secret, &secret_key) {
            Ok(v) => v,
            Err(e) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("failed to store credentials: {e}")})),
                )
                    .into_response();
            }
        };

    if let Err(e) = sqlx::query(
        "INSERT INTO s3_share_credentials (id, share_id, access_key, secret_key, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(&share.id)
    .bind(&access_key)
    .bind(&encrypted_secret)
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }

    let endpoint = format!("{}/s3", state.config.base_url.trim_end_matches('/'));
    Json(serde_json::json!({
        "access_key": access_key,
        "secret_key": secret_key,
        "expires_at": expires_at,
        "endpoint": endpoint,
        "bucket": "share",
        "region": "us-east-1",
    }))
    .into_response()
}

fn share_preview_segment_url_prefix(uri: &Uri) -> Option<String> {
    let path_and_query = uri.path_and_query()?.as_str();
    Some(format!("{path_and_query}&segment="))
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn verify_request_bearer(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    share_id: &str,
) -> Result<(), bearer::BearerError> {
    let token = extract_bearer(headers)
        .ok_or_else(|| bearer::BearerError::Invalid("missing bearer token".into()))?;
    bearer::verify_bearer(&state.config.session_secret, token, share_id)
}

fn verify_bearer_from_header_or_query(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    share_id: &str,
    query_token: Option<&str>,
) -> Result<(), bearer::BearerError> {
    let token = extract_bearer(headers)
        .or(query_token)
        .ok_or_else(|| bearer::BearerError::Invalid("missing bearer token".into()))?;
    bearer::verify_bearer(&state.config.session_secret, token, share_id)
}

fn reject_if_download_disallowed(allow_download: bool) -> Option<axum::response::Response> {
    (!allow_download).then(|| {
        (
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "download not allowed"})),
        )
            .into_response()
    })
}

/// Resolve the client IP for audit logging, honoring the configured trusted-proxy
/// depth so an unauthenticated caller cannot forge arbitrary `X-Forwarded-For`
/// entries into the share access log. Returns `None` when no proxy is trusted.
fn extract_ip(headers: &axum::http::HeaderMap, trusted_proxy_depth: u8) -> Option<String> {
    crate::auth::client_ip(headers, trusted_proxy_depth)
}

fn extract_user_agent(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_share_preview_requires_download_permission() {
        let response = reject_if_download_disallowed(false).expect("should reject");
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
        assert!(reject_if_download_disallowed(true).is_none());
    }
}
