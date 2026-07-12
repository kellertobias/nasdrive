use serde::{Deserialize, Serialize};

/// Represents a share in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    pub token_hash: String,
    pub owner_user_id: String,
    pub root_kind: String,
    pub root_key: String,
    pub relative_path: String,
    pub is_directory: bool,
    pub target_kind: TargetKind,
    pub target_user_id: Option<String>,
    pub password_hash: Option<String>,
    pub allow_upload: bool,
    pub allow_download: bool,
    pub share_type: ShareType,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

impl Share {
    /// The user-facing name of the shared item — the last path segment of
    /// `relative_path`, or `root_key` when the share targets an entire root.
    pub fn display_name(&self) -> &str {
        if self.relative_path.is_empty() {
            &self.root_key
        } else {
            self.relative_path
                .rsplit('/')
                .next()
                .unwrap_or(&self.relative_path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(root_key: &str, relative_path: &str) -> Share {
        Share {
            id: "id".to_string(),
            token_hash: "hash".to_string(),
            owner_user_id: "owner".to_string(),
            root_kind: "common".to_string(),
            root_key: root_key.to_string(),
            relative_path: relative_path.to_string(),
            is_directory: true,
            target_kind: TargetKind::Public,
            target_user_id: None,
            password_hash: None,
            allow_upload: false,
            allow_download: true,
            share_type: ShareType::Typical,
            expires_at: None,
            created_at: 0,
            revoked_at: None,
        }
    }

    #[test]
    fn display_name_uses_root_key_when_sharing_a_whole_root() {
        assert_eq!(share("Documents", "").display_name(), "Documents");
    }

    #[test]
    fn display_name_uses_last_path_segment_for_nested_shares() {
        assert_eq!(
            share("Documents", "reports/2026/q1.pdf").display_name(),
            "q1.pdf"
        );
    }
}

/// User-facing share preset. Permission booleans remain the enforcement layer,
/// while this preserves the actual share type selected in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareType {
    Typical,
    Gallery,
    Dropbox,
    Collaboration,
}

impl ShareType {
    pub fn as_str(&self) -> &str {
        match self {
            ShareType::Typical => "typical",
            ShareType::Gallery => "gallery",
            ShareType::Dropbox => "dropbox",
            ShareType::Collaboration => "collaboration",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "typical" => Some(ShareType::Typical),
            "gallery" => Some(ShareType::Gallery),
            "dropbox" => Some(ShareType::Dropbox),
            "collaboration" => Some(ShareType::Collaboration),
            _ => None,
        }
    }
}

/// The kind of share target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    User,
    Guest,
    Public,
}

impl TargetKind {
    pub fn as_str(&self) -> &str {
        match self {
            TargetKind::User => "user",
            TargetKind::Guest => "guest",
            TargetKind::Public => "public",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(TargetKind::User),
            "guest" => Some(TargetKind::Guest),
            "public" => Some(TargetKind::Public),
            _ => None,
        }
    }
}

/// Request body for creating a share.
#[derive(Debug, Deserialize)]
pub struct CreateShareRequest {
    pub root_key: String,
    pub path: String,
    pub target_kind: TargetKind,
    /// User ID for user-type shares
    pub target_user_id: Option<String>,
    /// Password for guest-type shares
    pub password: Option<String>,
    #[serde(default = "default_share_type")]
    pub share_type: ShareType,
    /// Expiry in seconds from now (null = never)
    pub expires_in: Option<i64>,
}

fn default_share_type() -> ShareType {
    ShareType::Typical
}

/// Public metadata for a share (returned to unauthenticated users).
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct ShareMetadata {
    pub name: String,
    pub is_directory: bool,
    pub requires_password: bool,
    pub owner_display_name: String,
    pub allow_upload: bool,
    pub allow_download: bool,
    pub share_type: ShareType,
}

/// Auth request body for guest shares.
#[derive(Debug, Deserialize)]
pub struct ShareAuthRequest {
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateShareRequest {
    pub password: Option<String>,
    pub share_type: Option<ShareType>,
}
