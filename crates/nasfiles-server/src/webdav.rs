use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header, request::Parts},
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
const MAX_XML_BODY: usize = 64 * 1024;
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_LOCK_TIMEOUT: Duration = Duration::from_secs(3600);

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

// Session-authenticated WebDAV requests deliberately skip the JSON API's
// `X-NasFiles-Request` CSRF header: native DAV clients cannot send it. Cross-site
// abuse of the session cookie is still blocked because the cookie is
// `SameSite=Lax` (never attached to cross-site PUT/DELETE/MKCOL/MOVE requests)
// and every non-simple method needs a CORS preflight that the origin-specific
// CORS layer refuses for foreign origins. Do not "fix" this by adding the header
// check, and do not relax either of those two settings without revisiting it.
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
    let decoded = Base64::decode_vec(encoded.trim()).ok()?;
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
    let (parts, body) = request.into_parts();

    match parts.method.as_str() {
        "PROPFIND" => propfind(&state, &user, &target, &parts.headers, body).await,
        "GET" => get(&state, &user, &target, &parts.headers).await,
        "HEAD" => head(&state, &user, &target).await,
        "PUT" => put(&state, &user, &target, &parts.headers, body).await,
        "MKCOL" => mkcol(&state, &user, &target, &parts.headers).await,
        "DELETE" => delete(&state, &user, &target, &parts.headers).await,
        "COPY" => copy_or_move(&state, &user, &target, &parts, false).await,
        "MOVE" => copy_or_move(&state, &user, &target, &parts, true).await,
        "LOCK" => lock(&state, &user, &target, &parts.headers, body).await,
        "UNLOCK" => unlock(&state, &user, &target, &parts.headers).await,
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn options() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("dav", "1, 2")
        .header(
            header::ALLOW,
            "OPTIONS, PROPFIND, GET, HEAD, PUT, MKCOL, DELETE, COPY, MOVE, LOCK, UNLOCK",
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

    /// Canonical key used by the lock table. Root-less targets never lock.
    fn lock_key(&self) -> String {
        match &self.root {
            Some(root) if self.relative.is_empty() => root.clone(),
            Some(root) => format!("{root}/{}", self.relative),
            None => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Locking
//
// Locks exist to satisfy DAV class 2 clients: macOS Finder mounts class-1
// servers read-only and the Windows redirector refuses writes without LOCK.
// They are advisory, in-memory, exclusive write locks that expire on their own.
// A restart forgets them, which clients handle by re-locking.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LockEntry {
    token: String,
    path: String,
    owner: Option<String>,
    depth_infinity: bool,
    timeout: Duration,
    expires_at: Instant,
}

#[derive(Clone, Default)]
pub struct LockTable {
    locks: Arc<Mutex<HashMap<String, LockEntry>>>,
}

impl LockTable {
    fn with<T>(&self, f: impl FnOnce(&mut HashMap<String, LockEntry>) -> T) -> T {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        locks.retain(|_, lock| lock.expires_at > now);
        f(&mut locks)
    }

    /// Return the live lock that blocks `path`, unless `tokens` names it.
    ///
    /// A lock blocks the exact path, every descendant when it was taken with
    /// `Depth: infinity`, and every ancestor (you cannot delete or move a folder
    /// that contains a locked child).
    fn conflict(&self, path: &str, tokens: &[String]) -> Option<LockEntry> {
        self.with(|locks| {
            locks
                .values()
                .filter(|lock| lock.covers(path) || is_descendant(path, &lock.path))
                .find(|lock| !tokens.iter().any(|t| t == &lock.token))
                .cloned()
        })
    }

    fn active_for(&self, path: &str) -> Vec<LockEntry> {
        self.with(|locks| {
            locks
                .values()
                .filter(|lock| lock.covers(path))
                .cloned()
                .collect()
        })
    }

    fn insert(&self, lock: LockEntry) {
        self.with(|locks| {
            locks.insert(lock.token.clone(), lock);
        });
    }

    fn refresh(&self, token: &str, path: &str, timeout: Duration) -> Option<LockEntry> {
        self.with(|locks| {
            let lock = locks.get_mut(token)?;
            if !lock.covers(path) {
                return None;
            }
            lock.timeout = timeout;
            lock.expires_at = Instant::now() + timeout;
            Some(lock.clone())
        })
    }

    fn remove(&self, token: &str, path: &str) -> bool {
        self.with(|locks| match locks.get(token) {
            Some(lock) if lock.path == path => {
                locks.remove(token);
                true
            }
            _ => false,
        })
    }

    /// Drop every lock at or below `path` after it was deleted or moved.
    fn remove_under(&self, path: &str) {
        self.with(|locks| {
            locks.retain(|_, lock| lock.path != path && !is_descendant(path, &lock.path));
        });
    }
}

impl LockEntry {
    fn covers(&self, path: &str) -> bool {
        self.path == path || (self.depth_infinity && is_descendant(&self.path, path))
    }

    fn xml(&self, href: &str) -> String {
        let depth = if self.depth_infinity { "infinity" } else { "0" };
        let owner = self
            .owner
            .as_deref()
            .map(|owner| format!("<D:owner>{owner}</D:owner>"))
            .unwrap_or_default();
        format!(
            "<D:activelock><D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:depth>{depth}</D:depth>{owner}<D:timeout>Second-{}</D:timeout><D:locktoken><D:href>{}</D:href></D:locktoken><D:lockroot><D:href>{}</D:href></D:lockroot></D:activelock>",
            self.timeout.as_secs(),
            xml_escape(&self.token),
            xml_escape(href),
        )
    }
}

fn is_descendant(ancestor: &str, path: &str) -> bool {
    !ancestor.is_empty()
        && path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
}

/// Lock tokens the client presents in `If:` (and, for UNLOCK, `Lock-Token:`).
/// The `If` grammar is richer than this, but tokens are the only part we act on.
fn submitted_tokens(headers: &HeaderMap) -> Vec<String> {
    let mut tokens = Vec::new();
    for name in ["if", "lock-token"] {
        for value in headers.get_all(name) {
            let Ok(value) = value.to_str() else { continue };
            let mut rest = value;
            while let Some(start) = rest.find('<') {
                let Some(end) = rest[start..].find('>') else {
                    break;
                };
                let candidate = &rest[start + 1..start + end];
                if candidate.starts_with("opaquelocktoken:") || candidate.starts_with("urn:uuid:") {
                    tokens.push(candidate.to_string());
                }
                rest = &rest[start + end + 1..];
            }
        }
    }
    tokens
}

fn locked(lock: &LockEntry) -> Response {
    Response::builder()
        .status(StatusCode::LOCKED)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body(Body::from(format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:error xmlns:D=\"DAV:\"><D:lock-token-submitted><D:href>{}</D:href></D:lock-token-submitted></D:error>",
            xml_escape(&lock.path)
        )))
        .expect("valid locked response")
}

fn lock_timeout(headers: &HeaderMap) -> Duration {
    headers
        .get("timeout")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| {
            value.split(',').map(str::trim).find_map(|entry| {
                if entry.eq_ignore_ascii_case("infinite") {
                    return Some(MAX_LOCK_TIMEOUT);
                }
                entry
                    .strip_prefix("Second-")
                    .and_then(|secs| secs.parse::<u64>().ok())
                    .map(Duration::from_secs)
            })
        })
        .map_or(DEFAULT_LOCK_TIMEOUT, |timeout| {
            timeout.min(MAX_LOCK_TIMEOUT)
        })
}

async fn lock(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let path = match resolve_write_target(state, user, target) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let body = match read_xml_body(body).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let timeout = lock_timeout(headers);
    let key = target.lock_key();

    // An empty body plus a token is a refresh (RFC 4918 §9.10.2).
    if body.is_empty() {
        let tokens = submitted_tokens(headers);
        let Some(token) = tokens.first() else {
            return bad_request("LOCK requires a body or a lock token to refresh");
        };
        return match state.webdav_locks.refresh(token, &key, timeout) {
            Some(lock) => lock_response(StatusCode::OK, &lock, target, path.is_dir()),
            None => StatusCode::PRECONDITION_FAILED.into_response(),
        };
    }

    if let Some(existing) = state.webdav_locks.conflict(&key, &[]) {
        return locked(&existing);
    }

    let depth_infinity = headers
        .get("depth")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|depth| depth != "0");
    let owner = extract_element(&body, "owner").map(|inner| xml_escape(inner.trim()));

    // Locking a missing resource creates an empty one (RFC 4918 §9.10.4).
    let mut created = false;
    if !path.exists() {
        if !path.parent().is_some_and(Path::is_dir) {
            return StatusCode::CONFLICT.into_response();
        }
        if let Err(error) = tokio::fs::File::create(&path).await {
            return io_response(error);
        }
        created = true;
    }

    let lock = LockEntry {
        token: format!("opaquelocktoken:{}", uuid::Uuid::new_v4()),
        path: key,
        owner,
        depth_infinity,
        timeout,
        expires_at: Instant::now() + timeout,
    };
    state.webdav_locks.insert(lock.clone());
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    lock_response(status, &lock, target, path.is_dir())
}

fn lock_response(
    status: StatusCode,
    lock: &LockEntry,
    target: &DavTarget,
    collection: bool,
) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:prop xmlns:D=\"DAV:\"><D:lockdiscovery>{}</D:lockdiscovery></D:prop>",
        lock.xml(&target.href(collection))
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("lock-token", format!("<{}>", lock.token))
        .body(Body::from(body))
        .expect("valid LOCK response")
}

async fn unlock(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
) -> Response {
    let Some(root) = target.root.as_deref() else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if let Err(error) = roots::resolve_root(&state.config, user, root, RequiredCap::Write) {
        return root_response(error);
    }
    let Some(token) = headers
        .get("lock-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    else {
        return bad_request("UNLOCK requires a Lock-Token header");
    };
    if state.webdav_locks.remove(&token, &target.lock_key()) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CONFLICT.into_response()
    }
}

// ---------------------------------------------------------------------------
// PROPFIND
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum PropRequest {
    AllProp,
    PropName,
    Props(Vec<String>),
}

impl PropRequest {
    /// Minimal PROPFIND body reader: recognises `allprop`, `propname` and the
    /// child element names of `prop`. Namespaces are ignored on purpose; only
    /// `DAV:` properties are served, and unknown names come back as 404.
    fn parse(body: &[u8]) -> Self {
        let text = String::from_utf8_lossy(body);
        let mut names = Vec::new();
        let mut in_prop = false;
        let mut saw_prop = false;
        for tag in xml_tags(&text) {
            match tag {
                XmlTag::Open(name) | XmlTag::Empty(name) if !in_prop => match name {
                    "propname" => return Self::PropName,
                    "allprop" => return Self::AllProp,
                    "prop" => {
                        in_prop = true;
                        saw_prop = true;
                    }
                    _ => {}
                },
                XmlTag::Open(name) | XmlTag::Empty(name) => names.push(name.to_string()),
                XmlTag::Close("prop") => in_prop = false,
                XmlTag::Close(_) => {}
            }
        }
        if saw_prop {
            Self::Props(names)
        } else {
            Self::AllProp
        }
    }
}

enum XmlTag<'a> {
    Open(&'a str),
    Empty(&'a str),
    Close(&'a str),
}

/// Iterate element tags of a small XML document, yielding local names only.
fn xml_tags(text: &str) -> impl Iterator<Item = XmlTag<'_>> {
    let mut rest = text;
    std::iter::from_fn(move || {
        loop {
            let start = rest.find('<')?;
            let len = rest[start..].find('>')?;
            let raw = &rest[start + 1..start + len];
            rest = &rest[start + len + 1..];
            if raw.starts_with('?') || raw.starts_with('!') {
                continue;
            }
            let (closing, raw) = match raw.strip_prefix('/') {
                Some(raw) => (true, raw),
                None => (false, raw),
            };
            let empty = raw.ends_with('/');
            let raw = raw.trim_end_matches('/').trim();
            let qualified = raw
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or_default();
            let local = qualified.rsplit(':').next().unwrap_or(qualified);
            if local.is_empty() {
                continue;
            }
            return Some(if closing {
                XmlTag::Close(local)
            } else if empty {
                XmlTag::Empty(local)
            } else {
                XmlTag::Open(local)
            });
        }
    })
}

/// Raw inner text of the first `<name>` element, prefix-insensitive.
fn extract_element<'a>(body: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(body).ok()?;
    let mut search = 0;
    let open = loop {
        let idx = text[search..].find('<')? + search;
        let end = text[idx..].find('>')? + idx;
        let tag = &text[idx + 1..end];
        let local = tag
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or_default();
        let local = local.rsplit(':').next().unwrap_or(local);
        if local == name && !tag.ends_with('/') && !tag.starts_with('/') {
            break end + 1;
        }
        search = end + 1;
    };
    let close_tag = text[open..].find(&format!("{name}>"))? + open;
    let close = text[..close_tag].rfind('<')?;
    Some(&text[open..close])
}

async fn read_xml_body(body: Body) -> Result<Bytes, Response> {
    axum::body::to_bytes(body, MAX_XML_BODY)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE.into_response())
}

async fn propfind(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    // RFC 4918 says a missing Depth means infinity, which we refuse. Treating
    // the omission as 0 is what clients that skip the header actually expect.
    let depth = headers
        .get("depth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0");
    if !matches!(depth, "0" | "1") {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(Body::from(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:error xmlns:D=\"DAV:\"><D:propfind-finite-depth/></D:error>",
            ))
            .expect("valid depth response");
    }
    let request = match read_xml_body(body).await {
        Ok(body) => PropRequest::parse(&body),
        Err(response) => return response,
    };

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
                    resources.push(DavResource::from_path(state, child, path).await);
                }
            }
        }
    } else {
        let path = match resolve_existing(state, user, target, RequiredCap::Read) {
            Ok(path) => path,
            Err(response) => return response,
        };
        let resource = DavResource::from_path(state, target.clone(), path.clone()).await;
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
                            children.push(DavResource::from_path(state, child, path).await);
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
        xml.push_str(&resource.xml(&request));
    }
    xml.push_str("</D:multistatus>");

    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("dav", "1, 2")
        .body(Body::from(xml))
        .expect("valid PROPFIND response")
}

const KNOWN_PROPS: [&str; 8] = [
    "displayname",
    "resourcetype",
    "getcontentlength",
    "getlastmodified",
    "getetag",
    "getcontenttype",
    "supportedlock",
    "lockdiscovery",
];

struct DavResource {
    target: DavTarget,
    display_name: String,
    is_collection: bool,
    size: u64,
    modified: Option<std::time::SystemTime>,
    content_type: String,
    locks: Vec<LockEntry>,
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
            locks: Vec::new(),
        }
    }

    async fn from_path(state: &AppState, target: DavTarget, path: PathBuf) -> Self {
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
        let locks = state.webdav_locks.active_for(&target.lock_key());
        Self {
            target,
            display_name,
            is_collection,
            size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
            modified: metadata.and_then(|m| m.modified().ok()),
            content_type,
            locks,
        }
    }

    fn prop_value(&self, name: &str) -> Option<String> {
        let href = self.target.href(self.is_collection);
        Some(match name {
            "displayname" => xml_escape(&self.display_name),
            "resourcetype" => {
                if self.is_collection {
                    "<D:collection/>".to_string()
                } else {
                    String::new()
                }
            }
            "getcontentlength" => self.size.to_string(),
            "getlastmodified" => self
                .modified
                .map(httpdate::fmt_http_date)
                .unwrap_or_default(),
            "getetag" => {
                let modified_secs = self
                    .modified
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_secs());
                format!("\"{:x}-{:x}\"", self.size, modified_secs)
            }
            "getcontenttype" => xml_escape(&self.content_type),
            "supportedlock" => {
                "<D:lockentry><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockentry>".to_string()
            }
            "lockdiscovery" => self
                .locks
                .iter()
                .map(|lock| lock.xml(&href))
                .collect::<String>(),
            _ => return None,
        })
    }

    fn xml(&self, request: &PropRequest) -> String {
        let mut found = String::new();
        let mut missing = String::new();
        let names: Vec<&str> = match request {
            PropRequest::AllProp | PropRequest::PropName => KNOWN_PROPS.to_vec(),
            PropRequest::Props(names) => names.iter().map(String::as_str).collect(),
        };
        for name in names {
            match (request, self.prop_value(name)) {
                (PropRequest::PropName, Some(_)) => {
                    found.push_str(&format!("<D:{name}/>"));
                }
                (_, Some(value)) if value.is_empty() => {
                    found.push_str(&format!("<D:{name}/>"));
                }
                (_, Some(value)) => {
                    found.push_str(&format!("<D:{name}>{value}</D:{name}>"));
                }
                (_, None) => {
                    let name = xml_escape(name);
                    missing.push_str(&format!("<D:{name}/>"));
                }
            }
        }

        let mut xml = format!(
            "<D:response><D:href>{}</D:href>",
            xml_escape(&self.target.href(self.is_collection))
        );
        if !found.is_empty() {
            xml.push_str(&format!(
                "<D:propstat><D:prop>{found}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>"
            ));
        }
        if !missing.is_empty() {
            xml.push_str(&format!(
                "<D:propstat><D:prop>{missing}</D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>"
            ));
        }
        xml.push_str("</D:response>");
        xml
    }
}

// ---------------------------------------------------------------------------
// GET / HEAD
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Staging: every write lands in a sibling temp entry first
// ---------------------------------------------------------------------------

/// A temporary filesystem entry that cleans itself up unless `keep` is called.
///
/// Dropping the guard removes the entry, or renames it back to its origin for
/// a same-filesystem MOVE that got as far as staging. Cleanup is synchronous
/// because `Drop` cannot await; it only runs on error or client disconnect.
struct Staged {
    path: PathBuf,
    restore_to: Option<PathBuf>,
    keep: bool,
}

impl Staged {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            restore_to: None,
            keep: false,
        }
    }

    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let result = match &self.restore_to {
            Some(origin) => std::fs::rename(&self.path, origin),
            None if self.path.is_dir() => std::fs::remove_dir_all(&self.path),
            None => std::fs::remove_file(&self.path),
        };
        if let Err(error) = result
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), ?error, "failed to clean up WebDAV staging entry");
        }
    }
}

fn staging_path(target: &Path, purpose: &str) -> Option<PathBuf> {
    let parent = target.parent()?;
    let name = target.file_name()?.to_str().unwrap_or("entry");
    Some(parent.join(format!(".webdav-{purpose}-{}-{name}", uuid::Uuid::new_v4())))
}

// ---------------------------------------------------------------------------
// PUT / MKCOL / DELETE
// ---------------------------------------------------------------------------

async fn put(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
    body: Body,
) -> Response {
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
    if !path.parent().is_some_and(Path::is_dir) {
        return StatusCode::CONFLICT.into_response();
    }
    if let Some(lock) = state
        .webdav_locks
        .conflict(&target.lock_key(), &submitted_tokens(headers))
    {
        return locked(&lock);
    }

    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|size| size > state.config.max_upload_file_size)
    {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }

    let Some(temp_path) = staging_path(&path, "upload") else {
        return bad_request("invalid target");
    };
    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(error) => return io_response(error),
    };
    let staged = Staged::new(temp_path.clone());

    let mut body = body.into_data_stream();
    let mut written = 0_u64;
    use futures_lite::StreamExt;
    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        };
        written = written.saturating_add(chunk.len() as u64);
        if written > state.config.max_upload_file_size {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        if let Err(error) = file.write_all(&chunk).await {
            return io_response(error);
        }
    }
    if let Err(error) = file.flush().await {
        return io_response(error);
    }
    drop(file);
    if let Err(error) = tokio::fs::rename(&temp_path, &path).await {
        return io_response(error);
    }
    staged.keep();

    if existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

async fn mkcol(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
) -> Response {
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
    if let Some(lock) = state
        .webdav_locks
        .conflict(&target.lock_key(), &submitted_tokens(headers))
    {
        return locked(&lock);
    }
    match tokio::fs::create_dir(path).await {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(error) => io_response(error),
    }
}

async fn delete(
    state: &AppState,
    user: &AuthUser,
    target: &DavTarget,
    headers: &HeaderMap,
) -> Response {
    if target.root.is_none() || target.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = match resolve_mutating_existing(state, user, target) {
        Ok(path) => path,
        Err(response) => return response,
    };
    let key = target.lock_key();
    if let Some(lock) = state
        .webdav_locks
        .conflict(&key, &submitted_tokens(headers))
    {
        return locked(&lock);
    }
    match remove_entry(&path).await {
        Ok(()) => {
            state.webdav_locks.remove_under(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => io_response(error),
    }
}

// ---------------------------------------------------------------------------
// COPY / MOVE
// ---------------------------------------------------------------------------

/// Parse the Destination header. A destination on another host is refused
/// with 502 as RFC 4918 §9.8.3 suggests; "this host" means either the request's
/// Host header or the configured BASE_URL, so reverse proxies that rewrite Host
/// keep working.
fn destination_target(state: &AppState, headers: &HeaderMap) -> Result<DavTarget, Response> {
    let uri = headers
        .get("destination")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Uri>().ok())
        .ok_or_else(|| bad_request("missing or invalid Destination header"))?;
    if let Some(authority) = uri.authority() {
        let request_host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(host_only);
        let base_host = state
            .config
            .base_url
            .parse::<Uri>()
            .ok()
            .and_then(|base| base.authority().map(|a| host_only(a.as_str())));
        let destination_host = host_only(authority.as_str());
        let local = [request_host, base_host]
            .into_iter()
            .flatten()
            .any(|host| host.eq_ignore_ascii_case(&destination_host));
        if !local {
            return Err((
                StatusCode::BAD_GATEWAY,
                "Destination must be on this server",
            )
                .into_response());
        }
    }
    DavTarget::from_path(uri.path())
}

fn host_only(authority: &str) -> String {
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .map_or(host, |v6| &host[..v6.len() + 1])
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host.to_ascii_lowercase()
}

async fn copy_or_move(
    state: &AppState,
    user: &AuthUser,
    source: &DavTarget,
    parts: &Parts,
    move_resource: bool,
) -> Response {
    let headers = &parts.headers;
    if source.root.is_none() || source.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let destination = match destination_target(state, headers) {
        Ok(target) => target,
        Err(response) => return response,
    };
    if destination.root.is_none() || destination.relative.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }

    let source_path = match if move_resource {
        resolve_mutating_existing(state, user, source)
    } else {
        resolve_existing(state, user, source, RequiredCap::Read)
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

    let tokens = submitted_tokens(headers);
    let source_key = source.lock_key();
    let destination_key = destination.lock_key();
    if move_resource && let Some(lock) = state.webdav_locks.conflict(&source_key, &tokens) {
        return locked(&lock);
    }
    if let Some(lock) = state.webdav_locks.conflict(&destination_key, &tokens) {
        return locked(&lock);
    }

    let existed = destination_path.exists();
    let overwrite = headers
        .get("overwrite")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.eq_ignore_ascii_case("F"));
    if existed && !overwrite {
        return StatusCode::PRECONDITION_FAILED.into_response();
    }

    // Stage next to the destination so an existing destination is only replaced
    // once the full copy (or the cross-device move) has succeeded.
    let Some(staging) = staging_path(&destination_path, "stage") else {
        return bad_request("invalid destination");
    };
    let mut staged = Staged::new(staging.clone());
    let mut remove_source_after = false;
    let stage_result = if move_resource {
        match tokio::fs::rename(&source_path, &staging).await {
            Ok(()) => {
                staged.restore_to = Some(source_path.clone());
                Ok(())
            }
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                remove_source_after = true;
                copy_entry(&source_path, &staging).await
            }
            Err(error) => Err(error),
        }
    } else {
        copy_entry(&source_path, &staging).await
    };
    if let Err(error) = stage_result {
        return io_response(error);
    }

    if existed && let Err(error) = remove_entry(&destination_path).await {
        return io_response(error);
    }
    if let Err(error) = tokio::fs::rename(&staging, &destination_path).await {
        return io_response(error);
    }
    staged.keep();

    if remove_source_after && let Err(error) = remove_entry(&source_path).await {
        // The destination is complete; report the leftover source honestly.
        return io_response(error);
    }
    if move_resource {
        state.webdav_locks.remove_under(&source_key);
    }
    if existed {
        state.webdav_locks.remove_under(&destination_key);
    }

    if existed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::CREATED.into_response()
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

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

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
#[path = "webdav_tests.rs"]
mod integration_tests;

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

    #[test]
    fn propfind_body_selects_properties() {
        assert_eq!(PropRequest::parse(b""), PropRequest::AllProp);
        assert_eq!(
            PropRequest::parse(
                b"<?xml version=\"1.0\"?><D:propfind xmlns:D=\"DAV:\"><D:allprop/></D:propfind>"
            ),
            PropRequest::AllProp
        );
        assert_eq!(
            PropRequest::parse(b"<propfind xmlns=\"DAV:\"><propname/></propfind>"),
            PropRequest::PropName
        );
        assert_eq!(
            PropRequest::parse(
                b"<D:propfind xmlns:D=\"DAV:\" xmlns:Z=\"urn:x\"><D:prop><D:getetag/><Z:creationdate /><D:resourcetype></D:resourcetype></D:prop></D:propfind>"
            ),
            PropRequest::Props(vec![
                "getetag".to_string(),
                "creationdate".to_string(),
                "resourcetype".to_string()
            ])
        );
    }

    #[test]
    fn extracts_lock_owner() {
        let body = b"<?xml version=\"1.0\"?><D:lockinfo xmlns:D=\"DAV:\"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype><D:owner><D:href>mailto:me</D:href></D:owner></D:lockinfo>";
        assert_eq!(
            extract_element(body, "owner"),
            Some("<D:href>mailto:me</D:href>")
        );
        assert_eq!(extract_element(b"<a/>", "owner"), None);
    }

    #[test]
    fn if_header_tokens_are_extracted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "if",
            HeaderValue::from_static(
                "</webdav/docs/a.txt> (<opaquelocktoken:abc> [\"etag\"]) (Not <urn:uuid:def>)",
            ),
        );
        assert_eq!(
            submitted_tokens(&headers),
            vec![
                "opaquelocktoken:abc".to_string(),
                "urn:uuid:def".to_string()
            ]
        );
    }

    #[test]
    fn lock_table_blocks_descendants_and_ancestors() {
        let table = LockTable::default();
        table.insert(LockEntry {
            token: "opaquelocktoken:t".to_string(),
            path: "docs/folder".to_string(),
            owner: None,
            depth_infinity: true,
            timeout: DEFAULT_LOCK_TIMEOUT,
            expires_at: Instant::now() + DEFAULT_LOCK_TIMEOUT,
        });
        assert!(table.conflict("docs/folder", &[]).is_some());
        assert!(table.conflict("docs/folder/file.txt", &[]).is_some());
        assert!(table.conflict("docs", &[]).is_some());
        assert!(table.conflict("docs/folder2", &[]).is_none());
        assert!(table.conflict("docs/other", &[]).is_none());
        assert!(
            table
                .conflict("docs/folder/file.txt", &["opaquelocktoken:t".to_string()])
                .is_none()
        );
        table.remove_under("docs");
        assert!(table.conflict("docs/folder", &[]).is_none());
    }

    #[test]
    fn lock_timeout_is_parsed_and_clamped() {
        let mut headers = HeaderMap::new();
        assert_eq!(lock_timeout(&headers), DEFAULT_LOCK_TIMEOUT);
        headers.insert("timeout", HeaderValue::from_static("Second-120"));
        assert_eq!(lock_timeout(&headers), Duration::from_secs(120));
        headers.insert("timeout", HeaderValue::from_static("Infinite, Second-30"));
        assert_eq!(lock_timeout(&headers), MAX_LOCK_TIMEOUT);
        headers.insert("timeout", HeaderValue::from_static("Second-999999"));
        assert_eq!(lock_timeout(&headers), MAX_LOCK_TIMEOUT);
    }

    #[test]
    fn host_only_strips_port_and_userinfo() {
        assert_eq!(host_only("Example.COM:8443"), "example.com");
        assert_eq!(host_only("user@nas.local"), "nas.local");
        assert_eq!(host_only("[::1]:3000"), "[::1]");
    }
}
