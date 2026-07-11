use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
};
use serde::Deserialize;
use tokio::sync::OnceCell;

use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use base64ct::{Base64UrlUnpadded, Encoding};

use crate::config::{self, AppConfig};
use crate::state::AppState;

/// Helper to encrypt OIDC tokens for DB storage
#[allow(deprecated)]
pub fn encrypt_token(token: &str, secret: &[u8]) -> Result<String, AppError> {
    if token.is_empty() {
        return Ok(String::new());
    }
    let key = Key::<Aes256Gcm>::from_slice(&secret[0..32]);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits

    let ciphertext = cipher
        .encrypt(&nonce, token.as_bytes())
        .map_err(|e| AppError::Internal(format!("encryption error: {e}")))?;

    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(Base64UrlUnpadded::encode_string(&combined))
}

/// Helper to decrypt OIDC tokens from DB storage
#[allow(deprecated)]
pub fn decrypt_token(encoded: &str, secret: &[u8]) -> Result<String, AppError> {
    if encoded.is_empty() {
        return Ok(String::new());
    }
    let combined = Base64UrlUnpadded::decode_vec(encoded)
        .map_err(|e| AppError::Internal(format!("base64 error: {e}")))?;

    if combined.len() < 12 {
        return Err(AppError::Internal("invalid encrypted token".into()));
    }

    let key = Key::<Aes256Gcm>::from_slice(&secret[0..32]);
    let cipher = Aes256Gcm::new(key);
    let nonce = aes_gcm::Nonce::from_slice(&combined[0..12]);
    let ciphertext = &combined[12..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| AppError::Internal(format!("decryption error: {e}")))?;

    String::from_utf8(plaintext).map_err(|e| AppError::Internal(format!("utf8 error: {e}")))
}

/// Concrete type for the configured OIDC client.
/// After from_provider_metadata + set_redirect_uri, these are the typestate params.
type ConfiguredClient = openidconnect::Client<
    openidconnect::EmptyAdditionalClaims,
    openidconnect::core::CoreAuthDisplay,
    openidconnect::core::CoreGenderClaim,
    openidconnect::core::CoreJweContentEncryptionAlgorithm,
    openidconnect::core::CoreJsonWebKey,
    openidconnect::core::CoreAuthPrompt,
    openidconnect::StandardErrorResponse<openidconnect::core::CoreErrorResponseType>,
    openidconnect::StandardTokenResponse<
        openidconnect::IdTokenFields<
            openidconnect::EmptyAdditionalClaims,
            openidconnect::EmptyExtraTokenFields,
            openidconnect::core::CoreGenderClaim,
            openidconnect::core::CoreJweContentEncryptionAlgorithm,
            openidconnect::core::CoreJwsSigningAlgorithm,
        >,
        openidconnect::core::CoreTokenType,
    >,
    openidconnect::StandardTokenIntrospectionResponse<
        openidconnect::EmptyExtraTokenFields,
        openidconnect::core::CoreTokenType,
    >,
    openidconnect::core::CoreRevocableToken,
    openidconnect::StandardErrorResponse<openidconnect::RevocationErrorResponseType>,
    EndpointSet,      // auth_url
    EndpointNotSet,   // device_authorization
    EndpointNotSet,   // introspection
    EndpointNotSet,   // revocation
    EndpointMaybeSet, // token_url (from provider metadata — may or may not be set)
    EndpointMaybeSet, // redirect_url (we set it but it goes through MaybeSet)
>;

pub struct OidcState {
    pub client: ConfiguredClient,
    pub userinfo_url: reqwest::Url,
}

/// Lazy-initialized OIDC client. Created on first use after server start.
pub static OIDC_CLIENT: OnceCell<OidcState> = OnceCell::const_new();

/// Initialize the OIDC client by fetching provider metadata.
pub async fn init_oidc_client(config: &AppConfig) -> anyhow::Result<()> {
    let oidc_config = config
        .oidc
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("OIDC not configured"))?;

    let issuer_url = IssuerUrl::new(oidc_config.issuer_url.clone())
        .map_err(|e| anyhow::anyhow!("Invalid OIDC issuer URL: {e}"))?;

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

    let provider_metadata = CoreProviderMetadata::discover_async(issuer_url, &http_client)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to discover OIDC provider: {e}"))?;

    let redirect_url = RedirectUrl::new(format!("{}/auth/oidc/callback", config.base_url))
        .map_err(|e| anyhow::anyhow!("Invalid redirect URL: {e}"))?;

    let userinfo_url = provider_metadata
        .userinfo_endpoint()
        .map(|ep| ep.url().clone())
        .ok_or_else(|| anyhow::anyhow!("OIDC provider lacks userinfo endpoint"))?;

    let client = CoreClient::from_provider_metadata(
        provider_metadata,
        ClientId::new(oidc_config.client_id.clone()),
        Some(ClientSecret::new(oidc_config.client_secret.clone())),
    )
    .set_redirect_uri(redirect_url);

    OIDC_CLIENT
        .set(OidcState {
            client,
            userinfo_url,
        })
        .map_err(|_| anyhow::anyhow!("OIDC client already initialized"))?;

    tracing::info!("OIDC client initialized for {}", oidc_config.issuer_url);
    Ok(())
}

/// GET /auth/oidc/login — Redirect to the OIDC provider for authentication.
pub async fn login(
    State(_state): State<AppState>,
    session: tower_sessions::Session,
) -> Result<Response, AppError> {
    let oidc_state = OIDC_CLIENT
        .get()
        .ok_or_else(|| AppError::Internal("OIDC not configured".into()))?;
    let client = &oidc_state.client;

    // Generate PKCE challenge
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate authorization URL with PKCE
    let mut auth_request = client.authorize_url(
        AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
        CsrfToken::new_random,
        Nonce::new_random,
    );
    auth_request = auth_request.add_scope(Scope::new("openid".to_string()));
    auth_request = auth_request.add_scope(Scope::new("profile".to_string()));
    auth_request = auth_request.add_scope(Scope::new("email".to_string()));
    auth_request = auth_request.add_scope(Scope::new("offline_access".to_string()));
    auth_request = auth_request.set_pkce_challenge(pkce_challenge);

    let (auth_url, csrf_token, nonce) = auth_request.url();

    // Store PKCE verifier, CSRF token, and nonce in session
    session
        .insert("oidc_pkce_verifier", pkce_verifier.secret().clone())
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;
    session
        .insert("oidc_csrf_token", csrf_token.secret().clone())
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;
    session
        .insert("oidc_nonce", nonce.secret().clone())
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;

    Ok(Redirect::temporary(auth_url.as_str()).into_response())
}

#[derive(Deserialize)]
pub struct OidcCallback {
    code: String,
    state: String,
}

/// GET /auth/oidc/callback — Handle the OIDC provider's redirect.
pub async fn callback(
    State(state): State<AppState>,
    session: tower_sessions::Session,
    Query(params): Query<OidcCallback>,
) -> Result<Response, AppError> {
    let oidc_state = OIDC_CLIENT
        .get()
        .ok_or_else(|| AppError::Internal("OIDC not configured".into()))?;
    let client = &oidc_state.client;

    // Verify CSRF / state
    let stored_csrf: String = session
        .get("oidc_csrf_token")
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?
        .ok_or_else(|| AppError::Internal("missing CSRF token in session".into()))?;

    if params.state != stored_csrf {
        return Err(AppError::Auth("CSRF token mismatch".into()));
    }

    // Retrieve PKCE verifier
    let pkce_verifier_secret: String = session
        .get("oidc_pkce_verifier")
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?
        .ok_or_else(|| AppError::Internal("missing PKCE verifier in session".into()))?;
    let pkce_verifier = PkceCodeVerifier::new(pkce_verifier_secret);

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("HTTP client error: {e}")))?;

    // Exchange authorization code for tokens
    let token_response = client
        .exchange_code(AuthorizationCode::new(params.code.clone()))
        .map_err(|e| AppError::Internal(format!("exchange_code config error: {e}")))?
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| AppError::Internal(format!("token exchange failed: {e}")))?;

    // Verify and extract ID token claims
    let stored_nonce: String = session
        .get("oidc_nonce")
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?
        .ok_or_else(|| AppError::Internal("missing nonce in session".into()))?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| AppError::Internal("no ID token in response".into()))?;

    let nonce = Nonce::new(stored_nonce);

    // Verify the ID token.
    // Zitadel places the project ID (not the client ID) in the `aud` claim,
    // so we configure extra trusted audiences from SSO_OIDC_EXTRA_AUDIENCES.
    let oidc_config = state
        .config
        .oidc
        .as_ref()
        .ok_or_else(|| AppError::Internal("OIDC not configured".into()))?;
    let extra_audiences: Vec<openidconnect::Audience> = oidc_config
        .additional_audiences
        .iter()
        .map(|a| openidconnect::Audience::new(a.clone()))
        .collect();
    let verifier = client
        .id_token_verifier()
        .set_other_audience_verifier_fn(move |aud| extra_audiences.iter().any(|a| a == aud));
    let claims = id_token
        .claims(&verifier, &nonce)
        .map_err(|e| AppError::Internal(format!("ID token verification failed: {e}")))?;

    // Extract user info from claims
    let subject = claims.subject().to_string();
    let issuer = claims.issuer().to_string();
    let external_id = format!("{issuer}:{subject}");

    // Get ALL claims from the raw JWT payload (not just the standard OIDC ones).
    // EmptyAdditionalClaims drops custom claims like Zitadel's role URN,
    // so we decode the JWT payload directly.
    let extra_claims: serde_json::Value = {
        let token_str = id_token.to_string();
        let parts: Vec<&str> = token_str.split('.').collect();
        if parts.len() >= 2 {
            // Decode base64url-encoded JWT payload
            use base64ct::{Base64UrlUnpadded, Encoding};
            let mut buf = vec![0u8; parts[1].len()];
            Base64UrlUnpadded::decode(parts[1].as_bytes(), &mut buf)
                .ok()
                .and_then(|decoded| serde_json::from_slice(decoded).ok())
                .unwrap_or_default()
        } else {
            serde_json::to_value(claims.additional_claims()).unwrap_or_default()
        }
    };

    tracing::debug!(
        "ID token claims keys: {:?}",
        extra_claims
            .as_object()
            .map(|o| o.keys().collect::<Vec<_>>())
    );

    let username = extract_claim(&extra_claims, &state.config.sso_username_claim)
        .or_else(|| claims.preferred_username().map(|u| u.to_string()))
        .unwrap_or_else(|| subject.clone());

    // Reject usernames that would not map injectively to a home directory.
    // The home path is keyed on `sanitize_username`, which collapses '/', '\\',
    // and '..'; allowing those characters would let two distinct identities
    // share one home folder. Deny rather than silently sanitize so the
    // collision can never occur.
    if let Err(reason) = nasfiles_core::models::validate_username(&username) {
        tracing::warn!(
            user = %username,
            external_id = %external_id,
            "User rejected at login: unsafe username ({reason})"
        );
        let msg =
            format!("Your account username is not allowed ({reason}). Contact your administrator.");
        return Ok((
            axum::http::StatusCode::FORBIDDEN,
            axum::response::Html(format!("<h1>Access Denied</h1><p>{}</p>", msg)),
        )
            .into_response());
    }

    let display_name = extract_claim(&extra_claims, &state.config.sso_display_name_claim)
        .or_else(|| {
            claims
                .name()
                .and_then(|n| n.get(None).map(|ln| ln.to_string()))
        })
        .unwrap_or_else(|| username.clone());

    let picture_url = extract_claim(&extra_claims, &state.config.sso_picture_claim).or_else(|| {
        claims
            .picture()
            .and_then(|p| p.get(None).map(|lp| lp.to_string()))
    });

    let groups: Vec<String> =
        extract_claim_array(&extra_claims, &state.config.sso_groups_claim).unwrap_or_default();

    // Compute permissions
    let folder_permissions = config::compute_folder_permissions(&state.config, &groups);
    let is_admin = config::is_admin(&state.config, &groups);

    // Check if home folder exists / should be created
    let has_home = if config::personal_folder_allowed(&state.config, &groups) {
        if let Some(ref home_root) = state.config.home_folder_root {
            let safe_username = nasfiles_core::models::AuthUser::sanitize_username(&username);
            let home_path = home_root.join(&safe_username);
            if !home_path.exists() {
                if let Err(e) = std::fs::create_dir_all(&home_path) {
                    tracing::warn!("Failed to create home folder for {username}: {e}");
                    false
                } else {
                    tracing::info!(
                        "Created home folder for {username} at {}",
                        home_path.display()
                    );
                    true
                }
            } else {
                true
            }
        } else {
            false
        }
    } else {
        false
    };

    let effectively_no_access = {
        let has_any_readable = folder_permissions.values().any(|c| c.read);
        !has_any_readable && !has_home && !is_admin
    };

    if effectively_no_access {
        tracing::warn!(
            user = %username,
            external_id = %external_id,
            groups = ?groups,
            "User rejected at login: effectively no access"
        );

        // The login itself completed and the verified token contains no usable
        // access. Unlike a stale/missing session token, this is authoritative.
        if let Ok(Some((existing_user_id,))) =
            sqlx::query_as::<_, (String,)>("SELECT id FROM users WHERE external_id = $1")
                .bind(&external_id)
                .fetch_optional(&state.pool)
                .await
            && let Err(e) = super::share_reconcile::reconcile_authoritative_permissions(
                &state.pool,
                &existing_user_id,
                &folder_permissions,
                "interactive_login",
            )
            .await
        {
            tracing::error!(
                user_id = %existing_user_id,
                "Failed to reconcile shares after authoritative no-access login: {e}"
            );
        }

        let msg = "Your account has no access to nasfiles. Contact your administrator.";
        return Ok((
            axum::http::StatusCode::FORBIDDEN,
            axum::response::Html(format!("<h1>Access Denied</h1><p>{}</p>", msg)),
        )
            .into_response());
    }

    // Prepare tokens for storage
    let access_token = token_response.access_token().secret().clone();
    let refresh_token = token_response.refresh_token().map(|t| t.secret().clone());

    let enc_access = encrypt_token(&access_token, &state.config.session_secret)?;
    let enc_refresh = match refresh_token.as_ref() {
        Some(rt) => Some(encrypt_token(rt, &state.config.session_secret)?),
        None => None,
    };

    // Upsert user in database
    let now = chrono::Utc::now().timestamp_millis();
    let user_id = upsert_user(
        &state.pool,
        &external_id,
        &username,
        &display_name,
        picture_url.as_deref(),
        is_admin,
        &folder_permissions,
        has_home,
        enc_access,
        enc_refresh,
        now,
    )
    .await?;

    // This completed authorization-code login and its verified ID token are an
    // authoritative permission snapshot. Restore still-valid shares when
    // access returned, and revoke only roots the verified claims no longer grant.
    super::share_reconcile::reconcile_authoritative_permissions(
        &state.pool,
        &user_id,
        &folder_permissions,
        "interactive_login",
    )
    .await
    .map_err(|e| AppError::Internal(format!("share reconciliation error: {e}")))?;

    // Session fixation prevention
    session
        .cycle_id()
        .await
        .map_err(|e| AppError::Internal(format!("session cycle error: {e}")))?;

    // Clean up OIDC temporary session data
    session.remove::<String>("oidc_pkce_verifier").await.ok();
    session.remove::<String>("oidc_csrf_token").await.ok();
    session.remove::<String>("oidc_nonce").await.ok();

    // Store AuthUser in session
    let auth_user = nasfiles_core::models::AuthUser {
        user_id,
        external_id,
        username,
        display_name,
        picture_url,
        folder_permissions,
        has_home,
        is_admin,
    };

    session
        .insert("oidc_access_token", &access_token)
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;
    if let Some(rt) = refresh_token {
        session
            .insert("oidc_refresh_token", &rt)
            .await
            .map_err(|e| AppError::Internal(format!("session error: {e}")))?;
    }
    session
        .insert("oidc_groups_refreshed_at", now / 1000)
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;

    session
        .insert("user", &auth_user)
        .await
        .map_err(|e| AppError::Internal(format!("session error: {e}")))?;

    tracing::info!(
        user = %auth_user.username,
        folders = ?auth_user.folder_permissions,
        admin = auth_user.is_admin,
        "User logged in via OIDC"
    );

    Ok(Redirect::temporary("/").into_response())
}

/// Upsert a user in the database, returning their user ID.
#[allow(clippy::too_many_arguments)]
async fn upsert_user(
    pool: &sqlx::AnyPool,
    external_id: &str,
    username: &str,
    display_name: &str,
    picture_url: Option<&str>,
    is_admin: bool,
    folder_permissions: &std::collections::HashMap<String, nasfiles_core::models::FolderCaps>,
    has_home: bool,
    enc_access: String,
    enc_refresh: Option<String>,
    now: i64,
) -> Result<String, AppError> {
    let folder_permissions_json = serde_json::to_string(folder_permissions)
        .map_err(|e| AppError::Internal(format!("permission serialization error: {e}")))?;
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE external_id = $1")
        .bind(external_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(format!("db error: {e}")))?;

    if let Some((id,)) = existing {
        sqlx::query(
            "UPDATE users SET username = $1, display_name = $2, picture_url = $3, is_admin = $4, folder_permissions_json = $5, has_home = $6, oidc_access_token = $7, oidc_refresh_token = $8, last_login_at = $9, auth_provider = 'oidc' WHERE id = $10"
        )
        .bind(username)
        .bind(display_name)
        .bind(picture_url)
        .bind(is_admin)
        .bind(&folder_permissions_json)
        .bind(has_home)
        .bind(&enc_access)
        .bind(&enc_refresh)
        .bind(now)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(format!("db error: {e}")))?;
        Ok(id)
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO users (id, external_id, username, display_name, picture_url, is_admin, folder_permissions_json, has_home, oidc_access_token, oidc_refresh_token, auth_provider, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'oidc', $11, $12)"
        )
        .bind(&id)
        .bind(external_id)
        .bind(username)
        .bind(display_name)
        .bind(picture_url)
        .bind(is_admin)
        .bind(&folder_permissions_json)
        .bind(has_home)
        .bind(&enc_access)
        .bind(&enc_refresh)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(format!("db error: {e}")))?;
        Ok(id)
    }
}

pub fn extract_claim(claims: &serde_json::Value, claim_name: &str) -> Option<String> {
    claims
        .get(claim_name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn extract_claim_array(claims: &serde_json::Value, claim_name: &str) -> Option<Vec<String>> {
    claims.get(claim_name).and_then(|v| {
        // Standard array format: ["group1", "group2"]
        v.as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
            // Zitadel project role format: {"role_key": {"org_id": "org_domain"}, ...}
            // Extract the role keys as group names.
            .or_else(|| v.as_object().map(|obj| obj.keys().cloned().collect()))
    })
}

/// Error type for auth operations.
#[derive(Debug)]
pub enum AppError {
    Auth(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Auth(msg) => {
                tracing::warn!("Auth error: {msg}");
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": msg})),
                )
                    .into_response()
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {msg}");
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "internal server error"})),
                )
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_token() {
        let secret = b"01234567890123456789012345678901"; // 32 bytes
        let token = "my_super_secret_refresh_token_12345";

        let encrypted = encrypt_token(token, secret).unwrap();
        assert_ne!(encrypted, token);
        assert!(!encrypted.is_empty());

        let decrypted = decrypt_token(&encrypted, secret).unwrap();
        assert_eq!(decrypted, token);
    }

    #[test]
    fn test_encrypt_decrypt_empty() {
        let secret = b"01234567890123456789012345678901";

        let encrypted = encrypt_token("", secret).unwrap();
        assert_eq!(encrypted, "");

        let decrypted = decrypt_token(&encrypted, secret).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_decrypt_invalid() {
        let secret = b"01234567890123456789012345678901";

        assert!(decrypt_token("invalid_base64!", secret).is_err());

        use base64ct::{Base64UrlUnpadded, Encoding};
        let short_data = Base64UrlUnpadded::encode_string(b"short");
        assert!(decrypt_token(&short_data, secret).is_err());
    }
}
