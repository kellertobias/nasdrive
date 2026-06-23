mod api;
mod assets;
mod auth;
mod blocklist;
mod config;
mod db;
mod fs;
mod sftp;
mod shares;
mod state;
mod thumb;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
    routing::{get, post},
};
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse config first so we know log level
    let config = config::AppConfig::from_env()?;

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_level.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting nasfiles server");

    if config.dev_mode {
        tracing::warn!("⚠️  Running in development mode — auth bypass enabled");
    }

    // Create database pool
    let pool = db::create_pool(&config.db_url).await?;

    // Run migrations
    db::run_migrations(&pool).await?;

    auth::local::ensure_setup_admin(&config, &pool).await?;

    // Initialize OIDC client if configured
    if config.oidc.is_some() {
        match auth::oidc::init_oidc_client(&config).await {
            Ok(()) => tracing::info!("OIDC client initialized"),
            Err(e) => {
                if config.dev_mode {
                    tracing::warn!("OIDC initialization failed (dev mode, continuing): {e}");
                } else {
                    return Err(e);
                }
            }
        }
    }

    // Session store (using memory store for now — can swap to sqlx store later)
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name(auth::session::cookie_name())
        .with_secure(auth::session::is_secure(&config))
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(
            tower_sessions::cookie::time::Duration::hours(24),
        ));

    let bind_addr = config.bind_addr.clone();
    let is_dev_mode = config.dev_mode;
    let content_security_policy = content_security_policy(&config)?;
    let cors_layer = cors_layer(&config)?;

    // Build app state
    let state = state::AppState::new(config, pool)?;
    state.search.spawn_refresh_loop();

    // Spawn daily share audit background task
    if matches!(state.config.auth_mode, config::AuthMode::Sso) {
        auth::share_audit::spawn_daily_share_audit(state.clone());
    }

    if state.config.sftp_enabled {
        sftp::server::spawn(state.clone()).await?;
    }

    if let Err(e) = fs::file_jobs::spawn_recovered_jobs(state.clone()).await {
        tracing::warn!("failed to recover file operation jobs: {e}");
    }

    if state.config.no_server_side_execution {
        tracing::info!(
            "NO_SERVER_SIDE_EXECUTION enabled — archive extraction, thumbnails, media previews, and media metadata probing disabled"
        );
    } else {
        // Check ffmpeg availability for video thumbnails
        if thumb::video::is_available().await {
            tracing::info!("ffmpeg detected — video thumbnails enabled");
        } else {
            tracing::warn!("ffmpeg not found — video thumbnails will be skipped");
        }

        if thumb::video::ffprobe_is_available().await {
            tracing::info!("ffprobe detected — media metadata thumbnails enabled");
        } else {
            tracing::warn!(
                "ffprobe not found — video timing and synthetic audio covers may be skipped"
            );
        }

        if thumb::pdf::is_available().await {
            tracing::info!("pdftoppm detected — PDF thumbnails enabled");
        } else {
            tracing::warn!("pdftoppm not found — PDF thumbnails will be skipped");
        }

        // Ensure thumbnail cache directory exists
        if let Err(e) = tokio::fs::create_dir_all(&state.config.thumbnail_cache_dir).await {
            tracing::warn!("Failed to create thumbnail cache dir: {e}");
        }
    }

    // Authenticated API routes
    let api_routes = Router::new()
        .route("/me", get(api::me::me))
        .route("/roots", get(api::files::list_roots))
        .route("/search", get(api::files::search_files))
        .route("/transfer-jobs", get(api::files::list_transfer_jobs))
        .route(
            "/transfer-jobs/{job_id}/cancel",
            post(api::files::cancel_transfer_job),
        )
        .route(
            "/file-jobs/{job_id}/resume",
            post(api::files::resume_file_job),
        )
        .route(
            "/file-jobs/{job_id}/cancel",
            post(api::files::cancel_file_job),
        )
        .route(
            "/file-jobs/{job_id}/cleanup",
            post(api::files::cleanup_file_job),
        )
        .route("/files/{root}/list", get(api::files::list_directory))
        .route("/files/{root}/tree", get(api::files::list_tree))
        .route("/files/{root}/download", get(api::files::download_file))
        .route("/files/{root}/preview", get(api::files::preview_file))
        .route(
            "/files/{root}/preview-status",
            get(api::files::preview_status),
        )
        .route("/files/{root}/info", get(api::files::file_info))
        .route(
            "/files/{root}/thumbnail",
            get(api::thumbnails::get_thumbnail),
        )
        // Write operations
        .route("/files/{root}/mkdir", post(api::files::mkdir))
        .route("/files/{root}/rename", post(api::files::rename))
        .route("/files/{root}/move", post(api::files::move_entries))
        .route("/files/{root}/transfer", post(api::files::transfer_entries))
        .route("/files/{root}/delete", post(api::files::delete_entries))
        .route("/files/{root}/upload", post(api::files::upload_file))
        .route("/files/{root}/extract", post(api::files::extract_archive))
        .route("/files/{root}/zip", post(api::files::download_zip))
        // Share management
        .route("/shares", post(api::shares::create_share))
        .route("/shares", get(api::shares::list_shares))
        .route("/shares/{id}", get(api::shares::get_share))
        .route(
            "/shares/{id}",
            axum::routing::delete(api::shares::revoke_share),
        )
        // SFTP key management
        .route("/sftp/keys", get(sftp::api::list_user_keys))
        .route("/sftp/keys", post(sftp::api::add_user_key))
        .route(
            "/sftp/keys/{id}",
            axum::routing::delete(sftp::api::revoke_user_key),
        )
        // Admin routes
        .route("/admin/shares", get(api::admin::list_all_shares))
        .route("/admin/access-log", get(api::admin::list_access_log))
        .route("/admin/ip-blocklist", get(api::admin::list_blocklist))
        .route("/admin/ip-blocklist", post(api::admin::add_blocklist_entry))
        .route(
            "/admin/ip-blocklist/{ip}",
            axum::routing::delete(api::admin::remove_blocklist_entry),
        )
        .route("/admin/users", get(api::admin::list_users))
        .route("/admin/users", post(auth::local::create_user))
        .route(
            "/admin/users/{id}",
            axum::routing::put(auth::local::update_user),
        )
        .route(
            "/admin/users/{id}/reset-password",
            post(auth::local::reset_user_password),
        )
        .route(
            "/admin/users/{id}/passkeys",
            get(auth::local::admin_list_passkeys),
        )
        .route(
            "/admin/users/{user_id}/passkeys/{passkey_id}",
            axum::routing::delete(auth::local::admin_revoke_passkey),
        )
        .route(
            "/admin/users/{id}/trusted-devices",
            get(auth::local::admin_list_trusted_devices),
        )
        .route(
            "/admin/users/{user_id}/trusted-devices/{device_id}",
            axum::routing::delete(auth::local::admin_revoke_trusted_device),
        )
        // Local-auth profile routes
        .route("/profile/password", post(auth::local::change_password))
        .route("/profile/totp/setup", post(auth::local::start_totp_setup))
        .route(
            "/profile/totp/confirm",
            post(auth::local::confirm_totp_setup),
        )
        .route(
            "/profile/totp",
            axum::routing::delete(auth::local::remove_totp),
        )
        .route(
            "/profile/trusted-devices",
            get(auth::local::list_trusted_devices),
        )
        .route(
            "/profile/trusted-devices/{id}",
            axum::routing::delete(auth::local::revoke_trusted_device),
        )
        .route("/profile/passkeys", get(auth::local::list_passkeys))
        .route(
            "/profile/passkeys/options",
            post(auth::local::start_passkey_registration),
        )
        .route(
            "/profile/passkeys/finish",
            post(auth::local::finish_passkey_registration),
        )
        .route(
            "/profile/passkeys/{id}",
            axum::routing::delete(auth::local::revoke_passkey),
        )
        .route("/admin/sftp-temp-users", get(sftp::api::list_temp_users))
        .route("/admin/sftp-temp-users", post(sftp::api::create_temp_user))
        .route("/admin/sftp-access-log", get(sftp::api::list_access_log))
        .route(
            "/admin/sftp-sessions",
            get(sftp::api::list_active_sessions),
        )
        .route(
            "/admin/sftp-temp-users/{id}/extend",
            post(sftp::api::extend_temp_user),
        )
        .route(
            "/admin/sftp-temp-users/{id}",
            axum::routing::delete(sftp::api::revoke_temp_user),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::require_auth,
        ));

    // Auth routes (not behind auth middleware)
    let auth_routes = Router::new().route("/logout", post(api::me::logout));
    let auth_routes = if matches!(state.config.auth_mode, config::AuthMode::Local) {
        auth_routes
            .route("/local/login", post(auth::local::login))
            .route("/local/login/totp", post(auth::local::login_totp))
            .route(
                "/local/passkey/options",
                post(auth::local::start_passkey_authentication),
            )
            .route(
                "/local/passkey/finish",
                post(auth::local::finish_passkey_authentication),
            )
    } else {
        auth_routes
            .route("/oidc/login", get(auth::oidc::login))
            .route("/oidc/callback", get(auth::oidc::callback))
    };

    let public_routes = Router::new()
        .route("/shares/{token}", get(api::public::share_metadata))
        .route("/shares/{token}/auth", post(api::public::share_auth))
        .route("/shares/{token}/list", get(api::public::share_list))
        .route("/shares/{token}/download", get(api::public::share_download))
        .route("/shares/{token}/info", get(api::public::share_info))
        .route("/shares/{token}/preview", get(api::public::share_preview))
        .route(
            "/shares/{token}/preview-status",
            get(api::public::share_preview_status),
        )
        .route("/shares/{token}/upload", post(api::public::share_upload))
        .route("/shares/{token}/zip", post(api::public::share_zip));

    // Health check
    let health = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/api/auth/config", get(auth::local::auth_config));

    // Main application router
    let app = Router::new()
        .nest("/api", api_routes)
        .nest("/auth", auth_routes)
        .nest("/api/public", public_routes)
        .merge(health)
        .fallback(assets::static_handler)
        .layer(session_layer)
        .layer(cors_layer)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            content_security_policy,
        ))
        .with_state(state);

    // Add HSTS in production
    let app = if !is_dev_mode {
        app.layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        ))
    } else {
        app
    };

    tracing::info!("Listening on {bind_addr}");

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn cors_layer(config: &config::AppConfig) -> anyhow::Result<CorsLayer> {
    let Some(origin) = origin_from_url(&config.base_url) else {
        return Ok(CorsLayer::new());
    };

    Ok(CorsLayer::new()
        .allow_origin(HeaderValue::from_str(&origin)?)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("x-nasfiles-request"),
        ]))
}

fn origin_from_url(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let after_scheme = scheme_end + 3;
    let rest = &url[after_scheme..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    if host_end == 0 {
        return None;
    }
    Some(format!("{}://{}", &url[..scheme_end], &rest[..host_end]))
}

fn content_security_policy(config: &config::AppConfig) -> anyhow::Result<HeaderValue> {
    let img_src = csp_directive_sources("'self' data: blob:", &config.csp_extra_img_src);
    let media_src = csp_directive_sources("'self' blob:", &config.csp_extra_media_src);
    let policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; img-src {img_src}; media-src {media_src}; connect-src 'self'; font-src 'self' data: https://fonts.gstatic.com; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'"
    );

    HeaderValue::from_str(&policy)
        .map_err(|e| anyhow::anyhow!("invalid content security policy: {e}"))
}

fn csp_directive_sources(base_sources: &str, extra_sources: &[String]) -> String {
    if extra_sources.is_empty() {
        return base_sources.to_string();
    }

    format!("{} {}", base_sources, extra_sources.join(" "))
}
