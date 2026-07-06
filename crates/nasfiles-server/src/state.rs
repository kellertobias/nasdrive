use crate::config::AppConfig;
use crate::fs::file_jobs::FileJobStore;
use crate::fs::preview::MediaPreviewService;
use crate::fs::search::SearchService;
use crate::sftp::sessions::SftpSessionRegistry;
use crate::shares::access::ShareRateLimiter;
use crate::thumb::cache::ThumbnailCache;
use chrono::{DateTime, Utc};
use sqlx::AnyPool;
use std::sync::Arc;
use webauthn_rs::prelude::Webauthn;

/// Shared application state available to all handlers via axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub pool: AnyPool,
    pub rate_limiter: ShareRateLimiter,
    pub thumb_cache: Option<ThumbnailCache>,
    pub media_preview: MediaPreviewService,
    pub file_jobs: FileJobStore,
    pub search: SearchService,
    pub webauthn: Option<Arc<Webauthn>>,
    pub sftp_sessions: SftpSessionRegistry,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(config: AppConfig, pool: AnyPool) -> anyhow::Result<Self> {
        let webauthn = crate::auth::local::build_webauthn(&config)?.map(Arc::new);
        let config = Arc::new(config);
        let thumb_cache = if config.no_server_side_execution {
            None
        } else {
            Some(ThumbnailCache::new(
                config.thumbnail_cache_dir.clone(),
                config.thumbnail_max_concurrent_generations,
            ))
        };

        Ok(Self {
            media_preview: MediaPreviewService::new(config.media_preview_max_concurrent_transcodes),
            search: SearchService::new(config.clone()),
            config,
            pool: pool.clone(),
            rate_limiter: ShareRateLimiter::new(),
            thumb_cache,
            file_jobs: FileJobStore::new(pool.clone()),
            webauthn,
            sftp_sessions: SftpSessionRegistry::new(),
            started_at: Utc::now(),
        })
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
