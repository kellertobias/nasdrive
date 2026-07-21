use crate::config::AppConfig;
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store,
};
use tower_sessions_sqlx_store::{PostgresStore, SqliteStore};

#[derive(Clone, Debug)]
pub enum PersistentSessionStore {
    Sqlite(SqliteStore),
    Postgres(PostgresStore),
}

impl PersistentSessionStore {
    // @tour authentication:60 Sessions live in the database
    // `PersistentSessionStore` is a two-variant enum over `SqliteStore` and
    // `PostgresStore`, chosen by sniffing the `DB_URL` scheme and rejecting anything else.
    // It runs `store.migrate()` at connect time, and the hand-written `impl SessionStore`
    // forwards `create`/`save`/`load`/`delete` to whichever backend was selected.
    //
    // Because sessions are rows rather than signed cookies, `session.delete()` on logout is
    // a genuine server-side revoke.

    pub async fn connect(db_url: &str) -> anyhow::Result<Self> {
        if is_sqlite_url(db_url) {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(5)
                .connect(db_url)
                .await
                .map_err(|e| anyhow::anyhow!("failed to connect SQLite session store: {e}"))?;
            let store = SqliteStore::new(pool);
            store
                .migrate()
                .await
                .map_err(|e| anyhow::anyhow!("failed to migrate SQLite session store: {e}"))?;
            return Ok(Self::Sqlite(store));
        }

        if is_postgres_url(db_url) {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(5)
                .connect(db_url)
                .await
                .map_err(|e| anyhow::anyhow!("failed to connect PostgreSQL session store: {e}"))?;
            let store = PostgresStore::new(pool);
            store
                .migrate()
                .await
                .map_err(|e| anyhow::anyhow!("failed to migrate PostgreSQL session store: {e}"))?;
            return Ok(Self::Postgres(store));
        }

        anyhow::bail!("Unsupported DB_URL scheme. Must start with sqlite:// or postgres://");
    }
}

#[async_trait::async_trait]
impl SessionStore for PersistentSessionStore {
    async fn create(&self, session_record: &mut Record) -> session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.create(session_record).await,
            Self::Postgres(store) => store.create(session_record).await,
        }
    }

    async fn save(&self, session_record: &Record) -> session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.save(session_record).await,
            Self::Postgres(store) => store.save(session_record).await,
        }
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        match self {
            Self::Sqlite(store) => store.load(session_id).await,
            Self::Postgres(store) => store.load(session_id).await,
        }
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        match self {
            Self::Sqlite(store) => store.delete(session_id).await,
            Self::Postgres(store) => store.delete(session_id).await,
        }
    }
}

/// Get the session cookie name.
pub fn cookie_name() -> &'static str {
    "nasfiles.sid"
}

/// Whether to set Secure flag on cookies.
pub fn is_secure(config: &AppConfig) -> bool {
    config.base_url.starts_with("https://")
}

fn is_sqlite_url(db_url: &str) -> bool {
    db_url.starts_with("sqlite://") || db_url.starts_with("sqlite:")
}

fn is_postgres_url(db_url: &str) -> bool {
    db_url.starts_with("postgres://") || db_url.starts_with("postgresql://")
}

#[cfg(test)]
mod tests {
    use super::{is_postgres_url, is_sqlite_url};

    #[test]
    fn detects_supported_database_urls() {
        assert!(is_sqlite_url("sqlite:///tmp/nasfiles.db?mode=rwc"));
        assert!(is_sqlite_url("sqlite::memory:"));
        assert!(is_postgres_url("postgres://user:pass@example/nasfiles"));
        assert!(is_postgres_url("postgresql://user:pass@example/nasfiles"));
    }

    #[test]
    fn rejects_unsupported_database_urls() {
        assert!(!is_sqlite_url("mysql://example"));
        assert!(!is_postgres_url("mysql://example"));
    }
}
