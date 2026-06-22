-- Internal IP blocklist. Populated automatically (e.g. when a client attempts a
-- root SFTP login) and managed by admins from the dashboard. Blocked IPs are
-- denied SFTP authentication regardless of credentials.
CREATE TABLE IF NOT EXISTS ip_blocklist (
    ip            TEXT PRIMARY KEY,
    reason        TEXT,
    blocked_at    BIGINT NOT NULL,
    last_seen_at  BIGINT NOT NULL,
    hit_count     BIGINT NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS ip_blocklist_blocked_at_idx ON ip_blocklist(blocked_at);
