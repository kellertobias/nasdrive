-- User-generated API tokens for S3 access.
-- secret_key cannot be hashed (SigV4 verification re-computes HMAC(secret, ...),
-- which a one-way hash can't do), so it is encrypted at rest with AES-256-GCM
-- (see crypto.rs) and only decrypted in-memory to verify a signature.
CREATE TABLE IF NOT EXISTS user_api_tokens (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id),
    label        TEXT NOT NULL,
    access_key   TEXT NOT NULL UNIQUE,
    secret_key   TEXT NOT NULL,
    created_at   BIGINT NOT NULL,
    expires_at   BIGINT,
    last_used_at BIGINT,
    revoked_at   BIGINT
);

CREATE INDEX IF NOT EXISTS user_api_tokens_user_idx ON user_api_tokens(user_id);
CREATE INDEX IF NOT EXISTS user_api_tokens_access_key_idx ON user_api_tokens(access_key);

-- Short-lived credentials issued to share recipients (exchange share token +
-- optional password → temporary access_key + secret_key pair).
CREATE TABLE IF NOT EXISTS s3_share_credentials (
    id           TEXT PRIMARY KEY,
    share_id     TEXT NOT NULL REFERENCES shares(id),
    access_key   TEXT NOT NULL UNIQUE,
    secret_key   TEXT NOT NULL,
    created_at   BIGINT NOT NULL,
    expires_at   BIGINT NOT NULL,
    last_used_at BIGINT
);

CREATE INDEX IF NOT EXISTS s3_share_creds_access_key_idx ON s3_share_credentials(access_key);
CREATE INDEX IF NOT EXISTS s3_share_creds_expires_idx ON s3_share_credentials(expires_at);

-- In-progress multipart uploads.
-- Parts are stored as files under {data_dir}/s3-parts/{upload_id}/part-{n}.
CREATE TABLE IF NOT EXISTS s3_multipart_uploads (
    upload_id  TEXT PRIMARY KEY,
    bucket     TEXT NOT NULL,
    key        TEXT NOT NULL,
    principal  TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    part_count INTEGER NOT NULL DEFAULT 0
);
