use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore};

use super::{
    audio as audio_thumb, epub as epub_thumb, image as img_thumb, kind::ThumbnailKind,
    pdf as pdf_thumb, storage::ThumbnailMeta, storage::ThumbnailStorage, svg as svg_thumb,
    text as text_thumb, video as vid_thumb,
};
use crate::config::AppConfig;

const THUMBNAIL_STORAGE_VERSION: u32 = 3;

pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

#[derive(Clone, Copy)]
pub enum ThumbFormat {
    Jpeg,
    Png,
}

impl ThumbFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ThumbFormat::Jpeg => "jpg",
            ThumbFormat::Png => "png",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            ThumbFormat::Jpeg => "image/jpeg",
            ThumbFormat::Png => "image/png",
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            ThumbFormat::Jpeg => "jpeg",
            ThumbFormat::Png => "png",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThumbnailCacheKeyInput<'a> {
    pub root_kind: &'a str,
    pub root_key: &'a str,
    pub relative_path: &'a str,
    pub mtime_ms: i64,
    pub file_size: u64,
    pub width: u32,
    pub kind: ThumbnailKind,
    pub format: ThumbFormat,
}

pub struct ThumbnailRequest<'a> {
    pub source_path: &'a Path,
    pub root_kind: &'a str,
    pub root_key: &'a str,
    pub relative_path: &'a str,
    pub width: u32,
    pub requested_format: Option<ThumbFormat>,
}

/// Thumbnail cache with per-file generation mutex.
///
/// Thumbnail keys include generator version, kind, format, root, relative path,
/// source mtime, source size, and requested width. Bytes are stored in xattrs
/// when practical, with the cache directory as a fallback.
#[derive(Clone)]
pub struct ThumbnailCache {
    storage: Arc<ThumbnailStorage>,
    /// Per-file mutex to prevent concurrent generation of the same thumbnail.
    locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
    generation_slots: Arc<Semaphore>,
}

impl ThumbnailCache {
    pub fn new(cache_dir: PathBuf, max_concurrent_generations: usize) -> Self {
        Self {
            storage: Arc::new(ThumbnailStorage::new(cache_dir)),
            locks: Arc::new(DashMap::new()),
            generation_slots: Arc::new(Semaphore::new(max_concurrent_generations.max(1))),
        }
    }

    /// Compute the cache key for a file.
    pub fn cache_key(
        root_kind: &str,
        root_key: &str,
        relative_path: &str,
        mtime_ms: i64,
        file_size: u64,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(root_kind.as_bytes());
        hasher.update(b":");
        hasher.update(root_key.as_bytes());
        hasher.update(b":");
        hasher.update(relative_path.as_bytes());
        hasher.update(b":");
        hasher.update(mtime_ms.to_le_bytes());
        hasher.update(b":");
        hasher.update(file_size.to_le_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn thumbnail_cache_key(input: ThumbnailCacheKeyInput<'_>) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"thumb-v");
        hasher.update(THUMBNAIL_STORAGE_VERSION.to_le_bytes());
        hasher.update(b":");
        hasher.update(input.kind.as_key().as_bytes());
        hasher.update(b":");
        hasher.update(input.format.as_key().as_bytes());
        hasher.update(b":");
        hasher.update(input.root_kind.as_bytes());
        hasher.update(b":");
        hasher.update(input.root_key.as_bytes());
        hasher.update(b":");
        hasher.update(input.relative_path.as_bytes());
        hasher.update(b":");
        hasher.update(input.mtime_ms.to_le_bytes());
        hasher.update(b":");
        hasher.update(input.file_size.to_le_bytes());
        hasher.update(b":");
        hasher.update(input.width.to_le_bytes());
        hex::encode(hasher.finalize())
    }

    /// Get or generate a thumbnail.
    ///
    /// Returns `Some(thumbnail)` if a thumbnail was generated or found in cache.
    /// Returns `None` if the file type is unsupported.
    pub async fn get_or_generate(
        &self,
        request: ThumbnailRequest<'_>,
        config: &AppConfig,
    ) -> Result<Option<Thumbnail>, ThumbError> {
        // Get file metadata for cache key
        let metadata = tokio::fs::metadata(request.source_path)
            .await
            .map_err(|e| ThumbError::Io(e.to_string()))?;

        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let Some(kind) = ThumbnailKind::from_path(request.source_path) else {
            return Ok(None);
        };
        let format = match (kind, request.requested_format) {
            (ThumbnailKind::Image | ThumbnailKind::Svg, Some(format)) => format,
            (ThumbnailKind::Svg, None) => ThumbFormat::Png,
            _ => ThumbFormat::Jpeg,
        };

        let key = Self::thumbnail_cache_key(ThumbnailCacheKeyInput {
            root_kind: request.root_kind,
            root_key: request.root_key,
            relative_path: request.relative_path,
            mtime_ms,
            file_size: metadata.len(),
            width: request.width,
            kind,
            format,
        });
        let storage_meta = ThumbnailMeta {
            version: THUMBNAIL_STORAGE_VERSION,
            key: key.clone(),
            format: format.as_key().to_string(),
            width: request.width,
            source_mtime_ms: mtime_ms,
            source_size: metadata.len(),
        };

        if let Some(bytes) = self
            .storage
            .read(request.source_path, &key, format, &storage_meta)
            .await?
        {
            return Ok(Some(Thumbnail {
                bytes,
                content_type: format.content_type(),
            }));
        }

        if matches!(
            kind,
            ThumbnailKind::Image | ThumbnailKind::Svg | ThumbnailKind::Epub
        ) && metadata.len() > config.thumbnail_max_source_file_size
        {
            return Err(ThumbError::TooLarge {
                size: metadata.len(),
                limit: config.thumbnail_max_source_file_size,
            });
        }

        // Acquire per-file lock to prevent concurrent generation
        let lock = self
            .locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();

        let _guard = lock.lock().await;

        if let Some(bytes) = self
            .storage
            .read(request.source_path, &key, format, &storage_meta)
            .await?
        {
            return Ok(Some(Thumbnail {
                bytes,
                content_type: format.content_type(),
            }));
        }

        let _slot = self
            .generation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| ThumbError::Io(e.to_string()))?;

        let thumb_bytes = match kind {
            ThumbnailKind::Image => {
                img_thumb::generate(
                    request.source_path,
                    request.width,
                    format,
                    config.thumbnail_max_image_width,
                    config.thumbnail_max_image_height,
                    config.thumbnail_max_image_alloc,
                )
                .await?
            }
            ThumbnailKind::Video => vid_thumb::generate(request.source_path, request.width).await?,
            ThumbnailKind::Audio => {
                audio_thumb::generate(
                    request.source_path,
                    request.width,
                    config.thumbnail_max_image_width,
                    config.thumbnail_max_image_height,
                    config.thumbnail_max_image_alloc,
                )
                .await?
            }
            ThumbnailKind::Pdf => pdf_thumb::generate(request.source_path, request.width).await?,
            ThumbnailKind::Text => text_thumb::generate(request.source_path, request.width).await?,
            ThumbnailKind::Epub => {
                epub_thumb::generate(
                    request.source_path,
                    request.width,
                    config.thumbnail_max_image_width,
                    config.thumbnail_max_image_height,
                    config.thumbnail_max_image_alloc,
                )
                .await?
            }
            ThumbnailKind::Svg => {
                svg_thumb::generate(
                    request.source_path,
                    request.width,
                    config.thumbnail_max_image_width,
                    config.thumbnail_max_image_height,
                )
                .await?
            }
        };

        if let Some(ref bytes) = thumb_bytes {
            self.storage
                .write(request.source_path, &key, format, &storage_meta, bytes)
                .await?;
        }

        Ok(thumb_bytes.map(|bytes| Thumbnail {
            bytes,
            content_type: format.content_type(),
        }))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub async fn get_or_generate_with_test_generator(
        &self,
        source_path: &Path,
        root_kind: &str,
        root_key: &str,
        relative_path: &str,
        width: u32,
        _config: &AppConfig,
        generator: impl std::future::Future<Output = Result<Option<Vec<u8>>, ThumbError>>,
    ) -> Result<Option<Thumbnail>, ThumbError> {
        let metadata = tokio::fs::metadata(source_path)
            .await
            .map_err(|e| ThumbError::Io(e.to_string()))?;
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let Some(kind) = ThumbnailKind::from_path(source_path) else {
            return Ok(None);
        };
        let format = if kind == ThumbnailKind::Svg {
            ThumbFormat::Png
        } else {
            ThumbFormat::Jpeg
        };
        let key = Self::thumbnail_cache_key(ThumbnailCacheKeyInput {
            root_kind,
            root_key,
            relative_path,
            mtime_ms,
            file_size: metadata.len(),
            width,
            kind,
            format,
        });
        let storage_meta = ThumbnailMeta {
            version: THUMBNAIL_STORAGE_VERSION,
            key: key.clone(),
            format: format.as_key().to_string(),
            width,
            source_mtime_ms: mtime_ms,
            source_size: metadata.len(),
        };

        if let Some(bytes) = self
            .storage
            .read(source_path, &key, format, &storage_meta)
            .await?
        {
            return Ok(Some(Thumbnail {
                bytes,
                content_type: format.content_type(),
            }));
        }

        let lock = self
            .locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        let _slot = self
            .generation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| ThumbError::Io(e.to_string()))?;

        let thumb_bytes = generator.await?;
        if let Some(ref bytes) = thumb_bytes {
            self.storage
                .write(source_path, &key, format, &storage_meta, bytes)
                .await?;
        }

        Ok(thumb_bytes.map(|bytes| Thumbnail {
            bytes,
            content_type: format.content_type(),
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("io error: {0}")]
    Io(String),
    #[error("image processing error: {0}")]
    Image(String),
    #[error("svg processing error: {0}")]
    Svg(String),
    #[error("video processing error: {0}")]
    Video(String),
    #[error("audio processing error: {0}")]
    Audio(String),
    #[error("pdf processing error: {0}")]
    Pdf(String),
    #[error("epub processing error: {0}")]
    Epub(String),
    #[error("source file too large for thumbnail generation: {size} bytes exceeds {limit} bytes")]
    TooLarge { size: u64, limit: u64 },
}

impl axum::response::IntoResponse for ThumbError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            ThumbError::TooLarge { .. } => axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            ThumbError::Io(_)
            | ThumbError::Image(_)
            | ThumbError::Svg(_)
            | ThumbError::Video(_)
            | ThumbError::Audio(_)
            | ThumbError::Pdf(_)
            | ThumbError::Epub(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        tracing::warn!("thumbnail error: {self}");
        (
            status,
            axum::Json(serde_json::json!({"error": "thumbnail generation failed"})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use nasfiles_core::models::FolderCaps;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Duration, sleep};

    #[test]
    fn thumbnail_cache_key_includes_width() {
        let a = ThumbnailCache::thumbnail_cache_key(ThumbnailCacheKeyInput {
            root_kind: "common",
            root_key: "Files",
            relative_path: "image.jpg",
            mtime_ms: 123,
            file_size: 456,
            width: 480,
            kind: ThumbnailKind::Image,
            format: ThumbFormat::Jpeg,
        });
        let b = ThumbnailCache::thumbnail_cache_key(ThumbnailCacheKeyInput {
            root_kind: "common",
            root_key: "Files",
            relative_path: "image.jpg",
            mtime_ms: 123,
            file_size: 456,
            width: 960,
            kind: ThumbnailKind::Image,
            format: ThumbFormat::Jpeg,
        });
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn generation_slots_queue_instead_of_failing() {
        let data_dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(data_dir.path().join("thumbs"), 1);
        let first = data_dir.path().join("first.jpg");
        let second = data_dir.path().join("second.jpg");
        tokio::fs::write(&first, b"fake").await.unwrap();
        tokio::fs::write(&second, b"fake").await.unwrap();
        let config = std::sync::Arc::new(test_config(data_dir.path().join("thumbs")));
        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let max_active = std::sync::Arc::new(AtomicUsize::new(0));

        let run_one = |path: std::path::PathBuf, name: &'static str| {
            let cache = cache.clone();
            let config = config.clone();
            let active = active.clone();
            let max_active = max_active.clone();
            tokio::spawn(async move {
                cache
                    .get_or_generate_with_test_generator(
                        &path,
                        "common",
                        "Files",
                        name,
                        480,
                        &config,
                        async move {
                            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(now, Ordering::SeqCst);
                            sleep(Duration::from_millis(50)).await;
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(Some(vec![0xff, 0xd8, 0xff, 0xd9]))
                        },
                    )
                    .await
                    .unwrap()
            })
        };

        let (a, b) = tokio::join!(run_one(first, "first.jpg"), run_one(second, "second.jpg"));
        assert!(a.unwrap().is_some());
        assert!(b.unwrap().is_some());
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    fn test_config(thumbnail_cache_dir: PathBuf) -> AppConfig {
        AppConfig {
            bind_addr: String::new(),
            base_url: String::new(),
            session_secret: vec![],
            data_dir: PathBuf::new(),
            dev_mode: true,
            auth_mode: crate::config::AuthMode::Sso,
            no_server_side_execution: false,
            csp_extra_img_src: Vec::new(),
            csp_extra_media_src: Vec::new(),
            db_url: String::new(),
            common_folders: HashMap::new(),
            home_folder_root: None,
            share_group_of_folder: HashMap::new(),
            oidc: None,
            sso_username_claim: String::new(),
            sso_display_name_claim: String::new(),
            sso_picture_claim: String::new(),
            sso_groups_claim: String::new(),
            group_folder_caps: HashMap::<String, HashMap<String, FolderCaps>>::new(),
            default_folder_caps: HashMap::new(),
            admin_groups: vec![],
            personal_folder_groups: None,
            groups_refresh_interval_secs: 0,
            dev_user: None,
            disable_passkeys: false,
            disable_totp: false,
            setup_admin: None,
            totp_trusted_device_ttl_days: 0,
            thumbnail_cache_dir,
            thumbnail_max_source_file_size: 1024 * 1024,
            thumbnail_max_image_width: 20_000,
            thumbnail_max_image_height: 20_000,
            thumbnail_max_image_alloc: 256 * 1024 * 1024,
            thumbnail_max_concurrent_generations: 1,
            media_preview_max_concurrent_transcodes: 1,
            search_max_results: 100,
            search_live_entry_budget: 25_000,
            search_live_time_budget_ms: 1_500,
            search_reindex_interval_secs: 300,
            share_token_bytes: 24,
            sftp_enabled: false,
            sftp_bind_addr: String::new(),
            sftp_host_key_path: PathBuf::new(),
            max_upload_file_size: 0,
            max_upload_request_size: 0,
            log_level: String::new(),
            trusted_proxy_depth: 1,
            custom_links: Vec::new(),
            sftp_public_port: None,
        }
    }
}
