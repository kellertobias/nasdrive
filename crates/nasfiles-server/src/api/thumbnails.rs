use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::auth::middleware::CurrentUser;
use crate::fs::roots;
use crate::state::AppState;
use crate::thumb::cache::ThumbFormat;

#[derive(Deserialize)]
pub struct ThumbnailQuery {
    pub path: String,
    /// Target width in pixels (default: 480)
    #[serde(default = "default_width")]
    pub w: u32,
    pub format: Option<String>,
}

fn default_width() -> u32 {
    480
}

/// GET /api/files/{root}/thumbnail?path=...&w=480
///
/// Returns a bitmap thumbnail for the requested file. Supported types:
/// - Images: jpg, png, gif, webp, bmp, tiff, svg
/// - Videos: mp4, mkv, avi, mov, webm, m4v, wmv, flv (requires ffmpeg)
/// - Audio: embedded cover art or generated tag cover (requires ffmpeg/ffprobe for media data)
/// - PDFs: first page (requires pdftoppm)
/// - Text files: first lines
/// - EPUBs: cover image
///
/// Returns 404 for unsupported file types.
/// Response is cached for 24 hours.
pub async fn get_thumbnail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ThumbnailQuery>,
) -> Result<Response, Response> {
    if state.config.no_server_side_execution {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "server-side execution is disabled"})),
        )
            .into_response());
    }

    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response()
    })?;

    // Determine root_kind for cache key
    let root_kind = if root_key == "~" { "home" } else { "common" };

    // Clamp width
    let width = query.w.clamp(48, 2560);
    let requested_format = match query.format.as_deref() {
        Some("png") => Some(ThumbFormat::Png),
        Some("jpeg") | Some("jpg") => Some(ThumbFormat::Jpeg),
        _ => None,
    };

    let thumb_cache = state.thumb_cache.as_ref().ok_or_else(|| {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "thumbnails not configured"})),
        )
            .into_response()
    })?;

    let result = thumb_cache
        .get_or_generate(
            &resolved,
            root_kind,
            &root_key,
            &query.path,
            width,
            requested_format,
            &state.config,
        )
        .await
        .map_err(|e| e.into_response())?;

    match result {
        Some(thumb) => Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, thumb.content_type)
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .body(Body::from(thumb.bytes))
            .map_err(|_| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "internal error"})),
                )
                    .into_response()
            }),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "no thumbnail available for this file type"})),
        )
            .into_response()),
    }
}
