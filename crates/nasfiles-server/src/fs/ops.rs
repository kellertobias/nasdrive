use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, sleep};

use crate::fs::roots;
use crate::state::AppState;

/// Validate a filename — reject path separators, NUL bytes, control characters,
/// and special names that could cause problems.
fn validate_filename(name: &str) -> Result<(), FileOpError> {
    if name.is_empty() {
        return Err(FileOpError::InvalidName("name cannot be empty".into()));
    }
    if name.len() > 255 {
        return Err(FileOpError::InvalidName("name too long (max 255)".into()));
    }
    if name == "." || name == ".." {
        return Err(FileOpError::InvalidName("invalid name".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(FileOpError::InvalidName(
            "name contains invalid characters".into(),
        ));
    }
    // Reject control characters (0x00-0x1F, 0x7F)
    if name.chars().any(|c| c.is_control()) {
        return Err(FileOpError::InvalidName(
            "name contains control characters".into(),
        ));
    }
    Ok(())
}

/// Resolve a user's root + relative path, returning the canonical filesystem path.
/// This is the shared helper that all write ops use.
fn resolve_path(
    config: &crate::config::AppConfig,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    relative_path: &str,
) -> Result<std::path::PathBuf, FileOpError> {
    resolve_path_with_cap(
        config,
        user,
        root_key,
        relative_path,
        roots::RequiredCap::Write,
    )
}

fn resolve_path_with_cap(
    config: &crate::config::AppConfig,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    relative_path: &str,
    cap: roots::RequiredCap,
) -> Result<std::path::PathBuf, FileOpError> {
    let root_path = roots::resolve_root(config, user, root_key, cap)
        .map_err(|e| FileOpError::Root(e.to_string()))?;

    nasfiles_core::safe_path::resolve(&root_path, relative_path)
        .map_err(|e| FileOpError::Path(e.to_string()))
}

#[allow(dead_code)]
async fn entry_size(source: &Path) -> Result<(u64, u64), FileOpError> {
    let metadata = fs::metadata(source).await.map_err(|e| {
        tracing::error!("size metadata failed: {e}");
        FileOpError::Io(e.to_string())
    })?;

    if metadata.is_dir() {
        let mut bytes = 0;
        let mut entries_count = 1;
        let mut pending = vec![source.to_path_buf()];

        while let Some(dir) = pending.pop() {
            let mut entries = fs::read_dir(&dir).await.map_err(|e| {
                tracing::error!("size read_dir failed: {e}");
                FileOpError::Io(e.to_string())
            })?;

            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                tracing::error!("size next_entry failed: {e}");
                FileOpError::Io(e.to_string())
            })? {
                let child_meta = entry.metadata().await.map_err(|e| {
                    tracing::error!("size child metadata failed: {e}");
                    FileOpError::Io(e.to_string())
                })?;
                entries_count += 1;

                if child_meta.is_dir() {
                    pending.push(entry.path());
                } else if child_meta.is_file() {
                    bytes += child_meta.len();
                } else {
                    return Err(FileOpError::InvalidPath);
                }
            }
        }

        Ok((bytes, entries_count))
    } else if metadata.is_file() {
        Ok((metadata.len(), 1))
    } else {
        Err(FileOpError::InvalidPath)
    }
}

#[allow(dead_code)]
async fn copy_file_with_progress(
    source: &Path,
    target: &Path,
    on_progress: &(dyn Fn(u64, u64) + Sync),
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), FileOpError> {
    if target.exists() {
        return Err(FileOpError::AlreadyExists);
    }

    let mut src = fs::File::open(source).await.map_err(|e| {
        tracing::error!("copy open source failed: {e}");
        FileOpError::Io(e.to_string())
    })?;
    let mut dst = fs::File::create(target).await.map_err(|e| {
        tracing::error!("copy create target failed: {e}");
        FileOpError::Io(e.to_string())
    })?;
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        if is_cancelled() {
            return Err(FileOpError::Cancelled);
        }
        let read = src.read(&mut buffer).await.map_err(|e| {
            tracing::error!("copy read failed: {e}");
            FileOpError::Io(e.to_string())
        })?;
        if read == 0 {
            break;
        }
        dst.write_all(&buffer[..read]).await.map_err(|e| {
            tracing::error!("copy write failed: {e}");
            FileOpError::Io(e.to_string())
        })?;
        on_progress(read as u64, 0);
        demo_transfer_delay().await;
    }

    dst.flush().await.map_err(|e| {
        tracing::error!("copy flush failed: {e}");
        FileOpError::Io(e.to_string())
    })?;
    on_progress(0, 1);

    Ok(())
}

#[allow(dead_code)]
async fn demo_transfer_delay() {
    static DELAY: OnceLock<Option<Duration>> = OnceLock::new();
    let delay = DELAY.get_or_init(|| {
        let millis = std::env::var("NASFILES_DEMO_TRANSFER_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        (millis > 0).then(|| Duration::from_millis(millis))
    });

    if let Some(delay) = delay {
        sleep(*delay).await;
    }
}

#[allow(dead_code)]
async fn copy_entry(
    source: &Path,
    target: &Path,
    on_progress: &(dyn Fn(u64, u64) + Sync),
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), FileOpError> {
    if is_cancelled() {
        return Err(FileOpError::Cancelled);
    }
    if target.exists() {
        return Err(FileOpError::AlreadyExists);
    }

    let metadata = fs::metadata(source).await.map_err(|e| {
        tracing::error!("copy metadata failed: {e}");
        FileOpError::Io(e.to_string())
    })?;

    if metadata.is_dir() {
        fs::create_dir(target).await.map_err(|e| {
            tracing::error!("copy mkdir failed: {e}");
            FileOpError::Io(e.to_string())
        })?;
        on_progress(0, 1);

        let mut pending = vec![(source.to_path_buf(), target.to_path_buf())];
        while let Some((src_dir, dst_dir)) = pending.pop() {
            if is_cancelled() {
                return Err(FileOpError::Cancelled);
            }
            let mut entries = fs::read_dir(&src_dir).await.map_err(|e| {
                tracing::error!("copy read_dir failed: {e}");
                FileOpError::Io(e.to_string())
            })?;

            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                tracing::error!("copy next_entry failed: {e}");
                FileOpError::Io(e.to_string())
            })? {
                if is_cancelled() {
                    return Err(FileOpError::Cancelled);
                }
                let src_child = entry.path();
                let dst_child = dst_dir.join(entry.file_name());
                let child_meta = entry.metadata().await.map_err(|e| {
                    tracing::error!("copy child metadata failed: {e}");
                    FileOpError::Io(e.to_string())
                })?;

                if child_meta.is_dir() {
                    if dst_child.exists() {
                        return Err(FileOpError::AlreadyExists);
                    }
                    fs::create_dir(&dst_child).await.map_err(|e| {
                        tracing::error!("copy child mkdir failed: {e}");
                        FileOpError::Io(e.to_string())
                    })?;
                    on_progress(0, 1);
                    pending.push((src_child, dst_child));
                } else if child_meta.is_file() {
                    copy_file_with_progress(&src_child, &dst_child, on_progress, is_cancelled)
                        .await?;
                } else {
                    return Err(FileOpError::InvalidPath);
                }
            }
        }
    } else if metadata.is_file() {
        copy_file_with_progress(source, target, on_progress, is_cancelled).await?;
    } else {
        return Err(FileOpError::InvalidPath);
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Operations
// -----------------------------------------------------------------------

/// Create a new directory.
pub async fn create_directory(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    parent_path: &str,
    name: &str,
) -> Result<(), FileOpError> {
    validate_filename(name)?;

    let parent = resolve_path(&state.config, user, root_key, parent_path)?;

    if !parent.is_dir() {
        return Err(FileOpError::NotDirectory);
    }

    let target = parent.join(name);
    if target.exists() {
        return Err(FileOpError::AlreadyExists);
    }

    fs::create_dir(&target).await.map_err(|e| {
        tracing::error!("mkdir failed: {e}");
        FileOpError::Io(e.to_string())
    })?;

    Ok(())
}

/// Rename a file or directory (same parent).
pub async fn rename_entry(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    path: &str,
    new_name: &str,
) -> Result<(), FileOpError> {
    validate_filename(new_name)?;

    let source = resolve_path(&state.config, user, root_key, path)?;

    let parent = source.parent().ok_or(FileOpError::InvalidPath)?;
    let target = parent.join(new_name);

    if target.exists() {
        return Err(FileOpError::AlreadyExists);
    }

    fs::rename(&source, &target).await.map_err(|e| {
        tracing::error!("rename failed: {e}");
        FileOpError::Io(e.to_string())
    })?;

    Ok(())
}

/// Move one or more entries to a new parent directory.
pub async fn move_entries(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    source_paths: &[String],
    dest_path: &str,
) -> Result<(), FileOpError> {
    let dest = resolve_path(&state.config, user, root_key, dest_path)?;

    if !dest.is_dir() {
        return Err(FileOpError::NotDirectory);
    }

    for src_path in source_paths {
        let source = resolve_path(&state.config, user, root_key, src_path)?;

        let name = source
            .file_name()
            .ok_or(FileOpError::InvalidPath)?
            .to_os_string();

        let target = dest.join(&name);
        if target.exists() {
            return Err(FileOpError::AlreadyExists);
        }
        if source.is_dir() && target.starts_with(&source) {
            return Err(FileOpError::InvalidPath);
        }

        fs::rename(&source, &target).await.map_err(|e| {
            tracing::error!("move failed for {:?}: {e}", name);
            FileOpError::Io(e.to_string())
        })?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct TransferProgress {
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub total_entries: u64,
    pub completed_entries: u64,
}

#[allow(dead_code)]
pub struct TransferSpec<'a> {
    pub source_root: &'a str,
    pub source_paths: &'a [String],
    pub dest_root: &'a str,
    pub dest_path: &'a str,
    pub operation: TransferOperation,
}

// @tour comment Dead code that looks authoritative
// This function and its helpers are all `#[allow(dead_code)]` — nothing calls them. The
// live transfer implementation is entirely in `file_jobs.rs`, which uses a durable item
// table instead of this in-memory `AtomicU64` progress model.
//
// It is worth knowing about precisely because it reads like the real thing. By contrast
// `ops::move_entries` above **is** live: it backs `POST /api/files/{root}/move` and does a
// synchronous, non-resumable rename loop that aborts on the first collision, leaving
// earlier entries already moved.

/// Copy or move entries between two roots and report server-side progress.
#[allow(dead_code)]
pub async fn transfer_entries_with_progress(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    spec: TransferSpec<'_>,
    progress: impl Fn(TransferProgress) + Sync,
    is_cancelled: impl Fn() -> bool + Sync,
) -> Result<(), FileOpError> {
    let source_cap = match spec.operation {
        TransferOperation::Copy => roots::RequiredCap::Read,
        TransferOperation::Move => roots::RequiredCap::Write,
    };
    let dest = resolve_path_with_cap(
        &state.config,
        user,
        spec.dest_root,
        spec.dest_path,
        roots::RequiredCap::Write,
    )?;

    if !dest.is_dir() {
        return Err(FileOpError::NotDirectory);
    }

    let mut planned = Vec::with_capacity(spec.source_paths.len());
    let mut total_bytes = 0;
    let mut total_entries = 0;

    for src_path in spec.source_paths {
        if is_cancelled() {
            return Err(FileOpError::Cancelled);
        }
        let source =
            resolve_path_with_cap(&state.config, user, spec.source_root, src_path, source_cap)?;

        let name = source
            .file_name()
            .ok_or(FileOpError::InvalidPath)?
            .to_os_string();

        let target = dest.join(&name);
        if target.exists() {
            return Err(FileOpError::AlreadyExists);
        }
        if source.is_dir() && target.starts_with(&source) {
            return Err(FileOpError::InvalidPath);
        }

        let (bytes, entries) = entry_size(&source).await?;
        total_bytes += bytes;
        total_entries += entries;
        planned.push((source, target, name));
    }

    let transferred_bytes = AtomicU64::new(0);
    let completed_entries = AtomicU64::new(0);
    progress(TransferProgress {
        total_bytes,
        transferred_bytes: 0,
        total_entries,
        completed_entries: 0,
    });

    let on_progress = |bytes_delta: u64, entries_delta: u64| {
        let transferred = transferred_bytes.fetch_add(bytes_delta, Ordering::Relaxed) + bytes_delta;
        let completed =
            completed_entries.fetch_add(entries_delta, Ordering::Relaxed) + entries_delta;
        progress(TransferProgress {
            total_bytes,
            transferred_bytes: transferred,
            total_entries,
            completed_entries: completed,
        });
    };

    for (source, target, name) in planned {
        if is_cancelled() {
            return Err(FileOpError::Cancelled);
        }
        if spec.source_root == spec.dest_root && spec.operation == TransferOperation::Move {
            fs::rename(&source, &target).await.map_err(|e| {
                tracing::error!("transfer move failed for {:?}: {e}", name);
                FileOpError::Io(e.to_string())
            })?;
            let (bytes, entries) = entry_size(&target).await?;
            on_progress(bytes, entries);
        } else {
            copy_entry(&source, &target, &on_progress, &is_cancelled).await?;
            if is_cancelled() {
                return Err(FileOpError::Cancelled);
            }
            if spec.operation == TransferOperation::Move {
                if source.is_dir() {
                    fs::remove_dir_all(&source).await.map_err(|e| {
                        tracing::error!("transfer remove source dir failed: {e}");
                        FileOpError::Io(e.to_string())
                    })?;
                } else {
                    fs::remove_file(&source).await.map_err(|e| {
                        tracing::error!("transfer remove source file failed: {e}");
                        FileOpError::Io(e.to_string())
                    })?;
                }
            }
        }
    }

    Ok(())
}

/// Delete one or more files/directories.
#[allow(dead_code)]
pub async fn delete_entries(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    paths: &[String],
) -> Result<(), FileOpError> {
    for path in paths {
        let resolved = resolve_path(&state.config, user, root_key, path)?;

        // Don't allow deleting the root itself
        let root_path =
            roots::resolve_root(&state.config, user, root_key, roots::RequiredCap::Write)
                .map_err(|e| FileOpError::Root(e.to_string()))?;
        if resolved == root_path {
            return Err(FileOpError::InvalidPath);
        }

        if resolved.is_dir() {
            fs::remove_dir_all(&resolved).await.map_err(|e| {
                tracing::error!("delete dir failed: {e}");
                FileOpError::Io(e.to_string())
            })?;
        } else {
            fs::remove_file(&resolved).await.map_err(|e| {
                tracing::error!("delete file failed: {e}");
                FileOpError::Io(e.to_string())
            })?;
        }
    }

    Ok(())
}

/// Receive an uploaded file, streaming to a validated destination directory.
///
/// Writes to a temp file first, then atomic-renames into place. On any error
/// after temp-file creation the temp file is removed before returning.
pub async fn receive_upload_raw(
    dest_dir: &Path,
    filename: &str,
    field: &mut axum::extract::multipart::Field<'_>,
    max_size: u64,
) -> Result<(), FileOpError> {
    let clean_name = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| FileOpError::InvalidName("invalid filename".into()))?;

    validate_filename(clean_name)?;

    let target = dest_dir.join(clean_name);
    let temp_path = dest_dir.join(format!(".upload-{}-{}", uuid::Uuid::new_v4(), clean_name));

    let mut file = fs::File::create(&temp_path).await.map_err(|e| {
        tracing::error!("create temp file failed: {e}");
        FileOpError::Io(e.to_string())
    })?;

    let mut written = 0u64;
    while let Some(chunk) = field.chunk().await.map_err(|e| {
        tracing::error!("read chunk failed: {e}");
        let _ = std::fs::remove_file(&temp_path);
        FileOpError::Io(e.to_string())
    })? {
        written += chunk.len() as u64;
        if written > max_size {
            let _ = std::fs::remove_file(&temp_path);
            return Err(FileOpError::TooLarge);
        }
        file.write_all(&chunk).await.map_err(|e| {
            tracing::error!("write chunk failed: {e}");
            let _ = std::fs::remove_file(&temp_path);
            FileOpError::Io(e.to_string())
        })?;
    }

    file.flush().await.map_err(|e| {
        tracing::error!("flush upload failed: {e}");
        let _ = std::fs::remove_file(&temp_path);
        FileOpError::Io(e.to_string())
    })?;

    // Remove exec bit (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        let _ = std::fs::set_permissions(&temp_path, perms);
    }

    fs::rename(&temp_path, &target).await.map_err(|e| {
        tracing::error!("rename upload failed: {e}");
        let _ = std::fs::remove_file(&temp_path);
        FileOpError::Io(e.to_string())
    })?;

    Ok(())
}

/// Receive an uploaded file for an authenticated user, streaming to disk.
/// Resolves and validates the destination via the user's access controls.
pub async fn receive_upload(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    dest_path: &str,
    filename: &str,
    field: &mut axum::extract::multipart::Field<'_>,
    max_size: u64,
) -> Result<(), FileOpError> {
    let dest_dir = resolve_path(&state.config, user, root_key, dest_path)?;
    if !dest_dir.is_dir() {
        return Err(FileOpError::NotDirectory);
    }
    receive_upload_raw(&dest_dir, filename, field, max_size).await
}

// -----------------------------------------------------------------------
// Error
// -----------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FileOpError {
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("path error: {0}")]
    Path(String),
    #[error("root error: {0}")]
    Root(String),
    #[error("target already exists")]
    AlreadyExists,
    #[error("target is not a directory")]
    NotDirectory,
    #[error("invalid path")]
    InvalidPath,
    #[error("file too large")]
    TooLarge,
    #[error("operation canceled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(String),
}

impl IntoResponse for FileOpError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            FileOpError::InvalidName(_) => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            FileOpError::Path(_) => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            FileOpError::Root(_) => (axum::http::StatusCode::FORBIDDEN, self.to_string()),
            FileOpError::AlreadyExists => (axum::http::StatusCode::CONFLICT, self.to_string()),
            FileOpError::NotDirectory => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            FileOpError::InvalidPath => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            FileOpError::TooLarge => (axum::http::StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            FileOpError::Cancelled => (axum::http::StatusCode::CONFLICT, self.to_string()),
            FileOpError::Io(_) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}

// -----------------------------------------------------------------------
// API request types
// -----------------------------------------------------------------------

#[derive(Deserialize)]
pub struct MkdirRequest {
    pub path: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub path: String,
    pub new_name: String,
}

#[derive(Deserialize)]
pub struct MoveRequest {
    pub paths: Vec<String>,
    pub dest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransferOperation {
    Move,
    Copy,
}

impl TransferOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Copy => "copy",
        }
    }
}

#[derive(Deserialize)]
pub struct TransferRequest {
    pub paths: Vec<String>,
    pub dest_root: String,
    pub dest: String,
    pub operation: TransferOperation,
}

#[derive(Deserialize)]
pub struct DeleteRequest {
    pub paths: Vec<String>,
}
