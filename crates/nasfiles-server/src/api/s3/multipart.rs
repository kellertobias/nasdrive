use std::path::{Path, PathBuf};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::io::AsyncWriteExt;

use crate::state::AppState;

use super::{auth::S3Principal, etag::compute_etag, object::xml_error, resolve_bucket_path, xml};

pub struct UploadPartQuery {
    pub upload_id: String,
    pub part_number: u32,
}

pub struct UploadIdQuery {
    pub upload_id: String,
}

// ---- CreateMultipartUpload ----

pub async fn create_multipart_upload_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
) -> Response {
    if let Err(e) = resolve_bucket_path(state, principal, bucket, true).await {
        return e.into_response();
    }

    let upload_id = uuid::Uuid::new_v4().to_string();
    let principal_str = format_principal(principal);
    let now = chrono::Utc::now().timestamp_millis();

    if let Err(e) = sqlx::query(
        "INSERT INTO s3_multipart_uploads (upload_id, bucket, key, principal, created_at, part_count) \
         VALUES ($1, $2, $3, $4, $5, 0)",
    )
    .bind(&upload_id)
    .bind(bucket)
    .bind(key)
    .bind(&principal_str)
    .bind(now)
    .execute(&state.pool)
    .await
    {
        return xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        );
    }

    let stage_dir = stage_dir(state, &upload_id);
    if let Err(e) = tokio::fs::create_dir_all(&stage_dir).await {
        return xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        );
    }

    let body = xml::create_multipart_upload_xml(bucket, key, &upload_id);
    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

// ---- UploadPart ----

pub async fn upload_part_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    q: &UploadPartQuery,
    req: axum::extract::Request,
) -> Response {
    let row = match get_upload(state, &q.upload_id, bucket, key).await {
        Some(r) => r,
        None => return xml_error(StatusCode::NOT_FOUND, "NoSuchUpload", "upload not found"),
    };
    if row.principal != format_principal(principal) {
        return xml_error(StatusCode::FORBIDDEN, "AccessDenied", "not your upload");
    }
    if q.part_number < 1 || q.part_number > 10_000 {
        return xml_error(
            StatusCode::BAD_REQUEST,
            "InvalidPart",
            "part number must be 1–10000",
        );
    }

    let max_size = state.config.max_upload_file_size;
    let stage = stage_dir(state, &q.upload_id);
    let part_path = stage.join(format!("part-{:05}", q.part_number));

    let mut file = match tokio::fs::File::create(&part_path).await {
        Ok(f) => f,
        Err(e) => {
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            );
        }
    };

    let expected_sha = super::object::expected_payload_sha256(req.headers());
    let mut hasher = expected_sha
        .as_ref()
        .map(|_| <sha2::Sha256 as sha2::Digest>::new());
    let mut body = req.into_body().into_data_stream();
    let mut written: u64 = 0;

    use futures_lite::StreamExt;
    while let Some(chunk) = body.next().await {
        match chunk {
            Ok(data) => {
                written += data.len() as u64;
                if written > max_size {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return xml_error(
                        StatusCode::BAD_REQUEST,
                        "EntityTooLarge",
                        "part exceeds maximum size",
                    );
                }
                if let Some(h) = hasher.as_mut() {
                    sha2::Digest::update(h, &data);
                }
                if let Err(e) = file.write_all(&data).await {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return xml_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "InternalError",
                        &e.to_string(),
                    );
                }
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                return xml_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    &e.to_string(),
                );
            }
        }
    }

    if let Err(e) = file.flush().await {
        let _ = tokio::fs::remove_file(&part_path).await;
        return xml_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            &e.to_string(),
        );
    }

    // Reject the part if the body doesn't match the signed payload hash.
    if let (Some(expected), Some(h)) = (&expected_sha, hasher) {
        let actual = hex::encode(sha2::Digest::finalize(h));
        if actual != *expected {
            let _ = tokio::fs::remove_file(&part_path).await;
            return xml_error(
                StatusCode::BAD_REQUEST,
                "XAmzContentSHA256Mismatch",
                "the provided x-amz-content-sha256 does not match the computed hash of the request body",
            );
        }
    }

    let _ = sqlx::query(
        "UPDATE s3_multipart_uploads SET part_count = part_count + 1 WHERE upload_id = $1",
    )
    .bind(&q.upload_id)
    .execute(&state.pool)
    .await;

    let etag = compute_etag(&part_path).await;
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

// ---- CompleteMultipartUpload ----

pub async fn complete_multipart_upload_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    q: &UploadIdQuery,
) -> Response {
    let row = match get_upload(state, &q.upload_id, bucket, key).await {
        Some(r) => r,
        None => return xml_error(StatusCode::NOT_FOUND, "NoSuchUpload", "upload not found"),
    };
    if row.principal != format_principal(principal) {
        return xml_error(StatusCode::FORBIDDEN, "AccessDenied", "not your upload");
    }

    let base_path = match resolve_bucket_path(state, principal, bucket, true).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    // Same create-then-validate pattern as put_object_inner: pre-check for
    // traversal sequences, create parent dirs, then let resolve_parent do the
    // full security check (containment + symlink guard at the final component).
    if key.contains('\0')
        || key.starts_with('/')
        || key.starts_with('\\')
        || key.split('/').any(|c| c == "..")
    {
        return xml_error(StatusCode::BAD_REQUEST, "InvalidArgument", "invalid key");
    }
    if let Some(parent) = base_path.join(key).parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            );
        }
    }

    let file_path = match nasfiles_core::safe_path::resolve_parent(&base_path, key) {
        Ok(p) => p,
        Err(_) => return xml_error(StatusCode::BAD_REQUEST, "InvalidArgument", "invalid key"),
    };

    let stage = stage_dir(state, &q.upload_id);
    let mut part_files = match collect_parts(&stage).await {
        Ok(p) => p,
        Err(e) => {
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            );
        }
    };
    part_files.sort();

    if part_files.is_empty() {
        return xml_error(StatusCode::BAD_REQUEST, "MalformedXML", "no parts uploaded");
    }

    let temp_path = file_path
        .parent()
        .unwrap_or(Path::new("/tmp"))
        .join(format!(".s3complete-{}", uuid::Uuid::new_v4()));

    let mut out = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            );
        }
    };

    for part_path in &part_files {
        let data = match tokio::fs::read(part_path).await {
            Ok(d) => d,
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return xml_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError",
                    &e.to_string(),
                );
            }
        };
        if let Err(e) = out.write_all(&data).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return xml_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "InternalError",
                &e.to_string(),
            );
        }
    }

    if let Err(e) = out.flush().await {
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

    cleanup_upload(state, &q.upload_id, &stage).await;

    let etag = compute_etag(&file_path).await;
    let location = format!("/s3/{bucket}/{key}");
    let body = xml::complete_multipart_upload_xml(&location, bucket, key, &etag);
    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

// ---- AbortMultipartUpload ----

pub async fn abort_multipart_upload_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    q: &UploadIdQuery,
) -> Response {
    let row = match get_upload(state, &q.upload_id, bucket, key).await {
        Some(r) => r,
        None => return StatusCode::NO_CONTENT.into_response(),
    };
    if row.principal != format_principal(principal) {
        return xml_error(StatusCode::FORBIDDEN, "AccessDenied", "not your upload");
    }

    let stage = stage_dir(state, &q.upload_id);
    cleanup_upload(state, &q.upload_id, &stage).await;
    StatusCode::NO_CONTENT.into_response()
}

// ---- ListParts ----

pub async fn list_parts_inner(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    key: &str,
    q: &UploadIdQuery,
) -> Response {
    let row = match get_upload(state, &q.upload_id, bucket, key).await {
        Some(r) => r,
        None => return xml_error(StatusCode::NOT_FOUND, "NoSuchUpload", "upload not found"),
    };
    if row.principal != format_principal(principal) {
        return xml_error(StatusCode::FORBIDDEN, "AccessDenied", "not your upload");
    }

    let stage = stage_dir(state, &q.upload_id);
    let part_files = collect_parts(&stage).await.unwrap_or_default();

    let mut parts = Vec::new();
    for path in &part_files {
        let num: u32 = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("part-"))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let size = tokio::fs::metadata(path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let etag = compute_etag(path).await;
        parts.push(xml::PartInfo {
            part_number: num,
            size,
            etag,
        });
    }

    let body = xml::list_parts_xml(bucket, key, &q.upload_id, &parts);
    (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
}

// ---- helpers ----

#[derive(sqlx::FromRow)]
pub struct UploadRow {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub principal: String,
}

async fn get_upload(
    state: &AppState,
    upload_id: &str,
    bucket: &str,
    key: &str,
) -> Option<UploadRow> {
    sqlx::query_as::<_, UploadRow>(
        "SELECT upload_id, bucket, key, principal FROM s3_multipart_uploads \
         WHERE upload_id = $1 AND bucket = $2 AND key = $3",
    )
    .bind(upload_id)
    .bind(bucket)
    .bind(key)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
}

fn stage_dir(state: &AppState, upload_id: &str) -> PathBuf {
    state.config.data_dir.join("s3-parts").join(upload_id)
}

async fn collect_parts(stage_dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut parts = Vec::new();
    let mut read_dir = tokio::fs::read_dir(stage_dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("part-") {
            parts.push(entry.path());
        }
    }
    Ok(parts)
}

async fn cleanup_upload(state: &AppState, upload_id: &str, stage_dir: &Path) {
    let _ = tokio::fs::remove_dir_all(stage_dir).await;
    let _ = sqlx::query("DELETE FROM s3_multipart_uploads WHERE upload_id = $1")
        .bind(upload_id)
        .execute(&state.pool)
        .await;
}

fn format_principal(principal: &S3Principal) -> String {
    match principal {
        S3Principal::UserToken { user_id, .. } => format!("user:{user_id}"),
        S3Principal::ShareCredential { cred_id, .. } => format!("share:{cred_id}"),
    }
}

/// Cleanup abandoned multipart uploads older than 24 hours.
pub async fn cleanup_abandoned(state: AppState) {
    let cutoff = chrono::Utc::now().timestamp_millis() - 24 * 60 * 60 * 1000;

    let rows = sqlx::query_as::<_, UploadRow>(
        "SELECT upload_id, bucket, key, principal FROM s3_multipart_uploads WHERE created_at < $1",
    )
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    for row in rows {
        let dir = stage_dir(&state, &row.upload_id);
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let _ = sqlx::query("DELETE FROM s3_multipart_uploads WHERE upload_id = $1")
            .bind(&row.upload_id)
            .execute(&state.pool)
            .await;
        tracing::debug!("cleaned up abandoned multipart upload {}", row.upload_id);
    }
}
