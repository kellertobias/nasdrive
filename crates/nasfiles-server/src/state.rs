use crate::config::AppConfig;
use crate::fs::preview::MediaPreviewService;
use crate::shares::access::ShareRateLimiter;
use crate::thumb::cache::ThumbnailCache;
use dashmap::DashMap;
use serde::Serialize;
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
    pub transfer_jobs: TransferJobStore,
    pub webauthn: Option<Arc<Webauthn>>,
}

impl AppState {
    pub fn new(config: AppConfig, pool: AnyPool) -> anyhow::Result<Self> {
        let webauthn = crate::auth::local::build_webauthn(&config)?.map(Arc::new);
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
            config: Arc::new(config),
            pool,
            rate_limiter: ShareRateLimiter::new(),
            thumb_cache,
            transfer_jobs: TransferJobStore::new(),
            webauthn,
        })
    }
}

#[derive(Clone)]
pub struct TransferJobStore {
    jobs: Arc<DashMap<String, TransferJob>>,
}

impl TransferJobStore {
    fn new() -> Self {
        Self {
            jobs: Arc::new(DashMap::new()),
        }
    }

    pub fn insert(&self, job: TransferJob) {
        self.jobs.insert(job.id.clone(), job);
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut TransferJob)) {
        if let Some(mut job) = self.jobs.get_mut(id) {
            f(&mut job);
            job.updated_at = now_ms();
        }
    }

    pub fn cancel_for_user(&self, id: &str, user_id: &str) -> bool {
        if let Some(mut job) = self.jobs.get_mut(id) {
            if job.owner_user_id != user_id {
                return false;
            }
            if !matches!(job.status, TransferJobStatus::Queued | TransferJobStatus::Running) {
                return false;
            }
            job.cancel_requested = true;
            job.updated_at = now_ms();
            return true;
        }
        false
    }

    pub fn is_cancel_requested(&self, id: &str) -> bool {
        self.jobs
            .get(id)
            .map(|job| job.cancel_requested)
            .unwrap_or(false)
    }

    pub fn list_for_user(&self, user_id: &str) -> Vec<TransferJob> {
        let mut jobs: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| job.owner_user_id == user_id)
            .map(|job| job.value().clone())
            .collect();
        jobs.sort_by_key(|job| job.created_at);
        jobs.reverse();
        jobs
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TransferJob {
    pub id: String,
    #[serde(skip_serializing)]
    pub owner_user_id: String,
    pub operation: String,
    pub source_root: String,
    pub dest_root: String,
    pub dest_path: String,
    pub paths: Vec<String>,
    pub status: TransferJobStatus,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub total_entries: u64,
    pub completed_entries: u64,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub cancel_requested: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferJobStatus {
    Queued,
    Running,
    Done,
    Error,
    Canceled,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
