use std::path::{Path, PathBuf};

use nasfiles_core::models::{AuthUser, FolderCaps, Root, RootKind, RootUsage};

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCap {
    Read,
    Write,
    Share,
}

/// Resolve a root key to a filesystem path, enforcing ACL.
///
/// Returns the canonical root path if the user is authorized,
/// or an error describing why access was denied.
pub fn resolve_root(
    config: &AppConfig,
    user: &AuthUser,
    root_key: &str,
    req_cap: RequiredCap,
) -> Result<PathBuf, RootError> {
    // Check capability first
    let has_cap = match req_cap {
        RequiredCap::Read => user.can_read(root_key),
        RequiredCap::Write => user.can_write(root_key),
        RequiredCap::Share => user.can_share(root_key),
    };

    if !has_cap {
        return Err(RootError::Forbidden);
    }

    // Check if it's the home folder (key: "~")
    if root_key == "~" {
        let home_root = config
            .home_folder_root
            .as_ref()
            .ok_or(RootError::NotFound)?;

        let home_path = home_root.join(user.safe_username());

        // Auto-create if missing
        if !home_path.exists() {
            std::fs::create_dir_all(&home_path).map_err(|e| {
                tracing::error!("Failed to create home dir for {}: {e}", user.username);
                RootError::Internal
            })?;
        }

        return Ok(home_path);
    }

    // Check common folders
    let folder_path = config
        .common_folders
        .get(root_key)
        .ok_or(RootError::NotFound)?;

    Ok(folder_path.clone())
}

/// Get the list of roots visible to the current user.
pub fn visible_roots(config: &AppConfig, user: &AuthUser) -> Vec<Root> {
    let mut roots = Vec::new();

    // Add allowed common folders (sorted alphabetically for consistent UI)
    let mut common_keys: Vec<&String> = user
        .readable_folders()
        .filter(|k| config.common_folders.contains_key(*k))
        .collect();
    common_keys.sort();

    for key in common_keys {
        roots.push(Root {
            key: key.clone(),
            display_name: key.clone(),
            kind: RootKind::Common,
            caps: user
                .folder_permissions
                .get(key)
                .copied()
                .unwrap_or_default(),
            group: config.share_group_of_folder.get(key).cloned(),
            usage: usage_for_path(config.common_folders.get(key).map(PathBuf::as_path)),
        });
    }

    // Add home folder if available
    if user.has_home {
        roots.push(Root {
            key: "~".to_string(),
            display_name: "Personal".to_string(),
            kind: RootKind::Home,
            caps: FolderCaps {
                read: true,
                write: true,
                share: true,
            },
            group: None,
            usage: usage_for_path(config.home_folder_root.as_deref()),
        });
    }

    order_roots_by_group(&mut roots);

    roots
}

/// Order roots so ungrouped entries come first, followed by each group's
/// members. `None` sorts before `Some`, and the sort is stable, so this
/// preserves the alphabetical common-folder order within the ungrouped section
/// and within each group, while keeping every group's members contiguous. A
/// group only appears here when at least one of its folders is readable by the
/// user, so empty groups never reach the sidebar.
fn order_roots_by_group(roots: &mut [Root]) {
    roots.sort_by(|a, b| a.group.cmp(&b.group));
}

fn usage_for_path(path: Option<&Path>) -> Option<RootUsage> {
    let path = path?;
    match filesystem_usage(path) {
        Ok(usage) => Some(usage),
        Err(e) => {
            tracing::debug!(
                "failed to read filesystem usage for {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(unix)]
fn statvfs_value<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

#[cfg(unix)]
fn filesystem_usage(path: &Path) -> std::io::Result<RootUsage> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };

    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let stats = unsafe { stats.assume_init() };
    let block_size = statvfs_value(stats.f_frsize);
    let total_blocks = statvfs_value(stats.f_blocks);
    let free_blocks = statvfs_value(stats.f_bfree);
    let total_bytes = total_blocks.saturating_mul(block_size);
    let free_bytes = free_blocks.saturating_mul(block_size);

    Ok(RootUsage {
        used_bytes: total_bytes.saturating_sub(free_bytes),
        total_bytes,
        available_bytes: free_bytes,
    })
}

#[cfg(not(unix))]
fn filesystem_usage(_path: &Path) -> std::io::Result<RootUsage> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "filesystem usage is not supported on this platform",
    ))
}

#[derive(Debug, thiserror::Error)]
pub enum RootError {
    #[error("root not found")]
    NotFound,
    #[error("access denied")]
    Forbidden,
    #[error("internal error")]
    Internal,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(key: &str, group: Option<&str>) -> Root {
        Root {
            key: key.to_string(),
            display_name: key.to_string(),
            kind: RootKind::Common,
            caps: FolderCaps::default(),
            group: group.map(str::to_string),
            usage: None,
        }
    }

    #[test]
    fn order_roots_groups_contiguously_with_ungrouped_first() {
        // Incoming order mirrors visible_roots: common folders alphabetical
        // (mixing grouped and ungrouped), then the home root last.
        let mut roots = vec![
            root("Documents", None),
            root("Movies", Some("Media")),
            root("Projects", Some("Work")),
            root("TV Shows", Some("Media")),
            root("Scratch", None),
            root("~", None),
        ];

        order_roots_by_group(&mut roots);

        let order: Vec<&str> = roots.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            order,
            vec![
                // Ungrouped first, original (alphabetical) order preserved,
                // with the home root kept last.
                "Documents",
                "Scratch",
                "~",
                // "Media" before "Work" (groups alphabetical), members stable.
                "Movies",
                "TV Shows",
                "Projects",
            ]
        );
    }

    #[test]
    fn no_groups_configured_keeps_a_flat_list_in_original_order() {
        // When SHARE_GROUPS is not configured every root has `group: None`,
        // which is exactly how the home and common roots look today. Ordering
        // must then be a no-op so the sidebar renders the same flat list as
        // before the feature existed.
        let original = vec![
            root("Documents", None),
            root("Media", None),
            root("Scratch", None),
            root("~", None),
        ];
        let mut roots = original.clone();

        order_roots_by_group(&mut roots);

        let before: Vec<&str> = original.iter().map(|r| r.key.as_str()).collect();
        let after: Vec<&str> = roots.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(before, after);
        assert!(roots.iter().all(|r| r.group.is_none()));
    }
}

impl axum::response::IntoResponse for RootError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            RootError::NotFound => (axum::http::StatusCode::NOT_FOUND, "root not found"),
            RootError::Forbidden => (axum::http::StatusCode::FORBIDDEN, "access denied"),
            RootError::Internal => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal error",
            ),
        };
        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}
