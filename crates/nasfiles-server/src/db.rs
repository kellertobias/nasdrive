use sqlx::AnyPool;
use sqlx::any::AnyPoolOptions;

/// Create a database pool from a URL. The scheme dispatches the driver:
/// - `sqlite://` → SQLite
/// - `postgres://` → PostgreSQL
pub async fn create_pool(db_url: &str) -> anyhow::Result<AnyPool> {
    // Install the drivers based on scheme
    if db_url.starts_with("sqlite://")
        || db_url.starts_with("sqlite:")
        || db_url.starts_with("postgres://")
        || db_url.starts_with("postgresql://")
    {
        sqlx::any::install_default_drivers();
    } else {
        anyhow::bail!("Unsupported DB_URL scheme. Must start with sqlite:// or postgres://");
    }

    let pool = AnyPoolOptions::new()
        .max_connections(20)
        .connect(db_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {e}"))?;

    tracing::info!("Database pool created for {}", redact_db_url(db_url));

    Ok(pool)
}

/// Run migrations against the pool.
pub async fn run_migrations(pool: &AnyPool) -> anyhow::Result<()> {
    // We use raw SQL executed via sqlx rather than sqlx::migrate! macro
    // because sqlx::migrate! doesn't support AnyPool well.
    let migrations = [
        include_str!("../../../migrations/001_initial.sql"),
        include_str!("../../../migrations/002_add_oidc_tokens.sql"),
        include_str!("../../../migrations/003_sftp.sql"),
        include_str!("../../../migrations/004_local_auth.sql"),
        include_str!("../../../migrations/005_file_operations.sql"),
        include_str!("../../../migrations/006_login_attempts_ip_idx.sql"),
        include_str!("../../../migrations/007_totp_replay_protection.sql"),
        include_str!("../../../migrations/008_ip_blocklist.sql"),
        include_str!("../../../migrations/009_s3.sql"),
        include_str!("../../../migrations/010_share_revocation_reason.sql"),
    ];

    for migration_sql in migrations {
        for statement in migration_sql.split(';') {
            let filtered: String = statement
                .lines()
                .filter(|line| !line.trim().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");
            let trimmed = filtered.trim();
            if !trimmed.is_empty()
                && let Err(e) = sqlx::query(trimmed).execute(pool).await
            {
                if is_idempotent_migration_error(&e) {
                    tracing::debug!("Migration statement already applied, continuing: {e}");
                } else {
                    anyhow::bail!("Migration failed for statement `{trimmed}`: {e}");
                }
            }
        }
    }

    tracing::info!("Database migrations applied");
    Ok(())
}

fn is_idempotent_migration_error(error: &sqlx::Error) -> bool {
    let Some(db_error) = error.as_database_error() else {
        return false;
    };

    if let Some(code) = db_error.code().as_deref() {
        // PostgreSQL: duplicate_column, duplicate_table, duplicate_object.
        if matches!(code, "42701" | "42P07" | "42710") {
            return true;
        }
    }

    let message = db_error.message().to_ascii_lowercase();
    // SQLite duplicate ALTER TABLE columns and object creation errors.
    message.contains("duplicate column") || message.contains("already exists")
}

/// Redact credentials from a DB URL for logging.
fn redact_db_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@')
        && let Some(scheme_end) = url.find("://")
    {
        return format!("{}://***@{}", &url[..scheme_end], &url[at_pos + 1..]);
    }
    // For SQLite URLs, no credentials to redact
    url.to_string()
}
