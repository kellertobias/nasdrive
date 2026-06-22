//! Internal IP blocklist.
//!
//! Blocked IPs are denied SFTP authentication regardless of the credentials
//! they present. Entries are added automatically (e.g. a client attempting a
//! root SFTP login) and managed by admins from the dashboard.

use serde::Serialize;
use sqlx::AnyPool;

#[derive(Serialize, sqlx::FromRow)]
pub struct BlocklistEntry {
    pub ip: String,
    pub reason: Option<String>,
    pub blocked_at: i64,
    pub last_seen_at: i64,
    pub hit_count: i64,
}

/// Returns true if `ip` is currently on the blocklist.
pub async fn is_blocked(pool: &AnyPool, ip: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ip_blocklist WHERE ip = $1")
        .bind(ip)
        .fetch_one(pool)
        .await
        .map(|count| count > 0)
        .unwrap_or(false)
}

/// Add `ip` to the blocklist, or bump its hit counter if already present. The
/// original `reason` and `blocked_at` are preserved on repeat hits.
pub async fn block(pool: &AnyPool, ip: &str, reason: &str) {
    let now = chrono::Utc::now().timestamp_millis();
    let result = sqlx::query(
        r#"
        INSERT INTO ip_blocklist (ip, reason, blocked_at, last_seen_at, hit_count)
        VALUES ($1, $2, $3, $3, 1)
        ON CONFLICT(ip) DO UPDATE SET
            hit_count = ip_blocklist.hit_count + 1,
            last_seen_at = $3
        "#,
    )
    .bind(ip)
    .bind(reason)
    .bind(now)
    .execute(pool)
    .await;

    match result {
        Ok(_) => tracing::warn!(ip, reason, "IP added to blocklist"),
        Err(e) => tracing::error!(ip, "failed to add IP to blocklist: {e}"),
    }
}

/// List all blocklist entries, most recently blocked first.
pub async fn list(pool: &AnyPool) -> Result<Vec<BlocklistEntry>, sqlx::Error> {
    sqlx::query_as::<_, BlocklistEntry>(
        r#"
        SELECT ip, reason, blocked_at, last_seen_at, hit_count
        FROM ip_blocklist
        ORDER BY blocked_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Remove `ip` from the blocklist. Returns true if an entry was removed.
pub async fn unblock(pool: &AnyPool, ip: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM ip_blocklist WHERE ip = $1")
        .bind(ip)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
