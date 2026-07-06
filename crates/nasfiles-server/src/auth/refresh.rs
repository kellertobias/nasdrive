#![allow(clippy::collapsible_if)]
use crate::config;
use crate::state::AppState;
use nasfiles_core::models::AuthUser;
use openidconnect::{OAuth2TokenResponse, RefreshToken};
use serde_json::Value;

pub enum RefreshOutcome {
    NoAccess,
    Expired,
}

pub async fn maybe_refresh_groups(
    state: &AppState,
    session: &tower_sessions::Session,
) -> Result<AuthUser, RefreshOutcome> {
    let mut user: AuthUser = match session.get("user").await {
        Ok(Some(u)) => u,
        _ => return Err(RefreshOutcome::Expired),
    };

    let interval = state.config.groups_refresh_interval_secs;
    if interval == 0 {
        return Ok(user);
    }

    let refreshed_at: i64 = session
        .get("oidc_groups_refreshed_at")
        .await
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let now = chrono::Utc::now().timestamp();

    if now - refreshed_at < interval as i64 {
        return Ok(user);
    }

    // Attempt refresh
    let oidc_state = match super::oidc::OIDC_CLIENT.get() {
        Some(s) => s,
        None => return Ok(user), // OIDC not configured, shouldn't happen but ignore
    };

    let mut access_token: String = match session.get("oidc_access_token").await {
        Ok(Some(t)) => t,
        _ => return Err(RefreshOutcome::Expired),
    };

    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();

    // GET userinfo
    let resp = http_client
        .get(oidc_state.userinfo_url.clone())
        .bearer_auth(&access_token)
        .send()
        .await;

    let mut userinfo_success = false;
    let mut userinfo_json = Value::Null;

    if let Ok(r) = resp {
        if r.status().is_success() {
            if let Ok(json) = r.json::<Value>().await {
                userinfo_json = json;
                userinfo_success = true;
            }
        }
    }

    if !userinfo_success {
        // Try refresh token if we got a 401
        let refresh_token: Option<String> = session.get("oidc_refresh_token").await.unwrap_or(None);
        if let Some(rt_str) = refresh_token {
            let rt = RefreshToken::new(rt_str);
            let token_req = match oidc_state.client.exchange_refresh_token(&rt) {
                Ok(req) => req,
                Err(_) => {
                    let _ = session.clear().await;
                    return Err(RefreshOutcome::Expired);
                }
            };
            let token_resp = token_req.request_async(&http_client).await;

            if let Ok(token_resp) = token_resp {
                access_token = token_resp.access_token().secret().clone();
                let new_refresh = token_resp.refresh_token().map(|t| t.secret().clone());

                // Update session
                let _ = session.insert("oidc_access_token", &access_token).await;
                if let Some(new_rt) = &new_refresh {
                    let _ = session.insert("oidc_refresh_token", new_rt).await;
                }

                // Update DB with encrypted tokens
                let enc_access =
                    super::oidc::encrypt_token(&access_token, &state.config.session_secret)
                        .unwrap_or_default();
                let enc_refresh = new_refresh
                    .as_deref()
                    .and_then(|t| super::oidc::encrypt_token(t, &state.config.session_secret).ok());

                let _ = sqlx::query("UPDATE users SET oidc_access_token = $1, oidc_refresh_token = COALESCE($2, oidc_refresh_token) WHERE id = $3")
                    .bind(&enc_access)
                    .bind(enc_refresh)
                    .bind(&user.user_id)
                    .execute(&state.pool)
                    .await;

                // Try userinfo again
                if let Ok(r) = http_client
                    .get(oidc_state.userinfo_url.clone())
                    .bearer_auth(&access_token)
                    .send()
                    .await
                {
                    if r.status().is_success() {
                        if let Ok(json) = r.json::<Value>().await {
                            userinfo_json = json;
                            userinfo_success = true;
                        }
                    }
                }
            } else {
                let _ = session.clear().await;
                return Err(RefreshOutcome::Expired);
            }
        } else {
            let _ = session.clear().await;
            return Err(RefreshOutcome::Expired);
        }
    }

    if !userinfo_success {
        // Still failed, return expired
        let _ = session.clear().await;
        return Err(RefreshOutcome::Expired);
    }

    // Extract groups and recompute
    let groups = super::oidc::extract_claim_array(&userinfo_json, &state.config.sso_groups_claim)
        .unwrap_or_default();

    let new_folder_permissions = config::compute_folder_permissions(&state.config, &groups);
    let is_admin = config::is_admin(&state.config, &groups);

    // Check if user has home access
    let mut has_home = if config::personal_folder_allowed(&state.config, &groups) {
        state.config.home_folder_root.is_some()
    } else {
        false
    };

    // Ensure home directory actually exists
    if has_home {
        if let Some(ref home_root) = state.config.home_folder_root {
            let home_path = home_root.join(user.safe_username());
            if !home_path.exists() {
                if let Err(e) = std::fs::create_dir_all(&home_path) {
                    tracing::warn!("Failed to create home folder for {}: {}", user.username, e);
                    has_home = false;
                }
            }
        }
    }

    let old_folder_permissions = user.folder_permissions.clone();

    user.folder_permissions = new_folder_permissions;
    user.is_admin = is_admin;
    user.has_home = has_home;

    if user.effectively_no_access() {
        let _ = session.clear().await;
        return Err(RefreshOutcome::NoAccess);
    }

    // Folders that still (or once again) grant capability: clear any pending
    // grace record so a stale first-observation can't confirm an unrelated,
    // later loss.
    for (folder, caps) in user.folder_permissions.iter() {
        if caps.share || caps.read {
            super::permission_grace::clear_permission_loss_grace(
                &state.pool,
                &user.user_id,
                folder,
            )
            .await;
        }
    }

    // Folders that lost capability: don't revoke on the first observation —
    // a single incomplete/transient IdP response shouldn't nuke a share.
    // Only revoke once the loss is confirmed on a later check.
    let mut lost_roots = Vec::new();
    for (folder, old_caps) in old_folder_permissions.iter() {
        if old_caps.share || old_caps.read {
            let new_caps = user
                .folder_permissions
                .get(folder)
                .copied()
                .unwrap_or_default();
            if !new_caps.share && !new_caps.read {
                lost_roots.push(folder.clone());
            }
        }
    }

    for root_key in lost_roots {
        if !super::permission_grace::confirm_permission_loss(
            &state.pool,
            &user.user_id,
            &root_key,
        )
        .await
        {
            tracing::info!(
                user_id = %user.user_id,
                root_key = %root_key,
                "Permission loss observed during live refresh, deferring revoke pending confirmation"
            );
            continue;
        }

        let revoked_at = chrono::Utc::now().timestamp_millis();
        let _ = sqlx::query(
            "UPDATE shares SET revoked_at = $1, revoke_reason = $2, revoke_source = $3
             WHERE owner_user_id = $4 AND root_key = $5 AND revoked_at IS NULL",
        )
        .bind(revoked_at)
        .bind("lost_permission")
        .bind("login_refresh")
        .bind(&user.user_id)
        .bind(&root_key)
        .execute(&state.pool)
        .await;

        tracing::info!(
            user_id = %user.user_id,
            root_key = %root_key,
            "Auto-revoked shares due to confirmed lost permission during live refresh"
        );
    }

    let _ = session.insert("user", &user).await;
    let _ = session.insert("oidc_groups_refreshed_at", now).await;

    if let Ok(folder_permissions_json) = serde_json::to_string(&user.folder_permissions) {
        let _ = sqlx::query(
            "UPDATE users SET folder_permissions_json = $1, has_home = $2, is_admin = $3 WHERE id = $4"
        )
        .bind(folder_permissions_json)
        .bind(user.has_home)
        .bind(user.is_admin)
        .bind(&user.user_id)
        .execute(&state.pool)
        .await;
    }

    Ok(user)
}
