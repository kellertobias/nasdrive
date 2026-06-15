use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, Row};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, sleep};

use crate::fs::roots;
use crate::state::{AppState, now_ms};
use nasfiles_core::models::AuthUser;

use super::ops::{FileOpError, TransferOperation};

#[derive(Clone)]
pub struct FileJobStore {
    pool: AnyPool,
    running: Arc<DashMap<String, ()>>,
}

impl FileJobStore {
    pub fn new(pool: AnyPool) -> Self {
        Self {
            pool,
            running: Arc::new(DashMap::new()),
        }
    }

    pub async fn create_transfer_job(
        &self,
        user: &AuthUser,
        source_root: &str,
        paths: &[String],
        dest_root: &str,
        dest_path: &str,
        operation: TransferOperation,
    ) -> Result<String, FileOpError> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let paths_json =
            serde_json::to_string(paths).map_err(|e| FileOpError::Io(e.to_string()))?;
        let owner_user_json =
            serde_json::to_string(user).map_err(|e| FileOpError::Io(e.to_string()))?;

        sqlx::query(
            "INSERT INTO file_operation_jobs \
             (id, owner_user_id, owner_user_json, operation, source_root, dest_root, dest_path, paths_json, status, \
              total_bytes, transferred_bytes, total_entries, completed_entries, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'queued', 0, 0, 0, 0, $9, $10)",
        )
        .bind(&job_id)
        .bind(&user.user_id)
        .bind(owner_user_json)
        .bind(operation.as_str())
        .bind(source_root)
        .bind(dest_root)
        .bind(dest_path)
        .bind(paths_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        Ok(job_id)
    }

    pub async fn create_delete_job(
        &self,
        user: &AuthUser,
        root: &str,
        paths: &[String],
    ) -> Result<String, FileOpError> {
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let paths_json =
            serde_json::to_string(paths).map_err(|e| FileOpError::Io(e.to_string()))?;
        let owner_user_json =
            serde_json::to_string(user).map_err(|e| FileOpError::Io(e.to_string()))?;

        sqlx::query(
            "INSERT INTO file_operation_jobs \
             (id, owner_user_id, owner_user_json, operation, source_root, dest_root, dest_path, paths_json, status, \
              total_bytes, transferred_bytes, total_entries, completed_entries, created_at, updated_at) \
             VALUES ($1, $2, $3, 'delete', $4, $5, '', $6, 'queued', 0, 0, 0, 0, $7, $8)",
        )
        .bind(&job_id)
        .bind(&user.user_id)
        .bind(owner_user_json)
        .bind(root)
        .bind(root)
        .bind(paths_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        Ok(job_id)
    }

    pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<FileJob>, FileOpError> {
        let rows = sqlx::query(
            "SELECT * FROM file_operation_jobs WHERE owner_user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;

        rows.into_iter().map(FileJob::from_row).collect()
    }

    pub async fn cancel_for_user(&self, id: &str, user_id: &str) -> Result<bool, FileOpError> {
        let now = now_ms();
        let result = sqlx::query(
            "UPDATE file_operation_jobs \
             SET cancel_requested = TRUE, updated_at = $1 \
             WHERE id = $2 AND owner_user_id = $3 AND status IN ('queued', 'running', 'paused_needs_confirmation')",
        )
        .bind(now)
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn resume_for_user(&self, id: &str, user_id: &str) -> Result<bool, FileOpError> {
        let now = now_ms();
        let result = sqlx::query(
            "UPDATE file_operation_jobs \
             SET status = 'queued', cancel_requested = FALSE, error = NULL, finished_at = NULL, updated_at = $1 \
             WHERE id = $2 AND owner_user_id = $3 AND status IN ('paused_needs_confirmation', 'error', 'canceled')",
        )
        .bind(now)
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn cleanup_for_user(
        &self,
        config: &crate::config::AppConfig,
        id: &str,
        user_id: &str,
    ) -> Result<bool, FileOpError> {
        let job = self.get_for_user(id, user_id).await?;
        let Some(job) = job else {
            return Ok(false);
        };
        if matches!(
            job.status,
            FileJobStatus::Queued | FileJobStatus::Running | FileJobStatus::Done
        ) {
            return Ok(false);
        }

        if job.operation != FileOperationKind::Delete {
            let rows = sqlx::query(
                "SELECT dest_path FROM file_operation_items WHERE job_id = $1 AND status != 'done'",
            )
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_error)?;

            for row in rows {
                let dest_path: String = row.try_get("dest_path").map_err(db_error)?;
                if dest_path.is_empty() {
                    continue;
                }
                if let Ok(target) = resolve_user_path(
                    config,
                    &job.owner_user,
                    &job.dest_root,
                    &dest_path,
                    roots::RequiredCap::Write,
                ) {
                    let _ = remove_temp_for_final(id, &dest_path, &target).await;
                }
            }
        }

        sqlx::query(
            "UPDATE file_operation_jobs SET status = 'canceled', cancel_requested = TRUE, updated_at = $1, finished_at = $2 \
             WHERE id = $3 AND owner_user_id = $4",
        )
        .bind(now_ms())
        .bind(now_ms())
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        Ok(true)
    }

    pub async fn recover_startup_jobs(&self) -> Result<Vec<String>, FileOpError> {
        let now = now_ms();
        sqlx::query(
            "UPDATE file_operation_jobs SET status = 'paused_needs_confirmation', updated_at = $1 \
             WHERE operation = 'delete' AND status IN ('queued', 'running')",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        sqlx::query(
            "UPDATE file_operation_jobs SET status = 'queued', updated_at = $1 \
             WHERE operation IN ('copy', 'move') AND status = 'running'",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        let rows = sqlx::query(
            "SELECT id FROM file_operation_jobs WHERE operation IN ('copy', 'move') AND status = 'queued'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;

        rows.into_iter()
            .map(|row| row.try_get("id").map_err(db_error))
            .collect()
    }

    async fn get(&self, id: &str) -> Result<Option<FileJob>, FileOpError> {
        let row = sqlx::query("SELECT * FROM file_operation_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_error)?;
        row.map(FileJob::from_row).transpose()
    }

    async fn get_for_user(&self, id: &str, user_id: &str) -> Result<Option<FileJob>, FileOpError> {
        let row =
            sqlx::query("SELECT * FROM file_operation_jobs WHERE id = $1 AND owner_user_id = $2")
                .bind(id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_error)?;
        row.map(FileJob::from_row).transpose()
    }

    async fn set_status(
        &self,
        id: &str,
        status: FileJobStatus,
        error: Option<String>,
        finished: bool,
    ) -> Result<(), FileOpError> {
        let now = now_ms();
        let finished_at = finished.then_some(now);
        sqlx::query(
            "UPDATE file_operation_jobs SET status = $1, error = $2, updated_at = $3, finished_at = $4 WHERE id = $5",
        )
        .bind(status.as_str())
        .bind(error)
        .bind(now)
        .bind(finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn is_cancel_requested(&self, id: &str) -> Result<bool, FileOpError> {
        let row = sqlx::query("SELECT cancel_requested FROM file_operation_jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(row
            .and_then(|row| row.try_get::<bool, _>("cancel_requested").ok())
            .unwrap_or(false))
    }

    async fn item_count(&self, job_id: &str) -> Result<i64, FileOpError> {
        let row =
            sqlx::query("SELECT COUNT(*) AS count FROM file_operation_items WHERE job_id = $1")
                .bind(job_id)
                .fetch_one(&self.pool)
                .await
                .map_err(db_error)?;
        row.try_get("count").map_err(db_error)
    }

    async fn insert_items(&self, job_id: &str, items: &[PlannedItem]) -> Result<(), FileOpError> {
        for item in items {
            let now = now_ms();
            sqlx::query(
                "INSERT INTO file_operation_items \
                 (id, job_id, ordinal, kind, source_path, dest_path, size_bytes, status, bytes_done, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 0, $8, $9)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(job_id)
            .bind(item.ordinal)
            .bind(item.kind.as_str())
            .bind(&item.source_path)
            .bind(&item.dest_path)
            .bind(item.size_bytes as i64)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        }
        Ok(())
    }

    async fn pending_items(&self, job_id: &str) -> Result<Vec<FileJobItem>, FileOpError> {
        let rows = sqlx::query(
            "SELECT * FROM file_operation_items WHERE job_id = $1 AND status != 'done' ORDER BY ordinal ASC",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        rows.into_iter().map(FileJobItem::from_row).collect()
    }

    async fn mark_item_done(&self, item: &FileJobItem, bytes_done: u64) -> Result<(), FileOpError> {
        let now = now_ms();
        sqlx::query(
            "UPDATE file_operation_items SET status = 'done', bytes_done = $1, error = NULL, updated_at = $2, finished_at = $3 WHERE id = $4",
        )
        .bind(bytes_done as i64)
        .bind(now)
        .bind(now)
        .bind(&item.id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        self.recompute_progress(&item.job_id).await
    }

    async fn update_item_bytes(
        &self,
        item: &FileJobItem,
        bytes_done: u64,
    ) -> Result<(), FileOpError> {
        sqlx::query(
            "UPDATE file_operation_items SET bytes_done = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(bytes_done as i64)
        .bind(now_ms())
        .bind(&item.id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        self.recompute_progress(&item.job_id).await
    }

    async fn recompute_progress(&self, job_id: &str) -> Result<(), FileOpError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(size_bytes), 0) AS total_bytes, \
                    COALESCE(SUM(bytes_done), 0) AS transferred_bytes, \
                    COUNT(*) AS total_entries, \
                    COALESCE(SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END), 0) AS completed_entries \
             FROM file_operation_items WHERE job_id = $1",
        )
        .bind(job_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)?;
        let total_bytes: i64 = row.try_get("total_bytes").map_err(db_error)?;
        let transferred_bytes: i64 = row.try_get("transferred_bytes").map_err(db_error)?;
        let total_entries: i64 = row.try_get("total_entries").map_err(db_error)?;
        let completed_entries: i64 = row.try_get("completed_entries").map_err(db_error)?;

        sqlx::query(
            "UPDATE file_operation_jobs SET total_bytes = $1, transferred_bytes = $2, total_entries = $3, completed_entries = $4, updated_at = $5 WHERE id = $6",
        )
        .bind(total_bytes)
        .bind(transferred_bytes)
        .bind(total_entries)
        .bind(completed_entries)
        .bind(now_ms())
        .bind(job_id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileJob {
    pub id: String,
    #[serde(skip_serializing)]
    pub owner_user: AuthUser,
    pub operation: FileOperationKind,
    pub source_root: String,
    pub dest_root: String,
    pub dest_path: String,
    pub paths: Vec<String>,
    pub status: FileJobStatus,
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

impl FileJob {
    fn from_row(row: sqlx::any::AnyRow) -> Result<Self, FileOpError> {
        let paths_json: String = row.try_get("paths_json").map_err(db_error)?;
        let paths = serde_json::from_str(&paths_json).unwrap_or_default();
        let owner_user_json: String = row.try_get("owner_user_json").map_err(db_error)?;
        let owner_user = serde_json::from_str(&owner_user_json)
            .map_err(|e| FileOpError::Io(format!("invalid stored file job user snapshot: {e}")))?;
        Ok(Self {
            id: row.try_get("id").map_err(db_error)?,
            owner_user,
            operation: FileOperationKind::from_str(
                &row.try_get::<String, _>("operation").map_err(db_error)?,
            )?,
            source_root: row.try_get("source_root").map_err(db_error)?,
            dest_root: row.try_get("dest_root").map_err(db_error)?,
            dest_path: row.try_get("dest_path").map_err(db_error)?,
            paths,
            status: FileJobStatus::from_str(
                &row.try_get::<String, _>("status").map_err(db_error)?,
            )?,
            total_bytes: row.try_get::<i64, _>("total_bytes").map_err(db_error)? as u64,
            transferred_bytes: row
                .try_get::<i64, _>("transferred_bytes")
                .map_err(db_error)? as u64,
            total_entries: row.try_get::<i64, _>("total_entries").map_err(db_error)? as u64,
            completed_entries: row
                .try_get::<i64, _>("completed_entries")
                .map_err(db_error)? as u64,
            error: row.try_get("error").map_err(db_error)?,
            created_at: row.try_get("created_at").map_err(db_error)?,
            updated_at: row.try_get("updated_at").map_err(db_error)?,
            finished_at: row.try_get("finished_at").map_err(db_error)?,
            cancel_requested: row.try_get("cancel_requested").map_err(db_error)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileJobStatus {
    Queued,
    Running,
    PausedNeedsConfirmation,
    Done,
    Error,
    Canceled,
}

impl FileJobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::PausedNeedsConfirmation => "paused_needs_confirmation",
            Self::Done => "done",
            Self::Error => "error",
            Self::Canceled => "canceled",
        }
    }

    fn from_str(value: &str) -> Result<Self, FileOpError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused_needs_confirmation" => Ok(Self::PausedNeedsConfirmation),
            "done" => Ok(Self::Done),
            "error" => Ok(Self::Error),
            "canceled" => Ok(Self::Canceled),
            _ => Err(FileOpError::Io(format!("unknown job status: {value}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOperationKind {
    Copy,
    Move,
    Delete,
}

impl FileOperationKind {
    fn from_str(value: &str) -> Result<Self, FileOpError> {
        match value {
            "copy" => Ok(Self::Copy),
            "move" => Ok(Self::Move),
            "delete" => Ok(Self::Delete),
            _ => Err(FileOpError::Io(format!("unknown operation: {value}"))),
        }
    }
}

#[derive(Clone, Debug)]
struct FileJobItem {
    id: String,
    job_id: String,
    kind: PlannedItemKind,
    source_path: String,
    dest_path: String,
    size_bytes: u64,
}

impl FileJobItem {
    fn from_row(row: sqlx::any::AnyRow) -> Result<Self, FileOpError> {
        Ok(Self {
            id: row.try_get("id").map_err(db_error)?,
            job_id: row.try_get("job_id").map_err(db_error)?,
            kind: PlannedItemKind::from_str(&row.try_get::<String, _>("kind").map_err(db_error)?)?,
            source_path: row.try_get("source_path").map_err(db_error)?,
            dest_path: row.try_get("dest_path").map_err(db_error)?,
            size_bytes: row.try_get::<i64, _>("size_bytes").map_err(db_error)? as u64,
        })
    }
}

#[derive(Clone, Debug)]
struct PlannedItem {
    ordinal: i64,
    kind: PlannedItemKind,
    source_path: String,
    dest_path: String,
    size_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlannedItemKind {
    File,
    Dir,
}

impl PlannedItemKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
        }
    }

    fn from_str(value: &str) -> Result<Self, FileOpError> {
        match value {
            "file" => Ok(Self::File),
            "dir" => Ok(Self::Dir),
            _ => Err(FileOpError::Io(format!("unknown item kind: {value}"))),
        }
    }
}

pub fn spawn_file_job(state: AppState, job_id: String) {
    if state.file_jobs.running.insert(job_id.clone(), ()).is_some() {
        return;
    }

    tokio::spawn(async move {
        let result = run_file_job(&state, &job_id).await;
        if let Err(err) = result {
            tracing::warn!("file job {job_id} failed: {err}");
            let _ = state
                .file_jobs
                .set_status(&job_id, FileJobStatus::Error, Some(err.to_string()), true)
                .await;
        }
        state.file_jobs.running.remove(&job_id);
    });
}

pub async fn spawn_recovered_jobs(state: AppState) -> Result<(), FileOpError> {
    let jobs = state.file_jobs.recover_startup_jobs().await?;
    for job_id in jobs {
        spawn_file_job(state.clone(), job_id);
    }
    Ok(())
}

async fn run_file_job(state: &AppState, job_id: &str) -> Result<(), FileOpError> {
    let Some(job) = state.file_jobs.get(job_id).await? else {
        return Ok(());
    };

    if job.status == FileJobStatus::PausedNeedsConfirmation {
        return Ok(());
    }

    state
        .file_jobs
        .set_status(job_id, FileJobStatus::Running, None, false)
        .await?;

    if state.file_jobs.item_count(job_id).await? == 0 {
        let items = plan_job(state, &job).await?;
        state.file_jobs.insert_items(job_id, &items).await?;
        state.file_jobs.recompute_progress(job_id).await?;
    }

    let result = match job.operation {
        FileOperationKind::Copy => execute_copy_like_job(state, &job, false).await,
        FileOperationKind::Move => execute_copy_like_job(state, &job, true).await,
        FileOperationKind::Delete => execute_delete_job(state, &job).await,
    };

    match result {
        Ok(()) => {
            state.file_jobs.recompute_progress(job_id).await?;
            state
                .file_jobs
                .set_status(job_id, FileJobStatus::Done, None, true)
                .await?;
            Ok(())
        }
        Err(FileOpError::Cancelled) => {
            state
                .file_jobs
                .set_status(job_id, FileJobStatus::Canceled, None, true)
                .await?;
            Ok(())
        }
        Err(err) => {
            state
                .file_jobs
                .set_status(job_id, FileJobStatus::Error, Some(err.to_string()), true)
                .await?;
            Err(err)
        }
    }
}

async fn plan_job(state: &AppState, job: &FileJob) -> Result<Vec<PlannedItem>, FileOpError> {
    match job.operation {
        FileOperationKind::Copy | FileOperationKind::Move => plan_copy_move_job(state, job).await,
        FileOperationKind::Delete => plan_delete_job(state, job).await,
    }
}

async fn plan_copy_move_job(
    state: &AppState,
    job: &FileJob,
) -> Result<Vec<PlannedItem>, FileOpError> {
    let source_cap = if job.operation == FileOperationKind::Move {
        roots::RequiredCap::Write
    } else {
        roots::RequiredCap::Read
    };
    let dest = resolve_user_path(
        &state.config,
        &job.owner_user,
        &job.dest_root,
        &job.dest_path,
        roots::RequiredCap::Write,
    )?;
    if !dest.is_dir() {
        return Err(FileOpError::NotDirectory);
    }

    let same_root_atomic_move =
        job.operation == FileOperationKind::Move && job.source_root == job.dest_root;
    let mut items = Vec::new();
    let mut ordinal = 0;

    for source_rel in &job.paths {
        let source = resolve_user_path(
            &state.config,
            &job.owner_user,
            &job.source_root,
            source_rel,
            source_cap,
        )?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(FileOpError::InvalidPath)?;
        let dest_rel = join_rel(&job.dest_path, name);
        let target = dest.join(name);
        if source.is_dir() && target.starts_with(&source) {
            return Err(FileOpError::InvalidPath);
        }

        if same_root_atomic_move {
            let (bytes, _) = tree_size(&source).await?;
            items.push(PlannedItem {
                ordinal,
                kind: if source.is_dir() {
                    PlannedItemKind::Dir
                } else {
                    PlannedItemKind::File
                },
                source_path: source_rel.clone(),
                dest_path: dest_rel,
                size_bytes: bytes,
            });
            ordinal += 1;
            continue;
        }

        append_copy_items(&mut items, &mut ordinal, source_rel, &dest_rel, &source).await?;
    }

    Ok(items)
}

async fn append_copy_items(
    items: &mut Vec<PlannedItem>,
    ordinal: &mut i64,
    source_rel: &str,
    dest_rel: &str,
    source_abs: &Path,
) -> Result<(), FileOpError> {
    let metadata = fs::metadata(source_abs).await.map_err(io_error)?;
    if metadata.is_file() {
        items.push(PlannedItem {
            ordinal: *ordinal,
            kind: PlannedItemKind::File,
            source_path: source_rel.to_string(),
            dest_path: dest_rel.to_string(),
            size_bytes: metadata.len(),
        });
        *ordinal += 1;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(FileOpError::InvalidPath);
    }

    items.push(PlannedItem {
        ordinal: *ordinal,
        kind: PlannedItemKind::Dir,
        source_path: source_rel.to_string(),
        dest_path: dest_rel.to_string(),
        size_bytes: 0,
    });
    *ordinal += 1;

    let mut pending = vec![(
        source_rel.to_string(),
        dest_rel.to_string(),
        source_abs.to_path_buf(),
    )];
    while let Some((src_rel, dst_rel, dir_abs)) = pending.pop() {
        let mut entries = fs::read_dir(&dir_abs).await.map_err(io_error)?;
        while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
            let name = entry
                .file_name()
                .to_str()
                .ok_or(FileOpError::InvalidPath)?
                .to_string();
            let child_src_rel = join_rel(&src_rel, &name);
            let child_dst_rel = join_rel(&dst_rel, &name);
            let meta = entry.metadata().await.map_err(io_error)?;
            if meta.is_dir() {
                items.push(PlannedItem {
                    ordinal: *ordinal,
                    kind: PlannedItemKind::Dir,
                    source_path: child_src_rel.clone(),
                    dest_path: child_dst_rel.clone(),
                    size_bytes: 0,
                });
                *ordinal += 1;
                pending.push((child_src_rel, child_dst_rel, entry.path()));
            } else if meta.is_file() {
                items.push(PlannedItem {
                    ordinal: *ordinal,
                    kind: PlannedItemKind::File,
                    source_path: child_src_rel,
                    dest_path: child_dst_rel,
                    size_bytes: meta.len(),
                });
                *ordinal += 1;
            } else {
                return Err(FileOpError::InvalidPath);
            }
        }
    }

    Ok(())
}

async fn plan_delete_job(state: &AppState, job: &FileJob) -> Result<Vec<PlannedItem>, FileOpError> {
    let mut items = Vec::new();
    let mut ordinal = 0;
    let root_path = roots::resolve_root(
        &state.config,
        &job.owner_user,
        &job.source_root,
        roots::RequiredCap::Write,
    )
    .map_err(|e| FileOpError::Root(e.to_string()))?;

    for rel in &job.paths {
        let source = nasfiles_core::safe_path::resolve(&root_path, rel)
            .map_err(|e| FileOpError::Path(e.to_string()))?;
        if source == root_path {
            return Err(FileOpError::InvalidPath);
        }
        append_delete_items(&mut items, &mut ordinal, rel, &source).await?;
    }

    items.sort_by_key(|item| std::cmp::Reverse(path_depth(&item.source_path)));
    for (index, item) in items.iter_mut().enumerate() {
        item.ordinal = index as i64;
    }
    Ok(items)
}

async fn append_delete_items(
    items: &mut Vec<PlannedItem>,
    ordinal: &mut i64,
    source_rel: &str,
    source_abs: &Path,
) -> Result<(), FileOpError> {
    let metadata = fs::metadata(source_abs).await.map_err(io_error)?;
    if metadata.is_file() {
        items.push(PlannedItem {
            ordinal: *ordinal,
            kind: PlannedItemKind::File,
            source_path: source_rel.to_string(),
            dest_path: String::new(),
            size_bytes: metadata.len(),
        });
        *ordinal += 1;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(FileOpError::InvalidPath);
    }

    let mut pending = vec![(source_rel.to_string(), source_abs.to_path_buf())];
    while let Some((rel, abs)) = pending.pop() {
        let mut entries = fs::read_dir(&abs).await.map_err(io_error)?;
        while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
            let name = entry
                .file_name()
                .to_str()
                .ok_or(FileOpError::InvalidPath)?
                .to_string();
            let child_rel = join_rel(&rel, &name);
            let meta = entry.metadata().await.map_err(io_error)?;
            if meta.is_dir() {
                pending.push((child_rel, entry.path()));
            } else if meta.is_file() {
                items.push(PlannedItem {
                    ordinal: *ordinal,
                    kind: PlannedItemKind::File,
                    source_path: child_rel,
                    dest_path: String::new(),
                    size_bytes: meta.len(),
                });
                *ordinal += 1;
            } else {
                return Err(FileOpError::InvalidPath);
            }
        }
        items.push(PlannedItem {
            ordinal: *ordinal,
            kind: PlannedItemKind::Dir,
            source_path: rel,
            dest_path: String::new(),
            size_bytes: 0,
        });
        *ordinal += 1;
    }

    Ok(())
}

async fn execute_copy_like_job(
    state: &AppState,
    job: &FileJob,
    cleanup_source: bool,
) -> Result<(), FileOpError> {
    let same_root_atomic_move = cleanup_source && job.source_root == job.dest_root;
    let items = state.file_jobs.pending_items(&job.id).await?;

    for item in items {
        ensure_not_cancelled(state, &job.id).await?;
        let source_cap = if cleanup_source {
            roots::RequiredCap::Write
        } else {
            roots::RequiredCap::Read
        };
        let source = resolve_user_path(
            &state.config,
            &job.owner_user,
            &job.source_root,
            &item.source_path,
            source_cap,
        )?;
        let target = resolve_user_path(
            &state.config,
            &job.owner_user,
            &job.dest_root,
            &item.dest_path,
            roots::RequiredCap::Write,
        )?;

        if same_root_atomic_move {
            execute_atomic_move_item(state, job, &item, &source, &target).await?;
            continue;
        }

        match item.kind {
            PlannedItemKind::Dir => {
                ensure_dir_target(&target).await?;
                state.file_jobs.mark_item_done(&item, 0).await?;
            }
            PlannedItemKind::File => {
                copy_file_item(state, job, &item, &source, &target).await?;
            }
        }
    }

    if cleanup_source {
        cleanup_move_sources(state, job).await?;
    }

    Ok(())
}

async fn execute_atomic_move_item(
    state: &AppState,
    job: &FileJob,
    item: &FileJobItem,
    source: &Path,
    target: &Path,
) -> Result<(), FileOpError> {
    if target.exists() {
        if verify_existing_target(item, target).await? && !source.exists() {
            state
                .file_jobs
                .mark_item_done(item, item.size_bytes)
                .await?;
            return Ok(());
        }
        return Err(FileOpError::AlreadyExists);
    }
    if !source.exists() {
        return Err(FileOpError::Path(format!(
            "source not found: {}",
            item.source_path
        )));
    }
    if source.is_dir() && target.starts_with(source) {
        return Err(FileOpError::InvalidPath);
    }
    fs::rename(source, target).await.map_err(io_error)?;
    state
        .file_jobs
        .mark_item_done(item, item.size_bytes)
        .await?;
    state.file_jobs.recompute_progress(&job.id).await
}

async fn copy_file_item(
    state: &AppState,
    job: &FileJob,
    item: &FileJobItem,
    source: &Path,
    target: &Path,
) -> Result<(), FileOpError> {
    if target.exists() {
        if verify_existing_target(item, target).await? {
            state
                .file_jobs
                .mark_item_done(item, item.size_bytes)
                .await?;
            return Ok(());
        }
        return Err(FileOpError::AlreadyExists);
    }
    if !source.exists() {
        return Err(FileOpError::Path(format!(
            "source not found: {}",
            item.source_path
        )));
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await.map_err(io_error)?;
    }
    remove_temp_for_final(&job.id, &item.dest_path, target).await?;
    let temp_path = temp_path_for_final(&job.id, &item.dest_path, target)?;

    let mut src = fs::File::open(source).await.map_err(io_error)?;
    let mut dst = fs::File::create(&temp_path).await.map_err(io_error)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut written = 0u64;
    loop {
        ensure_not_cancelled(state, &job.id).await?;
        let read = src.read(&mut buffer).await.map_err(io_error)?;
        if read == 0 {
            break;
        }
        dst.write_all(&buffer[..read]).await.map_err(io_error)?;
        written += read as u64;
        state.file_jobs.update_item_bytes(item, written).await?;
        demo_transfer_delay().await;
    }
    dst.flush().await.map_err(io_error)?;
    fs::rename(&temp_path, target).await.map_err(io_error)?;
    state.file_jobs.mark_item_done(item, item.size_bytes).await
}

async fn execute_delete_job(state: &AppState, job: &FileJob) -> Result<(), FileOpError> {
    let items = state.file_jobs.pending_items(&job.id).await?;
    for item in items {
        ensure_not_cancelled(state, &job.id).await?;
        let source = resolve_user_path(
            &state.config,
            &job.owner_user,
            &job.source_root,
            &item.source_path,
            roots::RequiredCap::Write,
        )?;
        if !source.exists() {
            state
                .file_jobs
                .mark_item_done(&item, item.size_bytes)
                .await?;
            continue;
        }
        match item.kind {
            PlannedItemKind::File => fs::remove_file(&source).await.map_err(io_error)?,
            PlannedItemKind::Dir => fs::remove_dir(&source).await.map_err(io_error)?,
        }
        state
            .file_jobs
            .mark_item_done(&item, item.size_bytes)
            .await?;
        demo_transfer_delay().await;
    }
    Ok(())
}

async fn cleanup_move_sources(state: &AppState, job: &FileJob) -> Result<(), FileOpError> {
    for source_rel in &job.paths {
        ensure_not_cancelled(state, &job.id).await?;
        let source = resolve_user_path(
            &state.config,
            &job.owner_user,
            &job.source_root,
            source_rel,
            roots::RequiredCap::Write,
        )?;
        if !source.exists() {
            continue;
        }
        if source.is_dir() {
            fs::remove_dir_all(&source).await.map_err(io_error)?;
        } else {
            fs::remove_file(&source).await.map_err(io_error)?;
        }
    }
    Ok(())
}

async fn ensure_not_cancelled(state: &AppState, job_id: &str) -> Result<(), FileOpError> {
    if state.file_jobs.is_cancel_requested(job_id).await? {
        return Err(FileOpError::Cancelled);
    }
    Ok(())
}

async fn ensure_dir_target(target: &Path) -> Result<(), FileOpError> {
    if target.exists() {
        if target.is_dir() {
            return Ok(());
        }
        return Err(FileOpError::AlreadyExists);
    }
    fs::create_dir(target).await.map_err(io_error)
}

async fn verify_existing_target(item: &FileJobItem, target: &Path) -> Result<bool, FileOpError> {
    let Ok(metadata) = fs::metadata(target).await else {
        return Ok(false);
    };
    match item.kind {
        PlannedItemKind::Dir => Ok(metadata.is_dir()),
        PlannedItemKind::File => Ok(metadata.is_file() && metadata.len() == item.size_bytes),
    }
}

async fn remove_temp_for_final(
    job_id: &str,
    dest_path: &str,
    target: &Path,
) -> Result<(), FileOpError> {
    let temp_path = temp_path_for_final(job_id, dest_path, target)?;
    if temp_path.exists() {
        fs::remove_file(temp_path).await.map_err(io_error)?;
    }
    Ok(())
}

fn temp_path_for_final(
    job_id: &str,
    dest_path: &str,
    target: &Path,
) -> Result<PathBuf, FileOpError> {
    let parent = target.parent().ok_or(FileOpError::InvalidPath)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(FileOpError::InvalidPath)?;
    let suffix = short_hash(dest_path);
    Ok(parent.join(format!(".nasfiles-op-{job_id}-{suffix}-{name}.part")))
}

fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(value.as_bytes());
    hex::encode(&hash[..6])
}

fn resolve_user_path(
    config: &crate::config::AppConfig,
    user: &AuthUser,
    root_key: &str,
    relative_path: &str,
    cap: roots::RequiredCap,
) -> Result<PathBuf, FileOpError> {
    let root = roots::resolve_root(config, user, root_key, cap)
        .map_err(|e| FileOpError::Root(e.to_string()))?;
    nasfiles_core::safe_path::resolve(&root, relative_path)
        .map_err(|e| FileOpError::Path(e.to_string()))
}

async fn tree_size(source: &Path) -> Result<(u64, u64), FileOpError> {
    let metadata = fs::metadata(source).await.map_err(io_error)?;
    if metadata.is_file() {
        return Ok((metadata.len(), 1));
    }
    if !metadata.is_dir() {
        return Err(FileOpError::InvalidPath);
    }
    let mut bytes = 0;
    let mut entries = 1;
    let mut pending = vec![source.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut children = fs::read_dir(dir).await.map_err(io_error)?;
        while let Some(entry) = children.next_entry().await.map_err(io_error)? {
            let meta = entry.metadata().await.map_err(io_error)?;
            entries += 1;
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
            } else {
                return Err(FileOpError::InvalidPath);
            }
        }
    }
    Ok((bytes, entries))
}

async fn demo_transfer_delay() {
    static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
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

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

fn path_depth(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}

fn io_error(error: std::io::Error) -> FileOpError {
    tracing::error!("file job I/O error: {error}");
    FileOpError::Io(error.to_string())
}

fn db_error(error: sqlx::Error) -> FileOpError {
    tracing::error!("file job database error: {error}");
    FileOpError::Io(error.to_string())
}
