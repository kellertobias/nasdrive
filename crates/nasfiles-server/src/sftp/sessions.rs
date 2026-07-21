use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SftpSessionRegistry {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
}

struct SessionEntry {
    principal_kind: String,
    principal_id: String,
    display_name: String,
    remote_ip: Option<String>,
    connected_at: i64,
    bytes_read: Arc<AtomicU64>,
    bytes_written: Arc<AtomicU64>,
}

#[derive(Serialize)]
pub struct ActiveSftpSession {
    pub session_id: String,
    pub principal_kind: String,
    pub principal_id: String,
    pub display_name: String,
    pub remote_ip: Option<String>,
    pub connected_at: i64,
    pub bytes_read: u64,
    pub bytes_written: u64,
}

impl SftpSessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // @tour sftp-server:90 Session bookkeeping
    // `register` inserts an entry under a fresh UUID and hands back two `Arc<AtomicU64>`
    // counters that the session increments on every read and write.
    //
    // The registry lives on `AppState` and is the live view behind the admin "active
    // connections" tab. `list_guests` filters to temporary principals, so ordinary user
    // sessions are tracked but not surfaced there.

    pub async fn register(
        &self,
        session_id: String,
        principal_kind: String,
        principal_id: String,
        display_name: String,
        remote_ip: Option<String>,
        connected_at: i64,
    ) -> (Arc<AtomicU64>, Arc<AtomicU64>) {
        let bytes_read = Arc::new(AtomicU64::new(0));
        let bytes_written = Arc::new(AtomicU64::new(0));
        let entry = SessionEntry {
            principal_kind,
            principal_id,
            display_name,
            remote_ip,
            connected_at,
            bytes_read: bytes_read.clone(),
            bytes_written: bytes_written.clone(),
        };
        self.sessions.lock().await.insert(session_id, entry);
        (bytes_read, bytes_written)
    }

    pub async fn unregister(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }

    pub async fn list_guests(&self) -> Vec<ActiveSftpSession> {
        let guard = self.sessions.lock().await;
        let mut sessions: Vec<_> = guard
            .iter()
            .filter(|(_, e)| e.principal_kind == "temp_user")
            .map(|(id, e)| ActiveSftpSession {
                session_id: id.clone(),
                principal_kind: e.principal_kind.clone(),
                principal_id: e.principal_id.clone(),
                display_name: e.display_name.clone(),
                remote_ip: e.remote_ip.clone(),
                connected_at: e.connected_at,
                bytes_read: e.bytes_read.load(Ordering::Relaxed),
                bytes_written: e.bytes_written.load(Ordering::Relaxed),
            })
            .collect();
        sessions.sort_by_key(|s| s.connected_at);
        sessions
    }
}
