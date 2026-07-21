use sqlx::AnyPool;

// @tour comment Permission loss needs two consecutive sightings
// During SSO group refresh, shares are not revoked the first time a capability disappears —
// a single truncated or transient identity-provider response would otherwise destroy every
// share on that root. The first sighting inserts a `permission_loss_grace` row and returns
// `false`; the second returns `true` and clears it.
//
// Correspondingly, `refresh.rs` clears grace rows for roots that still grant `share` or
// `read`, so a stale first observation cannot confirm an unrelated later loss. See
// [permission grace](glossary:permission-grace).

/// Call when a share owner appears to have lost `share`/`read` capability on
/// a root. Returns `true` once the loss is confirmed and the caller should
/// revoke; returns `false` the first time it's observed, so a single
/// transient or incomplete IdP response doesn't immediately revoke a share.
///
/// Confirmation happens on the *next* check that still sees the loss —
/// whether that's the owner's next login/session refresh or the next
/// nightly audit run.
pub async fn confirm_permission_loss(pool: &AnyPool, user_id: &str, root_key: &str) -> bool {
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT first_seen_at FROM permission_loss_grace WHERE user_id = $1 AND root_key = $2",
    )
    .bind(user_id)
    .bind(root_key)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);

    if existing.is_some() {
        clear_permission_loss_grace(pool, user_id, root_key).await;
        return true;
    }

    let now = chrono::Utc::now().timestamp_millis();
    let _ = sqlx::query(
        "INSERT INTO permission_loss_grace (user_id, root_key, first_seen_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(root_key)
    .bind(now)
    .execute(pool)
    .await;
    false
}

/// Call when a share owner's capability on a root is confirmed intact, so a
/// stale grace record from an earlier transient loss doesn't linger and
/// falsely confirm a later, unrelated loss.
pub async fn clear_permission_loss_grace(pool: &AnyPool, user_id: &str, root_key: &str) {
    let _ = sqlx::query(
        "DELETE FROM permission_loss_grace WHERE user_id = $1 AND root_key = $2",
    )
    .bind(user_id)
    .bind(root_key)
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::any::AnyPoolOptions;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE permission_loss_grace (
                user_id TEXT NOT NULL,
                root_key TEXT NOT NULL,
                first_seen_at BIGINT NOT NULL,
                PRIMARY KEY (user_id, root_key)
            )",
        )
        .execute(&pool)
        .await
        .expect("create table");
        pool
    }

    #[tokio::test]
    async fn first_observation_is_deferred_not_confirmed() {
        let pool = test_pool().await;
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);
    }

    #[tokio::test]
    async fn second_consecutive_observation_confirms_and_resets() {
        let pool = test_pool().await;
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);
        assert!(confirm_permission_loss(&pool, "user1", "Documents").await);

        // Confirming clears the record, so a later loss starts a fresh grace period.
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);
    }

    #[tokio::test]
    async fn clearing_after_a_first_observation_resets_the_grace_period() {
        let pool = test_pool().await;
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);

        // Capability was restored before the next check.
        clear_permission_loss_grace(&pool, "user1", "Documents").await;

        // The next loss is treated as a brand new first observation.
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);
    }

    #[tokio::test]
    async fn grace_records_are_scoped_per_root_key() {
        let pool = test_pool().await;
        assert!(!confirm_permission_loss(&pool, "user1", "Documents").await);
        // A different root for the same user has its own independent grace period.
        assert!(!confirm_permission_loss(&pool, "user1", "Photos").await);
    }
}
