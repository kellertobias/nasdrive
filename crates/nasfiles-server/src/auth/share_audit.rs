#![allow(clippy::collapsible_if)]
use crate::config;
use crate::state::AppState;
use openidconnect::{OAuth2TokenResponse, RefreshToken};
use serde_json::Value;

/// How long a stale share is kept around (for the admin history view) before
/// it's deleted for good.
const STALE_SHARE_GRACE_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const ACCESS_LOG_RETENTION_MS: i64 = 56 * 24 * 60 * 60 * 1000;

pub async fn run_retention_cleanup(pool: &sqlx::AnyPool) {
    cleanup_stale_shares(pool).await;
    cleanup_stale_access_logs(pool).await;
}

pub async fn run_startup_retention_migration(pool: &sqlx::AnyPool) {
    cleanup_startup_stale_shares(pool).await;
    cleanup_stale_access_logs(pool).await;
}

pub fn spawn_daily_retention_cleanup(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        interval.tick().await;

        loop {
            interval.tick().await;
            run_retention_cleanup(&state.pool).await;
        }
    })
}

pub fn spawn_daily_share_audit(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Sleep for 24h initially
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        interval.tick().await; // First tick fires immediately

        loop {
            interval.tick().await;

            tracing::info!("Starting daily share audit scan");

            // Query distinct users with active shares
            let user_ids_res: Result<Vec<(String,)>, sqlx::Error> = sqlx::query_as(
                "SELECT DISTINCT owner_user_id FROM shares WHERE revoked_at IS NULL",
            )
            .fetch_all(&state.pool)
            .await;

            let user_ids = match user_ids_res {
                Ok(u) => u.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
                Err(e) => {
                    tracing::error!("Failed to fetch active share owners: {}", e);
                    continue;
                }
            };

            if user_ids.is_empty() {
                tracing::info!("No active shares to audit");
                continue;
            }

            let oidc_state = match super::oidc::OIDC_CLIENT.get() {
                Some(s) => s,
                None => {
                    tracing::warn!("OIDC not configured, skipping daily share audit");
                    continue;
                }
            };

            let http_client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();

            for user_id in user_ids {
                // Fetch user data
                let user_row: Result<Option<(String, Option<String>)>, sqlx::Error> =
                    sqlx::query_as(
                        "SELECT external_id, oidc_refresh_token FROM users WHERE id = $1",
                    )
                    .bind(&user_id)
                    .fetch_optional(&state.pool)
                    .await;

                let (_external_id, enc_refresh_token) = match user_row {
                    Ok(Some((ext, Some(enc_rt)))) => (ext, enc_rt),
                    _ => {
                        // A missing stored credential only means we cannot run
                        // the audit. It says nothing about the user's access.
                        tracing::warn!(
                            "User {} missing or has no refresh token; skipping share audit",
                            user_id
                        );
                        continue;
                    }
                };

                let refresh_token = match super::oidc::decrypt_token(
                    &enc_refresh_token,
                    &state.config.session_secret,
                ) {
                    Ok(rt) => rt,
                    Err(_) => {
                        tracing::warn!(
                            "Failed to decrypt refresh token for user {}; skipping share audit",
                            user_id
                        );
                        continue;
                    }
                };

                // Exchange refresh token
                let rt = RefreshToken::new(refresh_token);
                let token_req = match oidc_state.client.exchange_refresh_token(&rt) {
                    Ok(req) => req,
                    Err(_) => {
                        tracing::warn!(
                            "Refresh token exchange could not be started for user {}; skipping share audit",
                            user_id
                        );
                        continue;
                    }
                };
                let token_resp = token_req.request_async(&http_client).await;

                let (access_token, _new_enc_refresh) = match token_resp {
                    Ok(resp) => {
                        let at = resp.access_token().secret().clone();
                        let new_rt = resp.refresh_token().map(|t| t.secret().clone());
                        let enc_rt = new_rt.as_deref().and_then(|t| {
                            super::oidc::encrypt_token(t, &state.config.session_secret).ok()
                        });
                        let enc_at = super::oidc::encrypt_token(&at, &state.config.session_secret)
                            .unwrap_or_default();

                        // Update DB
                        let _ = sqlx::query("UPDATE users SET oidc_access_token = $1, oidc_refresh_token = COALESCE($2, oidc_refresh_token) WHERE id = $3")
                            .bind(&enc_at)
                            .bind(&enc_rt)
                            .bind(&user_id)
                            .execute(&state.pool)
                            .await;

                        (at, enc_rt)
                    }
                    Err(_) => {
                        tracing::warn!(
                            "Refresh token exchange failed for user {}; clearing stale credentials and skipping share audit",
                            user_id
                        );
                        let _ = sqlx::query("UPDATE users SET oidc_access_token = NULL, oidc_refresh_token = NULL WHERE id = $1")
                            .bind(&user_id)
                            .execute(&state.pool)
                            .await;
                        continue;
                    }
                };

                // Get userinfo
                let resp = match http_client
                    .get(oidc_state.userinfo_url.clone())
                    .bearer_auth(&access_token)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            "Daily share audit skipped for user {} due to network error: {}",
                            user_id,
                            e
                        );
                        continue; // Skip, don't revoke
                    }
                };

                if resp.status() == 404 {
                    tracing::warn!("User {} not found in IdP, revoking active shares", user_id);
                    let empty_permissions = std::collections::HashMap::new();
                    if let Err(e) = super::share_reconcile::reconcile_authoritative_permissions(
                        &state.pool,
                        &user_id,
                        &empty_permissions,
                        "daily_audit_user_not_found",
                    )
                    .await
                    {
                        tracing::error!(
                            "Failed to reconcile shares for missing IdP user {}: {}",
                            user_id,
                            e
                        );
                    }
                    continue;
                } else if !resp.status().is_success() {
                    tracing::warn!(
                        "IdP returned {} for user {}, skipping",
                        resp.status(),
                        user_id
                    );
                    continue; // Skip, don't revoke
                }

                let userinfo_json = match resp.json::<Value>().await {
                    Ok(json) => json,
                    Err(_) => {
                        tracing::warn!("Failed to parse userinfo for user {}, skipping", user_id);
                        continue;
                    }
                };

                let groups = super::oidc::extract_claim_array(
                    &userinfo_json,
                    &state.config.sso_groups_claim,
                )
                .unwrap_or_default();

                let folder_permissions = config::compute_folder_permissions(&state.config, &groups);

                // Find shares where the user no longer has capability
                let active_shares: Result<Vec<(String, String)>, sqlx::Error> = sqlx::query_as(
                    "SELECT id, root_key FROM shares WHERE owner_user_id = $1 AND revoked_at IS NULL"
                )
                .bind(&user_id)
                .fetch_all(&state.pool)
                .await;

                if let Ok(shares) = active_shares {
                    let now = chrono::Utc::now().timestamp_millis();

                    // Group shares by root_key so each root's grace check runs once.
                    let mut roots: std::collections::HashMap<String, Vec<String>> =
                        std::collections::HashMap::new();
                    for (share_id, root_key) in shares {
                        roots.entry(root_key).or_default().push(share_id);
                    }

                    for (root_key, share_ids) in roots {
                        let caps = folder_permissions
                            .get(&root_key)
                            .copied()
                            .unwrap_or_default();

                        if caps.share || caps.read {
                            super::permission_grace::clear_permission_loss_grace(
                                &state.pool,
                                &user_id,
                                &root_key,
                            )
                            .await;
                            continue;
                        }

                        if !super::permission_grace::confirm_permission_loss(
                            &state.pool,
                            &user_id,
                            &root_key,
                        )
                        .await
                        {
                            tracing::info!(
                                "Permission loss observed for user {} on root_key {} during daily audit, deferring revoke pending confirmation",
                                user_id,
                                root_key
                            );
                            continue;
                        }

                        for share_id in share_ids {
                            let _ = sqlx::query(
                                "UPDATE shares SET revoked_at = $1, revoke_reason = $2, revoke_source = $3 WHERE id = $4"
                            )
                            .bind(now)
                            .bind("lost_permission")
                            .bind("daily_audit")
                            .bind(&share_id)
                            .execute(&state.pool)
                            .await;

                            tracing::info!(
                                "Revoked share {} for user {} (confirmed lost access to root_key {})",
                                share_id,
                                user_id,
                                root_key
                            );
                        }
                    }
                }
            }

            tracing::info!("Daily share audit scan complete");
        }
    })
}

/// Deletes shares that have been stale for more than [`STALE_SHARE_GRACE_MS`]:
/// - expired more than two weeks ago (whether or not it was also revoked), or
/// - revoked more than two weeks ago with no expiration date ever set.
///
/// A revoked share whose original expiry is still in the future is kept —
/// its revocation is recent history, not clutter — until that expiry itself
/// passes the two-week grace period.
async fn cleanup_stale_shares(pool: &sqlx::AnyPool) {
    let cutoff = chrono::Utc::now().timestamp_millis() - STALE_SHARE_GRACE_MS;

    let stale_ids: Result<Vec<(String,)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT id FROM shares
        WHERE (expires_at IS NOT NULL AND expires_at < $1)
           OR (revoked_at IS NOT NULL AND expires_at IS NULL AND revoked_at < $1)
        "#,
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await;

    let stale_ids = match stale_ids {
        Ok(rows) => rows.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!("Failed to query stale shares for cleanup: {}", e);
            return;
        }
    };

    delete_share_ids(
        pool,
        &stale_ids,
        "expired or revoked-with-no-expiry for more than 2 weeks",
    )
    .await;
}

async fn cleanup_startup_stale_shares(pool: &sqlx::AnyPool) {
    let cutoff = chrono::Utc::now().timestamp_millis() - STALE_SHARE_GRACE_MS;

    let stale_ids: Result<Vec<(String,)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT s.id FROM shares s
        WHERE (s.expires_at IS NOT NULL AND s.expires_at < $1)
           OR (
                s.expires_at IS NULL
                AND s.revoked_at IS NOT NULL
                AND COALESCE(
                    (SELECT MAX(sal.occurred_at) FROM share_access_log sal WHERE sal.share_id = s.id),
                    s.revoked_at
                ) < $1
           )
        "#,
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await;

    let stale_ids = match stale_ids {
        Ok(rows) => rows.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!("Failed to query startup stale shares for cleanup: {}", e);
            return;
        }
    };

    delete_share_ids(pool, &stale_ids, "startup retention migration").await;
}

async fn delete_share_ids(pool: &sqlx::AnyPool, stale_ids: &[String], reason: &str) {
    if stale_ids.is_empty() {
        return;
    }

    for share_id in stale_ids {
        let _ = sqlx::query("DELETE FROM share_access_log WHERE share_id = $1")
            .bind(share_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM s3_share_credentials WHERE share_id = $1")
            .bind(share_id)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM shares WHERE id = $1")
            .bind(share_id)
            .execute(pool)
            .await;
    }

    tracing::info!("Cleaned up {} stale share(s) ({})", stale_ids.len(), reason);
}

async fn cleanup_stale_access_logs(pool: &sqlx::AnyPool) {
    let cutoff = chrono::Utc::now().timestamp_millis() - ACCESS_LOG_RETENTION_MS;

    match sqlx::query(
        r#"
        DELETE FROM share_access_log
        WHERE occurred_at < $1
           OR NOT EXISTS (SELECT 1 FROM shares s WHERE s.id = share_access_log.share_id)
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            tracing::info!(
                "Cleaned up {} stale share access log row(s)",
                result.rows_affected()
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to clean share access logs: {}", e),
    }

    match sqlx::query("DELETE FROM sftp_access_log WHERE occurred_at < $1")
        .bind(cutoff)
        .execute(pool)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            tracing::info!(
                "Cleaned up {} stale SFTP access log row(s)",
                result.rows_affected()
            );
        }
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to clean SFTP access logs: {}", e),
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use sqlx::AnyPool;
    use sqlx::any::AnyPoolOptions;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE shares (
                id TEXT PRIMARY KEY,
                expires_at BIGINT,
                revoked_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .expect("create shares table");
        sqlx::query(
            "CREATE TABLE share_access_log (
                id TEXT,
                share_id TEXT NOT NULL,
                occurred_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .expect("create share_access_log table");
        sqlx::query(
            "CREATE TABLE sftp_access_log (
                id TEXT,
                occurred_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .expect("create sftp_access_log table");
        sqlx::query("CREATE TABLE s3_share_credentials (share_id TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create s3_share_credentials table");
        pool
    }

    async fn insert_share(
        pool: &AnyPool,
        id: &str,
        expires_at: Option<i64>,
        revoked_at: Option<i64>,
    ) {
        sqlx::query("INSERT INTO shares (id, expires_at, revoked_at) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(expires_at)
            .bind(revoked_at)
            .execute(pool)
            .await
            .expect("insert share");
    }

    async fn remaining_ids(pool: &AnyPool) -> Vec<String> {
        sqlx::query_as::<_, (String,)>("SELECT id FROM shares ORDER BY id")
            .fetch_all(pool)
            .await
            .expect("select shares")
            .into_iter()
            .map(|(id,)| id)
            .collect()
    }

    async fn insert_access_log(pool: &AnyPool, id: &str, share_id: &str, occurred_at: i64) {
        sqlx::query("INSERT INTO share_access_log (id, share_id, occurred_at) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(share_id)
            .bind(occurred_at)
            .execute(pool)
            .await
            .expect("insert access log row");
    }

    #[tokio::test]
    async fn removes_shares_expired_more_than_two_weeks_ago_regardless_of_revocation() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(
            &pool,
            "expired-long-ago-not-revoked",
            Some(now - 15 * DAY_MS),
            None,
        )
        .await;
        insert_share(
            &pool,
            "expired-long-ago-and-revoked",
            Some(now - 20 * DAY_MS),
            Some(now - DAY_MS),
        )
        .await;
        insert_share(&pool, "expired-recently", Some(now - 3 * DAY_MS), None).await;

        cleanup_stale_shares(&pool).await;

        assert_eq!(remaining_ids(&pool).await, vec!["expired-recently"]);
    }

    #[tokio::test]
    async fn removes_shares_revoked_more_than_two_weeks_ago_with_no_expiry() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(
            &pool,
            "revoked-long-ago-no-expiry",
            None,
            Some(now - 15 * DAY_MS),
        )
        .await;
        insert_share(
            &pool,
            "revoked-recently-no-expiry",
            None,
            Some(now - 2 * DAY_MS),
        )
        .await;

        cleanup_stale_shares(&pool).await;

        assert_eq!(
            remaining_ids(&pool).await,
            vec!["revoked-recently-no-expiry"]
        );
    }

    #[tokio::test]
    async fn keeps_revoked_shares_whose_original_expiry_is_still_in_the_future() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(
            &pool,
            "revoked-but-not-yet-expired",
            Some(now + 10 * DAY_MS),
            Some(now - DAY_MS),
        )
        .await;

        cleanup_stale_shares(&pool).await;

        assert_eq!(
            remaining_ids(&pool).await,
            vec!["revoked-but-not-yet-expired"]
        );
    }

    #[tokio::test]
    async fn deleting_a_share_also_deletes_its_dependent_rows() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(&pool, "stale", Some(now - 30 * DAY_MS), None).await;
        sqlx::query("INSERT INTO share_access_log (share_id) VALUES ($1)")
            .bind("stale")
            .execute(&pool)
            .await
            .expect("insert access log row");
        sqlx::query("INSERT INTO s3_share_credentials (share_id) VALUES ($1)")
            .bind("stale")
            .execute(&pool)
            .await
            .expect("insert s3 credential row");

        cleanup_stale_shares(&pool).await;

        assert!(remaining_ids(&pool).await.is_empty());
        let log_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM share_access_log WHERE share_id = $1")
                .bind("stale")
                .fetch_one(&pool)
                .await
                .expect("count access log rows");
        assert_eq!(log_count.0, 0);
        let cred_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM s3_share_credentials WHERE share_id = $1")
                .bind("stale")
                .fetch_one(&pool)
                .await
                .expect("count s3 credential rows");
        assert_eq!(cred_count.0, 0);
    }

    #[tokio::test]
    async fn startup_migration_uses_last_access_for_revoked_shares_without_expiry() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(&pool, "recently-accessed", None, Some(now - 30 * DAY_MS)).await;
        insert_access_log(&pool, "recent", "recently-accessed", now - DAY_MS).await;
        insert_share(&pool, "stale-last-access", None, Some(now - 30 * DAY_MS)).await;
        insert_access_log(&pool, "old", "stale-last-access", now - 30 * DAY_MS).await;

        cleanup_startup_stale_shares(&pool).await;

        assert_eq!(remaining_ids(&pool).await, vec!["recently-accessed"]);
    }

    #[tokio::test]
    async fn removes_access_logs_for_missing_shares_and_entries_older_than_eight_weeks() {
        let pool = test_pool().await;
        let now = chrono::Utc::now().timestamp_millis();

        insert_share(&pool, "kept-share", None, None).await;
        insert_access_log(&pool, "fresh", "kept-share", now - DAY_MS).await;
        insert_access_log(&pool, "old", "kept-share", now - 60 * DAY_MS).await;
        insert_access_log(&pool, "orphan", "missing-share", now - DAY_MS).await;
        sqlx::query("INSERT INTO sftp_access_log (id, occurred_at) VALUES ($1, $2)")
            .bind("fresh-sftp")
            .bind(now - DAY_MS)
            .execute(&pool)
            .await
            .expect("insert fresh sftp log");
        sqlx::query("INSERT INTO sftp_access_log (id, occurred_at) VALUES ($1, $2)")
            .bind("old-sftp")
            .bind(now - 60 * DAY_MS)
            .execute(&pool)
            .await
            .expect("insert old sftp log");

        cleanup_stale_access_logs(&pool).await;

        let access_ids: Vec<String> =
            sqlx::query_as::<_, (String,)>("SELECT id FROM share_access_log ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("select access log rows")
                .into_iter()
                .map(|(id,)| id)
                .collect();
        assert_eq!(access_ids, vec!["fresh"]);

        let sftp_ids: Vec<String> =
            sqlx::query_as::<_, (String,)>("SELECT id FROM sftp_access_log ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("select sftp log rows")
                .into_iter()
                .map(|(id,)| id)
                .collect();
        assert_eq!(sftp_ids, vec!["fresh-sftp"]);
    }
}
