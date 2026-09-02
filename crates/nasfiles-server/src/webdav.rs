#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use base64ct::{Base64, Encoding};
use nasfiles_core::models::AuthUser;
use tokio::io::AsyncWriteExt;

use crate::{
    auth::middleware::resolve_session_user,
    fs::roots::{self, RequiredCap},
    state::AppState,
};

const DAV_PREFIX: &str = "/webdav";

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(DAV_PREFIX, any(handle))
        .route("/webdav/", any(handle))
        .route("/webdav/{*path}", any(handle))
        .layer(middleware::from_fn_with_state(state, authenticate))
}

/// Preserve WebDAV discovery in front of the global CORS layer. Tower's CORS
/// middleware answers every OPTIONS request itself, even when it is not a
/// browser preflight, which would otherwise hide the DAV and Allow headers.
pub async fn discovery_options(request: Request, next: Next) -> Response {
    if request.method() == Method::OPTIONS
        && (request.uri().path() == DAV_PREFIX || request.uri().path().starts_with("/webdav/"))
    {
        return options();
    }
    next.run(request).await
}

async fn authenticate(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    mut request: Request,
    next: Next,
) -> Response {
    // OPTIONS is intentionally public so WebDAV clients can discover the
    // endpoint before prompting for credentials.
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }

    let user = if let Some((access_key, secret)) = basic_credentials(request.headers()) {
        match crate::api::s3::auth::verify_user_api_credential(
            &state.pool,
            &state.config,
            &access_key,
            &secret,
        )
        .await
        {
            Ok(user) => user,
            Err(error) => {
                tracing::debug!(?error, "WebDAV device credential rejected");
                return unauthorized();
            }
        }
    } else {
        match resolve_session_user(&state, &session).await {
            Ok(user) => user,
            Err(_) => return unauthorized(),
        }
    };

    request.extensions_mut().insert(user);
    next.run(request).await
}

fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = Base64::decode_vec(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            header::WWW_AUTHENTICATE,
            "Basic realm=\"NASDrive WebDAV\", charset=\"UTF-8\"",
        )
        .body(Body::from("WebDAV authentication required"))
        .expect("valid unauthorized response")
}

async fn handle(State(state): State<AppState>, request: Request) -> Response {
    if request.method() == Method::OPTIONS {
        return options();
    }

    let Some(user) = request.extensions().get::<AuthUser>().cloned() else {
        return unauthorized();
    };
    let target = match DavTarget::from_uri(request.uri()) {
        Ok(target) => target,
        Err(response) => return response,
    };

    match request.method().as_str() {
        "PROPFIND" => propfind(&state, &user, &target, request.headers()).await,
        "GET" => get(&state, &user, &target, request.headers()).await,
        "HEAD" => head(&state, &user, &target).await,
        "PUT" => put(&state, &user, &target, request).await,
        "MKCOL" => mkcol(&state, &user, &target).await,
        "DELETE" => delete(&state, &user, &target).await,
        "COPY" => copy_or_move(&state, &user, &target, request.headers(), false).await,
        "MOVE" => copy_or_move(&state, &user, &target, request.headers(), true).await,
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn options() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("dav", "1")
        .header(
            header::ALLOW,
            "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE",
        )
        .header("ms-author-via", "DAV")
        .body(Body::empty())
        .expect("valid OPTIONS response")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DavTarget {
    root: Option<String>,
    relative: String,
}

impl DavTarget {
    fn from_uri(uri: &Uri) -> Result<Self, Response> {
        Self::from_path(uri.path())
    }

    fn from_path(path: &str) -> Result<Self, Response> {
        let suffix = path
            .strip_prefix(DAV_PREFIX)
            .ok_or_else(|| StatusCode::NOT_FOUND.into_response())?;
        if !suffix.is_empty() && !suffix.starts_with('/') {
            return Err(StatusCode::NOT_FOUND.into_response());
        }

        let mut segments = Vec::new();
        for raw in suffix.trim_matches('/').split('/') {
            if raw.is_empty() {
                continue;
            }
            let decoded = percent_decode(raw).ok_or_else(|| bad_request("invalid DAV path"))?;
            if decoded == "."
                || decoded == ".."
                || decoded.contains('/')
                || decoded.contains('\\')
                || decoded.contains('\0')
            {
                return Err(bad_request("invalid DAV path"));
            }
            segments.push(decoded);
        }

        if segments.is_empty() {
            return Ok(Self {
                root: None,
                relative: String::new(),
            });
        }
        Ok(Self {
            root: Some(segments.remove(0)),
            relative: segments.join("/"),
        })
    }

    fn href(&self, collection: bool) -> String {
        let mut href = String::from(DAV_PREFIX);
        if let Some(root) = &self.root {
            href.push('/');
            href.push_str(&percent_encode(root));
            for segment in self.relative.split('/').filter(|s| !s.is_empty()) {
                href.push('/');
                href.push_str(&percent_encode(segment));
            }
        }
        if collection && !href.ends_with('/') {
            href.push('/');
        }
        href
    }

    fn child(&self, name: &str) -> Self {
        let relative = if self.relative.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", self.relative, name)
        };
        Self {
            root: self.root.clone(),
            relative,
        }
    }
}

async fn propfind(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
) -> Response {
    let depth = headers
        .get("depth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("infinity");
    if !matches!(depth, "0" | "1") {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(Body::from(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:error xmlns:D=\"DAV:\"><D:propfind-finite-depth/></D:error>",
            ))
            .expect("valid depth response");
    }

    let mut resources = Vec::new();
    if target.root.is_none() {
        resources.push(DavResource::virtual_root());
        if depth == "1" {
            for root in roots::visible_roots(&state.config, user) {
                let child = DavTarget {
                    root: Some(root.key),
                    relative: String::new(),
                };
                if let Ok(path) = resolve_existing(state, user, &child, RequiredCap::Read) {
                    resources.push(DavResource::from_path(child, path).await);
                }
            }
        }
    } else {
        let path = match resolve_existing(state, user, target, RequiredCap::Read) {
            Ok(path) => path,
            Err(response) => return response,
        };
        let resource = DavResource::from_path(target.clone(), path.clone()).await;
        let is_dir = resource.is_collection;
        resources.push(resource);

        if depth == "1" && is_dir {
            let mut entries = match tokio::fs::read_dir(&path).await {
                Ok(entries) => entries,
                Err(error) => return io_response(error),
            };
            let mut children = Vec::new();
            loop {
                match entries.next_entry().await {
                    Ok(Some(entry)) => {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let child = target.child(&name);
                        // Re-resolve every child through the containment chokepoint;
                        // escaping symlinks are omitted rather than exposed.
                        if let Ok(path) = resolve_existing(state, user, &child, RequiredCap::Read) {
                            children.push(DavResource::from_path(child, path).await);
                        }
                    }
                    Ok(None) => break,
                    Err(error) => return io_response(error),
                }
            }
            children.sort_by(|a, b| a.display_name.cmp(&b.display_name));
            resources.extend(children);
        }
    }

    let mut xml =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\">");
    for resource in resources {
        xml.push_str(&resource.xml());
    }
    xml.push_str("</D:multistatus>");

    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("dav", "1")
        .body(Body::from(xml))
        .expect("valid PROPFIND response")
}

struct DavResource {
    target: DavTarget,
    display_name: String,
    is_collection: bool,
    size: u64,
    modified: Option<std::time::SystemTime>,
    content_type: String,
}

impl DavResource {
    fn virtual_root() -> Self {
        Self {
            target: DavTarget {
                root: None,
                relative: String::new(),
            },
            display_name: "NASDrive".to_string(),
            is_collection: true,
            size: 0,
            modified: None,
            content_type: "httpd/unix-directory".to_string(),
        }
    }

    async fn from_path(target: DavTarget, path: PathBuf) -> Self {
        let metadata = tokio::fs::metadata(&path).await.ok();
        let is_collection = metadata.as_ref().is_some_and(|m| m.is_dir());
        let display_name = target
            .relative
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| target.root.clone())
            .unwrap_or_else(|| "NASDrive".to_string());
        let content_type = if is_collection {
            "httpd/unix-directory".to_string()
        } else {
            mime_guess::from_path(&path)
                .first_raw()
                .unwrap_or("application/octet-stream")
                .to_string()
        };
        Self {
            target,
            display_name,
            is_collection,
            size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
            modified: metadata.and_then(|m| m.modified().ok()),
            content_type,
        }
    }

    fn xml(&self) -> String {
        let resource_type = if self.is_collection {
            "<D:collection/>"
        } else {
            ""
        };
        let modified = self
            .modified
            .map(httpdate::fmt_http_date)
            .unwrap_or_default();
        let modified_secs = self
            .modified
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs());
        let etag = format!("\"{:x}-{:x}\"", self.size, modified_secs);
        format!(
            "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:displayname>{}</D:displayname><D:resourcetype>{}</D:resourcetype><D:getcontentlength>{}</D:getcontentlength><D:getlastmodified>{}</D:getlastmodified><D:getetag>{}</D:getetag><D:getcontenttype>{}</D:getcontenttype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
            xml_escape(&self.target.href(self.is_collection)),
            xml_escape(&self.display_name),
            resource_type,
            self.size,
            xml_escape(&modified),
            etag,
            xml_escape(&self.content_type),
        )
    }
}

async fn get(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
) -> Response {
    let path = match resolve_existing(state, user, target, RequiredCap::Read) {
        Ok(path) => path,
        Err(response) => return response,
    };
    if path.is_dir() {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    match crate::fs::stream::serve_file(&path, headers).await {
        Ok(response) => response,
        Err(crate::fs::stream::StreamError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(crate::fs::stream::StreamError::BadRange) => {
            StatusCode::RANGE_NOT_SATISFIABLE.into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read resource: {error}"),
        )
            .into_response(),
    }
}

async fn head(state: &AppState, user: &AuthUser, target: &DavTarget) -> Response {
    let path = match resolve_existing(state, user, target, RequiredCap::Read) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) => return io_response(error),
    };
    let content_type = if metadata.is_dir() {
        "httpd/unix-directory"
    } else {
        mime_guess::from_path(&path)
            .first_raw()
            .unwrap_or("application/octet-stream")
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, metadata.len())
        .body(Body::empty())
        .expect("valid HEAD response")
}

async fn put(state: &AppState, user: &AuthUser, target: &DavTarget, request: Request) -> Response {
    let (path, existed) = match resolve_write_target(state, user, target) {
        Ok(path) => {
            let existed = path.exists();
            if existed && path.is_dir() {
                return StatusCode::METHOD_NOT_ALLOWED.into_response();
            }
            (path, existed)
        }
        Err(response) => return response,
    };

    let Some(parent) = path.parent() else {
        return bad_request("invalid target");
    };
    if !parent.is_dir() {
        return StatusCode::CONFLICT.into_response();
    }

    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|size| size > state.config.max_upload_file_size)
    {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload");
    let temp_path = parent.join(format!(
        ".webdav-upload-{}-{filename}",
        uuid::Uuid::new_v4()
    ));
    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(error) => return io_response(error),
    };
    let mut body = request.into_body().into_data_stream();
    let mut written = 0_u64;
    use futures_lite::StreamExt;
    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
            }
        };
        written = written.saturating_add(chunk.len() as u64);
        if written > state.config.max_upload_file_size {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        if let Err(error) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return io_response(error);
        }
    }
    if let Err(error) = file.flush().await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return io_response(error);
    }
    drop(file);
    if let Err(error) = tokio::fs::rename(&temp_path, &path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return io_response(error);
    }

    if existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

async fn mkcol(state: &AppState, user: &AuthUser, target: &DavTarget) -> Response {
    let path = match resolve_write_target(state, user, target) {
        Ok(path) => path,
        Err(response) => return response,
    };
    if path.exists() {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return StatusCode::CONFLICT.into_response();
    }
    match tokio::fs::create_dir(path).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(error) => io_response(error),
    }
}

async fn delete(state: &AppState, user: &AuthUser, target: &DavTarget) -> Response {
    if target.root.is_none() || target.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = match resolve_mutating_existing(state, user, target) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let result = if path.is_dir() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    };
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => io_response(error),
    }
}

async fn copy_or_move(
    state: &AppState,
    user: &AuthUser,
    source: &DavTarget,
    headers: &HeaderMap,
    move_resource: bool,
) -> Response {
    if source.root.is_none() || source.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let destination = match headers
        .get("destination")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Uri>().ok())
        .map(|uri| DavTarget::from_path(uri.path()))
    {
        Some(Ok(target)) => target,
        _ => return bad_request("missing or invalid Destination header"),
    };
    if destination.root.is_none() || destination.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }

    let source_cap = if move_resource {
        RequiredCap::Write
    } else {
        RequiredCap::Read
    };
    let source_path = match if move_resource {
        resolve_mutating_existing(state, user, source)
    } else {
        resolve_existing(state, user, source, source_cap)
    } {
        Ok(path) => path,
        Err(response) => return response,
    };
    let destination_path = match resolve_write_target(state, user, &destination) {
        Ok(path) => path,
        Err(response) => return response,
    };
    if !destination_path.parent().is_some_and(Path::is_dir) {
        return StatusCode::CONFLICT.into_response();
    }
    if source_path == destination_path
        || (source_path.is_dir() && destination_path.starts_with(&source_path))
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    let existed = destination_path.exists();
    let overwrite = headers
        .get("overwrite")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.eq_ignore_ascii_case("F"));
    if existed && !overwrite {
        return StatusCode::PRECONDITION_FAILED.into_response();
    }
    if existed && let Err(error) = remove_entry(&destination_path).await {
        return io_response(error);
    }

    let result = if move_resource {
        match tokio::fs::rename(&source_path, &destination_path).await {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                match copy_entry(&source_path, &destination_path).await {
                    Ok(()) => remove_entry(&source_path).await,
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    } else {
        copy_entry(&source_path, &destination_path).await
    };

    match result {
        Ok(()) if existed => StatusCode::NO_CONTENT.into_response(),
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(error) => io_response(error),
    }
}

async fn copy_entry(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = tokio::fs::symlink_metadata(source).await?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "WebDAV does not copy symlinks",
        ));
    }
    if metadata.is_file() {
        tokio::fs::copy(source, destination).await?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported filesystem entry",
        ));
    }

    tokio::fs::create_dir(destination).await?;
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source_dir, destination_dir)) = pending.pop() {
        let mut entries = tokio::fs::read_dir(source_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let source_child = entry.path();
            let destination_child = destination_dir.join(entry.file_name());
            let child_metadata = tokio::fs::symlink_metadata(&source_child).await?;
            if child_metadata.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "WebDAV does not copy symlinks",
                ));
            }
            if child_metadata.is_dir() {
                tokio::fs::create_dir(&destination_child).await?;
                pending.push((source_child, destination_child));
            } else if child_metadata.is_file() {
                tokio::fs::copy(source_child, destination_child).await?;
            }
        }
    }
    Ok(())
}

async fn remove_entry(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    }
}

fn resolve_existing(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    cap: RequiredCap,
) -> Result<PathBuf, Response> {
    let root = target
        .root
        .as_deref()
        .ok_or_else(|| StatusCode::METHOD_NOT_ALLOWED.into_response())?;
    let root_path = roots::resolve_root(&state.config, user, root, cap).map_err(root_response)?;
    nasfiles_core::safe_path::resolve(&root_path, &target.relative)
        .map_err(|_| StatusCode::NOT_FOUND.into_response())
}

fn resolve_write_target(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
) -> Result<PathBuf, Response> {
    let root = target
        .root
        .as_deref()
        .ok_or_else(|| StatusCode::FORBIDDEN.into_response())?;
    if target.relative.is_empty() {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let root_path = roots::resolve_root(&state.config, user, root, RequiredCap::Write)
        .map_err(root_response)?;
    nasfiles_core::safe_path::resolve_parent(&root_path, &target.relative)
        .map_err(|_| StatusCode::CONFLICT.into_response())
}

fn resolve_mutating_existing(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
) -> Result<PathBuf, Response> {
    let root = target
        .root
        .as_deref()
        .ok_or_else(|| StatusCode::FORBIDDEN.into_response())?;
    if target.relative.is_empty() {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let root_path = roots::resolve_root(&state.config, user, root, RequiredCap::Write)
        .map_err(root_response)?;
    let path = nasfiles_core::safe_path::resolve_parent(&root_path, &target.relative)
        .map_err(|_| StatusCode::NOT_FOUND.into_response())?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_symlink() => Ok(path),
        Ok(_) => Err(StatusCode::FORBIDDEN.into_response()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(StatusCode::NOT_FOUND.into_response())
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

fn root_response(error: roots::RootError) -> Response {
    match error {
        roots::RootError::Forbidden => StatusCode::FORBIDDEN.into_response(),
        roots::RootError::NotFound => StatusCode::NOT_FOUND.into_response(),
        roots::RootError::Internal => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn io_response(error: std::io::Error) -> Response {
    let status = match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
        std::io::ErrorKind::AlreadyExists => StatusCode::PRECONDITION_FAILED,
        std::io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        std::io::ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, error.to_string()).into_response()
}

fn bad_request(message: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, message).into_response()
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn parses_and_decodes_dav_paths() {
        assert_eq!(
            DavTarget::from_path("/webdav/Team%20Files/folder/report%20one.pdf").unwrap(),
            DavTarget {
                root: Some("Team Files".to_string()),
                relative: "folder/report one.pdf".to_string(),
            }
        );
    }

    #[test]
    fn rejects_encoded_separators_and_traversal() {
        assert!(DavTarget::from_path("/webdav/root/%2e%2e/secret").is_err());
        assert!(DavTarget::from_path("/webdav/root/a%2Fb").is_err());
        assert!(DavTarget::from_path("/webdav/root/a%5Cb").is_err());
    }

    #[test]
    fn href_encodes_each_path_segment() {
        let target = DavTarget {
            root: Some("Team Files".to_string()),
            relative: "reports/Q&A.pdf".to_string(),
        };
        assert_eq!(target.href(false), "/webdav/Team%20Files/reports/Q%26A.pdf");
    }

    #[test]
    fn basic_auth_requires_access_key_and_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic TkFTRFJJVkU6c2VjcmV0"),
        );
        assert_eq!(
            basic_credentials(&headers),
            Some(("NASDRIVE".to_string(), "secret".to_string()))
        );
    }

    #[test]
    fn basic_auth_accepts_case_insensitive_scheme_and_padding() {
        let encoded =
            Base64::encode_string(b"NASDRIVE-device:forty-three-character-device-secret-value-123");
        assert!(encoded.ends_with('='));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("bAsIc {encoded}")).unwrap(),
        );
        assert_eq!(
            basic_credentials(&headers),
            Some((
                "NASDRIVE-device".to_string(),
                "forty-three-character-device-secret-value-123".to_string()
            ))
        );
    }
}
