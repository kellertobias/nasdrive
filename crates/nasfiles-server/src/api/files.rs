use axum::{
    Json,
    extract::{Multipart, OriginalUri, Path, Query, State},
    http::Uri,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::auth::middleware::CurrentUser;
use crate::fs::{archive, image_info, listing, media_info, ops, roots, stream, zip};
use crate::state::{AppState, TransferJob, TransferJobStatus, now_ms};
use crate::thumb::kind;

/// GET /api/roots — list available roots for the current user.
pub async fn list_roots(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    let roots = roots::visible_roots(&state.config, &user);
    Json(serde_json::json!({ "roots": roots }))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub path: String,
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    pub path: String,
    pub session: Option<String>,
    pub segment: Option<String>,
}

/// GET /api/files/:root/list?path=... — list directory contents.
pub async fn list_directory(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    let entries = listing::list_directory(&resolved, !state.config.no_server_side_execution)
        .map_err(|e| e.into_response())?;

    Ok(Json(serde_json::json!({
        "path": query.path,
        "entries": entries,
    })))
}

/// GET /api/files/:root/tree?path=... — list directory children (dirs only, for tree).
pub async fn list_tree(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    let dirs = listing::list_directories(&resolved, !state.config.no_server_side_execution)
        .map_err(|e| e.into_response())?;

    Ok(Json(serde_json::json!({
        "path": query.path,
        "children": dirs,
    })))
}

/// GET /api/files/:root/download?path=... — download a file with Range support.
pub async fn download_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ListQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    stream::serve_file(&resolved, &headers)
        .await
        .map_err(|e| e.into_response())
}

/// GET /api/files/:root/preview?path=... — stream a small ffmpeg media preview.
pub async fn preview_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<PreviewQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    state
        .media_preview
        .serve_media_preview(
            &resolved,
            query.session.as_deref(),
            query.segment.as_deref(),
            preview_segment_url_prefix(&uri).as_deref(),
            !state.config.no_server_side_execution,
        )
        .await
        .map_err(|e| e.into_response())
}

fn preview_segment_url_prefix(uri: &Uri) -> Option<String> {
    let path_and_query = uri.path_and_query()?.as_str();
    Some(format!("{path_and_query}&segment="))
}

/// GET /api/files/:root/preview-status?path=...&session=... — inspect ffmpeg preview status.
pub async fn preview_status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<PreviewQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    let Some(session) = query.session.as_deref() else {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session is required"})),
        )
            .into_response());
    };

    match state.media_preview.status(session, &resolved) {
        Some(status) => Ok(Json(status).into_response()),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "preview session not found"})),
        )
            .into_response()),
    }
}

/// GET /api/files/:root/info?path=... — get info about a single file/folder.
pub async fn file_info(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let resolved = nasfiles_core::safe_path::resolve(&root_path, &query.path).map_err(|e| {
        let status = match e {
            nasfiles_core::safe_path::SafePathError::Traversal => axum::http::StatusCode::FORBIDDEN,
            nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                axum::http::StatusCode::NOT_FOUND
            }
            _ => axum::http::StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
    })?;

    let metadata = std::fs::metadata(&resolved).map_err(|_| {
        (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not found"})),
        )
            .into_response()
    })?;

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

    let root_kind = if root_key == "~" { "home" } else { "common" };
    let media_info = if !state.config.no_server_side_execution
        && !is_dir
        && mime_type
            .as_ref()
            .is_some_and(|m| m.starts_with("video/") || m.starts_with("audio/"))
    {
        match media_info::get_or_probe(
            &state.config.thumbnail_cache_dir,
            &resolved,
            root_kind,
            &root_key,
            &query.path,
        )
        .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!("failed to read media info for {}: {e}", resolved.display());
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
        match image_info::get_or_probe(
            &state.config.thumbnail_cache_dir,
            &resolved,
            root_kind,
            &root_key,
            &query.path,
            state.config.thumbnail_max_image_width,
            state.config.thumbnail_max_image_height,
            state.config.thumbnail_max_image_alloc,
        )
        .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!("failed to read image info for {}: {e}", resolved.display());
                None
            }
        }
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "name": name,
        "size": size,
        "modified_at": modified_at,
        "is_dir": is_dir,
        "mime_type": mime_type,
        "has_thumbnail": has_thumbnail,
        "media_info": media_info,
        "image_info": image_info,
        "path": query.path,
    })))
}

// =======================================================================
// Write operations
// =======================================================================

/// POST /api/files/:root/mkdir — create a new directory.
pub async fn mkdir(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ops::MkdirRequest>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    ops::create_directory(&state, &user, &root_key, &body.path, &body.name).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/files/:root/rename — rename a file or directory.
pub async fn rename(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ops::RenameRequest>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    ops::rename_entry(&state, &user, &root_key, &body.path, &body.new_name).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/files/:root/move — move entries to a new parent directory.
pub async fn move_entries(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ops::MoveRequest>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    ops::move_entries(&state, &user, &root_key, &body.paths, &body.dest).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/files/:root/transfer — copy or move entries to another root/directory.
pub async fn transfer_entries(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ops::TransferRequest>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    let job_id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    let job = TransferJob {
        id: job_id.clone(),
        owner_user_id: user.user_id.clone(),
        operation: body.operation.as_str().to_string(),
        source_root: root_key.clone(),
        dest_root: body.dest_root.clone(),
        dest_path: body.dest.clone(),
        paths: body.paths.clone(),
        status: TransferJobStatus::Queued,
        total_bytes: 0,
        transferred_bytes: 0,
        total_entries: 0,
        completed_entries: 0,
        error: None,
        created_at: now,
        updated_at: now,
        finished_at: None,
        cancel_requested: false,
    };
    state.transfer_jobs.insert(job);

    let task_state = state.clone();
    let task_user = user.clone();
    let task_root = root_key;
    let task_body = body;
    let task_job_id = job_id.clone();

    tokio::spawn(async move {
        task_state.transfer_jobs.update(&task_job_id, |job| {
            job.status = TransferJobStatus::Running;
        });

        let result = ops::transfer_entries_with_progress(
            &task_state,
            &task_user,
            ops::TransferSpec {
                source_root: &task_root,
                source_paths: &task_body.paths,
                dest_root: &task_body.dest_root,
                dest_path: &task_body.dest,
                operation: task_body.operation,
            },
            |progress| {
                task_state.transfer_jobs.update(&task_job_id, |job| {
                    job.total_bytes = progress.total_bytes;
                    job.transferred_bytes = progress.transferred_bytes;
                    job.total_entries = progress.total_entries;
                    job.completed_entries = progress.completed_entries;
                });
            },
            || task_state.transfer_jobs.is_cancel_requested(&task_job_id),
        )
        .await;

        task_state.transfer_jobs.update(&task_job_id, |job| {
            job.finished_at = Some(now_ms());
            match result {
                Ok(()) => {
                    job.status = TransferJobStatus::Done;
                    job.transferred_bytes = job.total_bytes;
                    job.completed_entries = job.total_entries;
                }
                Err(ops::FileOpError::Cancelled) => {
                    job.status = TransferJobStatus::Canceled;
                    job.error = None;
                }
                Err(err) => {
                    job.status = TransferJobStatus::Error;
                    job.error = Some(err.to_string());
                }
            }
        });
    });

    Ok(Json(serde_json::json!({"ok": true, "job_id": job_id})))
}

/// POST /api/transfer-jobs/:job_id/cancel — cancel a queued/running transfer job.
pub async fn cancel_transfer_job(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(job_id): Path<String>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    let ok = state.transfer_jobs.cancel_for_user(&job_id, &user.user_id);
    Ok(Json(serde_json::json!({ "ok": ok })))
}

/// GET /api/transfer-jobs — list copy/move jobs for the current user.
pub async fn list_transfer_jobs(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, ops::FileOpError> {
    let jobs = state.transfer_jobs.list_for_user(&user.user_id);
    Ok(Json(serde_json::json!({ "jobs": jobs })))
}

/// POST /api/files/:root/delete — delete files/directories.
pub async fn delete_entries(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ops::DeleteRequest>,
) -> Result<impl IntoResponse, ops::FileOpError> {
    ops::delete_entries(&state, &user, &root_key, &body.paths).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/files/:root/upload?path=... — upload files (multipart/form-data).
pub async fn upload_file(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Query(query): Query<ListQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ops::FileOpError> {
    let max_size = state.config.max_upload_file_size;
    let mut count = 0u32;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ops::FileOpError::Io(format!("multipart error: {e}")))?
    {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("upload-{}", count));

        ops::receive_upload(
            &state,
            &user,
            &root_key,
            &query.path,
            &filename,
            &mut field,
            max_size,
        )
        .await?;

        count += 1;
    }

    Ok(Json(
        serde_json::json!({"ok": true, "files_uploaded": count}),
    ))
}

/// POST /api/files/:root/extract — extract an archive in-place.
pub async fn extract_archive(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ExtractArchiveRequest>,
) -> Result<impl IntoResponse, archive::ArchiveError> {
    archive::extract_archive(&state, &user, &root_key, &body.path, body.mode).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/files/:root/zip — download selected paths as a ZIP archive.
pub async fn download_zip(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(root_key): Path<String>,
    Json(body): Json<ZipDownloadRequest>,
) -> Result<impl IntoResponse, axum::response::Response> {
    let root_path = roots::resolve_root(&state.config, &user, &root_key, roots::RequiredCap::Read)
        .map_err(|e| e.into_response())?;

    let mut resolved_paths = Vec::new();
    for rel_path in &body.paths {
        let resolved = nasfiles_core::safe_path::resolve(&root_path, rel_path).map_err(|e| {
            let status = match e {
                nasfiles_core::safe_path::SafePathError::Traversal => {
                    axum::http::StatusCode::FORBIDDEN
                }
                nasfiles_core::safe_path::SafePathError::NotFound(_) => {
                    axum::http::StatusCode::NOT_FOUND
                }
                _ => axum::http::StatusCode::BAD_REQUEST,
            };
            (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        })?;
        resolved_paths.push(resolved);
    }

    let archive_name = if body.paths.len() == 1 {
        let name = std::path::Path::new(&body.paths[0])
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");
        format!("{name}.zip")
    } else {
        "download.zip".to_string()
    };

    zip::stream_zip(resolved_paths, &archive_name)
        .await
        .map_err(|e| e.into_response())
}

#[derive(Deserialize)]
pub struct ZipDownloadRequest {
    pub paths: Vec<String>,
}

#[derive(Deserialize)]
pub struct ExtractArchiveRequest {
    pub path: String,
    pub mode: archive::ExtractMode,
}
