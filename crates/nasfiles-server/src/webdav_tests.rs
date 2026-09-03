//! End-to-end tests for the WebDAV router: real temp roots, an in-memory
//! SQLite pool with a device credential, and requests driven through the
//! full middleware stack with `tower::ServiceExt::oneshot`.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{Request, Response, StatusCode, header},
    middleware,
};
use base64ct::{Base64, Encoding};
use sqlx::any::AnyPoolOptions;
use tempfile::TempDir;
use tower::ServiceExt;
use tower_sessions::{MemoryStore, SessionManagerLayer};

use crate::{config::test_config, state::AppState};

const ACCESS_KEY: &str = "NASDRIVEtestaccesskey";
const SECRET_KEY: &str = "forty-three-character-device-secret-value-1";
const MAX_UPLOAD: u64 = 64;

struct Harness {
    app: Router,
    state: AppState,
    _tmp: TempDir,
    docs: PathBuf,
    archive: PathBuf,
}

async fn harness() -> Harness {
    sqlx::any::install_default_drivers();
    let tmp = tempfile::tempdir().expect("tempdir");
    let docs = tmp.path().join("docs");
    let archive = tmp.path().join("archive");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(&archive).unwrap();

    let mut config = test_config();
    config.dev_mode = false;
    config.no_server_side_execution = true;
    config.disable_passkeys = true;
    config.home_folder_root = None;
    config.max_upload_file_size = MAX_UPLOAD;
    config.base_url = "https://nas.example.test".to_string();
    config.common_folders = HashMap::from([
        ("docs".to_string(), docs.clone()),
        ("archive".to_string(), archive.clone()),
    ]);

    let pool = AnyPoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite pool");
    for statement in [
        "CREATE TABLE users (
            id TEXT PRIMARY KEY,
            external_id TEXT NOT NULL UNIQUE,
            username TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            picture_url TEXT,
            is_admin BOOLEAN NOT NULL DEFAULT FALSE,
            folder_permissions_json TEXT,
            has_home BOOLEAN NOT NULL DEFAULT FALSE,
            created_at BIGINT NOT NULL,
            last_login_at BIGINT NOT NULL
        )",
        "CREATE TABLE user_api_tokens (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            label TEXT NOT NULL,
            access_key TEXT NOT NULL UNIQUE,
            secret_key TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            expires_at BIGINT,
            last_used_at BIGINT,
            revoked_at BIGINT
        )",
        "CREATE TABLE s3_share_credentials (
            id TEXT PRIMARY KEY,
            share_id TEXT NOT NULL,
            access_key TEXT NOT NULL UNIQUE,
            secret_key TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            expires_at BIGINT NOT NULL,
            last_used_at BIGINT
        )",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    let permissions = serde_json::json!({
        "docs": {"read": true, "write": true, "share": false},
        "archive": {"read": true, "write": false, "share": false},
    });
    sqlx::query(
        "INSERT INTO users (id, external_id, username, display_name, is_admin, folder_permissions_json, has_home, created_at, last_login_at) \
         VALUES ('u1', 'ext-u1', 'alice', 'Alice', FALSE, $1, FALSE, 0, 0)",
    )
    .bind(permissions.to_string())
    .execute(&pool)
    .await
    .unwrap();
    let encrypted = crate::crypto::encrypt_secret(&config.session_secret, SECRET_KEY).unwrap();
    sqlx::query(
        "INSERT INTO user_api_tokens (id, user_id, label, access_key, secret_key, created_at) \
         VALUES ('t1', 'u1', 'test', $1, $2, 0)",
    )
    .bind(ACCESS_KEY)
    .bind(encrypted)
    .execute(&pool)
    .await
    .unwrap();

    let state = AppState::new(config, pool).expect("app state");
    let app = Router::new()
        .merge(super::router(state.clone()))
        .with_state(state.clone())
        .layer(SessionManagerLayer::new(MemoryStore::default()))
        .layer(middleware::from_fn(super::discovery_options));
    Harness {
        app,
        state,
        _tmp: tmp,
        docs,
        archive,
    }
}

fn basic_auth() -> String {
    format!(
        "Basic {}",
        Base64::encode_string(format!("{ACCESS_KEY}:{SECRET_KEY}").as_bytes())
    )
}

fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, basic_auth())
        .header(header::HOST, "nas.example.test")
}

impl Harness {
    async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.app.clone().oneshot(request).await.unwrap()
    }

    async fn call(&self, method: &str, path: &str) -> Response<Body> {
        self.send(request(method, path).body(Body::empty()).unwrap())
            .await
    }
}

async fn body_string(response: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header_str<'a>(response: &'a Response<Body>, name: &str) -> &'a str {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn stray_temp_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with(".webdav-"))
        .collect()
}

// --- discovery and authentication ------------------------------------------

#[tokio::test]
async fn options_is_public_and_advertises_class_2() {
    let h = harness().await;
    let response = h
        .send(
            Request::builder()
                .method("OPTIONS")
                .uri("/webdav/")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(header_str(&response, "dav"), "1, 2");
    let allow = header_str(&response, "allow").to_string();
    for method in ["PROPFIND", "LOCK", "UNLOCK", "MOVE"] {
        assert!(allow.contains(method), "Allow lacks {method}: {allow}");
    }
}

#[tokio::test]
async fn missing_or_wrong_credentials_get_a_basic_challenge() {
    let h = harness().await;
    let anonymous = h
        .send(
            Request::builder()
                .method("PROPFIND")
                .uri("/webdav/")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert!(header_str(&anonymous, "www-authenticate").starts_with("Basic realm="));

    let wrong = h
        .send(
            Request::builder()
                .method("PROPFIND")
                .uri("/webdav/")
                .header(
                    header::AUTHORIZATION,
                    format!(
                        "Basic {}",
                        Base64::encode_string(format!("{ACCESS_KEY}:nope").as_bytes())
                    ),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
}

// --- PROPFIND ---------------------------------------------------------------

#[tokio::test]
async fn propfind_root_lists_visible_roots_and_defaults_depth_to_zero() {
    let h = harness().await;
    let depth1 = h
        .send(
            request("PROPFIND", "/webdav/")
                .header("depth", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(depth1.status(), StatusCode::MULTI_STATUS);
    let xml = body_string(depth1).await;
    assert!(xml.contains("<D:href>/webdav/</D:href>"));
    assert!(xml.contains("<D:href>/webdav/docs/</D:href>"));
    assert!(xml.contains("<D:href>/webdav/archive/</D:href>"));
    assert!(xml.contains("<D:supportedlock>"));

    let no_depth = h.call("PROPFIND", "/webdav/").await;
    assert_eq!(no_depth.status(), StatusCode::MULTI_STATUS);
    let xml = body_string(no_depth).await;
    assert_eq!(xml.matches("<D:response>").count(), 1);

    let infinity = h
        .send(
            request("PROPFIND", "/webdav/")
                .header("depth", "infinity")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(infinity.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn propfind_reports_files_and_honours_prop_selection() {
    let h = harness().await;
    std::fs::write(h.docs.join("Q&A.txt"), b"hello").unwrap();
    std::fs::create_dir(h.docs.join("sub")).unwrap();

    let listing = h
        .send(
            request("PROPFIND", "/webdav/docs/")
                .header("depth", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let xml = body_string(listing).await;
    assert!(xml.contains("<D:href>/webdav/docs/Q%26A.txt</D:href>"));
    assert!(xml.contains("<D:displayname>Q&amp;A.txt</D:displayname>"));
    assert!(xml.contains("<D:getcontentlength>5</D:getcontentlength>"));
    assert!(xml.contains("<D:href>/webdav/docs/sub/</D:href>"));

    let selected = h
        .send(
            request("PROPFIND", "/webdav/docs/Q%26A.txt")
                .header("depth", "0")
                .body(Body::from(
                    "<D:propfind xmlns:D=\"DAV:\"><D:prop><D:getetag/><D:creationdate/></D:prop></D:propfind>",
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(selected.status(), StatusCode::MULTI_STATUS);
    let xml = body_string(selected).await;
    assert!(xml.contains("<D:getetag>"));
    assert!(!xml.contains("<D:getcontentlength>"));
    assert!(
        xml.contains(
            "<D:prop><D:creationdate/></D:prop><D:status>HTTP/1.1 404 Not Found</D:status>"
        )
    );

    let names = h
        .send(
            request("PROPFIND", "/webdav/docs/Q%26A.txt")
                .body(Body::from(
                    "<propfind xmlns=\"DAV:\"><propname/></propfind>",
                ))
                .unwrap(),
        )
        .await;
    let xml = body_string(names).await;
    assert!(xml.contains("<D:getcontentlength/>"));
    assert!(!xml.contains("<D:getcontentlength>5"));

    let missing = h.call("PROPFIND", "/webdav/docs/nope.txt").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn traversal_and_unknown_roots_are_rejected() {
    let h = harness().await;
    assert_eq!(
        h.call("PROPFIND", "/webdav/docs/%2e%2e/secret")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.call("PROPFIND", "/webdav/private/").await.status(),
        StatusCode::FORBIDDEN
    );
}

// --- PUT / GET / HEAD -------------------------------------------------------

#[tokio::test]
async fn put_creates_then_overwrites_and_get_reads_back() {
    let h = harness().await;
    let created = h
        .send(
            request("PUT", "/webdav/docs/note.txt")
                .body(Body::from("first"))
                .unwrap(),
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(std::fs::read(h.docs.join("note.txt")).unwrap(), b"first");

    let replaced = h
        .send(
            request("PUT", "/webdav/docs/note.txt")
                .body(Body::from("second"))
                .unwrap(),
        )
        .await;
    assert_eq!(replaced.status(), StatusCode::NO_CONTENT);

    let get = h.call("GET", "/webdav/docs/note.txt").await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(body_string(get).await, "second");

    let head = h.call("HEAD", "/webdav/docs/note.txt").await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(header_str(&head, "content-length"), "6");

    assert_eq!(
        h.call("GET", "/webdav/docs/").await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert!(stray_temp_files(&h.docs).is_empty());
}

#[tokio::test]
async fn put_is_refused_on_read_only_roots_and_missing_parents() {
    let h = harness().await;
    let readonly = h
        .send(
            request("PUT", "/webdav/archive/x.txt")
                .body(Body::from("x"))
                .unwrap(),
        )
        .await;
    assert_eq!(readonly.status(), StatusCode::FORBIDDEN);
    assert!(!h.archive.join("x.txt").exists());

    let orphan = h
        .send(
            request("PUT", "/webdav/docs/missing/x.txt")
                .body(Body::from("x"))
                .unwrap(),
        )
        .await;
    assert_eq!(orphan.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn oversized_put_leaves_no_temp_file() {
    let h = harness().await;
    let declared = h
        .send(
            request("PUT", "/webdav/docs/big.bin")
                .header(header::CONTENT_LENGTH, (MAX_UPLOAD + 1).to_string())
                .body(Body::from(vec![0_u8; MAX_UPLOAD as usize + 1]))
                .unwrap(),
        )
        .await;
    assert_eq!(declared.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let stream = futures_lite::stream::iter(vec![
        Ok::<_, std::io::Error>(Bytes::from(vec![1_u8; MAX_UPLOAD as usize])),
        Ok(Bytes::from_static(b"overflow")),
    ]);
    let streamed = h
        .send(
            request("PUT", "/webdav/docs/big.bin")
                .body(Body::from_stream(stream))
                .unwrap(),
        )
        .await;
    assert_eq!(streamed.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!h.docs.join("big.bin").exists());
    assert!(stray_temp_files(&h.docs).is_empty());
}

#[tokio::test]
async fn aborted_put_cleans_up_its_temp_file() {
    let h = harness().await;
    use futures_lite::StreamExt;
    let stream = futures_lite::stream::once(Ok::<_, std::io::Error>(Bytes::from_static(b"part")))
        .chain(futures_lite::stream::pending());
    let app = h.app.clone();
    let task = tokio::spawn(async move {
        app.oneshot(
            request("PUT", "/webdav/docs/hanging.txt")
                .body(Body::from_stream(stream))
                .unwrap(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        stray_temp_files(&h.docs).len(),
        1,
        "upload should be staged while the body is pending"
    );
    task.abort();
    let _ = task.await;
    assert!(stray_temp_files(&h.docs).is_empty());
    assert!(!h.docs.join("hanging.txt").exists());
}

// --- MKCOL / DELETE ---------------------------------------------------------

#[tokio::test]
async fn mkcol_and_delete_round_trip() {
    let h = harness().await;
    assert_eq!(
        h.call("MKCOL", "/webdav/docs/new").await.status(),
        StatusCode::CREATED
    );
    assert!(h.docs.join("new").is_dir());
    assert_eq!(
        h.call("MKCOL", "/webdav/docs/new").await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        h.call("MKCOL", "/webdav/docs/a/b").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.call("MKCOL", "/webdav/archive/new").await.status(),
        StatusCode::FORBIDDEN
    );

    std::fs::write(h.docs.join("new/inner.txt"), b"x").unwrap();
    assert_eq!(
        h.call("DELETE", "/webdav/docs/new").await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(!h.docs.join("new").exists());
    assert_eq!(
        h.call("DELETE", "/webdav/docs/new").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.call("DELETE", "/webdav/docs/").await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.call("DELETE", "/webdav/").await.status(),
        StatusCode::FORBIDDEN
    );
}

// --- COPY / MOVE ------------------------------------------------------------

#[tokio::test]
async fn copy_and_move_respect_overwrite_and_containment() {
    let h = harness().await;
    std::fs::create_dir(h.docs.join("src")).unwrap();
    std::fs::write(h.docs.join("src/a.txt"), b"a").unwrap();
    std::fs::write(h.docs.join("existing.txt"), b"old").unwrap();

    let copy = h
        .send(
            request("COPY", "/webdav/docs/src")
                .header("destination", "/webdav/docs/copy")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(copy.status(), StatusCode::CREATED);
    assert_eq!(std::fs::read(h.docs.join("copy/a.txt")).unwrap(), b"a");
    assert!(h.docs.join("src/a.txt").exists());

    let no_overwrite = h
        .send(
            request("COPY", "/webdav/docs/src/a.txt")
                .header(
                    "destination",
                    "https://nas.example.test/webdav/docs/existing.txt",
                )
                .header("overwrite", "F")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(no_overwrite.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(std::fs::read(h.docs.join("existing.txt")).unwrap(), b"old");

    let overwrite = h
        .send(
            request("COPY", "/webdav/docs/src/a.txt")
                .header("destination", "/webdav/docs/existing.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(overwrite.status(), StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read(h.docs.join("existing.txt")).unwrap(), b"a");

    let into_self = h
        .send(
            request("MOVE", "/webdav/docs/src")
                .header("destination", "/webdav/docs/src/inner")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(into_self.status(), StatusCode::FORBIDDEN);

    let moved = h
        .send(
            request("MOVE", "/webdav/docs/src")
                .header("destination", "/webdav/docs/moved")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(moved.status(), StatusCode::CREATED);
    assert!(!h.docs.join("src").exists());
    assert_eq!(std::fs::read(h.docs.join("moved/a.txt")).unwrap(), b"a");

    let to_readonly = h
        .send(
            request("MOVE", "/webdav/docs/moved")
                .header("destination", "/webdav/archive/moved")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(to_readonly.status(), StatusCode::FORBIDDEN);
    assert!(h.docs.join("moved").exists());

    let foreign = h
        .send(
            request("MOVE", "/webdav/docs/moved")
                .header("destination", "https://elsewhere.example/webdav/docs/x")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(foreign.status(), StatusCode::BAD_GATEWAY);

    let missing_header = h.call("MOVE", "/webdav/docs/moved").await;
    assert_eq!(missing_header.status(), StatusCode::BAD_REQUEST);
    assert!(stray_temp_files(&h.docs).is_empty());
}

#[tokio::test]
async fn failed_copy_keeps_the_existing_destination() {
    let h = harness().await;
    std::fs::create_dir(h.docs.join("tree")).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", h.docs.join("tree/link")).unwrap();
    std::fs::write(h.docs.join("target"), b"keep me").unwrap();

    let copy = h
        .send(
            request("COPY", "/webdav/docs/tree")
                .header("destination", "/webdav/docs/target")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(copy.status(), StatusCode::FORBIDDEN);
    assert_eq!(std::fs::read(h.docs.join("target")).unwrap(), b"keep me");
    assert!(stray_temp_files(&h.docs).is_empty());
}

// --- LOCK / UNLOCK ----------------------------------------------------------

const LOCK_BODY: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><D:lockinfo xmlns:D=\"DAV:\"><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype><D:owner><D:href>finder</D:href></D:owner></D:lockinfo>";

#[tokio::test]
async fn lock_creates_missing_files_and_guards_writes() {
    let h = harness().await;
    let lock = h
        .send(
            request("LOCK", "/webdav/docs/draft.txt")
                .header("timeout", "Second-120")
                .body(Body::from(LOCK_BODY))
                .unwrap(),
        )
        .await;
    assert_eq!(lock.status(), StatusCode::CREATED);
    assert!(h.docs.join("draft.txt").exists());
    let token = header_str(&lock, "lock-token")
        .trim_matches(|c| c == '<' || c == '>')
        .to_string();
    assert!(token.starts_with("opaquelocktoken:"));
    let xml = body_string(lock).await;
    assert!(xml.contains("<D:timeout>Second-120</D:timeout>"));
    assert!(xml.contains("<D:owner>&lt;D:href&gt;finder&lt;/D:href&gt;</D:owner>"));

    // Another client without the token is blocked.
    let blocked = h
        .send(
            request("PUT", "/webdav/docs/draft.txt")
                .body(Body::from("intruder"))
                .unwrap(),
        )
        .await;
    assert_eq!(blocked.status(), StatusCode::LOCKED);
    assert_eq!(
        h.call("DELETE", "/webdav/docs/draft.txt").await.status(),
        StatusCode::LOCKED
    );

    // The holder writes with an If header.
    let allowed = h
        .send(
            request("PUT", "/webdav/docs/draft.txt")
                .header("if", format!("(<{token}>)"))
                .body(Body::from("owner"))
                .unwrap(),
        )
        .await;
    assert_eq!(allowed.status(), StatusCode::NO_CONTENT);

    // PROPFIND shows the active lock.
    let discovery = body_string(h.call("PROPFIND", "/webdav/docs/draft.txt").await).await;
    assert!(discovery.contains(&format!("<D:href>{token}</D:href>")));

    // Refresh with an empty body and the token.
    let refreshed = h
        .send(
            request("LOCK", "/webdav/docs/draft.txt")
                .header("if", format!("(<{token}>)"))
                .header("timeout", "Second-30")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(refreshed.status(), StatusCode::OK);
    assert!(
        body_string(refreshed)
            .await
            .contains("<D:timeout>Second-30</D:timeout>")
    );

    // A second exclusive lock is refused.
    let second = h
        .send(
            request("LOCK", "/webdav/docs/draft.txt")
                .body(Body::from(LOCK_BODY))
                .unwrap(),
        )
        .await;
    assert_eq!(second.status(), StatusCode::LOCKED);

    // UNLOCK releases it.
    assert_eq!(
        h.call("UNLOCK", "/webdav/docs/draft.txt").await.status(),
        StatusCode::BAD_REQUEST
    );
    let unlocked = h
        .send(
            request("UNLOCK", "/webdav/docs/draft.txt")
                .header("lock-token", format!("<{token}>"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(unlocked.status(), StatusCode::NO_CONTENT);
    let again = h
        .send(
            request("UNLOCK", "/webdav/docs/draft.txt")
                .header("lock-token", format!("<{token}>"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(again.status(), StatusCode::CONFLICT);
    assert_eq!(
        h.call("DELETE", "/webdav/docs/draft.txt").await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn locks_follow_deletes_and_moves_and_respect_read_only_roots() {
    let h = harness().await;
    std::fs::write(h.archive.join("ro.txt"), b"x").unwrap();
    let readonly = h
        .send(
            request("LOCK", "/webdav/archive/ro.txt")
                .body(Body::from(LOCK_BODY))
                .unwrap(),
        )
        .await;
    assert_eq!(readonly.status(), StatusCode::FORBIDDEN);

    std::fs::create_dir(h.docs.join("dir")).unwrap();
    std::fs::write(h.docs.join("dir/f.txt"), b"x").unwrap();
    let lock = h
        .send(
            request("LOCK", "/webdav/docs/dir")
                .body(Body::from(LOCK_BODY))
                .unwrap(),
        )
        .await;
    assert_eq!(lock.status(), StatusCode::OK);
    let token = header_str(&lock, "lock-token").to_string();

    // Depth-infinity lock on the folder covers its children.
    assert_eq!(
        h.call("DELETE", "/webdav/docs/dir/f.txt").await.status(),
        StatusCode::LOCKED
    );
    assert!(
        h.state
            .webdav_locks
            .conflict("docs/dir/f.txt", &[])
            .is_some()
    );

    // Moving the locked folder with the token drops its lock.
    let moved = h
        .send(
            request("MOVE", "/webdav/docs/dir")
                .header("destination", "/webdav/docs/dir2")
                .header("if", format!("({token})"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(moved.status(), StatusCode::CREATED);
    assert!(h.state.webdav_locks.conflict("docs/dir", &[]).is_none());
    assert_eq!(
        h.call("DELETE", "/webdav/docs/dir2/f.txt").await.status(),
        StatusCode::NO_CONTENT
    );
}
