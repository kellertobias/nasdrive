use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FolderCaps {
    pub read: bool,
    pub write: bool,
    pub share: bool,
}

/// Authenticated user extracted from session.
/// Stored in the session after OIDC/SAML login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub user_id: String,
    pub external_id: String,
    pub username: String,
    pub display_name: String,
    pub picture_url: Option<String>,
    pub folder_permissions: HashMap<String, FolderCaps>,
    pub has_home: bool,
    pub is_admin: bool,
}

impl AuthUser {
    pub fn can_read(&self, key: &str) -> bool {
        if key == "~" {
            return self.has_home;
        }
        self.folder_permissions
            .get(key)
            .map(|c| c.read)
            .unwrap_or(false)
    }

    pub fn can_write(&self, key: &str) -> bool {
        if key == "~" {
            return self.has_home;
        }
        self.folder_permissions
            .get(key)
            .map(|c| c.write)
            .unwrap_or(false)
    }

    pub fn can_share(&self, key: &str) -> bool {
        if key == "~" {
            return self.has_home;
        }
        self.folder_permissions
            .get(key)
            .map(|c| c.share)
            .unwrap_or(false)
    }

    pub fn readable_folders(&self) -> impl Iterator<Item = &String> {
        self.folder_permissions
            .iter()
            .filter(|(_, caps)| caps.read)
            .map(|(k, _)| k)
    }

    pub fn effectively_no_access(&self) -> bool {
        // If there are no readable folders, and no home, and not admin -> no access
        let has_any_readable = self.folder_permissions.values().any(|c| c.read);
        !has_any_readable && !self.has_home && !self.is_admin
    }

    /// Returns a sanitized version of the username safe for filesystem paths.
    pub fn safe_username(&self) -> String {
        Self::sanitize_username(&self.username)
    }

    /// Sanitizes a username string for use in filesystem paths.
    pub fn sanitize_username(username: &str) -> String {
        username.replace(['/', '\\'], "_").replace("..", "_")
    }
}

/// Validate a username for safety before it is stored or used to derive a
/// home-directory path.
///
/// Home directories are keyed on `sanitize_username`, which collapses path
/// separators and `..` to `_`. That mapping is *not* injective — `a/b`,
/// `a..b`, and `a_b` would all map to the same directory — so two distinct
/// identities could otherwise share one home folder. Rejecting these
/// characters at the source keeps the sanitized name a faithful, collision-free
/// representation of the username. Control characters and NUL are rejected for
/// the same filesystem-safety reasons as `validate_filename`.
pub fn validate_username(username: &str) -> Result<(), &'static str> {
    if username.is_empty() {
        return Err("username is required");
    }
    if username.contains('/') || username.contains('\\') {
        return Err("username must not contain '/' or '\\'");
    }
    if username.contains("..") {
        return Err("username must not contain '..'");
    }
    if username.chars().any(|c| c.is_control()) {
        return Err("username must not contain control characters");
    }
    Ok(())
}

/// A root folder visible to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    pub key: String,
    pub display_name: String,
    pub kind: RootKind,
    pub caps: FolderCaps,
    /// Optional sidebar group this root belongs to. Roots that share the same
    /// group name are displayed nested under a single collapsible group header.
    /// A group is purely an organizational overlay for the sidebar — it is never
    /// a filesystem location and cannot be a target for file operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<RootUsage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    Common,
    Home,
}

/// A single entry in a directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub modified_at: i64,
    pub is_dir: bool,
    pub mime_type: Option<String>,
    pub has_thumbnail: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_info: Option<MediaInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_info: Option<ImageInfo>,
}

/// Extracted metadata for audio/video files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaInfo {
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate_bps: Option<u64>,
    pub format_name: Option<String>,
    pub video_mime_codec: Option<String>,
    pub audio_mime_codec: Option<String>,
    #[serde(default)]
    pub audio_languages: Vec<String>,
}

/// Extracted metadata for image files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub format: Option<String>,
    pub has_alpha: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exif: BTreeMap<String, String>,
}

/// Share target kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareTargetKind {
    User,
    Guest,
    Public,
}

/// Share model stored in database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    pub token_hash: String,
    pub owner_user_id: String,
    pub root_kind: RootKind,
    pub root_key: String,
    pub relative_path: String,
    pub is_directory: bool,
    pub target_kind: ShareTargetKind,
    pub target_user_id: Option<String>,
    pub password_hash: Option<String>,
    pub allow_upload: bool,
    pub allow_download: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// Access log entry for a share.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareAccessLog {
    pub id: String,
    pub share_id: String,
    pub occurred_at: i64,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub action: ShareAction,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareAction {
    Open,
    Download,
    Upload,
    List,
    AuthFail,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_user_capabilities() {
        let mut permissions = HashMap::new();
        permissions.insert(
            "shared".to_string(),
            FolderCaps {
                read: true,
                write: false,
                share: false,
            },
        );
        permissions.insert(
            "docs".to_string(),
            FolderCaps {
                read: true,
                write: true,
                share: true,
            },
        );

        let user = AuthUser {
            user_id: "1".into(),
            external_id: "ext1".into(),
            username: "test".into(),
            display_name: "Test User".into(),
            picture_url: None,
            folder_permissions: permissions,
            has_home: true,
            is_admin: false,
        };

        // Home folder
        assert!(user.can_read("~"));
        assert!(user.can_write("~"));
        assert!(user.can_share("~"));

        // shared folder
        assert!(user.can_read("shared"));
        assert!(!user.can_write("shared"));
        assert!(!user.can_share("shared"));

        // docs folder
        assert!(user.can_read("docs"));
        assert!(user.can_write("docs"));
        assert!(user.can_share("docs"));

        // unknown folder
        assert!(!user.can_read("unknown"));
        assert!(!user.can_write("unknown"));
        assert!(!user.can_share("unknown"));

        let readable: Vec<&String> = user.readable_folders().collect();
        assert_eq!(readable.len(), 2);

        assert!(!user.effectively_no_access());
    }

    #[test]
    fn test_auth_user_no_access() {
        let mut user = AuthUser {
            user_id: "1".into(),
            external_id: "ext1".into(),
            username: "test".into(),
            display_name: "Test User".into(),
            picture_url: None,
            folder_permissions: HashMap::new(),
            has_home: false,
            is_admin: false,
        };

        assert!(user.effectively_no_access());

        // Admin has access even without folders
        user.is_admin = true;
        assert!(!user.effectively_no_access());

        user.is_admin = false;
        user.has_home = true;
        assert!(!user.effectively_no_access());

        user.has_home = false;
        user.folder_permissions.insert(
            "test".to_string(),
            FolderCaps {
                read: true,
                write: false,
                share: false,
            },
        );
        assert!(!user.effectively_no_access());

        user.folder_permissions.insert(
            "test2".to_string(),
            FolderCaps {
                read: false,
                write: false,
                share: false,
            },
        );
        user.folder_permissions.remove("test");
        assert!(user.effectively_no_access());
    }

    #[test]
    fn validate_username_accepts_normal_names() {
        for name in ["alice", "bob.smith", "user_01", "jane-doe", "auth0|abc123"] {
            assert!(validate_username(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn validate_username_rejects_collision_prone_and_unsafe_names() {
        for name in ["", "a/b", "a\\b", "a..b", "..", "a\0b", "a\tb"] {
            assert!(
                validate_username(name).is_err(),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn validate_username_rejects_what_sanitize_would_collapse() {
        // Any name that survives validation must already be a faithful,
        // collision-free home-dir key (sanitize is then a no-op on it).
        for name in ["alice", "bob.smith", "user_01"] {
            assert_eq!(AuthUser::sanitize_username(name), name);
        }
    }
}
