use std::path::{Path, PathBuf};

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use tokio::io::AsyncWriteExt;

use crate::state::AppState;

use super::{auth::S3Principal, etag::compute_etag, resolve_bucket_path, xml};

pub fn xml_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        [("content-type", "application/xml")],
        xml::error_xml(code, message),
    )
        .into_response()
}

// ---- List Objects ----

pub async fn list_objects_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    prefix: &str,
    delimiter: Option<&str>,
    max_keys: u32,
) -> Response {
    let base_path = match resolve_bucket_path(state, principal, bucket, false).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let mut objects = Vec::new();
    let mut common_prefixes = Vec::new();

    collect_objects(
        &base_path,
        &base_path,
        prefix,
        delimiter,
        max_keys,
        &mut objects,
        &mut common_prefixes,
    )
    .await;

    let is_truncated = objects.len() > max_keys as usize;
    if is_truncated {
        objects.truncate(max_keys as usize);
    }

    let key_count = objects.len() as u32 + common_prefixes.len() as u32;
    let result = xml::ListObjectsV2Result {
        bucket: bucket.to_string(),
        prefix: prefix.to_string(),
        delimiter: delimiter.map(str::to_string),
        max_keys,
        is_truncated,
        key_count,
        objects,
        common_prefixes,
    };

    (
        StatusCode::OK,
        [("content-type", "application/xml")],
        xml::list_objects_v2_xml(&result),
    )
        .into_response()
}

pub async fn collect_objects(
    base_path: &Path,
    dir: &Path,
    prefix: &str,
    delimiter: Option<&str>,
    max_keys: u32,
    objects: &mut Vec<xml::S3Object>,
    common_prefixes: &mut Vec<String>,
) {
    if !dir.is_dir() {
        return;
    }
    let mut read_dir = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(_) => return,
    };

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        if objects.len() > max_keys as usize {
            break;
        }
        let entry_path = entry.path();
        let rel_key = match entry_path.strip_prefix(base_path) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if !prefix.is_empty() && !rel_key.starts_with(prefix) {
            continue;
        }

        let meta = match tokio::fs::metadata(&entry_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_dir() {
            match delimiter {
                Some("/") => {
                    let cp = format!("{rel_key}/");
                    if !common_prefixes.contains(&cp) {
                        common_prefixes.push(cp);
                    }
                }
                _ => {
                    Box::pin(collect_objects(
                        base_path,
                        &entry_path,
                        prefix,
                        delimiter,
                        max_keys,
                        objects,
                        common_prefixes,
                    ))
                    .await;
                }
            }
        } else if meta.is_file() {
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let etag = compute_etag(&entry_path).await;
            objects.push(xml::S3Object {
                key: rel_key,
                size: meta.len(),
                last_modified: modified_ms,
                etag,
            });
        }
    }
}

// ---- HeadObject ----

pub async fn head_object_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
) -> Response {
    let base_path = match resolve_bucket_path(state, principal, bucket, false).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let file_path = match nasfiles_core::safe_path::resolve(&base_path, key) {
        Ok(p) => p,
        Err(_) => return s3_not_found(),
    };

    if !file_path.is_file() {
        return s3_not_found();
    }

    let meta = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(_) => return s3_not_found(),
    };

    let etag = compute_etag(&file_path).await;
    let content_type = mime_guess::from_path(&file_path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string();
    let last_modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(d.as_secs() as i64, 0)
                .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    (
        StatusCode::OK,
        [
            ("content-type".to_string(), content_type),
            ("content-length".to_string(), meta.len().to_string()),
            ("etag".to_string(), format!("\"{etag}\"")),
            ("last-modified".to_string(), last_modified),
            ("accept-ranges".to_string(), "bytes".to_string()),
        ],
        "",
    )
        .into_response()
}

// ---- GetObject ----

pub async fn get_object_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    headers: &HeaderMap,
) -> Response {
    let base_path = match resolve_bucket_path(state, principal, bucket, false).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let file_path = match nasfiles_core::safe_path::resolve(&base_path, key) {
        Ok(p) => p,
        Err(_) => return s3_not_found(),
    };

    match crate::fs::stream::serve_file(&file_path, headers).await {
        Ok(resp) => resp,
        Err(crate::fs::stream::StreamError::NotFound) => s3_not_found(),
        Err(crate::fs::stream::StreamError::BadRange) => {
            (StatusCode::RANGE_NOT_SATISFIABLE, "invalid range").into_response()
        }
        Err(e) => xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        ),
    }
}

// ---- PutObject ----

pub async fn put_object_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    req: axum::extract::Request,
) -> Response {
    let base_path = match resolve_bucket_path(state, principal, bucket, true).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    // Pre-validate key before touching the filesystem. resolve_parent does the
    // full security check, but it requires the parent directory to exist first,
    // so we create it — safely, only after ruling out traversal sequences.
    if key.contains('\0')
        || key.starts_with('/')
        || key.starts_with('\\')
        || key.split('/').any(|c| c == "..")
    {
        return xml_error(StatusCode::BAD_REQUEST, "InvalidArgument", "invalid key");
    }
    if let Some(parent) = base_path.join(key).parent() {
        if !parent.exists() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return xml_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    &e.to_string(),
                );
            }
        }
    }

    // resolve_parent validates root containment, rejects symlinks at the final
    // component, and succeeds even when the target file does not yet exist.
    let file_path = match nasfiles_core::safe_path::resolve_parent(&base_path, key) {
        Ok(p) => p,
        Err(_) => return xml_error(StatusCode::BAD_REQUEST, "InvalidArgument", "invalid key"),
    };

    let filename = match file_path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => return xml_error(StatusCode::BAD_REQUEST, "InvalidArgument", "invalid key"),
    };

    let max_size = state.config.max_upload_file_size;
    let temp_path = file_path
        .parent()
        .unwrap_or(Path::new("/tmp"))
        .join(format!(".s3upload-{}-{filename}", uuid::Uuid::new_v4()));

    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            )
        }
    };

    let mut body = req.into_body().into_data_stream();
    let mut written: u64 = 0;

    use futures_lite::StreamExt;
    while let Some(chunk) = body.next().await {
        match chunk {
            Ok(data) => {
                written += data.len() as u64;
                if written > max_size {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    return xml_error(
                        StatusCode::BAD_REQUEST,
                        "EntityTooLarge",
                        "object exceeds maximum size",
                    );
                }
                if let Err(e) = file.write_all(&data).await {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    return xml_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "InternalError",
                        &e.to_string(),
                    );
                }
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return xml_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    &e.to_string(),
                );
            }
        }
    }

    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        let _ = std::fs::set_permissions(&temp_path, perms);
    }

    if let Err(e) = tokio::fs::rename(&temp_path, &file_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        );
    }

    let etag = compute_etag(&file_path).await;
    (
        StatusCode::OK,
        [
            ("etag".to_string(), format!("\"{etag}\"")),
            ("content-length".to_string(), "0".to_string()),
        ],
        "",
    )
        .into_response()
}

// ---- DeleteObject ----

pub async fn delete_object_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
) -> Response {
    let base_path = match resolve_bucket_path(state, principal, bucket, true).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let file_path = match nasfiles_core::safe_path::resolve(&base_path, key) {
        Ok(p) => p,
        Err(_) => return StatusCode::NO_CONTENT.into_response(),
    };

    if file_path.is_file() {
        let _ = tokio::fs::remove_file(&file_path).await;
    }

    StatusCode::NO_CONTENT.into_response()
}

fn s3_not_found() -> Response {
    xml_error(StatusCode::NOT_FOUND, "NoSuchKey", "no such object")
}
