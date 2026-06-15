use std::path::Path;

use nasfiles_core::models::FileEntry;

use crate::thumb::kind;

/// List directory contents, returning sorted file entries.
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

/// List only directories (for tree view).
pub fn list_directories(
    path: &Path,
    thumbnails_enabled: bool,
) -> Result<Vec<FileEntry>, ListingError> {
    let all = list_directory(path, thumbnails_enabled)?;
    Ok(all.into_iter().filter(|e| e.is_dir).collect())
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
