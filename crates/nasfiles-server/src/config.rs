use std::collections::HashMap;
use std::path::PathBuf;

use nasfiles_core::models::FolderCaps;
use serde::Deserialize;

/// Application configuration, parsed entirely from environment variables.
#[derive(Debug, Clone)]
pub struct AppConfig {
    // Server
    pub bind_addr: String,
    pub base_url: String,
    pub session_secret: Vec<u8>,
    #[allow(dead_code)]
    pub data_dir: PathBuf,
    pub dev_mode: bool,
    pub auth_mode: AuthMode,
    pub no_server_side_execution: bool,
    pub csp_extra_img_src: Vec<String>,
    pub csp_extra_media_src: Vec<String>,

    // Database
    pub db_url: String,

    // Folder mounts
    pub common_folders: HashMap<String, PathBuf>,
    pub home_folder_root: Option<PathBuf>,

    // SSO — OIDC
    pub oidc: Option<OidcConfig>,

    // SSO claim names
    pub sso_username_claim: String,
    pub sso_display_name_claim: String,
    pub sso_picture_claim: String,
    pub sso_groups_claim: String,

    // Group → folder mapping
    pub group_folder_caps: HashMap<String, HashMap<String, FolderCaps>>,
    pub default_folder_caps: HashMap<String, FolderCaps>,
    pub admin_groups: Vec<String>,

    pub personal_folder_groups: Option<Vec<String>>,
    pub groups_refresh_interval_secs: u64,

    // Dev bypass user
    pub dev_user: Option<DevUserConfig>,

    // Local auth
    pub disable_passkeys: bool,
    pub disable_totp: bool,
    pub setup_admin: Option<SetupAdminConfig>,
    pub totp_trusted_device_ttl_days: u64,

    // Thumbnails
    pub thumbnail_cache_dir: PathBuf,
    pub thumbnail_max_source_file_size: u64,
    pub thumbnail_max_image_width: u32,
    pub thumbnail_max_image_height: u32,
    pub thumbnail_max_image_alloc: u64,
    pub thumbnail_max_concurrent_generations: usize,
    pub media_preview_max_concurrent_transcodes: usize,

    // Shares
    pub share_token_bytes: usize,

    // SFTP
    pub sftp_enabled: bool,
    pub sftp_bind_addr: String,
    pub sftp_host_key_path: PathBuf,

    // Upload limits
    pub max_upload_file_size: u64,
    #[allow(dead_code)]
    pub max_upload_request_size: u64,

    // Trusted proxy depth for IP extraction from X-Forwarded-For.
    // 0 = no proxy, ignore headers; 1 = one proxy (e.g. Traefik), default.
    pub trusted_proxy_depth: u8,

    // Logging
    pub log_level: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Sso,
    Local,
}

impl AuthMode {
    fn from_env_value(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sso" => Ok(Self::Sso),
            "local" => Ok(Self::Local),
            other => anyhow::bail!("AUTH_MODE must be `sso` or `local`, got `{other}`"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sso => "sso",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Extra audience values to trust in ID tokens (e.g. Zitadel project ID).
    pub additional_audiences: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SetupAdminConfig {
    pub username: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevUserConfig {
    pub username: String,
    pub display_name: String,
    pub groups: Vec<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let dev_mode = env_bool("NASFILES_DEV");
        let auth_mode = std::env::var("AUTH_MODE")
            .map(|v| AuthMode::from_env_value(&v))
            .unwrap_or(Ok(AuthMode::Sso))?;
        let no_server_side_execution = env_bool("NO_SERVER_SIDE_EXECUTION");
        let csp_extra_img_src = parse_source_list_env("CSP_IMG_SRC_EXTRA");
        let csp_extra_media_src = parse_source_list_env("CSP_MEDIA_SRC_EXTRA");

        // BIND_ADDR
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        // BASE_URL
        let base_url =
            std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

        // Enforce HTTPS in production. Local screenshot demos can opt into
        // plain HTTP without enabling the auth-bypass development user.
        let allow_insecure_local_http = env_bool("NASFILES_ALLOW_INSECURE_LOCAL_HTTP")
            && (base_url.starts_with("http://127.0.0.1")
                || base_url.starts_with("http://localhost"));
        if !dev_mode && !base_url.starts_with("https://") && !allow_insecure_local_http {
            anyhow::bail!(
                "BASE_URL must use https:// in production. Set NASFILES_DEV=1 for development or NASFILES_ALLOW_INSECURE_LOCAL_HTTP=1 for local-only HTTP."
            );
        }

        // SESSION_SECRET
        let session_secret = if dev_mode {
            std::env::var("SESSION_SECRET").unwrap_or_else(|_| {
                // In dev mode, use a deterministic secret for convenience
                "0".repeat(128)
            })
        } else {
            std::env::var("SESSION_SECRET")
                .map_err(|_| anyhow::anyhow!("SESSION_SECRET is required"))?
        };
        let secret_bytes = hex::decode(&session_secret)
            .map_err(|_| anyhow::anyhow!("SESSION_SECRET must be valid hex"))?;
        if secret_bytes.len() < 64 {
            anyhow::bail!("SESSION_SECRET must be at least 64 bytes (128 hex chars)");
        }

        // DATA_DIR
        let data_dir = PathBuf::from(
            std::env::var("DATA_DIR").unwrap_or_else(|_| "/tmp/nasfiles-data".to_string()),
        );
        std::fs::create_dir_all(&data_dir)?;

        // DB_URL
        let db_url = std::env::var("DB_URL")
            .unwrap_or_else(|_| format!("sqlite://{}?mode=rwc", data_dir.join("app.db").display()));

        // COMMON_FOLDERS — JSON map
        let common_folders: HashMap<String, PathBuf> = match std::env::var("COMMON_FOLDERS") {
            Ok(json) => {
                let map: HashMap<String, String> = serde_json::from_str(&json)
                    .map_err(|e| anyhow::anyhow!("invalid COMMON_FOLDERS JSON: {e}"))?;
                map.into_iter()
                    .map(|(k, v)| (k, PathBuf::from(v)))
                    .collect()
            }
            Err(_) => HashMap::new(),
        };

        // HOME_FOLDER_ROOT
        let home_folder_root = std::env::var("HOME_FOLDER_ROOT").ok().map(PathBuf::from);

        // OIDC config
        let configured_oidc = match (
            std::env::var("SSO_OIDC_ISSUER_URL"),
            std::env::var("SSO_OIDC_CLIENT_ID"),
            std::env::var("SSO_OIDC_CLIENT_SECRET"),
        ) {
            (Ok(issuer), Ok(client_id), Ok(secret)) => {
                // SSO_OIDC_EXTRA_AUDIENCES: comma-separated list of extra trusted audience values.
                // Needed for Zitadel which includes the project ID (not the client ID) in `aud`.
                let additional_audiences = std::env::var("SSO_OIDC_EXTRA_AUDIENCES")
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                Some(OidcConfig {
                    issuer_url: issuer,
                    client_id,
                    client_secret: secret,
                    additional_audiences,
                })
            }
            _ => None,
        };
        let oidc = match auth_mode {
            AuthMode::Sso => configured_oidc,
            AuthMode::Local => None,
        };
        if matches!(auth_mode, AuthMode::Sso) && !dev_mode && oidc.is_none() {
            anyhow::bail!(
                "SSO auth mode requires SSO_OIDC_ISSUER_URL, SSO_OIDC_CLIENT_ID, and SSO_OIDC_CLIENT_SECRET in production"
            );
        }

        // SSO claim names
        let sso_username_claim = std::env::var("SSO_USERNAME_CLAIM")
            .unwrap_or_else(|_| "preferred_username".to_string());
        let sso_display_name_claim =
            std::env::var("SSO_DISPLAY_NAME_CLAIM").unwrap_or_else(|_| "name".to_string());
        let sso_picture_claim =
            std::env::var("SSO_PICTURE_CLAIM").unwrap_or_else(|_| "picture".to_string());
        let sso_groups_claim =
            std::env::var("SSO_GROUPS_CLAIM").unwrap_or_else(|_| "groups".to_string());

        // Group → folder capabilities
        let group_folder_caps = discover_group_folder_caps();

        // Default common folders
        let default_folder_caps = discover_default_folder_caps();

        // Personal folder groups
        let personal_folder_groups = std::env::var("SSO_PERSONAL_FOLDER_GROUPS").ok().map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });

        let groups_refresh_interval_secs: u64 = std::env::var("SSO_GROUPS_REFRESH_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        // Admin groups
        let admin_groups = std::env::var("SSO_ADMIN_GROUPS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Dev user
        let dev_user = if dev_mode {
            Some(
                std::env::var("NASFILES_DEV_USER")
                    .ok()
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or(DevUserConfig {
                        username: "devuser".to_string(),
                        display_name: "Development User".to_string(),
                        groups: vec!["STAFF".to_string()],
                    }),
            )
        } else {
            None
        };

        let disable_passkeys = env_bool("DISABLE_PASSKEYS");
        let disable_totp = env_bool("DISABLE_TOTP");
        let setup_admin = match (
            std::env::var("SETUP_ADMIN_USER"),
            std::env::var("SETUP_ADMIN_PASSWORD"),
        ) {
            (Ok(username), Ok(password)) => {
                let username = username.trim().to_string();
                if username.is_empty() {
                    anyhow::bail!("SETUP_ADMIN_USER must not be empty");
                }
                if password.is_empty() {
                    anyhow::bail!("SETUP_ADMIN_PASSWORD must not be empty");
                }
                let display_name = std::env::var("SETUP_ADMIN_DISPLAY_NAME")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| username.clone());
                Some(SetupAdminConfig {
                    username,
                    password,
                    display_name,
                })
            }
            (Err(_), Err(_)) => None,
            _ => anyhow::bail!("SETUP_ADMIN_USER and SETUP_ADMIN_PASSWORD must be set together"),
        };
        let totp_trusted_device_ttl_days: u64 = std::env::var("TOTP_TRUSTED_DEVICE_TTL_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        // Thumbnail cache
        let thumbnail_cache_dir = std::env::var("THUMBNAIL_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("thumbs"));
        if !no_server_side_execution {
            std::fs::create_dir_all(&thumbnail_cache_dir)?;
        }
        let thumbnail_max_source_file_size: u64 = std::env::var("THUMBNAIL_MAX_SOURCE_FILE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(512 * 1024 * 1024);
        let thumbnail_max_image_width: u32 = std::env::var("THUMBNAIL_MAX_IMAGE_WIDTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);
        let thumbnail_max_image_height: u32 = std::env::var("THUMBNAIL_MAX_IMAGE_HEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);
        let thumbnail_max_image_alloc: u64 = std::env::var("THUMBNAIL_MAX_IMAGE_ALLOC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256 * 1024 * 1024);
        let thumbnail_max_concurrent_generations: usize =
            std::env::var("THUMBNAIL_MAX_CONCURRENT_GENERATIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2)
                .max(1);
        let media_preview_max_concurrent_transcodes: usize =
            std::env::var("MEDIA_PREVIEW_MAX_CONCURRENT_TRANSCODES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2)
                .max(1);

        // Share token bytes. Clamp to a minimum of 16 bytes (128 bits) so an
        // operator cannot configure a dangerously short, brute-forceable token.
        let share_token_bytes: usize = std::env::var("SHARE_TOKEN_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24)
            .max(16);

        let sftp_enabled = std::env::var("SFTP_ENABLED")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);
        let sftp_bind_addr =
            std::env::var("SFTP_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:2222".to_string());
        let sftp_host_key_path = std::env::var("SFTP_HOST_KEY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("sftp_host_key"));

        // Upload limits (default: 10 GB per file, 50 GB per request)
        let max_upload_file_size: u64 = std::env::var("MAX_UPLOAD_FILE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10 * 1024 * 1024 * 1024);
        let max_upload_request_size: u64 = std::env::var("MAX_UPLOAD_REQUEST_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50 * 1024 * 1024 * 1024);

        // Trusted proxy depth (default 1: assume a single reverse proxy like Traefik)
        let trusted_proxy_depth: u8 = std::env::var("TRUSTED_PROXY_DEPTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        // Log level
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        Ok(AppConfig {
            bind_addr,
            base_url,
            session_secret: secret_bytes,
            data_dir,
            dev_mode,
            auth_mode,
            no_server_side_execution,
            csp_extra_img_src,
            csp_extra_media_src,
            db_url,
            common_folders,
            home_folder_root,
            oidc,
            sso_username_claim,
            sso_display_name_claim,
            sso_picture_claim,
            sso_groups_claim,
            group_folder_caps,
            default_folder_caps,
            admin_groups,
            personal_folder_groups,
            groups_refresh_interval_secs,
            dev_user,
            disable_passkeys,
            disable_totp,
            setup_admin,
            totp_trusted_device_ttl_days,
            thumbnail_cache_dir,
            thumbnail_max_source_file_size,
            thumbnail_max_image_width,
            thumbnail_max_image_height,
            thumbnail_max_image_alloc,
            thumbnail_max_concurrent_generations,
            media_preview_max_concurrent_transcodes,
            share_token_bytes,
            sftp_enabled,
            sftp_bind_addr,
            sftp_host_key_path,
            max_upload_file_size,
            max_upload_request_size,
            trusted_proxy_depth,
            log_level,
        })
    }
}

fn env_bool(key: &str) -> bool {
    parse_env_bool(std::env::var(key).ok().as_deref())
}

fn parse_env_bool(value: Option<&str>) -> bool {
    value
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn parse_source_list_env(key: &str) -> Vec<String> {
    std::env::var(key)
        .unwrap_or_default()
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Discover all SSO_GROUP_*_FOLDERS_{READ,WRITE,SHARE} and SSO_GROUP_*_COMMON_FOLDERS
/// env vars and parse them into a mapping of group name → folder capabilities.
fn discover_group_folder_caps() -> HashMap<String, HashMap<String, FolderCaps>> {
    let mut mapping: HashMap<String, HashMap<String, FolderCaps>> = HashMap::new();

    let prefix = "SSO_GROUP_";

    for (key, value) in std::env::vars() {
        if !key.starts_with(prefix) {
            continue;
        }

        let rest = &key[prefix.len()..];

        let parse_folders = |v: &str| -> Vec<String> {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        if let Some(group_name) = rest.strip_suffix("_COMMON_FOLDERS") {
            let folders = parse_folders(&value);
            let group_map = mapping.entry(group_name.to_string()).or_default();
            for f in folders {
                let caps = group_map.entry(f).or_default();
                caps.read = true;
                caps.write = true;
                caps.share = true;
            }
        } else if let Some(group_name) = rest.strip_suffix("_FOLDERS_READ") {
            let folders = parse_folders(&value);
            let group_map = mapping.entry(group_name.to_string()).or_default();
            for f in folders {
                let caps = group_map.entry(f).or_default();
                caps.read = true;
            }
        } else if let Some(group_name) = rest.strip_suffix("_FOLDERS_WRITE") {
            let folders = parse_folders(&value);
            let group_map = mapping.entry(group_name.to_string()).or_default();
            for f in folders {
                let caps = group_map.entry(f).or_default();
                caps.write = true;
            }
        } else if let Some(group_name) = rest.strip_suffix("_FOLDERS_SHARE") {
            let folders = parse_folders(&value);
            let group_map = mapping.entry(group_name.to_string()).or_default();
            for f in folders {
                let caps = group_map.entry(f).or_default();
                caps.share = true;
                // Share implies read
                caps.read = true;
            }
        }
    }

    mapping
}

/// Discover default folder capabilities for all users.
fn discover_default_folder_caps() -> HashMap<String, FolderCaps> {
    let mut caps_map: HashMap<String, FolderCaps> = HashMap::new();

    let parse_folders = |v: String| -> Vec<String> {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    if let Ok(v) = std::env::var("SSO_DEFAULT_COMMON_FOLDERS") {
        for f in parse_folders(v) {
            let caps = caps_map.entry(f).or_default();
            caps.read = true;
            caps.write = true;
            caps.share = true;
        }
    }

    if let Ok(v) = std::env::var("SSO_DEFAULT_FOLDERS_READ") {
        for f in parse_folders(v) {
            let caps = caps_map.entry(f).or_default();
            caps.read = true;
        }
    }

    if let Ok(v) = std::env::var("SSO_DEFAULT_FOLDERS_WRITE") {
        for f in parse_folders(v) {
            let caps = caps_map.entry(f).or_default();
            caps.write = true;
        }
    }

    if let Ok(v) = std::env::var("SSO_DEFAULT_FOLDERS_SHARE") {
        for f in parse_folders(v) {
            let caps = caps_map.entry(f).or_default();
            caps.share = true;
            caps.read = true;
        }
    }

    caps_map
}

/// Compute the folder capabilities a user has based on their SSO groups.
pub fn compute_folder_permissions(
    config: &AppConfig,
    user_groups: &[String],
) -> HashMap<String, FolderCaps> {
    let mut allowed = config.default_folder_caps.clone();

    for group in user_groups {
        if let Some(group_caps) = config.group_folder_caps.get(group) {
            for (folder, caps) in group_caps {
                let entry = allowed.entry(folder.clone()).or_default();
                entry.read |= caps.read;
                entry.write |= caps.write;
                entry.share |= caps.share;

                if entry.share {
                    entry.read = true;
                }
            }
        }
    }

    // Filter to only folders that actually exist in config
    allowed
        .into_iter()
        .filter(|(f, _)| config.common_folders.contains_key(f))
        .collect()
}

/// Check if a user is allowed to access their personal folder.
pub fn personal_folder_allowed(config: &AppConfig, user_groups: &[String]) -> bool {
    match &config.personal_folder_groups {
        None => true, // No gating, default behavior
        Some(groups) => groups.iter().any(|g| user_groups.contains(g)),
    }
}

/// Check if a user is an admin based on their SSO groups.
pub fn is_admin(config: &AppConfig, user_groups: &[String]) -> bool {
    config
        .admin_groups
        .iter()
        .any(|ag| user_groups.iter().any(|ug| ug == ag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mode_parses_expected_values() {
        assert_eq!(AuthMode::from_env_value("sso").unwrap(), AuthMode::Sso);
        assert_eq!(
            AuthMode::from_env_value(" LOCAL ").unwrap(),
            AuthMode::Local
        );
        assert!(AuthMode::from_env_value("password").is_err());
    }

    #[test]
    fn test_compute_folder_permissions() {
        let mut group_caps = HashMap::new();
        let mut admin_caps = HashMap::new();
        admin_caps.insert(
            "docs".to_string(),
            FolderCaps {
                read: true,
                write: true,
                share: true,
            },
        );
        group_caps.insert("admin".to_string(), admin_caps);

        let mut user_caps = HashMap::new();
        user_caps.insert(
            "media".to_string(),
            FolderCaps {
                read: true,
                write: false,
                share: false,
            },
        );
        group_caps.insert("user".to_string(), user_caps);

        let mut common_folders = HashMap::new();
        common_folders.insert("docs".to_string(), PathBuf::from("/docs"));
        common_folders.insert("media".to_string(), PathBuf::from("/media"));

        let config = AppConfig {
            bind_addr: "".into(),
            base_url: "".into(),
            session_secret: vec![],
            data_dir: PathBuf::new(),
            dev_mode: false,
            auth_mode: AuthMode::Sso,
            no_server_side_execution: false,
            csp_extra_img_src: Vec::new(),
            csp_extra_media_src: Vec::new(),
            db_url: "".into(),
            common_folders,
            home_folder_root: None,
            oidc: None,
            sso_username_claim: "".into(),
            sso_display_name_claim: "".into(),
            sso_picture_claim: "".into(),
            sso_groups_claim: "".into(),
            group_folder_caps: group_caps,
            default_folder_caps: HashMap::new(),
            admin_groups: vec![],
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
            share_token_bytes: 24,
            sftp_enabled: false,
            sftp_bind_addr: String::new(),
            sftp_host_key_path: PathBuf::new(),
            max_upload_file_size: 0,
            max_upload_request_size: 0,
            trusted_proxy_depth: 1,
            log_level: "".into(),
        };

        let admin_perms = compute_folder_permissions(&config, &[String::from("admin")]);
        assert_eq!(admin_perms.len(), 1);
        assert!(admin_perms.get("docs").unwrap().write);

        let user_perms = compute_folder_permissions(&config, &[String::from("user")]);
        assert_eq!(user_perms.len(), 1);
        assert!(user_perms.get("media").unwrap().read);
        assert!(!user_perms.get("media").unwrap().write);

        let both_perms =
            compute_folder_permissions(&config, &[String::from("user"), String::from("admin")]);
        assert_eq!(both_perms.len(), 2);
    }

    #[test]
    fn env_bool_uses_existing_boolean_convention() {
        assert!(!parse_env_bool(None));
        assert!(!parse_env_bool(Some("0")));
        assert!(!parse_env_bool(Some("false")));
        assert!(parse_env_bool(Some("1")));
        assert!(parse_env_bool(Some("true")));
        assert!(parse_env_bool(Some("TRUE")));
    }
}
