#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64ct::{Base64UrlUnpadded, Encoding};
use hmac::{Hmac, Mac};
use nasfiles_core::models::{AuthUser, FolderCaps};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use sqlx::{AnyPool, Row};
use subtle::ConstantTimeEq;
use totp_rs::{Algorithm, TOTP};
use webauthn_rs::prelude::{
    CredentialID, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Uuid, Webauthn, WebauthnBuilder,
};

use crate::auth::middleware::CurrentUser;
use crate::config::{AppConfig, AuthMode};
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

const SESSION_USER: &str = "user";
const TOTP_CHALLENGE_SESSION: &str = "local_totp_challenge";
const TOTP_SETUP_SESSION: &str = "local_totp_setup";
const PASSKEY_REG_SESSION: &str = "local_passkey_registration";
const PASSKEY_AUTH_SESSION: &str = "local_passkey_authentication";
const LOCAL_AUTH_AT_SESSION: &str = "local_auth_at";

#[derive(Debug, Serialize, Deserialize)]
struct TotpChallengeSession {
    user_id: String,
    challenge_id: String,
    expires_at: i64,
    attempts: u8,
}

#[derive(Debug, Serialize, Deserialize)]
struct TotpSetupSession {
    secret: Vec<u8>,
    created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PasskeyRegistrationSession {
    user_id: String,
    state: PasskeyRegistration,
}

#[derive(Debug, Serialize, Deserialize)]
struct PasskeyAuthenticationSession {
    user_id: String,
    state: PasskeyAuthentication,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub trusted_device: Option<TrustedDeviceProof>,
}

#[derive(Debug, Deserialize)]
pub struct TrustedDeviceProof {
    pub id: String,
    pub hash: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpLoginRequest {
    pub challenge_id: String,
    pub code: String,
    #[serde(default)]
    pub trust_computer: bool,
    pub device_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmTotpRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct PasskeyOptionsRequest {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub folder_permissions: HashMap<String, FolderCaps>,
    #[serde(default)]
    pub has_home: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub is_admin: Option<bool>,
    pub folder_permissions: Option<HashMap<String, FolderCaps>>,
    pub has_home: Option<bool>,
}

#[derive(sqlx::FromRow)]
struct LocalUserRow {
    id: String,
    external_id: String,
    username: String,
    display_name: String,
    picture_url: Option<String>,
    is_admin: i64,
    has_home: i64,
    folder_permissions_json: Option<String>,
    password_hash: Option<String>,
    password_changed_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct PasskeyRow {
    id: String,
    credential_json: String,
    label: Option<String>,
    created_at: i64,
    last_used_at: Option<i64>,
    revoked_at: Option<i64>,
}

pub fn build_webauthn(config: &AppConfig) -> anyhow::Result<Option<Webauthn>> {
    if !matches!(config.auth_mode, AuthMode::Local) || config.disable_passkeys {
        return Ok(None);
    }

    let origin = Url::parse(&config.base_url)
        .map_err(|e| anyhow::anyhow!("BASE_URL is not a valid WebAuthn origin: {e}"))?;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("BASE_URL must include a host for WebAuthn"))?;
    let mut builder = WebauthnBuilder::new(rp_id, &origin)
        .map_err(|e| anyhow::anyhow!("invalid WebAuthn configuration: {e}"))?
        .rp_name("NASDrive");
    if config.dev_mode {
        builder = builder.allow_any_port(true);
    }
    Ok(Some(builder.build().map_err(|e| {
        anyhow::anyhow!("invalid WebAuthn configuration: {e}")
    })?))
}

pub async fn ensure_setup_admin(config: &AppConfig, pool: &AnyPool) -> anyhow::Result<()> {
    if !matches!(config.auth_mode, AuthMode::Local) {
        return Ok(());
    }

    let local_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE auth_provider = 'local'")
            .fetch_one(pool)
            .await?;

    let Some(setup) = &config.setup_admin else {
        if local_count == 0 {
            anyhow::bail!(
                "AUTH_MODE=local requires SETUP_ADMIN_USER and SETUP_ADMIN_PASSWORD until at least one local user exists"
            );
        }
        return Ok(());
    };

    let username = setup.username.trim();
    nasfiles_core::models::validate_username(username)
        .map_err(|msg| anyhow::anyhow!("invalid SETUP_ADMIN_USER: {msg}"))?;
    let normalized = normalize_username(username);
    validate_setup_admin_password(&setup.password)?;
    let fingerprint = setup_password_fingerprint(config, &setup.password)?;
    let password_hash = hash_password(&setup.password)?;
    let now = now_ms();
    let permissions = full_folder_permissions(config);
    let permissions_json = serde_json::to_string(&permissions)?;
    let has_home = config.home_folder_root.is_some();

    let existing = sqlx::query(
        r#"
        SELECT id, setup_password_fingerprint
        FROM users
        WHERE auth_provider = 'local' AND username_normalized = $1
        "#,
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        let id: String = row.get("id");
        let current_fingerprint: Option<String> = row.get("setup_password_fingerprint");
        if should_apply_setup_password(current_fingerprint.as_deref(), &fingerprint) {
            sqlx::query(
                r#"
                UPDATE users
                SET password_hash = $1,
                    setup_password_fingerprint = $2,
                    setup_password_source = 'setup',
                    password_changed_at = $3,
                    is_admin = TRUE,
                    display_name = $4
                WHERE id = $5
                "#,
            )
            .bind(&password_hash)
            .bind(&fingerprint)
            .bind(now)
            .bind(&setup.display_name)
            .bind(&id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query("UPDATE users SET is_admin = TRUE, display_name = $1 WHERE id = $2")
                .bind(&setup.display_name)
                .bind(&id)
                .execute(pool)
                .await?;
        }
        return Ok(());
    }

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO users
            (id, external_id, username, username_normalized, display_name, picture_url,
             is_admin, folder_permissions_json, has_home, password_hash,
             setup_password_fingerprint, setup_password_source, password_changed_at,
             auth_provider, created_at, last_login_at)
        VALUES ($1, $2, $3, $4, $5, NULL, TRUE, $6, $7, $8, $9, 'setup', $10, 'local', $11, $12)
        "#,
    )
    .bind(&id)
    .bind(format!("local:{normalized}"))
    .bind(username)
    .bind(&normalized)
    .bind(&setup.display_name)
    .bind(&permissions_json)
    .bind(has_home)
    .bind(&password_hash)
    .bind(&fingerprint)
    .bind(now)
    .bind(now)
    .bind(0_i64)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn auth_config(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "mode": state.config.auth_mode.as_str(),
        "local_enabled": matches!(state.config.auth_mode, AuthMode::Local),
        "sso_enabled": matches!(state.config.auth_mode, AuthMode::Sso),
        "passkeys_enabled": matches!(state.config.auth_mode, AuthMode::Local) && !state.config.disable_passkeys && state.webauthn.is_some(),
        "totp_enabled": matches!(state.config.auth_mode, AuthMode::Local) && !state.config.disable_totp,
        // True only when dev mode is active AND a synthetic dev user is configured,
        // i.e. the auth middleware is bypassing real authentication. Drives the
        // red dev-mode warning bar in the UI.
        "dev_auth_bypass": state.config.dev_mode && state.config.dev_user.is_some(),
    }))
}

pub async fn login(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    require_local_auth_header(&headers)?;
    let normalized = normalize_username(&body.username);
    let ip = crate::auth::client_ip(&headers, state.config.trusted_proxy_depth);

    if is_login_rate_limited(&state.pool, &normalized).await
        || is_login_rate_limited_by_ip(&state.pool, ip.as_deref()).await
    {
        record_attempt(
            &state.pool,
            &normalized,
            ip.as_deref(),
            false,
            Some("rate_limited"),
        )
        .await;
        return Err(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many login attempts",
        ));
    }

    let Some(row) = load_local_user_by_normalized(&state.pool, &normalized)
        .await
        .map_err(internal_error)?
    else {
        record_attempt(
            &state.pool,
            &normalized,
            ip.as_deref(),
            false,
            Some("unknown_user"),
        )
        .await;
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid username or password",
        ));
    };

    let Some(hash) = row.password_hash.as_deref() else {
        record_attempt(
            &state.pool,
            &normalized,
            ip.as_deref(),
            false,
            Some("no_password"),
        )
        .await;
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid username or password",
        ));
    };
    if !verify_password(hash, &body.password) {
        record_attempt(
            &state.pool,
            &normalized,
            ip.as_deref(),
            false,
            Some("bad_password"),
        )
        .await;
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "invalid username or password",
        ));
    }

    if !state.config.disable_passkeys && active_passkey_count(&state.pool, &row.id).await? > 0 {
        record_attempt(
            &state.pool,
            &normalized,
            ip.as_deref(),
            false,
            Some("passkey_required"),
        )
        .await;
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "passkey login is required for this user",
        ));
    }

    if !state.config.disable_totp && user_has_totp(&state.pool, &row.id).await? {
        if let Some(proof) = &body.trusted_device
            && verify_trusted_device(&state, &row.id, proof).await?
        {
            let user = row.into_auth_user();
            finish_login(&state, &session, user).await?;
            record_attempt(
                &state.pool,
                &normalized,
                ip.as_deref(),
                true,
                Some("trusted_totp"),
            )
            .await;
            return Ok(Json(json!({"ok": true, "requires_totp": false})));
        }

        let challenge_id = random_token(24);
        let challenge = TotpChallengeSession {
            user_id: row.id,
            challenge_id: challenge_id.clone(),
            expires_at: now_ms() + 5 * 60 * 1000,
            attempts: 0,
        };
        session
            .insert(TOTP_CHALLENGE_SESSION, &challenge)
            .await
            .map_err(session_error)?;
        return Ok(Json(json!({
            "ok": false,
            "requires_totp": true,
            "challenge_id": challenge_id,
        })));
    }

    let user = row.into_auth_user();
    finish_login(&state, &session, user).await?;
    record_attempt(
        &state.pool,
        &normalized,
        ip.as_deref(),
        true,
        Some("password"),
    )
    .await;
    Ok(Json(json!({"ok": true, "requires_totp": false})))
}

pub async fn login_totp(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    headers: HeaderMap,
    Json(body): Json<TotpLoginRequest>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    require_local_auth_header(&headers)?;
    if state.config.disable_totp {
        return Err(api_error(StatusCode::NOT_FOUND, "TOTP is disabled"));
    }

    let mut challenge: TotpChallengeSession = session
        .get(TOTP_CHALLENGE_SESSION)
        .await
        .map_err(session_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "login challenge expired"))?;

    if challenge.challenge_id != body.challenge_id || challenge.expires_at < now_ms() {
        session
            .remove::<TotpChallengeSession>(TOTP_CHALLENGE_SESSION)
            .await
            .ok();
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "login challenge expired",
        ));
    }

    if !verify_user_totp(&state, &challenge.user_id, &body.code).await? {
        challenge.attempts = challenge.attempts.saturating_add(1);
        if challenge.attempts >= 5 {
            session
                .remove::<TotpChallengeSession>(TOTP_CHALLENGE_SESSION)
                .await
                .ok();
        } else {
            session
                .insert(TOTP_CHALLENGE_SESSION, &challenge)
                .await
                .map_err(session_error)?;
        }
        return Err(api_error(StatusCode::UNAUTHORIZED, "invalid TOTP code"));
    }

    let trusted_device = if body.trust_computer {
        Some(create_trusted_device(&state, &challenge.user_id, body.device_label.as_deref()).await?)
    } else {
        None
    };

    let user = load_local_auth_user(&state.pool, &challenge.user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "user not found"))?;
    finish_login(&state, &session, user).await?;
    session
        .remove::<TotpChallengeSession>(TOTP_CHALLENGE_SESSION)
        .await
        .ok();

    Ok(Json(json!({
        "ok": true,
        "trusted_device": trusted_device,
    })))
}

pub async fn change_password(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: tower_sessions::Session,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let row = load_local_user_row_by_id(&state.pool, &user.user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "user not found"))?;
    let Some(current_hash) = row.password_hash.as_deref() else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "password login is not configured",
        ));
    };
    if !verify_password(current_hash, &body.current_password) {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "current password is incorrect",
        ));
    }
    validate_new_password(&body.new_password)?;
    let changed_at =
        set_user_password(&state.pool, &user.user_id, &body.new_password, "user", None).await?;
    session
        .insert(LOCAL_AUTH_AT_SESSION, changed_at)
        .await
        .map_err(session_error)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn start_totp_setup(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: tower_sessions::Session,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    if state.config.disable_totp {
        return Err(api_error(StatusCode::NOT_FOUND, "TOTP is disabled"));
    }
    if !state.config.disable_passkeys && active_passkey_count(&state.pool, &user.user_id).await? > 0
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "TOTP can only be enabled while no passkey is assigned",
        ));
    }

    let secret = random_bytes(20);
    let totp = build_totp(&secret, &user.username)?;
    let setup = TotpSetupSession {
        secret,
        created_at: now_ms(),
    };
    session
        .insert(TOTP_SETUP_SESSION, &setup)
        .await
        .map_err(session_error)?;
    Ok(Json(json!({
        "secret": totp.get_secret_base32(),
        "url": totp.get_url(),
    })))
}

pub async fn confirm_totp_setup(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: tower_sessions::Session,
    Json(body): Json<ConfirmTotpRequest>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let setup: TotpSetupSession = session
        .get(TOTP_SETUP_SESSION)
        .await
        .map_err(session_error)?
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "no TOTP setup is pending"))?;
    if setup.created_at + 10 * 60 * 1000 < now_ms() {
        session
            .remove::<TotpSetupSession>(TOTP_SETUP_SESSION)
            .await
            .ok();
        return Err(api_error(StatusCode::BAD_REQUEST, "TOTP setup expired"));
    }
    let totp = build_totp(&setup.secret, &user.username)?;
    if !totp
        .check_current(&body.code)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid TOTP code"))?
    {
        return Err(api_error(StatusCode::UNAUTHORIZED, "invalid TOTP code"));
    }

    let now = now_ms();
    let secret_enc = encrypt_secret(&setup.secret, &state.config.session_secret)?;
    sqlx::query(
        r#"
        INSERT INTO local_totp (user_id, secret_enc, created_at, confirmed_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT(user_id) DO UPDATE
        SET secret_enc = $2, created_at = $3, confirmed_at = $4
        "#,
    )
    .bind(&user.user_id)
    .bind(secret_enc)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    session
        .remove::<TotpSetupSession>(TOTP_SETUP_SESSION)
        .await
        .ok();
    Ok(Json(json!({"ok": true})))
}

pub async fn remove_totp(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let now = now_ms();
    sqlx::query("DELETE FROM local_totp WHERE user_id = $1")
        .bind(&user.user_id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    sqlx::query(
        "UPDATE local_totp_trusted_devices SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&user.user_id)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn list_trusted_devices(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let devices = trusted_devices_for_user(&state.pool, &user.user_id).await?;
    Ok(Json(json!({"devices": devices})))
}

pub async fn revoke_trusted_device(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    revoke_trusted_device_for_user(&state.pool, &user.user_id, &id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn start_passkey_registration(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: tower_sessions::Session,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let webauthn = webauthn(&state)?;
    let exclude_credentials = passkeys_for_user(&state.pool, &user.user_id)
        .await?
        .into_iter()
        .filter_map(|row| serde_json::from_str::<Passkey>(&row.credential_json).ok())
        .map(|passkey| passkey.cred_id().clone())
        .collect::<Vec<CredentialID>>();
    let user_uuid = Uuid::parse_str(&user.user_id)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "local user id is invalid"))?;
    let (challenge, reg_state) = webauthn
        .start_passkey_registration(
            user_uuid,
            &user.username,
            &user.display_name,
            Some(exclude_credentials),
        )
        .map_err(|e| {
            api_error(
                StatusCode::BAD_REQUEST,
                format!("passkey setup failed: {e}"),
            )
        })?;
    session
        .insert(
            PASSKEY_REG_SESSION,
            &PasskeyRegistrationSession {
                user_id: user.user_id,
                state: reg_state,
            },
        )
        .await
        .map_err(session_error)?;
    Ok(Json(challenge))
}

pub async fn finish_passkey_registration(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    session: tower_sessions::Session,
    Json(body): Json<RegisterPublicKeyCredential>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let webauthn = webauthn(&state)?;
    let reg_session: PasskeyRegistrationSession = session
        .get(PASSKEY_REG_SESSION)
        .await
        .map_err(session_error)?
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "no passkey setup is pending"))?;
    if reg_session.user_id != user.user_id {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "passkey setup user mismatch",
        ));
    }
    let passkey = webauthn
        .finish_passkey_registration(&body, &reg_session.state)
        .map_err(|e| {
            api_error(
                StatusCode::BAD_REQUEST,
                format!("passkey setup failed: {e}"),
            )
        })?;
    insert_passkey(&state.pool, &user.user_id, passkey, None).await?;
    session
        .remove::<PasskeyRegistrationSession>(PASSKEY_REG_SESSION)
        .await
        .ok();
    Ok(Json(json!({"ok": true})))
}

pub async fn list_passkeys(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    let keys = public_passkeys_for_user(&state.pool, &user.user_id).await?;
    Ok(Json(json!({"passkeys": keys})))
}

pub async fn revoke_passkey(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    revoke_passkey_for_user(&state.pool, &user.user_id, &id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn start_passkey_authentication(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    headers: HeaderMap,
    Json(body): Json<PasskeyOptionsRequest>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    require_local_auth_header(&headers)?;
    let webauthn = webauthn(&state)?;
    let normalized = normalize_username(&body.username);
    let row = load_local_user_by_normalized(&state.pool, &normalized)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "passkey login failed"))?;
    let passkeys = passkeys_for_user(&state.pool, &row.id).await?;
    if passkeys.is_empty() {
        return Err(api_error(StatusCode::UNAUTHORIZED, "passkey login failed"));
    }
    let credentials = passkeys
        .iter()
        .map(|row| serde_json::from_str::<Passkey>(&row.credential_json))
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal_error)?;
    let (challenge, auth_state) = webauthn
        .start_passkey_authentication(&credentials)
        .map_err(|e| {
            api_error(
                StatusCode::BAD_REQUEST,
                format!("passkey login failed: {e}"),
            )
        })?;
    session
        .insert(
            PASSKEY_AUTH_SESSION,
            &PasskeyAuthenticationSession {
                user_id: row.id,
                state: auth_state,
            },
        )
        .await
        .map_err(session_error)?;
    Ok(Json(challenge))
}

pub async fn finish_passkey_authentication(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    headers: HeaderMap,
    Json(body): Json<PublicKeyCredential>,
) -> Result<impl IntoResponse, Response> {
    require_local_mode(&state)?;
    require_local_auth_header(&headers)?;
    let webauthn = webauthn(&state)?;
    let auth_session: PasskeyAuthenticationSession = session
        .get(PASSKEY_AUTH_SESSION)
        .await
        .map_err(session_error)?
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "no passkey login is pending"))?;
    let auth_result = webauthn
        .finish_passkey_authentication(&body, &auth_session.state)
        .map_err(|e| {
            api_error(
                StatusCode::UNAUTHORIZED,
                format!("passkey login failed: {e}"),
            )
        })?;

    let mut matched = false;
    for row in passkeys_for_user(&state.pool, &auth_session.user_id).await? {
        let mut passkey =
            serde_json::from_str::<Passkey>(&row.credential_json).map_err(internal_error)?;
        if passkey.update_credential(&auth_result).is_some() {
            let credential_json = serde_json::to_string(&passkey).map_err(internal_error)?;
            sqlx::query(
                "UPDATE local_passkeys SET credential_json = $1, last_used_at = $2 WHERE id = $3",
            )
            .bind(credential_json)
            .bind(now_ms())
            .bind(&row.id)
            .execute(&state.pool)
            .await
            .map_err(internal_error)?;
            matched = true;
            break;
        }
    }
    if !matched {
        return Err(api_error(StatusCode::UNAUTHORIZED, "passkey login failed"));
    }

    let user = load_local_auth_user(&state.pool, &auth_session.user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "user not found"))?;
    finish_login(&state, &session, user).await?;
    session
        .remove::<PasskeyAuthenticationSession>(PASSKEY_AUTH_SESSION)
        .await
        .ok();
    Ok(Json(json!({"ok": true})))
}

pub async fn create_user(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Json(body): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    let username = body.username.trim();
    nasfiles_core::models::validate_username(username)
        .map_err(|msg| api_error(StatusCode::BAD_REQUEST, msg))?;
    let normalized = normalize_username(username);
    let password = generate_xkcd_password();
    let password_hash = hash_password(&password).map_err(internal_error)?;
    let now = now_ms();
    let id = uuid::Uuid::new_v4().to_string();
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(username);
    let folder_permissions = sanitize_folder_permissions(&state.config, body.folder_permissions);
    let folder_permissions_json =
        serde_json::to_string(&folder_permissions).map_err(internal_error)?;
    sqlx::query(
        r#"
        INSERT INTO users
            (id, external_id, username, username_normalized, display_name, picture_url,
             is_admin, folder_permissions_json, has_home, password_hash,
             setup_password_source, password_changed_at, auth_provider, created_at, last_login_at)
        VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, $8, $9, 'admin', $10, 'local', $11, 0)
        "#,
    )
    .bind(&id)
    .bind(format!("local:{normalized}"))
    .bind(username)
    .bind(&normalized)
    .bind(display_name)
    .bind(body.is_admin)
    .bind(folder_permissions_json)
    .bind(body.has_home && state.config.home_folder_root.is_some())
    .bind(password_hash)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if e.to_string().to_ascii_lowercase().contains("unique") {
            api_error(StatusCode::CONFLICT, "username already exists")
        } else {
            internal_error(e)
        }
    })?;

    Ok(Json(json!({
        "id": id,
        "username": username,
        "display_name": display_name,
        "password": password,
    })))
}

pub async fn update_user(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path(user_id): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    let existing = load_local_user_row_by_id(&state.pool, &user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "user not found"))?;

    let display_name = body.display_name.unwrap_or(existing.display_name);
    let is_admin = body.is_admin.unwrap_or(existing.is_admin != 0);
    let folder_permissions = body
        .folder_permissions
        .map(|p| sanitize_folder_permissions(&state.config, p))
        .unwrap_or_else(|| parse_permissions(existing.folder_permissions_json.as_deref()));
    let folder_permissions_json =
        serde_json::to_string(&folder_permissions).map_err(internal_error)?;
    let has_home =
        body.has_home.unwrap_or(existing.has_home != 0) && state.config.home_folder_root.is_some();

    sqlx::query(
        r#"
        UPDATE users
        SET display_name = $1, is_admin = $2, folder_permissions_json = $3, has_home = $4
        WHERE id = $5 AND auth_provider = 'local'
        "#,
    )
    .bind(display_name)
    .bind(is_admin)
    .bind(folder_permissions_json)
    .bind(has_home)
    .bind(&user_id)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn reset_user_password(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path(user_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    let _ = load_local_user_row_by_id(&state.pool, &user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "user not found"))?;
    let password = generate_xkcd_password();
    set_user_password(&state.pool, &user_id, &password, "admin", None).await?;
    Ok(Json(json!({"ok": true, "password": password})))
}

pub async fn admin_list_passkeys(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path(user_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    let keys = public_passkeys_for_user(&state.pool, &user_id).await?;
    Ok(Json(json!({"passkeys": keys})))
}

pub async fn admin_revoke_passkey(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path((user_id, passkey_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    revoke_passkey_for_user(&state.pool, &user_id, &passkey_id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn admin_list_trusted_devices(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path(user_id): Path<String>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    let devices = trusted_devices_for_user(&state.pool, &user_id).await?;
    Ok(Json(json!({"devices": devices})))
}

pub async fn admin_revoke_trusted_device(
    State(state): State<AppState>,
    CurrentUser(admin): CurrentUser,
    Path((user_id, device_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, Response> {
    require_admin_local(&state, &admin)?;
    revoke_trusted_device_for_user(&state.pool, &user_id, &device_id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn current_session_user(
    state: &AppState,
    session: &tower_sessions::Session,
) -> Result<AuthUser, Response> {
    let session_user = session
        .get::<AuthUser>(SESSION_USER)
        .await
        .map_err(session_error)?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "not authenticated"))?;

    let Some(row) = load_local_user_row_by_id(&state.pool, &session_user.user_id)
        .await
        .map_err(internal_error)?
    else {
        session.delete().await.ok();
        return Err(api_error(StatusCode::UNAUTHORIZED, "not authenticated"));
    };
    let auth_at = session
        .get::<i64>(LOCAL_AUTH_AT_SESSION)
        .await
        .map_err(session_error)?
        .unwrap_or(0);
    if row.password_changed_at.unwrap_or(0) > auth_at {
        session.delete().await.ok();
        return Err(api_error(StatusCode::UNAUTHORIZED, "session expired"));
    }
    let refreshed = row.into_auth_user();
    session
        .insert(SESSION_USER, &refreshed)
        .await
        .map_err(session_error)?;
    Ok(refreshed)
}

fn require_local_mode(state: &AppState) -> Result<(), Response> {
    if matches!(state.config.auth_mode, AuthMode::Local) {
        Ok(())
    } else {
        Err(api_error(StatusCode::NOT_FOUND, "local auth is disabled"))
    }
}

fn require_admin_local(state: &AppState, user: &AuthUser) -> Result<(), Response> {
    require_local_mode(state)?;
    if user.is_admin {
        Ok(())
    } else {
        Err(api_error(StatusCode::FORBIDDEN, "admin access required"))
    }
}

fn require_local_auth_header(headers: &HeaderMap) -> Result<(), Response> {
    if headers
        .get("X-NasFiles-Request")
        .is_some_and(|value| value == "1")
    {
        Ok(())
    } else {
        Err(api_error(StatusCode::FORBIDDEN, "CSRF header missing"))
    }
}

fn webauthn(state: &AppState) -> Result<&Webauthn, Response> {
    if state.config.disable_passkeys {
        return Err(api_error(StatusCode::NOT_FOUND, "passkeys are disabled"));
    }
    state
        .webauthn
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "passkeys are unavailable"))
}

async fn finish_login(
    state: &AppState,
    session: &tower_sessions::Session,
    mut user: AuthUser,
) -> Result<(), Response> {
    ensure_home_folder(state, &mut user)?;
    let now = now_ms();
    session.cycle_id().await.map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("session error: {e}"),
        )
    })?;
    session
        .insert(LOCAL_AUTH_AT_SESSION, now)
        .await
        .map_err(session_error)?;
    session
        .insert(SESSION_USER, &user)
        .await
        .map_err(session_error)?;
    sqlx::query("UPDATE users SET last_login_at = $1 WHERE id = $2")
        .bind(now)
        .bind(&user.user_id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(())
}

fn ensure_home_folder(state: &AppState, user: &mut AuthUser) -> Result<(), Response> {
    if !user.has_home {
        return Ok(());
    }
    let Some(home_root) = &state.config.home_folder_root else {
        user.has_home = false;
        return Ok(());
    };
    let home_path = home_root.join(user.safe_username());
    if !home_path.exists() {
        std::fs::create_dir_all(&home_path).map_err(internal_error)?;
    }
    Ok(())
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hashing error: {e}"))?;
    Ok(hash.to_string())
}

fn verify_password(stored_hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn setup_password_fingerprint(config: &AppConfig, password: &str) -> anyhow::Result<String> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&config.session_secret)
        .map_err(|e| anyhow::anyhow!("HMAC setup error: {e}"))?;
    mac.update(b"nasfiles.setup-admin-password.v1");
    mac.update(password.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn should_apply_setup_password(current_fingerprint: Option<&str>, next_fingerprint: &str) -> bool {
    matches!(current_fingerprint, Some(current) if current != next_fingerprint)
}

async fn set_user_password(
    pool: &AnyPool,
    user_id: &str,
    password: &str,
    source: &str,
    fingerprint: Option<&str>,
) -> Result<i64, Response> {
    validate_new_password(password)?;
    let hash = hash_password(password).map_err(internal_error)?;
    let changed_at = now_ms();
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = $1,
            setup_password_source = $2,
            setup_password_fingerprint = $3,
            password_changed_at = $4
        WHERE id = $5 AND auth_provider = 'local'
        "#,
    )
    .bind(hash)
    .bind(source)
    .bind(fingerprint)
    .bind(changed_at)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(internal_error)?;
    Ok(changed_at)
}

fn validate_new_password(password: &str) -> Result<(), Response> {
    if let Some(message) = password_policy_error(password) {
        return Err(api_error(StatusCode::BAD_REQUEST, message));
    }
    Ok(())
}

fn validate_setup_admin_password(password: &str) -> anyhow::Result<()> {
    if std::env::var("NASFILES_DEMO_ALLOW_WEAK_ADMIN_PASSWORD")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(message) = password_policy_error(password) {
        anyhow::bail!("SETUP_ADMIN_PASSWORD {message}");
    }
    Ok(())
}

fn password_policy_error(password: &str) -> Option<&'static str> {
    if password.len() < 12 {
        Some("password must be at least 12 characters")
    } else {
        None
    }
}

fn encrypt_secret(secret: &[u8], session_secret: &[u8]) -> Result<String, Response> {
    let cipher = Aes256Gcm::new_from_slice(&session_secret[0..32]).map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("encryption error: {e}"),
        )
    })?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, secret).map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("encryption error: {e}"),
        )
    })?;
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(Base64UrlUnpadded::encode_string(&combined))
}

fn decrypt_secret(encoded: &str, session_secret: &[u8]) -> Result<Vec<u8>, Response> {
    let combined = Base64UrlUnpadded::decode_vec(encoded).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid encrypted secret",
        )
    })?;
    if combined.len() < 12 {
        return Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid encrypted secret",
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(&session_secret[0..32]).map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("encryption error: {e}"),
        )
    })?;
    let nonce_bytes: [u8; 12] = combined[0..12].try_into().map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid encrypted secret",
        )
    })?;
    let nonce = aes_gcm::Nonce::from(nonce_bytes);
    cipher
        .decrypt(&nonce, &combined[12..])
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "secret decrypt failed"))
}

fn build_totp(secret: &[u8], username: &str) -> Result<TOTP, Response> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_vec(),
        Some("NASDrive".to_string()),
        username.to_string(),
    )
    .map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("TOTP error: {e}"),
        )
    })
}

/// Find the TOTP step counter that `code` matches within the accepted skew
/// window, or `None` if it doesn't match.
///
/// The replay guard records the matched counter so a given physical code is
/// usable exactly once. Recording the *wall-clock* step instead (as the earlier
/// code did) let a code valid for the next window be accepted early — recording
/// the current, lower step — and then replayed once real time advanced into that
/// window. Counters are scanned high-to-low so a code maps to a single counter
/// even at a window boundary. The comparison is constant-time.
fn matched_totp_step(totp: &TOTP, code: &str) -> Option<i64> {
    matched_totp_step_at(totp, code, chrono::Utc::now().timestamp())
}

fn matched_totp_step_at(totp: &TOTP, code: &str, now_secs: i64) -> Option<i64> {
    if now_secs < 0 {
        return None;
    }
    let step = totp.step.max(1);
    let base = now_secs as u64 / step;
    let skew = totp.skew as u64;
    let lowest = base.saturating_sub(skew);
    let mut counter = base.saturating_add(skew);
    loop {
        let candidate = totp.generate(counter.saturating_mul(step));
        if bool::from(candidate.as_bytes().ct_eq(code.as_bytes())) {
            return i64::try_from(counter).ok();
        }
        if counter <= lowest {
            return None;
        }
        counter -= 1;
    }
}

async fn verify_user_totp(state: &AppState, user_id: &str, code: &str) -> Result<bool, Response> {
    let row = sqlx::query("SELECT u.username, t.secret_enc FROM local_totp t JOIN users u ON u.id = t.user_id WHERE t.user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "TOTP is not configured"))?;
    let username: String = row.get("username");
    let secret_enc: String = row.get("secret_enc");
    let secret = decrypt_secret(&secret_enc, &state.config.session_secret)?;
    let totp = build_totp(&secret, &username)?;
    let Some(step) = matched_totp_step(&totp, code) else {
        return Ok(false);
    };
    // Replay guard: atomically record the counter the code actually corresponds
    // to (not the wall-clock step). The UPDATE only succeeds if this counter
    // hasn't been used yet, so a code is accepted exactly once across its whole
    // skew window.
    let affected = sqlx::query(
        "UPDATE local_totp SET last_used_step = $1 WHERE user_id = $2 AND (last_used_step IS NULL OR last_used_step < $1)",
    )
    .bind(step)
    .bind(user_id)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?
    .rows_affected();
    if affected == 0 {
        return Ok(false);
    }
    Ok(true)
}

async fn create_trusted_device(
    state: &AppState,
    user_id: &str,
    label: Option<&str>,
) -> Result<serde_json::Value, Response> {
    let username = sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let id = uuid::Uuid::new_v4().to_string();
    let secret = random_bytes(20);
    let totp = build_totp(&secret, &username)?;
    let setup_code = totp.get_secret_base32();
    let hash = trusted_device_hash(&state.config, user_id, &id, &setup_code)?;
    let secret_enc = encrypt_secret(&secret, &state.config.session_secret)?;
    let now = now_ms();
    let expires_at = if state.config.totp_trusted_device_ttl_days == 0 {
        None
    } else {
        Some(now + (state.config.totp_trusted_device_ttl_days as i64) * 24 * 60 * 60 * 1000)
    };
    sqlx::query(
        r#"
        INSERT INTO local_totp_trusted_devices
            (id, user_id, secret_enc, secret_hash, label, created_at, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(&id)
    .bind(user_id)
    .bind(secret_enc)
    .bind(&hash)
    .bind(label.map(str::to_string))
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(json!({
        "id": id,
        "secret": setup_code,
        "hash": hash,
        "label": label,
        "expires_at": expires_at,
    }))
}

async fn verify_trusted_device(
    state: &AppState,
    user_id: &str,
    proof: &TrustedDeviceProof,
) -> Result<bool, Response> {
    let row = sqlx::query(
        r#"
        SELECT secret_enc, secret_hash, expires_at
        FROM local_totp_trusted_devices
        WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(&proof.id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    let Some(row) = row else {
        return Ok(false);
    };
    let expires_at: Option<i64> = row.get("expires_at");
    if expires_at.is_some_and(|exp| exp <= now_ms()) {
        return Ok(false);
    }
    let stored_hash: String = row.get("secret_hash");
    if stored_hash
        .as_bytes()
        .ct_eq(proof.hash.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Ok(false);
    }
    let username = sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let secret_enc: String = row.get("secret_enc");
    let secret = decrypt_secret(&secret_enc, &state.config.session_secret)?;
    let totp = build_totp(&secret, &username)?;
    let Some(step) = matched_totp_step(&totp, &proof.code) else {
        return Ok(false);
    };
    // Replay guard keyed on the matched counter (see verify_user_totp).
    let affected = sqlx::query(
        "UPDATE local_totp_trusted_devices SET last_used_at = $1, last_used_step = $2 WHERE id = $3 AND user_id = $4 AND (last_used_step IS NULL OR last_used_step < $2)",
    )
    .bind(now_ms())
    .bind(step)
    .bind(&proof.id)
    .bind(user_id)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?
    .rows_affected();
    if affected == 0 {
        return Ok(false);
    }
    Ok(true)
}

fn trusted_device_hash(
    config: &AppConfig,
    user_id: &str,
    device_id: &str,
    setup_code: &str,
) -> Result<String, Response> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&config.session_secret).map_err(|e| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("HMAC error: {e}"),
        )
    })?;
    mac.update(b"nasfiles.trusted-totp-device.v1");
    mac.update(user_id.as_bytes());
    mac.update(device_id.as_bytes());
    mac.update(setup_code.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

async fn load_local_user_by_normalized(
    pool: &AnyPool,
    normalized: &str,
) -> anyhow::Result<Option<LocalUserRow>> {
    sqlx::query_as::<_, LocalUserRow>(
        r#"
        SELECT id, external_id, username, display_name, picture_url,
               CASE WHEN is_admin THEN 1 ELSE 0 END AS is_admin,
               CASE WHEN has_home THEN 1 ELSE 0 END AS has_home,
               folder_permissions_json, password_hash, password_changed_at
        FROM users
        WHERE auth_provider = 'local' AND username_normalized = $1
        "#,
    )
    .bind(normalized)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_local_user_row_by_id(
    pool: &AnyPool,
    user_id: &str,
) -> anyhow::Result<Option<LocalUserRow>> {
    sqlx::query_as::<_, LocalUserRow>(
        r#"
        SELECT id, external_id, username, display_name, picture_url,
               CASE WHEN is_admin THEN 1 ELSE 0 END AS is_admin,
               CASE WHEN has_home THEN 1 ELSE 0 END AS has_home,
               folder_permissions_json, password_hash, password_changed_at
        FROM users
        WHERE auth_provider = 'local' AND id = $1
        "#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_local_auth_user(pool: &AnyPool, user_id: &str) -> anyhow::Result<Option<AuthUser>> {
    Ok(load_local_user_row_by_id(pool, user_id)
        .await?
        .map(LocalUserRow::into_auth_user))
}

impl LocalUserRow {
    fn into_auth_user(self) -> AuthUser {
        AuthUser {
            user_id: self.id,
            external_id: self.external_id,
            username: self.username,
            display_name: self.display_name,
            picture_url: self.picture_url,
            folder_permissions: parse_permissions(self.folder_permissions_json.as_deref()),
            has_home: self.has_home != 0,
            is_admin: self.is_admin != 0,
        }
    }
}

fn parse_permissions(json: Option<&str>) -> HashMap<String, FolderCaps> {
    json.and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

fn full_folder_permissions(config: &AppConfig) -> HashMap<String, FolderCaps> {
    config
        .common_folders
        .keys()
        .map(|key| {
            (
                key.clone(),
                FolderCaps {
                    read: true,
                    write: true,
                    share: true,
                },
            )
        })
        .collect()
}

fn sanitize_folder_permissions(
    config: &AppConfig,
    permissions: HashMap<String, FolderCaps>,
) -> HashMap<String, FolderCaps> {
    permissions
        .into_iter()
        .filter(|(key, _)| config.common_folders.contains_key(key))
        .map(|(key, mut caps)| {
            if caps.share {
                caps.read = true;
            }
            (key, caps)
        })
        .collect()
}

async fn active_passkey_count(pool: &AnyPool, user_id: &str) -> Result<i64, Response> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM local_passkeys WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error)
}

async fn user_has_totp(pool: &AnyPool, user_id: &str) -> Result<bool, Response> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM local_totp WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .map_err(internal_error)?
            > 0,
    )
}

async fn passkeys_for_user(pool: &AnyPool, user_id: &str) -> Result<Vec<PasskeyRow>, Response> {
    sqlx::query_as::<_, PasskeyRow>(
        r#"
        SELECT id, credential_json, label, created_at, last_used_at, revoked_at
        FROM local_passkeys
        WHERE user_id = $1 AND revoked_at IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(internal_error)
}

async fn insert_passkey(
    pool: &AnyPool,
    user_id: &str,
    passkey: Passkey,
    label: Option<String>,
) -> Result<(), Response> {
    let id = uuid::Uuid::new_v4().to_string();
    let credential_id = serde_json::to_string(passkey.cred_id()).map_err(internal_error)?;
    let credential_json = serde_json::to_string(&passkey).map_err(internal_error)?;
    sqlx::query(
        r#"
        INSERT INTO local_passkeys
            (id, user_id, credential_id, credential_json, label, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(credential_id)
    .bind(credential_json)
    .bind(label)
    .bind(now_ms())
    .execute(pool)
    .await
    .map_err(internal_error)?;
    Ok(())
}

async fn public_passkeys_for_user(
    pool: &AnyPool,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, Response> {
    let rows = sqlx::query_as::<_, PasskeyRow>(
        r#"
        SELECT id, credential_json, label, created_at, last_used_at, revoked_at
        FROM local_passkeys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(internal_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "label": row.label,
                "created_at": row.created_at,
                "last_used_at": row.last_used_at,
                "revoked_at": row.revoked_at,
            })
        })
        .collect())
}

async fn revoke_passkey_for_user(pool: &AnyPool, user_id: &str, id: &str) -> Result<(), Response> {
    let result = sqlx::query(
        "UPDATE local_passkeys SET revoked_at = $1 WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(now_ms())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(api_error(StatusCode::NOT_FOUND, "passkey not found"));
    }
    Ok(())
}

async fn trusted_devices_for_user(
    pool: &AnyPool,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, Response> {
    let rows = sqlx::query(
        r#"
        SELECT id, label, created_at, last_used_at, expires_at, revoked_at
        FROM local_totp_trusted_devices
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(internal_error)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get::<String, _>("id"),
                "label": row.get::<Option<String>, _>("label"),
                "created_at": row.get::<i64, _>("created_at"),
                "last_used_at": row.get::<Option<i64>, _>("last_used_at"),
                "expires_at": row.get::<Option<i64>, _>("expires_at"),
                "revoked_at": row.get::<Option<i64>, _>("revoked_at"),
            })
        })
        .collect())
}

async fn revoke_trusted_device_for_user(
    pool: &AnyPool,
    user_id: &str,
    id: &str,
) -> Result<(), Response> {
    let result = sqlx::query(
        "UPDATE local_totp_trusted_devices SET revoked_at = $1 WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL",
    )
    .bind(now_ms())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(api_error(StatusCode::NOT_FOUND, "trusted device not found"));
    }
    Ok(())
}

/// Max failed login attempts for a single username within the window before
/// the username is throttled.
const LOGIN_RATE_LIMIT_PER_USERNAME: i64 = 10;
/// Max failed login attempts from a single client IP (across all usernames)
/// within the window before that IP is throttled. Higher than the per-username
/// limit so a shared/NAT'd source is not locked out by a few users, but low
/// enough to throttle password-spraying across many usernames from one source.
const LOGIN_RATE_LIMIT_PER_IP: i64 = 30;
/// Rolling window for login rate limiting, in milliseconds.
const LOGIN_RATE_LIMIT_WINDOW_MS: i64 = 5 * 60 * 1000;

async fn is_login_rate_limited(pool: &AnyPool, normalized: &str) -> bool {
    let since = now_ms() - LOGIN_RATE_LIMIT_WINDOW_MS;
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM local_auth_attempts WHERE username_normalized = $1 AND success = FALSE AND occurred_at >= $2",
    )
    .bind(normalized)
    .bind(since)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        >= LOGIN_RATE_LIMIT_PER_USERNAME
}

/// Throttle by client IP so password-spraying across many usernames from one
/// source is caught even though each individual username stays under its own
/// limit. Skips throttling when no client IP can be determined (e.g. no trusted
/// proxy headers), matching `record_attempt`'s best-effort IP capture.
async fn is_login_rate_limited_by_ip(pool: &AnyPool, ip: Option<&str>) -> bool {
    let Some(ip) = ip else {
        return false;
    };
    let since = now_ms() - LOGIN_RATE_LIMIT_WINDOW_MS;
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM local_auth_attempts WHERE ip = $1 AND success = FALSE AND occurred_at >= $2",
    )
    .bind(ip)
    .bind(since)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        >= LOGIN_RATE_LIMIT_PER_IP
}

async fn record_attempt(
    pool: &AnyPool,
    normalized: &str,
    ip: Option<&str>,
    success: bool,
    reason: Option<&str>,
) {
    let _ = sqlx::query(
        r#"
        INSERT INTO local_auth_attempts
            (id, username_normalized, ip, occurred_at, success, reason)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(normalized)
    .bind(ip)
    .bind(now_ms())
    .bind(success)
    .bind(reason)
    .execute(pool)
    .await;
}

fn normalize_username(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::fill(&mut bytes[..]);
    bytes
}

fn random_token(len: usize) -> String {
    Base64UrlUnpadded::encode_string(&random_bytes(len))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn generate_xkcd_password() -> String {
    const WORDS: &[&str] = &[
        "amber", "anchor", "apricot", "atlas", "baker", "beacon", "bison", "breeze", "cactus",
        "canyon", "cedar", "cobalt", "comet", "copper", "coral", "cotton", "delta", "denim",
        "drift", "ember", "falcon", "fern", "flannel", "frost", "galaxy", "garden", "ginger",
        "glacier", "harbor", "hazel", "honey", "indigo", "island", "jacket", "jasper", "juniper",
        "kettle", "lagoon", "lantern", "lemon", "linen", "magnet", "maple", "marble", "meadow",
        "melon", "meteor", "mint", "monsoon", "mosaic", "nectar", "nickel", "olive", "onyx",
        "orchid", "otter", "pepper", "picket", "plasma", "prairie", "quartz", "radar", "raven",
        "river", "rocket", "saffron", "shadow", "signal", "silver", "summit", "sunset", "tango",
        "timber", "topaz", "tundra", "velvet", "violet", "walnut", "willow", "winter",
    ];
    let mut tokens = Vec::with_capacity(4);
    for _ in 0..3 {
        let idx = random_index(WORDS.len());
        tokens.push(WORDS[idx].to_string());
    }
    tokens.push(random_index(10_000).to_string());

    for i in (1..tokens.len()).rev() {
        let j = random_index(i + 1);
        tokens.swap(i, j);
    }
    tokens.join("-")
}

fn random_index(upper: usize) -> usize {
    let mut bytes = [0u8; 8];
    rand::fill(&mut bytes);
    (u64::from_le_bytes(bytes) as usize) % upper
}

fn api_error(status: StatusCode, msg: impl ToString) -> Response {
    (status, Json(json!({"error": msg.to_string()}))).into_response()
}

fn internal_error(err: impl std::fmt::Display) -> Response {
    tracing::error!("local auth error: {err}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

fn session_error(err: tower_sessions::session::Error) -> Response {
    internal_error(format!("session error: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn generated_password_has_four_hyphenated_parts() {
        let password = generate_xkcd_password();
        let parts: Vec<_> = password.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(
            parts.iter().filter(|p| p.parse::<usize>().is_ok()).count(),
            1
        );
    }

    #[test]
    fn password_hash_round_trips() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(verify_password(&hash, "correct horse battery"));
        assert!(!verify_password(&hash, "wrong"));
    }

    fn test_totp() -> TOTP {
        // Same shape as build_totp: SHA1, 6 digits, skew=1, 30s step.
        TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            b"12345678901234567890".to_vec(),
            Some("NASDrive".to_string()),
            "user".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn totp_match_records_the_codes_own_counter_not_wallclock_step() {
        let totp = test_totp();
        let now = 1_700_000_000i64;
        let base = now as u64 / 30;
        // A code valid for the NEXT window (base+1), submitted early while the
        // wall clock is still in `base` (allowed by skew=1). The recorded step
        // must be base+1 — the code's real counter — otherwise the same code is
        // replayable once real time crosses into base+1.
        let next_code = totp.generate((base + 1) * 30);
        assert_eq!(
            matched_totp_step_at(&totp, &next_code, now),
            i64::try_from(base + 1).ok(),
        );
    }

    #[test]
    fn totp_match_reproduces_the_submitted_code() {
        let totp = test_totp();
        let now = 1_700_000_000i64;
        let base = now as u64 / 30;
        let code = totp.generate(base * 30);
        let matched = matched_totp_step_at(&totp, &code, now).expect("current code matches");
        // The matched counter regenerates the exact submitted code, and lies
        // within the accepted ±1 skew window.
        assert_eq!(totp.generate(matched as u64 * 30), code);
        assert!((matched - base as i64).abs() <= 1);
    }

    #[test]
    fn totp_rejects_code_outside_skew_window() {
        let totp = test_totp();
        let now = 1_700_000_000i64;
        let base = now as u64 / 30;
        // A code five windows away is well outside ±1 skew and must not match.
        let stale = totp.generate((base + 5) * 30);
        assert_eq!(matched_totp_step_at(&totp, &stale, now), None);
    }

    #[test]
    fn setup_password_fingerprint_detects_password_changes() {
        let config = test_config();
        let first = setup_password_fingerprint(&config, "first-password").unwrap();
        let first_again = setup_password_fingerprint(&config, "first-password").unwrap();
        let second = setup_password_fingerprint(&config, "second-password").unwrap();
        assert_eq!(first, first_again);
        assert_ne!(first, second);
    }

    #[test]
    fn setup_password_update_skips_user_changed_passwords() {
        assert!(!should_apply_setup_password(None, "new-env-fingerprint"));
        assert!(!should_apply_setup_password(
            Some("new-env-fingerprint"),
            "new-env-fingerprint"
        ));
        assert!(should_apply_setup_password(
            Some("old-env-fingerprint"),
            "new-env-fingerprint"
        ));
    }

    #[test]
    fn password_policy_applies_to_setup_admin_passwords() {
        assert_eq!(
            password_policy_error("short"),
            Some("password must be at least 12 characters")
        );
        assert!(validate_setup_admin_password("long-enough-password").is_ok());
    }

    #[test]
    fn local_auth_header_is_required() {
        let empty = HeaderMap::new();
        assert!(require_local_auth_header(&empty).is_err());

        let mut headers = HeaderMap::new();
        headers.insert("X-NasFiles-Request", "1".parse().unwrap());
        assert!(require_local_auth_header(&headers).is_ok());
    }

    #[test]
    fn encrypted_secrets_round_trip() {
        let config = test_config();
        let secret = b"totp secret bytes";
        let encrypted = encrypt_secret(secret, &config.session_secret).unwrap();
        assert!(!encrypted.contains("totp"));
        assert_eq!(
            decrypt_secret(&encrypted, &config.session_secret).unwrap(),
            secret
        );
    }

    #[test]
    fn trusted_device_hash_binds_user_device_and_secret() {
        let config = test_config();
        let hash = trusted_device_hash(&config, "user-1", "device-1", "setup").unwrap();
        assert_eq!(
            hash,
            trusted_device_hash(&config, "user-1", "device-1", "setup").unwrap()
        );
        assert_ne!(
            hash,
            trusted_device_hash(&config, "user-2", "device-1", "setup").unwrap()
        );
        assert_ne!(
            hash,
            trusted_device_hash(&config, "user-1", "device-2", "setup").unwrap()
        );
        assert_ne!(
            hash,
            trusted_device_hash(&config, "user-1", "device-1", "changed").unwrap()
        );
    }

    #[test]
    fn sanitize_permissions_ignores_unknown_roots_and_share_implies_read() {
        let config = test_config();
        let mut input = HashMap::new();
        input.insert(
            "docs".to_string(),
            FolderCaps {
                read: false,
                write: false,
                share: true,
            },
        );
        input.insert(
            "unknown".to_string(),
            FolderCaps {
                read: true,
                write: true,
                share: true,
            },
        );
        let sanitized = sanitize_folder_permissions(&config, input);
        assert_eq!(sanitized.len(), 1);
        let docs = sanitized.get("docs").unwrap();
        assert!(docs.read);
        assert!(docs.share);
        assert!(!sanitized.contains_key("unknown"));
    }

    fn test_config() -> AppConfig {
        let mut common_folders = HashMap::new();
        common_folders.insert("docs".to_string(), PathBuf::from("/docs"));
        common_folders.insert("media".to_string(), PathBuf::from("/media"));
        AppConfig {
            bind_addr: String::new(),
            base_url: "http://localhost:3000".into(),
            session_secret: vec![7; 32],
            data_dir: PathBuf::new(),
            dev_mode: true,
            auth_mode: AuthMode::Local,
            no_server_side_execution: false,
            csp_extra_img_src: Vec::new(),
            csp_extra_media_src: Vec::new(),
            db_url: String::new(),
            common_folders,
            home_folder_root: Some(PathBuf::from("/home")),
            share_group_of_folder: HashMap::new(),
            oidc: None,
            sso_username_claim: String::new(),
            sso_display_name_claim: String::new(),
            sso_picture_claim: String::new(),
            sso_groups_claim: String::new(),
            group_folder_caps: HashMap::new(),
            default_folder_caps: HashMap::new(),
            admin_groups: Vec::new(),
            personal_folder_groups: None,
            groups_refresh_interval_secs: 0,
            dev_user: None,
            disable_passkeys: false,
            disable_totp: false,
            setup_admin: None,
            totp_trusted_device_ttl_days: 0,
            thumbnail_cache_dir: PathBuf::new(),
            thumbnail_max_source_file_size: 0,
            thumbnail_max_image_width: 0,
            thumbnail_max_image_height: 0,
            thumbnail_max_image_alloc: 0,
            thumbnail_max_concurrent_generations: 1,
            media_preview_max_concurrent_transcodes: 1,
            search_max_results: 100,
            search_live_entry_budget: 25_000,
            search_live_time_budget_ms: 1_500,
            search_reindex_interval_secs: 300,
            search_full_reindex_interval_secs: 0,
            search_disk_state_file: None,
            search_hdd_pools: Default::default(),
            share_token_bytes: 24,
            sftp_enabled: false,
            sftp_bind_addr: String::new(),
            sftp_host_key_path: PathBuf::new(),
            max_upload_file_size: 0,
            max_upload_request_size: 0,
            log_level: String::new(),
            trusted_proxy_depth: 1,
            custom_links: Vec::new(),
            sftp_public_port: None,
        }
    }
}
