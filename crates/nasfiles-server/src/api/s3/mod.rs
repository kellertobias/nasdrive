pub mod auth;
pub mod bucket;
mod etag;
pub mod multipart;
pub mod object;
pub mod xml;

use std::{collections::HashMap, path::PathBuf};

use axum::{
    Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, head, post, put},
};

use crate::state::AppState;

use auth::{S3Auth, S3AuthError, S3Principal};

/// Build the S3-compatible API router, mounted at `/s3` in main.rs.
/// No session or CSRF middleware — SigV4 is the only auth.
pub fn router() -> Router<AppState> {
    Router::new()
        // ListBuckets
        .route("/", get(bucket::list_buckets))
        // Bucket-level operations (key = bucket name)
        .route("/{bucket}", head(bucket::head_bucket))
        .route("/{bucket}/", head(bucket::head_bucket))
        .route("/{bucket}", get(handle_bucket_get))
        .route("/{bucket}/", get(handle_bucket_get))
        // Object-level: dispatch on query params inside each handler
        .route("/{bucket}/{*key}", get(handle_get))
        .route("/{bucket}/{*key}", head(handle_head))
        .route("/{bucket}/{*key}", put(handle_put))
        .route("/{bucket}/{*key}", post(handle_post))
        .route("/{bucket}/{*key}", delete(handle_delete))
}

/// GET /{bucket} — ListObjectsV2 (or v1)
async fn handle_bucket_get(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path(bucket): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let prefix = q.get("prefix").map(String::as_str).unwrap_or("");
    let delimiter = q.get("delimiter").map(String::as_str);
    let max_keys: u32 = q
        .get("max-keys")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
        .min(1000);

    object::list_objects_inner(&state, &principal, &bucket, prefix, delimiter, max_keys).await
}

/// GET /{bucket}/{key} — GetObject OR ListParts if ?uploadId= present
async fn handle_get(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Some(upload_id) = q.get("uploadId") {
        let qp = multipart::UploadIdQuery {
            upload_id: upload_id.clone(),
        };
        return multipart::list_parts_inner(&state, &principal, &bucket, &key, &qp).await;
    }
    object::get_object_inner(&state, &principal, &bucket, &key, &headers).await
}

/// HEAD /{bucket}/{key} — HeadObject
async fn handle_head(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path((bucket, key)): Path<(String, String)>,
) -> Response {
    object::head_object_inner(&state, &principal, &bucket, &key).await
}

/// PUT /{bucket}/{key} — PutObject OR UploadPart if ?partNumber=&uploadId= present
async fn handle_put(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    if let Some(upload_id) = q.get("uploadId") {
        let part_number: u32 = q
            .get("partNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let qp = multipart::UploadPartQuery {
            upload_id: upload_id.clone(),
            part_number,
        };
        return multipart::upload_part_inner(&state, &principal, &bucket, &key, &qp, req).await;
    }
    object::put_object_inner(&state, &principal, &bucket, &key, req).await
}

/// POST /{bucket}/{key} — CreateMultipartUpload if ?uploads, else CompleteMultipartUpload if ?uploadId
async fn handle_post(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if q.contains_key("uploads") {
        return multipart::create_multipart_upload_inner(&state, &principal, &bucket, &key).await;
    }
    if let Some(upload_id) = q.get("uploadId") {
        let qp = multipart::UploadIdQuery {
            upload_id: upload_id.clone(),
        };
        return multipart::complete_multipart_upload_inner(&state, &principal, &bucket, &key, &qp)
            .await;
    }
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

/// DELETE /{bucket}/{key} — AbortMultipartUpload if ?uploadId, else DeleteObject
async fn handle_delete(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Some(upload_id) = q.get("uploadId") {
        let qp = multipart::UploadIdQuery {
            upload_id: upload_id.clone(),
        };
        return multipart::abort_multipart_upload_inner(&state, &principal, &bucket, &key, &qp)
            .await;
    }
    object::delete_object_inner(&state, &principal, &bucket, &key).await
}

/// Resolve a bucket name to a filesystem base path, enforcing permissions.
pub async fn resolve_bucket_path(
    state: &AppState,
    principal: &S3Principal,
    bucket: &str,
    write: bool,
) -> Result<PathBuf, S3AuthError> {
    match principal {
        S3Principal::UserToken { user, .. } => {
            let cap = if write {
                crate::fs::roots::RequiredCap::Write
            } else {
                crate::fs::roots::RequiredCap::Read
            };
            crate::fs::roots::resolve_root(&state.config, user, bucket, cap)
                .map_err(|_| S3AuthError::NoSuchBucket)
        }
        S3Principal::ShareCredential { share, .. } => {
            if bucket != "share" {
                return Err(S3AuthError::NoSuchBucket);
            }
            if write && !share.allow_upload {
                return Err(S3AuthError::AccessDenied);
            }
            if !write && !share.allow_download {
                return Err(S3AuthError::AccessDenied);
            }
            crate::shares::access::resolve_share_path(&state.pool, &state.config, share, "")
                .await
                .map_err(|_| S3AuthError::AccessDenied)
        }
    }
}
