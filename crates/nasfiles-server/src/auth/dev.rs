//! Development-mode user persistence.
//!
//! The dev auth bypass in [`super::middleware::require_auth`] fabricates an
//! `AuthUser` per request and keeps it in the session only. That is enough for
//! everything that reads the session, but not for anything that writes a row
//! referencing the user: `file_operation_jobs.owner_user_id` carries a foreign
//! key to `users(id)`, so on a fresh dev database every transfer and every
//! delete failed with `FOREIGN KEY constraint failed` — a 500 with no useful
//! message, for features that are perfectly fine in production.
//!
//! Persisting the same synthetic user once at startup closes that gap, the way
//! a real OIDC or local login persists its user on the way in.

use sqlx::AnyPool;

use crate::config::{self, AppConfig};
use crate::state::now_ms;

/// The user id the dev bypass fabricates. Anything that has to agree with the
/// session user — most importantly the `users` row backing it — uses this.
pub const DEV_USER_ID: &str = "dev-user-id";

/// The external id of the dev user, namespaced so it can never collide with a
/// real identity provider's subject.
pub const DEV_EXTERNAL_ID: &str = "dev:dev-user";

/// Insert or refresh the `users` row backing the dev bypass.
///
/// No-op unless dev mode is active *and* a dev user is configured — the same
/// pair of conditions the bypass itself checks, so production databases never
/// see this row.
pub async fn ensure_dev_user(config: &AppConfig, pool: &AnyPool) -> anyhow::Result<()> {
    if !config.dev_mode {
        return Ok(());
    }
    let Some(dev_user) = &config.dev_user else {
        return Ok(());
    };

    let folder_permissions = config::compute_folder_permissions(config, &dev_user.groups);
    let folder_permissions_json = serde_json::to_string(&folder_permissions)?;
    let is_admin = config::is_admin(config, &dev_user.groups);
    let has_home = config::personal_folder_allowed(config, &dev_user.groups)
        && config.home_folder_root.is_some();
    let now = now_ms();

    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE id = $1")
        .bind(DEV_USER_ID)
        .fetch_optional(pool)
        .await?;

    if existing.is_some() {
        // Config can change between restarts (groups, roots, home folder), so
        // refresh rather than leaving a stale row behind.
        sqlx::query(
            "UPDATE users SET external_id = $1, username = $2, display_name = $3, is_admin = $4, \
             folder_permissions_json = $5, has_home = $6, last_login_at = $7 WHERE id = $8",
        )
        .bind(DEV_EXTERNAL_ID)
        .bind(&dev_user.username)
        .bind(&dev_user.display_name)
        .bind(is_admin)
        .bind(&folder_permissions_json)
        .bind(has_home)
        .bind(now)
        .bind(DEV_USER_ID)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO users (id, external_id, username, display_name, picture_url, is_admin, \
             folder_permissions_json, has_home, auth_provider, created_at, last_login_at) \
             VALUES ($1, $2, $3, $4, NULL, $5, $6, $7, 'oidc', $8, $9)",
        )
        .bind(DEV_USER_ID)
        .bind(DEV_EXTERNAL_ID)
        .bind(&dev_user.username)
        .bind(&dev_user.display_name)
        .bind(is_admin)
        .bind(&folder_permissions_json)
        .bind(has_home)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    tracing::warn!(
        user_id = DEV_USER_ID,
        username = %dev_user.username,
        "⚠️  Dev mode: persisted the bypass user so file jobs can reference it"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DevUserConfig, test_config};
    use sqlx::any::AnyPoolOptions;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY,
                external_id TEXT NOT NULL UNIQUE,
                username TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL,
                picture_url TEXT,
                is_admin BOOLEAN NOT NULL DEFAULT FALSE,
                folder_permissions_json TEXT,
                has_home BOOLEAN NOT NULL DEFAULT FALSE,
                auth_provider TEXT NOT NULL DEFAULT 'oidc',
                created_at BIGINT NOT NULL,
                last_login_at BIGINT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create users table");
        pool
    }

    fn dev_config() -> AppConfig {
        let mut config = test_config();
        config.dev_mode = true;
        config.dev_user = Some(DevUserConfig {
            username: "devuser".to_string(),
            display_name: "Development User".to_string(),
            groups: vec!["STAFF".to_string()],
        });
        config
    }

    async fn user_count(pool: &AnyPool) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(pool)
            .await
            .expect("count users")
    }

    #[tokio::test]
    async fn creates_the_row_file_jobs_reference() {
        let pool = test_pool().await;
        ensure_dev_user(&dev_config(), &pool).await.unwrap();

        let id: String = sqlx::query_scalar("SELECT id FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(id, DEV_USER_ID);
    }

    #[tokio::test]
    async fn is_idempotent_across_restarts() {
        let pool = test_pool().await;
        let config = dev_config();
        ensure_dev_user(&config, &pool).await.unwrap();
        ensure_dev_user(&config, &pool).await.unwrap();

        assert_eq!(user_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn refreshes_permissions_when_config_changed() {
        let pool = test_pool().await;
        let mut config = dev_config();
        ensure_dev_user(&config, &pool).await.unwrap();

        config.default_folder_caps.insert(
            "docs".to_string(),
            nasfiles_core::models::FolderCaps {
                read: true,
                write: true,
                share: false,
            },
        );
        ensure_dev_user(&config, &pool).await.unwrap();

        let permissions: String = sqlx::query_scalar("SELECT folder_permissions_json FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(permissions.contains("docs"));
        assert_eq!(user_count(&pool).await, 1);
    }

    #[tokio::test]
    async fn production_databases_never_get_a_dev_row() {
        let pool = test_pool().await;
        let mut config = dev_config();
        config.dev_mode = false;
        ensure_dev_user(&config, &pool).await.unwrap();

        assert_eq!(user_count(&pool).await, 0);
    }
}
