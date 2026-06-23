use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::state::AppState;

use super::{
    auth::{S3Auth, S3Principal},
    xml,
};

/// GET /s3/ — ListBuckets
pub async fn list_buckets(State(state): State<AppState>, S3Auth(principal): S3Auth) -> Response {
    match &principal {
        S3Principal::UserToken { user_id, user } => {
            let roots = crate::fs::roots::visible_roots(&state.config, user);
            let buckets: Vec<(String, i64)> = roots
                .into_iter()
                .map(|r| (r.key, chrono::Utc::now().timestamp_millis()))
                .collect();
            let body = xml::list_buckets_xml(user_id, &user.display_name, &buckets);
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
        S3Principal::ShareCredential { share, cred_id } => {
            let buckets = vec![("share".to_string(), share.created_at)];
            let body = xml::list_buckets_xml(cred_id, "share", &buckets);
            (StatusCode::OK, [("content-type", "application/xml")], body).into_response()
        }
    }
}

/// HEAD /s3/{bucket}/ — HeadBucket
pub async fn head_bucket(
    State(state): State<AppState>,
    S3Auth(principal): S3Auth,
    axum::extract::Path(bucket): axum::extract::Path<String>,
) -> Response {
    match super::resolve_bucket_path(&state, &principal, &bucket, false).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => e.into_response(),
    }
}
