use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nasfiles_core::models::{AuthUser, FolderCaps, Root};
use openidconnect::{OAuth2TokenResponse, RefreshToken};
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::fs::roots::{self, RequiredCap};
use crate::sftp::keys::normalize_russh_public_key;
use crate::state::AppState;
use crate::{auth, config};

const MAX_SFTP_READ_BYTES: usize = 4 * 1024 * 1024;
const MAX_SFTP_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SFTP_READDIR_ENTRIES: usize = 256;
/// Upper bound on concurrently open SFTP handles (files + directories) per
/// session. Each open directory holds a live file descriptor until `close`, and
/// clients are not required to close handles, so without a cap a single session
/// can exhaust file descriptors / memory.
const MAX_SFTP_OPEN_HANDLES: usize = 256;
const MAX_SFTP_CLIENT_PACKET_BYTES: u32 = (MAX_SFTP_WRITE_BYTES as u32) + 1024;

#[derive(Clone)]
pub struct SftpServer {
    state: AppState,
}

pub async fn spawn(state: AppState) -> anyhow::Result<()> {
    let key = load_or_create_host_key(&state.config.sftp_host_key_path)?;
    let preferred = russh::Preferred {
        compression: Cow::Borrowed(&[russh::compression::NONE]),
        ..Default::default()
    };
    let config = russh::server::Config {
        auth_rejection_time: Duration::from_secs(3),
        auth_rejection_time_initial: Some(Duration::from_millis(250)),
        keys: vec![key],
        preferred,
        ..Default::default()
    };

    let bind_addr = state.config.sftp_bind_addr.clone();
    let mut server = SftpServer { state };

    tokio::spawn(async move {
        tracing::info!("SFTP listening on {bind_addr}");
        if let Err(e) = server.run_on_address(Arc::new(config), bind_addr).await {
            tracing::error!("SFTP server stopped: {e}");
        }
    });

    Ok(())
}

impl russh::server::Server for SftpServer {
    type Handler = SshSession;

    fn new_client(&mut self, remote_addr: Option<SocketAddr>) -> Self::Handler {
        SshSession {
            state: self.state.clone(),
            remote_addr,
            clients: Arc::new(Mutex::new(HashMap::new())),
            principal: None,
            fingerprint: None,
        }
    }
}

pub struct SshSession {
    state: AppState,
    remote_addr: Option<SocketAddr>,
    clients: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    principal: Option<Principal>,
    fingerprint: Option<String>,
}

impl SshSession {
    async fn get_channel(&mut self, channel_id: ChannelId) -> Option<Channel<Msg>> {
        let mut clients = self.clients.lock().await;
        clients.remove(&channel_id)
    }

    fn reject() -> Auth {
        Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        }
    }

    /// Pre-authentication guard applied to every auth attempt. Returns
    /// `Some(reject)` when the connection must be denied outright:
    ///
    /// - the client IP is already on the blocklist, or
    /// - the username is `root` — a root login attempt immediately adds the
    ///   source IP to the blocklist.
    ///
    /// Returns `None` to let normal authentication proceed.
    async fn auth_guard(&self, user: &str) -> Option<Auth> {
        let ip = self.remote_addr.map(|addr| addr.ip().to_string());

        if let Some(ip) = ip.as_deref()
            && crate::blocklist::is_blocked(&self.state.pool, ip).await
        {
            tracing::warn!(
                user,
                remote = ?self.remote_addr,
                "SFTP auth rejected: source IP is blocklisted"
            );
            return Some(Self::reject());
        }

        if user == "root" {
            if let Some(ip) = ip.as_deref() {
                crate::blocklist::block(&self.state.pool, ip, "root SFTP login attempt").await;
            }
            audit_event(
                &self.state,
                "auth_root",
                user,
                "blocked",
                None,
                None,
                self.remote_addr,
                false,
                Some("root SFTP login attempt — IP blocklisted"),
            )
            .await;
            return Some(Self::reject());
        }

        None
    }
}

impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        if let Some(rejection) = self.auth_guard(user).await {
            return Ok(rejection);
        }
        Ok(Self::reject())
    }

    async fn auth_password(&mut self, user: &str, _password: &str) -> Result<Auth, Self::Error> {
        if let Some(rejection) = self.auth_guard(user).await {
            return Ok(rejection);
        }
        Ok(Self::reject())
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        if let Some(rejection) = self.auth_guard(user).await {
            return Ok(rejection);
        }
        let normalized = normalize_russh_public_key(public_key)?;
        if public_key_can_authenticate(&self.state, user, &normalized.fingerprint).await? {
            tracing::info!(
                user,
                fingerprint = %normalized.fingerprint,
                remote = ?self.remote_addr,
                "SFTP public key offer accepted"
            );
            Ok(Auth::Accept)
        } else {
            audit_event(
                &self.state,
                "auth_unknown",
                user,
                "auth_offer",
                None,
                None,
                self.remote_addr,
                false,
                Some("public key is not authorized"),
            )
            .await;
            Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        if let Some(rejection) = self.auth_guard(user).await {
            return Ok(rejection);
        }
        let normalized = normalize_russh_public_key(public_key)?;
        let principal = resolve_principal(&self.state, user, &normalized.fingerprint).await?;

        match principal {
            Some(principal) => {
                tracing::info!(
                    user,
                    principal = principal.log_name(),
                    fingerprint = %normalized.fingerprint,
                    remote = ?self.remote_addr,
                    "SFTP public key authentication accepted"
                );
                mark_key_used(&self.state, &principal, &normalized.fingerprint).await;
                self.principal = Some(principal);
                self.fingerprint = Some(normalized.fingerprint);
                Ok(Auth::Accept)
            }
            None => {
                tracing::warn!(
                    user,
                    fingerprint = %normalized.fingerprint,
                    remote = ?self.remote_addr,
                    "SFTP public key authentication failed"
                );
                audit_event(
                    &self.state,
                    "auth_unknown",
                    user,
                    "auth_fail",
                    None,
                    None,
                    self.remote_addr,
                    false,
                    Some("public key is not authorized"),
                )
                .await;
                Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        tracing::info!(
            channel = %channel.id(),
            remote = ?self.remote_addr,
            "SFTP session channel opened"
        );
        let mut clients = self.clients.lock().await;
        clients.insert(channel.id(), channel);
        Ok(true)
    }

    async fn channel_eof(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            channel = %_channel,
            remote = ?self.remote_addr,
            "SFTP channel EOF received"
        );
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            channel = %channel_id,
            subsystem = name,
            remote = ?self.remote_addr,
            "SFTP subsystem requested"
        );
        if name != "sftp" {
            tracing::warn!(
                channel = %channel_id,
                subsystem = name,
                remote = ?self.remote_addr,
                "SFTP subsystem rejected"
            );
            session.channel_failure(channel_id)?;
            return Ok(());
        }

        let (Some(principal), Some(fingerprint)) =
            (self.principal.clone(), self.fingerprint.clone())
        else {
            tracing::warn!(
                channel = %channel_id,
                remote = ?self.remote_addr,
                "SFTP subsystem rejected without authenticated principal"
            );
            session.channel_failure(channel_id)?;
            return Ok(());
        };

        let Some(channel) = self.get_channel(channel_id).await else {
            tracing::warn!(
                channel = %channel_id,
                remote = ?self.remote_addr,
                "SFTP subsystem rejected without channel"
            );
            session.channel_failure(channel_id)?;
            return Ok(());
        };

        session.channel_success(channel_id)?;
        tracing::info!(
            channel = %channel_id,
            principal = principal.log_name(),
            remote = ?self.remote_addr,
            "SFTP subsystem accepted"
        );
        let handler =
            SftpSession::new(self.state.clone(), principal, fingerprint, self.remote_addr);
        russh_sftp::server::run_with_config(
            channel.into_stream(),
            handler,
            russh_sftp::server::Config {
                max_client_packet_len: MAX_SFTP_CLIENT_PACKET_BYTES,
            },
        )
        .await;
        Ok(())
    }
}

#[derive(Clone)]
enum Principal {
    User(AuthUser),
    Temp(TempPrincipal),
}

impl Principal {
    fn log_name(&self) -> &str {
        match self {
            Principal::User(user) => &user.user_id,
            Principal::Temp(temp) => &temp.id,
        }
    }
}

#[derive(Clone)]
struct TempPrincipal {
    id: String,
    root_kind: String,
    root_key: String,
    relative_path: String,
    can_write: bool,
    expires_at: i64,
}

struct SftpSession {
    state: AppState,
    principal: Principal,
    fingerprint: String,
    remote_addr: Option<SocketAddr>,
    version: Option<u32>,
    handles: HashMap<String, HandleEntry>,
    last_oidc_refresh_at: i64,
}

enum HandleEntry {
    Directory {
        pending: Vec<File>,
        reader: Option<tokio::fs::ReadDir>,
    },
    File {
        root_key: String,
        path: PathBuf,
        write: bool,
    },
}

enum ResolvedPath {
    VirtualRoot,
    Real {
        root_key: String,
        relative_path: String,
        path: PathBuf,
        can_write: bool,
        is_root: bool,
    },
}

impl SftpSession {
    fn new(
        state: AppState,
        principal: Principal,
        fingerprint: String,
        remote_addr: Option<SocketAddr>,
    ) -> Self {
        Self {
            state,
            principal,
            fingerprint,
            remote_addr,
            version: None,
            handles: HashMap::new(),
            last_oidc_refresh_at: chrono::Utc::now().timestamp(),
        }
    }

    fn roots(&self) -> Vec<Root> {
        match &self.principal {
            Principal::User(user) => roots::visible_roots(&self.state.config, user),
            Principal::Temp(temp) => vec![Root {
                key: temp.root_key.clone(),
                display_name: "share".to_string(),
                kind: if temp.root_kind == "home" {
                    nasfiles_core::models::RootKind::Home
                } else {
                    nasfiles_core::models::RootKind::Common
                },
                caps: FolderCaps {
                    read: true,
                    write: temp.can_write,
                    share: false,
                },
                group: None,
                usage: None,
            }],
        }
    }

    fn resolve_path(&self, path: &str, cap: RequiredCap) -> Result<ResolvedPath, StatusCode> {
        match &self.principal {
            Principal::User(user) => self.resolve_user_path(user, path, cap),
            Principal::Temp(temp) => self.resolve_temp_path(temp, path, cap),
        }
    }

    fn resolve_create_path(
        &self,
        path: &str,
        cap: RequiredCap,
    ) -> Result<ResolvedPath, StatusCode> {
        match &self.principal {
            Principal::User(user) => self.resolve_user_create_path(user, path, cap),
            Principal::Temp(temp) => self.resolve_temp_create_path(temp, path, cap),
        }
    }

    fn resolve_user_path(
        &self,
        user: &AuthUser,
        path: &str,
        cap: RequiredCap,
    ) -> Result<ResolvedPath, StatusCode> {
        let normalized = normalize_sftp_path(path);
        if normalized == "/" {
            return Ok(ResolvedPath::VirtualRoot);
        }

        let (root_segment, rest) =
            split_first_segment(&normalized).ok_or(StatusCode::NoSuchFile)?;
        let roots = roots::visible_roots(&self.state.config, user);
        let root = roots
            .iter()
            .find(|r| r.display_name == root_segment || r.key == root_segment)
            .ok_or(StatusCode::NoSuchFile)?;

        let root_path =
            roots::resolve_root(&self.state.config, user, &root.key, cap).map_err(|e| match e {
                roots::RootError::Forbidden => StatusCode::PermissionDenied,
                roots::RootError::NotFound => StatusCode::NoSuchFile,
                roots::RootError::Internal => StatusCode::Failure,
            })?;

        let real_path = nasfiles_core::safe_path::resolve(&root_path, &rest)
            .map_err(|_| StatusCode::NoSuchFile)?;

        Ok(ResolvedPath::Real {
            root_key: root.key.clone(),
            is_root: rest.is_empty(),
            relative_path: rest,
            path: real_path,
            can_write: user.can_write(&root.key),
        })
    }

    fn resolve_user_create_path(
        &self,
        user: &AuthUser,
        path: &str,
        cap: RequiredCap,
    ) -> Result<ResolvedPath, StatusCode> {
        let normalized = normalize_sftp_path(path);
        if normalized == "/" {
            return Ok(ResolvedPath::VirtualRoot);
        }

        let (root_segment, rest) =
            split_first_segment(&normalized).ok_or(StatusCode::NoSuchFile)?;
        if rest.is_empty() {
            return self.resolve_user_path(user, path, cap);
        }

        let roots = roots::visible_roots(&self.state.config, user);
        let root = roots
            .iter()
            .find(|r| r.display_name == root_segment || r.key == root_segment)
            .ok_or(StatusCode::NoSuchFile)?;

        let root_path =
            roots::resolve_root(&self.state.config, user, &root.key, cap).map_err(|e| match e {
                roots::RootError::Forbidden => StatusCode::PermissionDenied,
                roots::RootError::NotFound => StatusCode::NoSuchFile,
                roots::RootError::Internal => StatusCode::Failure,
            })?;

        let real_path = nasfiles_core::safe_path::resolve(&root_path, &rest)
            .or_else(|_| nasfiles_core::safe_path::resolve_parent(&root_path, &rest))
            .map_err(|_| StatusCode::NoSuchFile)?;

        Ok(ResolvedPath::Real {
            root_key: root.key.clone(),
            is_root: false,
            relative_path: rest,
            path: real_path,
            can_write: user.can_write(&root.key),
        })
    }

    fn resolve_temp_path(
        &self,
        temp: &TempPrincipal,
        path: &str,
        cap: RequiredCap,
    ) -> Result<ResolvedPath, StatusCode> {
        if chrono::Utc::now().timestamp_millis() >= temp.expires_at {
            return Err(StatusCode::PermissionDenied);
        }
        if matches!(cap, RequiredCap::Write | RequiredCap::Share) && !temp.can_write {
            return Err(StatusCode::PermissionDenied);
        }

        let normalized = normalize_sftp_path(path);
        let rel = if normalized == "/" {
            String::new()
        } else {
            normalized.trim_start_matches('/').to_string()
        };

        let root_path = if temp.root_key == "~" {
            let user = temp_owner_like_user(temp);
            roots::resolve_root(&self.state.config, &user, "~", RequiredCap::Read)
                .map_err(|_| StatusCode::NoSuchFile)?
        } else {
            self.state
                .config
                .common_folders
                .get(&temp.root_key)
                .cloned()
                .ok_or(StatusCode::NoSuchFile)?
        };

        let base_path = nasfiles_core::safe_path::resolve(&root_path, &temp.relative_path)
            .map_err(|_| StatusCode::NoSuchFile)?;
        let real_path = nasfiles_core::safe_path::resolve(&base_path, &rel)
            .map_err(|_| StatusCode::NoSuchFile)?;

        Ok(ResolvedPath::Real {
            root_key: temp.root_key.clone(),
            is_root: rel.is_empty(),
            relative_path: rel,
            path: real_path,
            can_write: temp.can_write,
        })
    }

    fn resolve_temp_create_path(
        &self,
        temp: &TempPrincipal,
        path: &str,
        cap: RequiredCap,
    ) -> Result<ResolvedPath, StatusCode> {
        if chrono::Utc::now().timestamp_millis() >= temp.expires_at {
            return Err(StatusCode::PermissionDenied);
        }
        if matches!(cap, RequiredCap::Write | RequiredCap::Share) && !temp.can_write {
            return Err(StatusCode::PermissionDenied);
        }

        let normalized = normalize_sftp_path(path);
        let rel = if normalized == "/" {
            String::new()
        } else {
            normalized.trim_start_matches('/').to_string()
        };
        if rel.is_empty() {
            return self.resolve_temp_path(temp, path, cap);
        }

        let root_path = if temp.root_key == "~" {
            let user = temp_owner_like_user(temp);
            roots::resolve_root(&self.state.config, &user, "~", RequiredCap::Read)
                .map_err(|_| StatusCode::NoSuchFile)?
        } else {
            self.state
                .config
                .common_folders
                .get(&temp.root_key)
                .cloned()
                .ok_or(StatusCode::NoSuchFile)?
        };

        let base_path = nasfiles_core::safe_path::resolve(&root_path, &temp.relative_path)
            .map_err(|_| StatusCode::NoSuchFile)?;
        let real_path = nasfiles_core::safe_path::resolve(&base_path, &rel)
            .or_else(|_| nasfiles_core::safe_path::resolve_parent(&base_path, &rel))
            .map_err(|_| StatusCode::NoSuchFile)?;

        Ok(ResolvedPath::Real {
            root_key: temp.root_key.clone(),
            is_root: false,
            relative_path: rel,
            path: real_path,
            can_write: temp.can_write,
        })
    }

    fn reject_root_write(resolved: &ResolvedPath) -> Result<(), StatusCode> {
        if matches!(resolved, ResolvedPath::Real { is_root: true, .. }) {
            Err(StatusCode::PermissionDenied)
        } else {
            Ok(())
        }
    }

    async fn ensure_active(&mut self) -> Result<(), StatusCode> {
        match &mut self.principal {
            Principal::User(user) => {
                let active = sqlx::query_scalar::<_, i64>(
                    r#"
                    SELECT COUNT(*)
                    FROM user_public_keys
                    WHERE user_id = $1 AND key_fingerprint = $2 AND revoked_at IS NULL
                    "#,
                )
                .bind(&user.user_id)
                .bind(&self.fingerprint)
                .fetch_one(&self.state.pool)
                .await
                .map_err(|_| StatusCode::ConnectionLost)?;

                if active == 0 {
                    return Err(StatusCode::ConnectionLost);
                }

                let interval = self.state.config.groups_refresh_interval_secs;
                let now_secs = chrono::Utc::now().timestamp();
                if self.state.config.oidc.is_some()
                    && interval > 0
                    && now_secs - self.last_oidc_refresh_at >= interval as i64
                {
                    let row = sqlx::query(
                        "SELECT oidc_access_token, oidc_refresh_token FROM users WHERE id = $1",
                    )
                    .bind(&user.user_id)
                    .fetch_optional(&self.state.pool)
                    .await
                    .map_err(|_| StatusCode::ConnectionLost)?;

                    let Some(row) = row else {
                        return Err(StatusCode::ConnectionLost);
                    };
                    let enc_access: Option<String> = row.get("oidc_access_token");
                    let enc_refresh: Option<String> = row.get("oidc_refresh_token");
                    let refreshed = refresh_sftp_user_from_oidc(
                        &self.state,
                        user.clone(),
                        enc_access,
                        enc_refresh,
                    )
                    .await
                    .map_err(|e| {
                        tracing::warn!(
                            user_id = %user.user_id,
                            "SFTP OIDC refresh failed during active session: {e}"
                        );
                        StatusCode::ConnectionLost
                    })?;

                    if refreshed.effectively_no_access() {
                        return Err(StatusCode::ConnectionLost);
                    }

                    *user = refreshed;
                    self.last_oidc_refresh_at = now_secs;
                }
                Ok(())
            }
            Principal::Temp(temp) => {
                let now = chrono::Utc::now().timestamp_millis();
                let active = sqlx::query_scalar::<_, i64>(
                    r#"
                    SELECT COUNT(*)
                    FROM sftp_temp_user_keys k
                    JOIN sftp_temp_users t ON t.id = k.temp_user_id
                    WHERE t.id = $1
                      AND k.key_fingerprint = $2
                      AND k.revoked_at IS NULL
                      AND t.revoked_at IS NULL
                      AND t.expires_at > $3
                    "#,
                )
                .bind(&temp.id)
                .bind(&self.fingerprint)
                .bind(now)
                .fetch_one(&self.state.pool)
                .await
                .map_err(|_| StatusCode::ConnectionLost)?;

                if active == 0 {
                    return Err(StatusCode::ConnectionLost);
                }
                Ok(())
            }
        }
    }

    fn current_can_read(&self, root_key: &str) -> bool {
        match &self.principal {
            Principal::User(user) => user.can_read(root_key),
            Principal::Temp(temp) => temp.root_key == root_key,
        }
    }

    fn current_can_write(&self, root_key: &str) -> bool {
        match &self.principal {
            Principal::User(user) => user.can_write(root_key),
            Principal::Temp(temp) => temp.root_key == root_key && temp.can_write,
        }
    }

    async fn open_dir_handle(&self, path: &str) -> Result<HandleEntry, StatusCode> {
        match self.resolve_path(path, RequiredCap::Read)? {
            ResolvedPath::VirtualRoot => Ok(HandleEntry::Directory {
                pending: virtual_root_entries(self.roots()),
                reader: None,
            }),
            ResolvedPath::Real { path, .. } => {
                let reader = tokio::fs::read_dir(path).await.map_err(map_io_error)?;
                Ok(HandleEntry::Directory {
                    pending: Vec::new(),
                    reader: Some(reader),
                })
            }
        }
    }

    async fn read_dir_page(entry: &mut HandleEntry) -> Result<Vec<File>, StatusCode> {
        let HandleEntry::Directory { pending, reader } = entry else {
            return Err(StatusCode::Failure);
        };

        if !pending.is_empty() {
            return Ok(take_readdir_page(pending));
        }

        let Some(reader) = reader else {
            return Ok(Vec::new());
        };

        let mut files = Vec::new();
        while files.len() < MAX_SFTP_READDIR_ENTRIES {
            let Some(fs_entry) = reader.next_entry().await.map_err(map_io_error)? else {
                break;
            };
            let metadata = fs_entry.metadata().await.map_err(map_io_error)?;
            files.push(File::new(
                fs_entry.file_name().to_string_lossy().to_string(),
                FileAttributes::from(&metadata),
            ));
        }
        Ok(files)
    }

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        }
    }

    async fn audit(
        &self,
        action: &str,
        resolved: Option<&ResolvedPath>,
        success: bool,
        error: Option<&str>,
    ) {
        let (principal_kind, principal_id) = match &self.principal {
            Principal::User(user) => ("user", user.user_id.as_str()),
            Principal::Temp(temp) => ("temp_user", temp.id.as_str()),
        };
        let (root_key, path) = match resolved {
            Some(ResolvedPath::Real {
                root_key,
                relative_path,
                ..
            }) => (Some(root_key.as_str()), Some(relative_path.as_str())),
            _ => (None, None),
        };
        audit_event(
            &self.state,
            principal_kind,
            principal_id,
            action,
            root_key,
            path,
            self.remote_addr,
            success,
            error,
        )
        .await;
    }

    async fn audit_path_failure(&self, action: &str, path: &str, error: &str) {
        let (principal_kind, principal_id) = match &self.principal {
            Principal::User(user) => ("user", user.user_id.as_str()),
            Principal::Temp(temp) => ("temp_user", temp.id.as_str()),
        };
        audit_event(
            &self.state,
            principal_kind,
            principal_id,
            action,
            None,
            Some(path),
            self.remote_addr,
            false,
            Some(error),
        )
        .await;
    }

    async fn audit_result<T>(
        &self,
        action: &str,
        input_path: &str,
        result: Result<T, StatusCode>,
    ) -> Result<T, StatusCode> {
        match result {
            Ok(value) => Ok(value),
            Err(code) => {
                let msg = code.to_string();
                self.audit_path_failure(action, input_path, &msg).await;
                Err(code)
            }
        }
    }
}

impl russh_sftp::server::Handler for SftpSession {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        version: u32,
        extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        if self.version.is_some() {
            return Err(StatusCode::ConnectionLost);
        }
        tracing::info!(
            version,
            extensions = ?extensions.keys().collect::<Vec<_>>(),
            principal = self.principal.log_name(),
            remote = ?self.remote_addr,
            "SFTP initialized"
        );
        self.version = Some(version);
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        self.ensure_active().await?;
        let normalized = normalize_sftp_path(&path);
        if normalized == "/" {
            tracing::info!(
                id,
                path,
                principal = self.principal.log_name(),
                remote = ?self.remote_addr,
                "SFTP realpath root"
            );
        }
        self.audit_result(
            "realpath",
            &path,
            self.resolve_path(&normalized, RequiredCap::Read),
        )
        .await?;
        Ok(Name {
            id,
            files: vec![File::dummy(normalized)],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.lstat(id, path).await
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.ensure_active().await?;
        match self
            .audit_result("stat", &path, self.resolve_path(&path, RequiredCap::Read))
            .await?
        {
            ResolvedPath::VirtualRoot => Ok(Attrs {
                id,
                attrs: dir_attrs(),
            }),
            ResolvedPath::Real { path, .. } => {
                let metadata = tokio::fs::metadata(&path).await.map_err(map_io_error);
                let metadata = self.audit_result("stat", "", metadata).await?;
                Ok(Attrs {
                    id,
                    attrs: FileAttributes::from(&metadata),
                })
            }
        }
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        self.ensure_active().await?;
        if self.handles.len() >= MAX_SFTP_OPEN_HANDLES {
            self.audit_path_failure("opendir", &path, "too many open handles")
                .await;
            return Err(StatusCode::Failure);
        }
        if normalize_sftp_path(&path) == "/" {
            tracing::info!(
                id,
                principal = self.principal.log_name(),
                remote = ?self.remote_addr,
                "SFTP opendir root"
            );
        }
        let entry = match self.open_dir_handle(&path).await {
            Ok(entry) => entry,
            Err(code) => {
                self.audit_path_failure("opendir", &path, &code.to_string())
                    .await;
                return Err(code);
            }
        };
        let handle = uuid::Uuid::new_v4().to_string();
        self.handles.insert(handle.clone(), entry);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        self.ensure_active().await?;
        let Some(mut entry) = self.handles.remove(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };

        if !matches!(entry, HandleEntry::Directory { .. }) {
            self.handles.insert(handle, entry);
            return Err(StatusCode::Failure);
        }

        let out = Self::read_dir_page(&mut entry).await?;
        tracing::info!(
            id,
            handle,
            count = out.len(),
            principal = self.principal.log_name(),
            remote = ?self.remote_addr,
            "SFTP readdir"
        );
        self.handles.insert(handle, entry);
        if out.is_empty() {
            Err(StatusCode::Eof)
        } else {
            Ok(Name { id, files: out })
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        self.ensure_active().await?;
        if self.handles.len() >= MAX_SFTP_OPEN_HANDLES {
            self.audit_path_failure("open", &filename, "too many open handles")
                .await;
            return Err(StatusCode::Failure);
        }
        let wants_write = pflags.intersects(
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::APPEND,
        );
        let cap = if wants_write {
            RequiredCap::Write
        } else {
            RequiredCap::Read
        };
        let resolved = if pflags.contains(OpenFlags::CREATE) {
            self.audit_result("open", &filename, self.resolve_create_path(&filename, cap))
                .await?
        } else {
            self.audit_result("open", &filename, self.resolve_path(&filename, cap))
                .await?
        };
        let ResolvedPath::Real {
            root_key,
            path,
            can_write,
            ..
        } = &resolved
        else {
            return Err(StatusCode::Failure);
        };
        if wants_write && !can_write {
            self.audit(
                "open",
                Some(&resolved),
                false,
                Some("write permission denied"),
            )
            .await;
            return Err(StatusCode::PermissionDenied);
        }

        let mut options = tokio::fs::OpenOptions::new();
        options.read(pflags.contains(OpenFlags::READ) || !wants_write);
        options.write(wants_write);
        options.create(pflags.contains(OpenFlags::CREATE));
        options.truncate(pflags.contains(OpenFlags::TRUNCATE));
        options.append(pflags.contains(OpenFlags::APPEND));
        if pflags.contains(OpenFlags::EXCLUDE) {
            options.create_new(true);
        }

        let file = options.open(path).await.map_err(map_io_error);
        let file = self.audit_result("open", &filename, file).await?;
        drop(file);

        let handle = uuid::Uuid::new_v4().to_string();
        self.handles.insert(
            handle.clone(),
            HandleEntry::File {
                root_key: root_key.clone(),
                path: path.clone(),
                write: wants_write,
            },
        );
        self.audit("open", Some(&resolved), true, None).await;
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        self.ensure_active().await?;
        let Some(HandleEntry::File { root_key, path, .. }) = self.handles.get(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };
        if !self.current_can_read(root_key) {
            self.audit_path_failure("read", "", "read permission denied")
                .await;
            return Err(StatusCode::PermissionDenied);
        }
        let mut file = tokio::fs::File::open(path).await.map_err(map_io_error)?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(map_io_error)?;
        let read_len = (len as usize).min(MAX_SFTP_READ_BYTES);
        let mut buf = vec![0; read_len];
        let read = file.read(&mut buf).await.map_err(map_io_error)?;
        if read == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(read);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        let Some(HandleEntry::File {
            root_key,
            path,
            write: true,
        }) = self.handles.get(&handle)
        else {
            return Err(StatusCode::PermissionDenied);
        };
        if !self.current_can_write(root_key) {
            self.audit_path_failure("write", "", "write permission denied")
                .await;
            return Err(StatusCode::PermissionDenied);
        }
        if data.len() > MAX_SFTP_WRITE_BYTES {
            self.audit_path_failure("write", "", "write packet too large")
                .await;
            return Err(StatusCode::Failure);
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await
            .map_err(map_io_error)?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(map_io_error)?;
        file.write_all(&data).await.map_err(map_io_error)?;
        Ok(Self::ok(id))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        self.handles.remove(&handle);
        Ok(Self::ok(id))
    }

    async fn extended(
        &mut self,
        id: u32,
        request: String,
        _data: Vec<u8>,
    ) -> Result<russh_sftp::protocol::Packet, Self::Error> {
        tracing::info!(
            id,
            request,
            principal = self.principal.log_name(),
            remote = ?self.remote_addr,
            "SFTP unsupported extension requested"
        );
        Err(StatusCode::OpUnsupported)
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.ensure_active().await?;
        let Some(HandleEntry::File { root_key, path, .. }) = self.handles.get(&handle) else {
            return Err(StatusCode::NoSuchFile);
        };
        if !self.current_can_read(root_key) {
            self.audit_path_failure("fstat", "", "read permission denied")
                .await;
            return Err(StatusCode::PermissionDenied);
        }
        let metadata = tokio::fs::metadata(path).await.map_err(map_io_error)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        let resolved = self
            .audit_result(
                "mkdir",
                &path,
                self.resolve_create_path(&path, RequiredCap::Write),
            )
            .await?;
        if let Err(code) = Self::reject_root_write(&resolved) {
            self.audit(
                "mkdir",
                Some(&resolved),
                false,
                Some("root mutation denied"),
            )
            .await;
            return Err(code);
        }
        let ResolvedPath::Real {
            path, can_write, ..
        } = &resolved
        else {
            return Err(StatusCode::PermissionDenied);
        };
        if !can_write {
            self.audit(
                "mkdir",
                Some(&resolved),
                false,
                Some("write permission denied"),
            )
            .await;
            return Err(StatusCode::PermissionDenied);
        }
        let result = tokio::fs::create_dir(path).await.map_err(map_io_error);
        self.audit_result("mkdir", "", result).await?;
        self.audit("mkdir", Some(&resolved), true, None).await;
        Ok(Self::ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        let resolved = self
            .audit_result(
                "remove",
                &filename,
                self.resolve_path(&filename, RequiredCap::Write),
            )
            .await?;
        if let Err(code) = Self::reject_root_write(&resolved) {
            self.audit(
                "remove",
                Some(&resolved),
                false,
                Some("root mutation denied"),
            )
            .await;
            return Err(code);
        }
        let ResolvedPath::Real {
            path, can_write, ..
        } = &resolved
        else {
            return Err(StatusCode::PermissionDenied);
        };
        if !can_write {
            self.audit(
                "remove",
                Some(&resolved),
                false,
                Some("write permission denied"),
            )
            .await;
            return Err(StatusCode::PermissionDenied);
        }
        let result = tokio::fs::remove_file(path).await.map_err(map_io_error);
        self.audit_result("remove", &filename, result).await?;
        self.audit("remove", Some(&resolved), true, None).await;
        Ok(Self::ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        let resolved = self
            .audit_result("rmdir", &path, self.resolve_path(&path, RequiredCap::Write))
            .await?;
        if let Err(code) = Self::reject_root_write(&resolved) {
            self.audit(
                "rmdir",
                Some(&resolved),
                false,
                Some("root mutation denied"),
            )
            .await;
            return Err(code);
        }
        let ResolvedPath::Real {
            path, can_write, ..
        } = &resolved
        else {
            return Err(StatusCode::PermissionDenied);
        };
        if !can_write {
            self.audit(
                "rmdir",
                Some(&resolved),
                false,
                Some("write permission denied"),
            )
            .await;
            return Err(StatusCode::PermissionDenied);
        }
        let result = tokio::fs::remove_dir(path).await.map_err(map_io_error);
        self.audit_result("rmdir", "", result).await?;
        self.audit("rmdir", Some(&resolved), true, None).await;
        Ok(Self::ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        self.ensure_active().await?;
        let old_resolved = self
            .audit_result(
                "rename",
                &oldpath,
                self.resolve_path(&oldpath, RequiredCap::Write),
            )
            .await?;
        let new_resolved = self
            .audit_result(
                "rename",
                &newpath,
                self.resolve_create_path(&newpath, RequiredCap::Write),
            )
            .await?;
        if let Err(code) = Self::reject_root_write(&old_resolved) {
            self.audit(
                "rename",
                Some(&old_resolved),
                false,
                Some("root mutation denied"),
            )
            .await;
            return Err(code);
        }
        if let Err(code) = Self::reject_root_write(&new_resolved) {
            self.audit(
                "rename",
                Some(&new_resolved),
                false,
                Some("root mutation denied"),
            )
            .await;
            return Err(code);
        }
        let (
            ResolvedPath::Real {
                path: old_real,
                can_write: old_can_write,
                ..
            },
            ResolvedPath::Real {
                path: new_real,
                can_write: new_can_write,
                ..
            },
        ) = (&old_resolved, &new_resolved)
        else {
            return Err(StatusCode::PermissionDenied);
        };
        if !old_can_write || !new_can_write {
            self.audit(
                "rename",
                Some(&old_resolved),
                false,
                Some("write permission denied"),
            )
            .await;
            return Err(StatusCode::PermissionDenied);
        }
        let result = tokio::fs::rename(old_real, new_real)
            .await
            .map_err(map_io_error);
        self.audit_result("rename", &oldpath, result).await?;
        self.audit("rename", Some(&old_resolved), true, None).await;
        Ok(Self::ok(id))
    }
}

async fn resolve_principal(
    state: &AppState,
    username: &str,
    fingerprint: &str,
) -> anyhow::Result<Option<Principal>> {
    if username == "guest" {
        let now = chrono::Utc::now().timestamp_millis();
        let row = sqlx::query(
            r#"
            SELECT t.id, t.root_kind, t.root_key, t.relative_path,
                   CASE WHEN t.can_write THEN 1 ELSE 0 END AS can_write,
                   t.expires_at
            FROM sftp_temp_user_keys k
            JOIN sftp_temp_users t ON t.id = k.temp_user_id
            WHERE k.key_fingerprint = $1
              AND k.revoked_at IS NULL
              AND t.revoked_at IS NULL
              AND t.expires_at > $2
            "#,
        )
        .bind(fingerprint)
        .bind(now)
        .fetch_optional(&state.pool)
        .await?;

        return Ok(row.map(|row| {
            Principal::Temp(TempPrincipal {
                id: row.get("id"),
                root_kind: row.get("root_kind"),
                root_key: row.get("root_key"),
                relative_path: row.get("relative_path"),
                can_write: row.get::<i64, _>("can_write") != 0,
                expires_at: row.get("expires_at"),
            })
        }));
    }

    let row = sqlx::query(
        r#"
        SELECT u.id, u.external_id, u.username, u.display_name, u.picture_url,
               CASE WHEN u.is_admin THEN 1 ELSE 0 END AS is_admin,
               CASE WHEN u.has_home THEN 1 ELSE 0 END AS has_home,
               u.folder_permissions_json,
               u.oidc_access_token,
               u.oidc_refresh_token
        FROM user_public_keys k
        JOIN users u ON u.id = k.user_id
        WHERE k.key_fingerprint = $1
          AND k.revoked_at IS NULL
          AND u.username = $2
        "#,
    )
    .bind(fingerprint)
    .bind(username)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let folder_permissions_json: Option<String> = row.get("folder_permissions_json");
    let folder_permissions = folder_permissions_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();

    let mut user = AuthUser {
        user_id: row.get("id"),
        external_id: row.get("external_id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        picture_url: row.get("picture_url"),
        folder_permissions,
        has_home: row.get::<i64, _>("has_home") != 0,
        is_admin: row.get::<i64, _>("is_admin") != 0,
    };

    if state.config.oidc.is_some() {
        let enc_access: Option<String> = row.get("oidc_access_token");
        let enc_refresh: Option<String> = row.get("oidc_refresh_token");
        user = refresh_sftp_user_from_oidc(state, user, enc_access, enc_refresh).await?;
        if user.effectively_no_access() {
            return Ok(None);
        }
    }

    Ok(Some(Principal::User(user)))
}

async fn public_key_can_authenticate(
    state: &AppState,
    username: &str,
    fingerprint: &str,
) -> anyhow::Result<bool> {
    if username == "guest" {
        let now = chrono::Utc::now().timestamp_millis();
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM sftp_temp_user_keys k
            JOIN sftp_temp_users t ON t.id = k.temp_user_id
            WHERE k.key_fingerprint = $1
              AND k.revoked_at IS NULL
              AND t.revoked_at IS NULL
              AND t.expires_at > $2
            "#,
        )
        .bind(fingerprint)
        .bind(now)
        .fetch_one(&state.pool)
        .await?;
        return Ok(count > 0);
    }

    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM user_public_keys k
        JOIN users u ON u.id = k.user_id
        WHERE k.key_fingerprint = $1
          AND k.revoked_at IS NULL
          AND u.username = $2
        "#,
    )
    .bind(fingerprint)
    .bind(username)
    .fetch_one(&state.pool)
    .await?;

    Ok(count > 0)
}

async fn mark_key_used(state: &AppState, principal: &Principal, fingerprint: &str) {
    let now = chrono::Utc::now().timestamp_millis();
    match principal {
        Principal::User(_) => {
            let _ = sqlx::query(
                "UPDATE user_public_keys SET last_used_at = $1 WHERE key_fingerprint = $2",
            )
            .bind(now)
            .bind(fingerprint)
            .execute(&state.pool)
            .await;
        }
        Principal::Temp(_) => {
            let _ = sqlx::query(
                "UPDATE sftp_temp_user_keys SET last_used_at = $1 WHERE key_fingerprint = $2",
            )
            .bind(now)
            .bind(fingerprint)
            .execute(&state.pool)
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn audit_event(
    state: &AppState,
    principal_kind: &str,
    principal_id: &str,
    action: &str,
    root_key: Option<&str>,
    path: Option<&str>,
    remote_addr: Option<SocketAddr>,
    success: bool,
    error: Option<&str>,
) {
    let ip = remote_addr.map(|a| a.ip().to_string());
    let _ = sqlx::query(
        r#"
        INSERT INTO sftp_access_log
            (id, principal_kind, principal_id, occurred_at, action, root_key,
             path, ip, success, error)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(principal_kind)
    .bind(principal_id)
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(action)
    .bind(root_key)
    .bind(path)
    .bind(ip)
    .bind(success)
    .bind(error)
    .execute(&state.pool)
    .await;
}

async fn refresh_sftp_user_from_oidc(
    state: &AppState,
    mut user: AuthUser,
    enc_access: Option<String>,
    enc_refresh: Option<String>,
) -> anyhow::Result<AuthUser> {
    let oidc_state = auth::oidc::OIDC_CLIENT
        .get()
        .ok_or_else(|| anyhow::anyhow!("OIDC client is not initialized"))?;

    let mut access_token = enc_access
        .as_deref()
        .filter(|t| !t.is_empty())
        .map(|t| auth::oidc::decrypt_token(t, &state.config.session_secret))
        .transpose()
        .map_err(|e| anyhow::anyhow!("failed to decrypt stored OIDC access token: {e:?}"))?
        .ok_or_else(|| anyhow::anyhow!("missing stored OIDC access token"))?;

    let mut refresh_token = enc_refresh
        .as_deref()
        .filter(|t| !t.is_empty())
        .map(|t| auth::oidc::decrypt_token(t, &state.config.session_secret))
        .transpose()
        .map_err(|e| anyhow::anyhow!("failed to decrypt stored OIDC refresh token: {e:?}"))?;

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let mut userinfo =
        fetch_userinfo(oidc_state.userinfo_url.clone(), &http_client, &access_token).await;

    if userinfo.is_none() {
        let Some(rt_str) = refresh_token.clone() else {
            anyhow::bail!("OIDC userinfo failed and no refresh token is stored");
        };
        let token_response = oidc_state
            .client
            .exchange_refresh_token(&RefreshToken::new(rt_str))?
            .request_async(&http_client)
            .await?;

        access_token = token_response.access_token().secret().clone();
        if let Some(new_refresh) = token_response.refresh_token() {
            refresh_token = Some(new_refresh.secret().clone());
        }

        userinfo =
            fetch_userinfo(oidc_state.userinfo_url.clone(), &http_client, &access_token).await;

        let enc_access = auth::oidc::encrypt_token(&access_token, &state.config.session_secret)
            .map_err(|e| anyhow::anyhow!("failed to encrypt OIDC access token: {e:?}"))?;
        let enc_refresh = refresh_token
            .as_deref()
            .map(|t| auth::oidc::encrypt_token(t, &state.config.session_secret))
            .transpose()
            .map_err(|e| anyhow::anyhow!("failed to encrypt OIDC refresh token: {e:?}"))?;
        let _ = sqlx::query(
            "UPDATE users SET oidc_access_token = $1, oidc_refresh_token = COALESCE($2, oidc_refresh_token) WHERE id = $3",
        )
        .bind(enc_access)
        .bind(enc_refresh)
        .bind(&user.user_id)
        .execute(&state.pool)
        .await;
    }

    let userinfo = userinfo.ok_or_else(|| anyhow::anyhow!("OIDC userinfo refresh failed"))?;
    let groups = auth::oidc::extract_claim_array(&userinfo, &state.config.sso_groups_claim)
        .unwrap_or_default();

    user.folder_permissions = config::compute_folder_permissions(&state.config, &groups);
    user.is_admin = config::is_admin(&state.config, &groups);
    user.has_home = if config::personal_folder_allowed(&state.config, &groups) {
        state.config.home_folder_root.is_some()
    } else {
        false
    };

    if user.has_home
        && let Some(ref home_root) = state.config.home_folder_root
    {
        let home_path = home_root.join(user.safe_username());
        if !home_path.exists()
            && let Err(e) = std::fs::create_dir_all(&home_path)
        {
            tracing::warn!(
                "Failed to create home folder for {} during SFTP login: {e}",
                user.username
            );
            user.has_home = false;
        }
    }

    let folder_permissions_json = serde_json::to_string(&user.folder_permissions)?;
    let _ = sqlx::query(
        "UPDATE users SET folder_permissions_json = $1, has_home = $2, is_admin = $3 WHERE id = $4",
    )
    .bind(folder_permissions_json)
    .bind(user.has_home)
    .bind(user.is_admin)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await;

    Ok(user)
}

async fn fetch_userinfo(
    userinfo_url: reqwest::Url,
    http_client: &reqwest::Client,
    access_token: &str,
) -> Option<serde_json::Value> {
    let response = http_client
        .get(userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<serde_json::Value>().await.ok()
}

fn load_or_create_host_key(path: &Path) -> anyhow::Result<russh::keys::PrivateKey> {
    if path.exists() {
        return russh::keys::PrivateKey::read_openssh_file(path)
            .map_err(|e| anyhow::anyhow!("failed to read SFTP host key: {e}"));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let key = russh::keys::PrivateKey::random(
        &mut rand_10::rng(),
        russh::keys::ssh_key::Algorithm::Ed25519,
    )
    .map_err(|e| anyhow::anyhow!("failed to generate SFTP host key: {e}"))?;

    // Write the key to a temporary file and atomically rename it into place.
    // On Unix the temp file is pre-created with owner-only (0600) permissions
    // *before* any key material is written, so the private key is never exposed
    // with the default world-readable mode (the previous code chmod'd only
    // after writing, leaving a readable window on the final path).
    let tmp_path = path.with_extension("tmp");

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .map_err(|e| anyhow::anyhow!("failed to create SFTP host key temp file: {e}"))?;
        drop(file);
    }

    key.write_openssh_file(&tmp_path, russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("failed to write SFTP host key: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Re-assert 0600 in case the writer recreated the file with a wider mode.
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| anyhow::anyhow!("failed to set SFTP host key permissions: {e}"))?;
    }

    std::fs::rename(&tmp_path, path)
        .map_err(|e| anyhow::anyhow!("failed to finalize SFTP host key: {e}"))?;

    Ok(key)
}

fn normalize_sftp_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn split_first_segment(path: &str) -> Option<(String, String)> {
    let stripped = path.trim_start_matches('/');
    if stripped.is_empty() {
        return None;
    }
    let mut parts = stripped.splitn(2, '/');
    let root = parts.next()?.to_string();
    let rest = parts.next().unwrap_or("").to_string();
    Some((root, rest))
}

fn dir_attrs() -> FileAttributes {
    let mut attrs = FileAttributes::default();
    attrs.set_dir(true);
    attrs.permissions = attrs.permissions.map(|mode| mode | 0o755);
    attrs.size = Some(0);
    attrs.uid = Some(0);
    attrs.gid = Some(0);
    attrs.atime = Some(0);
    attrs.mtime = Some(0);
    attrs
}

fn virtual_root_entries(roots: Vec<Root>) -> Vec<File> {
    let mut entries = vec![File::new(".", dir_attrs()), File::new("..", dir_attrs())];
    entries.extend(
        roots
            .into_iter()
            .map(|root| File::new(root.display_name, dir_attrs())),
    );
    entries
}

fn map_io_error(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn take_readdir_page(files: &mut Vec<File>) -> Vec<File> {
    let take = files.len().min(MAX_SFTP_READDIR_ENTRIES);
    files.drain(..take).collect()
}

fn temp_owner_like_user(temp: &TempPrincipal) -> AuthUser {
    let mut folder_permissions = HashMap::new();
    folder_permissions.insert(
        temp.root_key.clone(),
        FolderCaps {
            read: true,
            write: temp.can_write,
            share: false,
        },
    );
    AuthUser {
        user_id: temp.id.clone(),
        external_id: format!("sftp-temp:{}", temp.id),
        username: temp.id.clone(),
        display_name: "SFTP Guest".to_string(),
        picture_url: None,
        folder_permissions,
        has_home: temp.root_key == "~",
        is_admin: false,
    }
}
