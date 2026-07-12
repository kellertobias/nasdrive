use std::path::{Path, PathBuf};

use axum::{
    Json,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::auth::middleware::CurrentUser;
use crate::fs::image_info::{self, ImageInfoProbeRequest};
use crate::shares::{access, bearer};
use crate::state::{AppState, now_ms};
use crate::thumb::cache::{ThumbFormat, ThumbnailRequest};

const GALLERY_THUMB_WIDTH: u32 = 480;
const GALLERY_PREVIEW_WIDTH: u32 = 2048;

#[derive(Clone)]
struct PreparedGalleryItem {
    id: String,
    relative_path: String,
    source_path: PathBuf,
}

#[derive(Clone, Copy)]
enum GalleryAssetKind {
    Thumbnail,
    Preview,
}

impl GalleryAssetKind {
    fn from_route(asset: &str) -> Option<Self> {
        match asset {
            "thumbnail" => Some(Self::Thumbnail),
            "preview" => Some(Self::Preview),
            _ => None,
        }
    }

    fn width(self) -> u32 {
        match self {
            Self::Thumbnail => GALLERY_THUMB_WIDTH,
            Self::Preview => GALLERY_PREVIEW_WIDTH,
        }
    }

    fn ready_column(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumbnail_ready",
            Self::Preview => "preview_ready",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GalleryJob {
    pub id: String,
    pub share_id: String,
    pub status: String,
    pub total_items: i64,
    pub processed_items: i64,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct GalleryItem {
    pub id: String,
    pub relative_path: String,
    pub filename: String,
    pub sequence: i64,
    pub source_mtime_ms: i64,
    pub source_size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub captured_at: Option<i64>,
    pub mime_type: Option<String>,
    pub thumbnail_ready: bool,
    pub preview_ready: bool,
    pub error: Option<String>,
    pub marked: bool,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GalleryFeedbackRequest {
    pub marked: bool,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GalleryAssetQuery {
    pub t: Option<String>,
}

pub fn spawn_gallery_preparation(state: AppState, share_id: String) {
    tokio::spawn(async move {
        if let Err(e) = prepare_gallery(state.clone(), &share_id).await {
            tracing::error!("gallery preparation failed for share {share_id}: {e}");
            let now = now_ms();
            let _ = sqlx::query(
                "UPDATE gallery_preparation_jobs SET status = 'error', error = $1, updated_at = $2, finished_at = $3 \
                 WHERE share_id = $4 AND status IN ('indexing', 'thumbnails', 'previews')",
            )
            .bind(e.to_string())
            .bind(now)
            .bind(now)
            .bind(&share_id)
            .execute(&state.pool)
            .await;
        }
    });
}

pub async fn list_gallery_jobs(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT id, share_id, status, total_items, processed_items, error, created_at, updated_at, finished_at \
         FROM gallery_preparation_jobs WHERE owner_user_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&user.user_id)
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(rows) => {
            let jobs = rows
                .into_iter()
                .map(gallery_job_from_row)
                .collect::<Vec<_>>();
            Json(serde_json::json!({ "jobs": jobs })).into_response()
        }
        Err(e) => {
            tracing::error!("list gallery jobs error: {e}");
            internal_error()
        }
    }
}

pub async fn owner_gallery_feedback(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    AxumPath(share_id): AxumPath<String>,
) -> impl IntoResponse {
    let owner_id =
        sqlx::query_scalar::<_, String>("SELECT owner_user_id FROM shares WHERE id = $1")
            .bind(&share_id)
            .fetch_optional(&state.pool)
            .await;

    match owner_id {
        Ok(Some(owner_id)) if owner_id == user.user_id => {}
        Ok(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "share not found"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("owner gallery feedback lookup error: {e}");
            return internal_error();
        }
    }

    match load_gallery_items(&state, &share_id, true).await {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => {
            tracing::error!("owner gallery feedback error: {e}");
            internal_error()
        }
    }
}

pub async fn public_gallery(
    State(state): State<AppState>,
    AxumPath(raw_token): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    if share.share_type.as_str() != "gallery" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "share is not a gallery"})),
        )
            .into_response();
    }
    if let Err(e) = verify_gallery_bearer(&state, &headers, &share.id) {
        return e.into_response();
    }

    match load_gallery_items(&state, &share.id, false).await {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => {
            tracing::error!("public gallery load error: {e}");
            internal_error()
        }
    }
}

pub async fn public_gallery_asset(
    State(state): State<AppState>,
    AxumPath((raw_token, item_id, asset)): AxumPath<(String, String, String)>,
    Query(query): Query<GalleryAssetQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    if share.share_type.as_str() != "gallery" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "share is not a gallery"})),
        )
            .into_response();
    }
    if let Err(e) =
        verify_gallery_bearer_with_query(&state, &headers, query.t.as_deref(), &share.id)
    {
        return e.into_response();
    }

    let asset_kind = match GalleryAssetKind::from_route(&asset) {
        Some(kind) => kind,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "asset not found"})),
            )
                .into_response();
        }
    };

    let relative_path = match gallery_item_path(&state, &share.id, &item_id).await {
        Ok(Some(path)) => path,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "gallery item not found"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("gallery asset item lookup error: {e}");
            return internal_error();
        }
    };

    let resolved = match access::resolve_share_path(
        &state.pool,
        &state.config,
        &share,
        &relative_path,
    )
    .await
    {
        Ok(path) => path,
        Err(e) => return e.into_response(),
    };
    match generate_gallery_asset(
        &state,
        &share,
        &relative_path,
        &resolved,
        asset_kind.width(),
    )
    .await
    {
        Ok(Some(response)) => {
            if let Err(e) = mark_gallery_asset_ready(&state, &share.id, &item_id, asset_kind).await
            {
                tracing::warn!("failed to mark gallery asset ready: {e}");
            }
            response
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no gallery asset available"})),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("gallery asset generation failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to generate gallery asset"})),
            )
                .into_response()
        }
    }
}

pub async fn public_gallery_feedback(
    State(state): State<AppState>,
    AxumPath((raw_token, item_id)): AxumPath<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(body): Json<GalleryFeedbackRequest>,
) -> impl IntoResponse {
    let share = match access::resolve_share(&state.pool, &raw_token).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    if share.share_type.as_str() != "gallery" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "share is not a gallery"})),
        )
            .into_response();
    }
    if let Err(e) = verify_gallery_bearer(&state, &headers, &share.id) {
        return e.into_response();
    }

    match gallery_item_path(&state, &share.id, &item_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "gallery item not found"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("gallery feedback item lookup error: {e}");
            return internal_error();
        }
    }

    let note = body
        .note
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty());
    let now = now_ms();
    let result = sqlx::query(
        "INSERT INTO share_gallery_feedback (share_id, item_id, marked, note, updated_at) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT(share_id, item_id) DO UPDATE SET marked = excluded.marked, note = excluded.note, updated_at = excluded.updated_at",
    )
    .bind(&share.id)
    .bind(&item_id)
    .bind(body.marked)
    .bind(note)
    .bind(now)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => {
            tracing::error!("gallery feedback update error: {e}");
            internal_error()
        }
    }
}

async fn prepare_gallery(state: AppState, share_id: &str) -> anyhow::Result<()> {
    let share = access::resolve_share_by_id(&state.pool, share_id).await?;
    if share.share_type.as_str() != "gallery" {
        return Ok(());
    }

    let job_id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO gallery_preparation_jobs (id, share_id, owner_user_id, status, created_at, updated_at) \
         VALUES ($1, $2, $3, 'indexing', $4, $5)",
    )
    .bind(&job_id)
    .bind(&share.id)
    .bind(&share.owner_user_id)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;

    let base = access::resolve_share_path(&state.pool, &state.config, &share, "").await?;
    let images = scan_gallery_images(&base)?;
    let total = i64::try_from(images.len()).unwrap_or(i64::MAX);
    sqlx::query(
        "UPDATE gallery_preparation_jobs SET total_items = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(total)
    .bind(now_ms())
    .bind(&job_id)
    .execute(&state.pool)
    .await?;

    sqlx::query("DELETE FROM share_gallery_feedback WHERE share_id = $1")
        .bind(&share.id)
        .execute(&state.pool)
        .await?;

    sqlx::query("DELETE FROM share_gallery_items WHERE share_id = $1")
        .bind(&share.id)
        .execute(&state.pool)
        .await?;

    let mut prepared_items = Vec::with_capacity(images.len());

    for (idx, path) in images.iter().enumerate() {
        let relative_path = relative_path(&base, path);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let item_id = uuid::Uuid::new_v4().to_string();
        let metadata = std::fs::metadata(path)?;
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mime_type = mime_guess::from_path(path)
            .first()
            .map(|mime| mime.to_string());

        let mut width = None;
        let mut height = None;
        let mut captured_at = None;
        if !state.config.no_server_side_execution
            && let Ok(Some(info)) = image_info::get_or_probe(ImageInfoProbeRequest {
                cache_dir: &state.config.thumbnail_cache_dir,
                source_path: path,
                root_kind: &share.root_kind,
                root_key: &share.root_key,
                relative_path: &relative_path,
                max_image_width: state.config.thumbnail_max_image_width,
                max_image_height: state.config.thumbnail_max_image_height,
                max_alloc: state.config.thumbnail_max_image_alloc,
            })
            .await
        {
            width = Some(i64::from(info.width));
            height = Some(i64::from(info.height));
            captured_at = info
                .exif
                .get("DateTimeOriginal")
                .or_else(|| info.exif.get("DateTime"))
                .and_then(|value| parse_exif_datetime(value));
        }

        let item_now = now_ms();
        sqlx::query(
            "INSERT INTO share_gallery_items \
             (id, share_id, relative_path, filename, sequence, source_mtime_ms, source_size, width, height, captured_at, mime_type, thumbnail_ready, preview_ready, error, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
        )
        .bind(&item_id)
        .bind(&share.id)
        .bind(&relative_path)
        .bind(&filename)
        .bind(i64::try_from(idx + 1).unwrap_or(i64::MAX))
        .bind(mtime_ms)
        .bind(i64::try_from(metadata.len()).unwrap_or(i64::MAX))
        .bind(width)
        .bind(height)
        .bind(captured_at)
        .bind(mime_type)
        .bind(false)
        .bind(false)
        .bind(Option::<String>::None)
        .bind(item_now)
        .bind(item_now)
        .execute(&state.pool)
        .await?;

        prepared_items.push(PreparedGalleryItem {
            id: item_id,
            relative_path,
            source_path: path.clone(),
        });

        sqlx::query("UPDATE gallery_preparation_jobs SET processed_items = $1, updated_at = $2 WHERE id = $3")
            .bind(i64::try_from(idx + 1).unwrap_or(i64::MAX))
            .bind(now_ms())
            .bind(&job_id)
            .execute(&state.pool)
            .await?;
    }

    generate_gallery_assets_for_job(
        &state,
        &share,
        &job_id,
        &prepared_items,
        GalleryAssetKind::Thumbnail,
    )
    .await?;
    generate_gallery_assets_for_job(
        &state,
        &share,
        &job_id,
        &prepared_items,
        GalleryAssetKind::Preview,
    )
    .await?;

    let finished = now_ms();
    sqlx::query(
        "UPDATE gallery_preparation_jobs SET status = 'done', updated_at = $1, finished_at = $2 WHERE id = $3",
    )
    .bind(finished)
    .bind(finished)
    .bind(&job_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}

async fn generate_gallery_assets_for_job(
    state: &AppState,
    share: &crate::shares::model::Share,
    job_id: &str,
    items: &[PreparedGalleryItem],
    asset_kind: GalleryAssetKind,
) -> anyhow::Result<()> {
    let status = match asset_kind {
        GalleryAssetKind::Thumbnail => "thumbnails",
        GalleryAssetKind::Preview => "previews",
    };
    sqlx::query(
        "UPDATE gallery_preparation_jobs SET status = $1, total_items = $2, processed_items = 0, updated_at = $3 WHERE id = $4",
    )
    .bind(status)
    .bind(i64::try_from(items.len()).unwrap_or(i64::MAX))
    .bind(now_ms())
    .bind(job_id)
    .execute(&state.pool)
    .await?;

    for (idx, item) in items.iter().enumerate() {
        let result = generate_gallery_asset(
            state,
            share,
            &item.relative_path,
            &item.source_path,
            asset_kind.width(),
        )
        .await;

        match result {
            Ok(Some(_)) => {
                mark_gallery_asset_ready(state, &share.id, &item.id, asset_kind).await?;
            }
            Ok(None) => {
                mark_gallery_asset_error(state, &share.id, &item.id, "no gallery asset available")
                    .await?;
            }
            Err(e) => {
                mark_gallery_asset_error(state, &share.id, &item.id, &e.to_string()).await?;
            }
        }

        sqlx::query(
            "UPDATE gallery_preparation_jobs SET processed_items = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(i64::try_from(idx + 1).unwrap_or(i64::MAX))
        .bind(now_ms())
        .bind(job_id)
        .execute(&state.pool)
        .await?;
    }

    Ok(())
}

fn scan_gallery_images(base: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if is_gallery_image(&path) {
                out.push(path);
            }
        }
    }
    out.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    Ok(out)
}

fn is_gallery_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "arw"
            | "cr2"
            | "cr3"
            | "nef"
            | "nrw"
            | "raf"
            | "rw2"
            | "orf"
            | "dng"
            | "pef"
            | "srw"
    )
}

fn relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

async fn generate_gallery_asset(
    state: &AppState,
    share: &crate::shares::model::Share,
    relative_path: &str,
    source_path: &Path,
    width: u32,
) -> anyhow::Result<Option<Response>> {
    if state.config.no_server_side_execution {
        return Ok(None);
    }
    let Some(thumb_cache) = state.thumb_cache.as_ref() else {
        return Ok(None);
    };
    let result = thumb_cache
        .get_or_generate(
            ThumbnailRequest {
                source_path,
                root_kind: &share.root_kind,
                root_key: &share.root_key,
                relative_path,
                width,
                requested_format: Some(ThumbFormat::Jpeg),
            },
            &state.config,
        )
        .await?;

    Ok(result.map(|thumb| {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, thumb.content_type)
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .body(Body::from(thumb.bytes))
            .unwrap_or_else(|_| internal_error())
    }))
}

async fn mark_gallery_asset_ready(
    state: &AppState,
    share_id: &str,
    item_id: &str,
    asset_kind: GalleryAssetKind,
) -> anyhow::Result<()> {
    let now = now_ms();
    let query = format!(
        "UPDATE share_gallery_items SET {} = TRUE, error = NULL, updated_at = $1 WHERE share_id = $2 AND id = $3",
        asset_kind.ready_column()
    );
    sqlx::query(&query)
        .bind(now)
        .bind(share_id)
        .bind(item_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

async fn mark_gallery_asset_error(
    state: &AppState,
    share_id: &str,
    item_id: &str,
    error: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE share_gallery_items SET error = $1, updated_at = $2 WHERE share_id = $3 AND id = $4",
    )
    .bind(error)
    .bind(now_ms())
    .bind(share_id)
    .bind(item_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn load_gallery_items(
    state: &AppState,
    share_id: &str,
    selected_only: bool,
) -> anyhow::Result<Vec<GalleryItem>> {
    let filter = if selected_only {
        "AND COALESCE(f.marked, FALSE)"
    } else {
        ""
    };
    let rows = sqlx::query(&format!(
        "SELECT i.id, i.relative_path, i.filename, i.sequence, i.source_mtime_ms, i.source_size, \
         i.width, i.height, i.captured_at, i.mime_type, \
         CASE WHEN i.thumbnail_ready THEN 1 ELSE 0 END AS thumbnail_ready, \
         CASE WHEN i.preview_ready THEN 1 ELSE 0 END AS preview_ready, \
         i.error, CASE WHEN COALESCE(f.marked, FALSE) THEN 1 ELSE 0 END AS marked, f.note \
         FROM share_gallery_items i \
         LEFT JOIN share_gallery_feedback f ON f.share_id = i.share_id AND f.item_id = i.id \
         WHERE i.share_id = $1 {filter} ORDER BY i.sequence"
    ))
    .bind(share_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| GalleryItem {
            id: row.get("id"),
            relative_path: row.get("relative_path"),
            filename: row.get("filename"),
            sequence: row.get("sequence"),
            source_mtime_ms: row.get("source_mtime_ms"),
            source_size: row.get("source_size"),
            width: row.get("width"),
            height: row.get("height"),
            captured_at: row.get("captured_at"),
            mime_type: row.get("mime_type"),
            thumbnail_ready: row.get::<i64, _>("thumbnail_ready") != 0,
            preview_ready: row.get::<i64, _>("preview_ready") != 0,
            error: row.get("error"),
            marked: row.get::<i64, _>("marked") != 0,
            note: row.get("note"),
        })
        .collect())
}

async fn gallery_item_path(
    state: &AppState,
    share_id: &str,
    item_id: &str,
) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT relative_path FROM share_gallery_items WHERE share_id = $1 AND id = $2",
    )
    .bind(share_id)
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await?)
}

fn gallery_job_from_row(row: sqlx::any::AnyRow) -> GalleryJob {
    GalleryJob {
        id: row.get("id"),
        share_id: row.get("share_id"),
        status: row.get("status"),
        total_items: row.get("total_items"),
        processed_items: row.get("processed_items"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        finished_at: row.get("finished_at"),
    }
}

fn verify_gallery_bearer(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    share_id: &str,
) -> Result<(), bearer::BearerError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| bearer::BearerError::Invalid("missing bearer token".into()))?;
    bearer::verify_bearer(&state.config.session_secret, token, share_id)
}

fn verify_gallery_bearer_with_query(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
    share_id: &str,
) -> Result<(), bearer::BearerError> {
    if let Some(token) = query_token {
        return bearer::verify_bearer(&state.config.session_secret, token, share_id);
    }
    verify_gallery_bearer(state, headers, share_id)
}

fn parse_exif_datetime(value: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y:%m:%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal error"})),
    )
        .into_response()
}
