-- Records why/how a share was revoked, so admins can distinguish an owner's
-- explicit revoke from an automatic one triggered by permission loss.
ALTER TABLE shares ADD COLUMN revoke_reason TEXT;
ALTER TABLE shares ADD COLUMN revoke_source TEXT;

-- Tracks a single observed permission loss per (user, root) before it is
-- acted on. The first observation is recorded but not revoked; only if the
-- loss is still present on a later check (login refresh or the nightly
-- audit) is it treated as confirmed and the share revoked. This absorbs a
-- single transient/incomplete IdP response instead of revoking on it.
CREATE TABLE IF NOT EXISTS permission_loss_grace (
    user_id       TEXT NOT NULL,
    root_key      TEXT NOT NULL,
    first_seen_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, root_key)
);
