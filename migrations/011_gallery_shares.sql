-- Adds explicit share types and gallery proofing state.
ALTER TABLE shares ADD COLUMN share_type TEXT NOT NULL DEFAULT 'typical';

CREATE TABLE IF NOT EXISTS gallery_preparation_jobs (
    id                TEXT PRIMARY KEY,
    share_id          TEXT NOT NULL REFERENCES shares(id),
    owner_user_id     TEXT NOT NULL REFERENCES users(id),
    status            TEXT NOT NULL,
    total_items       INTEGER NOT NULL DEFAULT 0,
    processed_items   INTEGER NOT NULL DEFAULT 0,
    error             TEXT,
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL,
    finished_at       BIGINT
);

CREATE INDEX IF NOT EXISTS gallery_jobs_owner_idx ON gallery_preparation_jobs(owner_user_id, created_at);
CREATE INDEX IF NOT EXISTS gallery_jobs_share_idx ON gallery_preparation_jobs(share_id);

CREATE TABLE IF NOT EXISTS share_gallery_items (
    id                TEXT PRIMARY KEY,
    share_id          TEXT NOT NULL REFERENCES shares(id),
    relative_path     TEXT NOT NULL,
    filename          TEXT NOT NULL,
    sequence          INTEGER NOT NULL,
    source_mtime_ms   BIGINT NOT NULL,
    source_size       BIGINT NOT NULL,
    width             INTEGER,
    height            INTEGER,
    captured_at       BIGINT,
    mime_type         TEXT,
    thumbnail_ready   BOOLEAN NOT NULL DEFAULT FALSE,
    preview_ready     BOOLEAN NOT NULL DEFAULT FALSE,
    error             TEXT,
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL,
    UNIQUE(share_id, relative_path)
);

CREATE INDEX IF NOT EXISTS gallery_items_share_idx ON share_gallery_items(share_id, sequence);

CREATE TABLE IF NOT EXISTS share_gallery_feedback (
    share_id          TEXT NOT NULL REFERENCES shares(id),
    item_id           TEXT NOT NULL REFERENCES share_gallery_items(id),
    marked            BOOLEAN NOT NULL DEFAULT FALSE,
    note              TEXT,
    updated_at        BIGINT NOT NULL,
    PRIMARY KEY (share_id, item_id)
);
