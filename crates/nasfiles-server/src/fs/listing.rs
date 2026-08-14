use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nasfiles_core::models::FileEntry;

use crate::thumb::kind;

/// List directory contents, returning sorted file entries.
///
/// Deliberately does *not* populate `item_count`: filling it in requires one
/// extra `read_dir` per subdirectory, which turns listing a folder with N
/// subfolders into N+1 directory scans. On a spinning pool that is N extra
/// seeks on the critical path of the first paint. Child counts are served
/// separately by [`child_counts`], which the browser fetches after the listing
/// has already rendered.
pub fn list_directory(
    path: &Path,
    thumbnails_enabled: bool,
) -> Result<Vec<FileEntry>, ListingError> {
    if !path.is_dir() {
        return Err(ListingError::NotADirectory);
    }

    let mut entries = Vec::new();

    let dir = std::fs::read_dir(path).map_err(|e| {
        tracing::error!("Failed to read directory {}: {e}", path.display());
        ListingError::Io(e)
    })?;

    for entry_result in dir {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Skipping unreadable entry in {}: {e}", path.display());
                continue;
            }
        };

        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files (dotfiles) — they're usually system files
        // TODO: make this configurable
        if name.starts_with('.') {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Skipping entry with unreadable metadata {name}: {e}");
                continue;
            }
        };

        let is_dir = metadata.is_dir();
        let size = if is_dir { 0 } else { metadata.len() };

        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let mime_type = if is_dir {
            None
        } else {
            mime_guess::from_path(&name).first().map(|m| m.to_string())
        };

        let has_thumbnail =
            !is_dir && kind::supports_thumbnail_path(&entry.path(), thumbnails_enabled);

        entries.push(FileEntry {
            name,
            size,
            modified_at,
            is_dir,
            mime_type,
            has_thumbnail,
            media_info: None,
            image_info: None,
            item_count: None,
            gallery_feedback: None,
        });
    }

    // Sort: directories first (alphabetical), then files (alphabetical)
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

/// Count the visible children of every subdirectory of `path`.
///
/// This is the expensive half of a listing — one `read_dir` per subdirectory —
/// so it lives behind its own endpoint and is never on the path of the first
/// render. Subdirectories that cannot be read are omitted rather than reported
/// as zero, so the browser can tell "empty" from "unknown".
pub fn child_counts(path: &Path) -> Result<HashMap<String, u64>, ListingError> {
    if !path.is_dir() {
        return Err(ListingError::NotADirectory);
    }

    let dir = std::fs::read_dir(path).map_err(|e| {
        tracing::error!("Failed to read directory {}: {e}", path.display());
        ListingError::Io(e)
    })?;

    let mut counts = HashMap::new();

    for entry in dir.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // `file_type` is served from the directory entry on most filesystems,
        // so this avoids a stat per child just to find the subdirectories.
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(children) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let count = children
            .filter_map(Result::ok)
            .filter(|child| !child.file_name().to_string_lossy().starts_with('.'))
            .count() as u64;
        counts.insert(name, count);
    }

    Ok(counts)
}

/// List only directories (for tree view).
pub fn list_directories(
    path: &Path,
    thumbnails_enabled: bool,
) -> Result<Vec<FileEntry>, ListingError> {
    let all = list_directory(path, thumbnails_enabled)?;
    Ok(all.into_iter().filter(|e| e.is_dir).collect())
}

/// `spawn_blocking` wrappers.
///
/// Every function above is synchronous `std::fs` work. On a spun-down or busy
/// pool a single `read_dir` or `metadata` call can park for seconds, and doing
/// that on a runtime worker stalls every other in-flight request behind it —
/// thumbnails, transfer-job polls, unrelated listings. Handlers must go through
/// these wrappers rather than calling the sync functions directly.
pub async fn list_directory_async(
    path: PathBuf,
    thumbnails_enabled: bool,
) -> Result<Vec<FileEntry>, ListingError> {
    blocking(move || list_directory(&path, thumbnails_enabled)).await
}

pub async fn list_directories_async(
    path: PathBuf,
    thumbnails_enabled: bool,
) -> Result<Vec<FileEntry>, ListingError> {
    blocking(move || list_directories(&path, thumbnails_enabled)).await
}

pub async fn child_counts_async(path: PathBuf) -> Result<HashMap<String, u64>, ListingError> {
    blocking(move || child_counts(&path)).await
}

async fn blocking<T, F>(work: F) -> Result<T, ListingError>
where
    F: FnOnce() -> Result<T, ListingError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("directory listing task failed: {e}");
            Err(ListingError::Io(std::io::Error::other(
                "listing task cancelled",
            )))
        }
    }
}

impl axum::response::IntoResponse for ListingError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ListingError::NotADirectory => (axum::http::StatusCode::BAD_REQUEST, "not a directory"),
            ListingError::Io(_) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read directory",
            ),
        };
        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ListingError {
    #[error("path is not a directory")]
    NotADirectory,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn listing_leaves_item_counts_unset() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("a.txt"), b"a").unwrap();

        let entries = list_directory(dir.path(), true).unwrap();

        let sub = entries.iter().find(|e| e.name == "sub").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.item_count, None);
    }

    #[test]
    fn counts_visible_children_of_each_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("two")).unwrap();
        fs::write(dir.path().join("two").join("a.txt"), b"a").unwrap();
        fs::write(dir.path().join("two").join("b.txt"), b"b").unwrap();
        fs::write(dir.path().join("two").join(".hidden"), b"h").unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        fs::create_dir(dir.path().join(".hidden-dir")).unwrap();
        fs::write(dir.path().join("loose.txt"), b"x").unwrap();

        let counts = child_counts(dir.path()).unwrap();

        assert_eq!(counts.get("two"), Some(&2));
        assert_eq!(counts.get("empty"), Some(&0));
        // Files and dotfile directories are not subdirectories to count.
        assert_eq!(counts.get("loose.txt"), None);
        assert_eq!(counts.get(".hidden-dir"), None);
    }

    #[test]
    fn disables_thumbnail_flags_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("image.jpg"), b"fake").unwrap();
        fs::write(dir.path().join("video.mp4"), b"fake").unwrap();
        fs::write(dir.path().join("audio.mp3"), b"fake").unwrap();
        fs::write(dir.path().join("book.epub"), b"fake").unwrap();
        fs::write(dir.path().join("document.pdf"), b"fake").unwrap();
        fs::write(dir.path().join("notes.txt"), b"fake").unwrap();

        let entries = list_directory(dir.path(), false).unwrap();

        assert!(entries.iter().all(|entry| !entry.has_thumbnail));
    }

    #[test]
    fn marks_supported_thumbnail_types_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("image.jpg"), b"fake").unwrap();
        fs::write(dir.path().join("video.mp4"), b"fake").unwrap();
        fs::write(dir.path().join("audio.mp3"), b"fake").unwrap();
        fs::write(dir.path().join("book.epub"), b"fake").unwrap();
        fs::write(dir.path().join("document.pdf"), b"fake").unwrap();
        fs::write(dir.path().join("notes.txt"), b"fake").unwrap();

        let entries = list_directory(dir.path(), true).unwrap();

        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "image.jpg")
                .unwrap()
                .has_thumbnail
        );
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "video.mp4")
                .unwrap()
                .has_thumbnail
        );
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "audio.mp3")
                .unwrap()
                .has_thumbnail
        );
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "book.epub")
                .unwrap()
                .has_thumbnail
        );
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "document.pdf")
                .unwrap()
                .has_thumbnail
        );
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "notes.txt")
                .unwrap()
                .has_thumbnail
        );
    }
}
