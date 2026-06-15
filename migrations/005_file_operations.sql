-- Durable resumable file operation jobs.
-- Compatible with both SQLite and PostgreSQL.

CREATE TABLE IF NOT EXISTS file_operation_jobs (
    id                  TEXT PRIMARY KEY,
    owner_user_id       TEXT NOT NULL REFERENCES users(id),
    owner_user_json     TEXT NOT NULL DEFAULT '{}',
    operation           TEXT NOT NULL,
    source_root         TEXT NOT NULL,
    dest_root           TEXT NOT NULL,
    dest_path           TEXT NOT NULL,
    paths_json          TEXT NOT NULL,
    status              TEXT NOT NULL,
    total_bytes         BIGINT NOT NULL DEFAULT 0,
    transferred_bytes   BIGINT NOT NULL DEFAULT 0,
    total_entries       BIGINT NOT NULL DEFAULT 0,
    completed_entries   BIGINT NOT NULL DEFAULT 0,
    error               TEXT,
    cancel_requested    BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          BIGINT NOT NULL,
    updated_at          BIGINT NOT NULL,
    finished_at         BIGINT
);

CREATE INDEX IF NOT EXISTS file_operation_jobs_owner_idx ON file_operation_jobs(owner_user_id, created_at);
CREATE INDEX IF NOT EXISTS file_operation_jobs_status_idx ON file_operation_jobs(status);

CREATE TABLE IF NOT EXISTS file_operation_items (
    id              TEXT PRIMARY KEY,
    job_id          TEXT NOT NULL REFERENCES file_operation_jobs(id),
    ordinal         BIGINT NOT NULL,
    kind            TEXT NOT NULL,
    source_path     TEXT NOT NULL,
    dest_path       TEXT NOT NULL,
    size_bytes      BIGINT NOT NULL DEFAULT 0,
    status          TEXT NOT NULL,
    bytes_done      BIGINT NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL,
    finished_at     BIGINT
);

CREATE INDEX IF NOT EXISTS file_operation_items_job_idx ON file_operation_items(job_id, ordinal);
CREATE INDEX IF NOT EXISTS file_operation_items_status_idx ON file_operation_items(job_id, status);
