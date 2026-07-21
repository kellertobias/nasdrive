use std::collections::HashMap;

use nasfiles_core::models::FolderCaps;
use sqlx::AnyPool;

// @tour comment Automatic revocation is reversible; manual is not
// The restore path only clears `revoked_at` where `revoke_reason` is one of the automatic
// values and the original expiry is still in the future — so a manually revoked share stays
// dead forever.
//
// The second half treats a successful login as authoritative and revokes active shares for
// any root absent from the permission snapshot. The nightly audit drives the same path, but
// only after `permission_grace::confirm_permission_loss` agrees, and it deliberately skips
// revocation on network errors.

/// Apply an authoritative, user-level permission snapshot to the user's shares.
///
/// Only shares that were automatically revoked for permission loss are restored;
/// manual revocations and already-expired shares remain untouched.
pub async fn reconcile_authoritative_permissions(
    pool: &AnyPool,
    user_id: &str,
    folder_permissions: &HashMap<String, FolderCaps>,
    source: &str,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp_millis();

    for (root_key, caps) in folder_permissions {
        if !caps.read && !caps.share {
            continue;
        }

        super::permission_grace::clear_permission_loss_grace(pool, user_id, root_key).await;

        let restored = sqlx::query(
            "UPDATE shares
             SET revoked_at = NULL, revoke_reason = NULL, revoke_source = NULL
             WHERE owner_user_id = $1 AND root_key = $2
               AND revoked_at IS NOT NULL
               AND revoke_reason IN (
                   'lost_permission', 'refresh_token_missing',
                   'refresh_token_invalid', 'user_not_found_in_idp'
               )
               AND (expires_at IS NULL OR expires_at > $3)",
        )
        .bind(user_id)
        .bind(root_key)
        .bind(now)
        .execute(pool)
        .await?;

        if restored.rows_affected() > 0 {
            tracing::info!(
                user_id,
                root_key,
                source,
                restored = restored.rows_affected(),
                "Restored auto-revoked shares after access was regained"
            );
        }
    }

    // A successful interactive login is authoritative for every configured
    // root. Revoke active shares for roots absent from that permission snapshot.
    let active_roots: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT root_key FROM shares WHERE owner_user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    for (root_key,) in active_roots {
        let caps = folder_permissions
            .get(&root_key)
            .copied()
            .unwrap_or_default();
        if caps.read || caps.share {
            continue;
        }

        let revoked = sqlx::query(
            "UPDATE shares SET revoked_at = $1, revoke_reason = 'lost_permission', revoke_source = $2
             WHERE owner_user_id = $3 AND root_key = $4 AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(source)
        .bind(user_id)
        .bind(&root_key)
        .execute(pool)
        .await?;

        if revoked.rows_affected() > 0 {
            tracing::info!(
                user_id,
                root_key,
                source,
                revoked = revoked.rows_affected(),
                "Auto-revoked shares after authoritative permission loss"
            );
        }
    }

    Ok(())
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
            .expect("connect sqlite");
        sqlx::query(
            "CREATE TABLE shares (
                id TEXT PRIMARY KEY, owner_user_id TEXT NOT NULL, root_key TEXT NOT NULL,
                expires_at BIGINT, revoked_at BIGINT, revoke_reason TEXT, revoke_source TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create shares");
        sqlx::query(
            "CREATE TABLE permission_loss_grace (
                user_id TEXT NOT NULL, root_key TEXT NOT NULL, first_seen_at BIGINT NOT NULL,
                PRIMARY KEY (user_id, root_key)
            )",
        )
        .execute(&pool)
        .await
        .expect("create grace");
        pool
    }

    async fn insert_share(
        pool: &AnyPool,
        id: &str,
        root: &str,
        expires_at: Option<i64>,
        revoked_at: Option<i64>,
        reason: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO shares (id, owner_user_id, root_key, expires_at, revoked_at, revoke_reason)
             VALUES ($1, 'user-1', $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(root)
        .bind(expires_at)
        .bind(revoked_at)
        .bind(reason)
        .execute(pool)
        .await
        .expect("insert share");
    }

    fn readable(root: &str) -> HashMap<String, FolderCaps> {
        HashMap::from([(
            root.to_string(),
            FolderCaps {
                read: true,
                write: false,
                share: true,
            },
        )])
    }

    #[tokio::test]
    async fn restores_only_unexpired_permission_revocations() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();
        insert_share(
            &pool,
            "restore",
            "docs",
            Some(now + 60_000),
            Some(now),
            Some("lost_permission"),
        )
        .await;
        insert_share(
            &pool,
            "expired",
            "docs",
            Some(now - 1),
            Some(now),
            Some("lost_permission"),
        )
        .await;
        insert_share(
            &pool,
            "manual",
            "docs",
            Some(now + 60_000),
            Some(now),
            Some("manual"),
        )
        .await;
        insert_share(
            &pool,
            "old-session-bug",
            "docs",
            Some(now + 60_000),
            Some(now),
            Some("refresh_token_invalid"),
        )
        .await;

        reconcile_authoritative_permissions(&pool, "user-1", &readable("docs"), "test")
            .await
            .expect("reconcile");

        let rows: Vec<(String, Option<i64>)> =
            sqlx::query_as("SELECT id, revoked_at FROM shares ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("expired".into(), Some(now)),
                ("manual".into(), Some(now)),
                ("old-session-bug".into(), None),
                ("restore".into(), None)
            ]
        );
    }

    #[tokio::test]
    async fn authoritative_snapshot_revokes_only_missing_roots() {
        let pool = test_pool().await;
        insert_share(&pool, "kept", "docs", None, None, None).await;
        insert_share(&pool, "revoked", "photos", None, None, None).await;

        reconcile_authoritative_permissions(&pool, "user-1", &readable("docs"), "test")
            .await
            .expect("reconcile");

        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT id, revoke_reason FROM shares ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("kept".into(), None),
                ("revoked".into(), Some("lost_permission".into()))
            ]
        );
    }
}
