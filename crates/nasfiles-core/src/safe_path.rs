use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SafePathError {
    #[error("path traversal attempt detected")]
    Traversal,

    #[error("path not found: {0}")]
    NotFound(String),

    #[error("symlink escapes root directory")]
    SymlinkEscape,

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// @tour comment The single path-containment chokepoint
// Every path in NASDrive — HTTP, SFTP and S3 alike — reaches the filesystem through this
// function or its sibling below. It rejects NUL bytes, leading separators and Windows drive
// prefixes, then canonicalizes *both* the root and the joined path and asserts the result
// still starts with the root.
//
// Canonicalizing after the join is what defeats `../` sequences and symlinks in one move.
// Note the ordering constraint across the codebase: `roots::resolve_root` enforces the
// capability *first*, and only then is containment checked. Both gates must pass. See
// [safe path](glossary:safe-path).

/// Resolve a relative path within a root directory, ensuring the
/// result is contained within the root. This is the **single chokepoint**
/// for all filesystem access — no other module should perform path joins
/// on untrusted input.
///
/// # Security properties
/// - Canonicalizes both root and joined path to resolve any `..`, `.`, or symlink components
/// - Asserts the canonical result starts_with the canonical root
/// - Rejects NUL bytes in the relative path
/// - Rejects empty root paths
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf, SafePathError> {
    // Reject NUL bytes — these can confuse C-based filesystem APIs
    if relative.contains('\0') {
        return Err(SafePathError::InvalidPath(
            "path contains NUL byte".to_string(),
        ));
    }

    // Reject absolute paths in the relative portion
    if relative.starts_with('/') || relative.starts_with('\\') {
        return Err(SafePathError::Traversal);
    }

    // Reject Windows-style absolute paths (e.g., C:\...)
    if relative.len() >= 2 && relative.as_bytes()[1] == b':' {
        return Err(SafePathError::Traversal);
    }

    // Canonicalize the root first — this resolves symlinks in the root itself
    let canonical_root = root.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SafePathError::NotFound(root.display().to_string())
        } else {
            SafePathError::Io(e)
        }
    })?;

    // Handle empty relative path — means requesting the root itself
    if relative.is_empty() || relative == "." {
        return Ok(canonical_root);
    }

    // Join and canonicalize
    let joined = root.join(relative);
    let canonical_joined = joined.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SafePathError::NotFound(joined.display().to_string())
        } else {
            SafePathError::Io(e)
        }
    })?;

    // Assert containment — the canonical path must start with the canonical root
    if !canonical_joined.starts_with(&canonical_root) {
        return Err(SafePathError::Traversal);
    }

    Ok(canonical_joined)
}

// @tour comment Creating a file needs a different guarantee
// A path about to be created cannot be canonicalized, so containment is asserted on the
// canonical *parent* and the final component is validated separately by
// `validate_filename`.
//
// The subtle part is the `symlink_metadata` check: a final component that already exists as
// a symlink is rejected, including a dangling one, because a later create, rename or mkdir
// would follow it straight out of the root. The in-code comment above it describes the
// exact bypass being closed.

/// Check if a path would be valid without requiring the target to exist.
/// Used for create operations (mkdir, upload) where the target doesn't exist yet.
/// Validates the parent directory exists and is within the root.
pub fn resolve_parent(root: &Path, relative: &str) -> Result<PathBuf, SafePathError> {
    if relative.contains('\0') {
        return Err(SafePathError::InvalidPath(
            "path contains NUL byte".to_string(),
        ));
    }

    if relative.starts_with('/') || relative.starts_with('\\') {
        return Err(SafePathError::Traversal);
    }

    if relative.len() >= 2 && relative.as_bytes()[1] == b':' {
        return Err(SafePathError::Traversal);
    }

    let canonical_root = root.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SafePathError::NotFound(root.display().to_string())
        } else {
            SafePathError::Io(e)
        }
    })?;

    let joined = root.join(relative);

    // The filename component is the new entry — validate parent exists
    let parent = joined
        .parent()
        .ok_or_else(|| SafePathError::InvalidPath("no parent directory".to_string()))?;

    let canonical_parent = parent.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SafePathError::NotFound(parent.display().to_string())
        } else {
            SafePathError::Io(e)
        }
    })?;

    if !canonical_parent.starts_with(&canonical_root) {
        return Err(SafePathError::Traversal);
    }

    // Extract and validate the filename
    let filename = joined
        .file_name()
        .ok_or_else(|| SafePathError::InvalidPath("no filename".to_string()))?;

    let filename_str = filename.to_string_lossy();
    validate_filename(&filename_str)?;

    let target = canonical_parent.join(filename);

    // Reject when the final component already exists as a symlink. Unlike
    // `resolve`, this function does not canonicalize the final component (the
    // target may not exist yet), so the canonicalize-based containment check is
    // bypassed here. A symlink at this position — including a dangling one
    // pointing outside the root — would otherwise be *followed* by a subsequent
    // create/open/rename/mkdir, escaping the root. Symlinks that stay within the
    // root are still readable via `resolve`; only the create path forbids them.
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(SafePathError::SymlinkEscape);
        }
        _ => {}
    }

    Ok(target)
}

/// Validate a filename for safety.
/// Rejects control characters, path separators, and other dangerous patterns.
pub fn validate_filename(name: &str) -> Result<(), SafePathError> {
    if name.is_empty() {
        return Err(SafePathError::InvalidPath("empty filename".to_string()));
    }

    if name == "." || name == ".." {
        return Err(SafePathError::Traversal);
    }

    // Reject control characters
    if name.chars().any(|c| c.is_control()) {
        return Err(SafePathError::InvalidPath(
            "filename contains control characters".to_string(),
        ));
    }

    // Reject path separators
    if name.contains('/') || name.contains('\\') {
        return Err(SafePathError::InvalidPath(
            "filename contains path separator".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create some nested directories and files
        fs::create_dir_all(dir.path().join("subdir/nested")).unwrap();
        fs::write(dir.path().join("file.txt"), "hello").unwrap();
        fs::write(dir.path().join("subdir/file2.txt"), "world").unwrap();
        fs::write(dir.path().join("subdir/nested/deep.txt"), "deep").unwrap();

        dir
    }

    #[test]
    fn test_resolve_clean_path() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "file.txt").unwrap();
        assert!(result.exists());
        assert!(result.ends_with("file.txt"));
    }

    #[test]
    fn test_resolve_empty_path_returns_root() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "").unwrap();
        assert_eq!(result, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_dot_returns_root() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), ".").unwrap();
        assert_eq!(result, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_nested_path() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "subdir/nested/deep.txt").unwrap();
        assert!(result.exists());
        assert!(result.ends_with("deep.txt"));
    }

    #[test]
    fn test_reject_dotdot_traversal() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "../../../etc/passwd");
        assert!(matches!(
            result,
            Err(SafePathError::Traversal) | Err(SafePathError::NotFound(_))
        ));
    }

    #[test]
    fn test_reject_dotdot_in_middle() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "subdir/../../etc/passwd");
        assert!(matches!(
            result,
            Err(SafePathError::Traversal) | Err(SafePathError::NotFound(_))
        ));
    }

    #[test]
    fn test_reject_absolute_path() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "/etc/passwd");
        assert!(matches!(result, Err(SafePathError::Traversal)));
    }

    #[test]
    fn test_reject_nul_byte() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "file\0.txt");
        assert!(matches!(result, Err(SafePathError::InvalidPath(_))));
    }

    #[test]
    fn test_reject_windows_absolute() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "C:\\Windows\\System32");
        assert!(matches!(result, Err(SafePathError::Traversal)));
    }

    #[test]
    fn test_nonexistent_file() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "nonexistent.txt");
        assert!(matches!(result, Err(SafePathError::NotFound(_))));
    }

    #[test]
    fn test_double_dot_in_valid_filename() {
        let dir = setup_test_dir();
        // A file named "file..txt" is valid — it's not a traversal
        fs::write(dir.path().join("file..txt"), "test").unwrap();
        let result = resolve(dir.path(), "file..txt").unwrap();
        assert!(result.exists());
    }

    #[test]
    fn test_deeply_nested_traversal() {
        let dir = setup_test_dir();
        let result = resolve(dir.path(), "subdir/nested/../../../../../../../etc/passwd");
        assert!(matches!(
            result,
            Err(SafePathError::Traversal) | Err(SafePathError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_within_root_allowed() {
        let dir = setup_test_dir();
        std::os::unix::fs::symlink(dir.path().join("subdir"), dir.path().join("link_to_subdir"))
            .unwrap();
        let result = resolve(dir.path(), "link_to_subdir/file2.txt").unwrap();
        assert!(result.exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escaping_root_rejected() {
        let dir = setup_test_dir();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape_link")).unwrap();

        let result = resolve(dir.path(), "escape_link/secret.txt");
        assert!(matches!(result, Err(SafePathError::Traversal)));
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_parent_rejects_escaping_symlink_target() {
        let dir = setup_test_dir();
        let outside = tempfile::tempdir().unwrap();
        // A dangling/escaping symlink as the *final* component of a create path.
        std::os::unix::fs::symlink(outside.path().join("evil.txt"), dir.path().join("evil"))
            .unwrap();
        let result = resolve_parent(dir.path(), "evil");
        assert!(matches!(result, Err(SafePathError::SymlinkEscape)));
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_parent_rejects_in_root_symlink_target() {
        let dir = setup_test_dir();
        // Even an in-root symlink must not be a create target — creating through
        // it would follow the link rather than create the intended entry.
        std::os::unix::fs::symlink(dir.path().join("file.txt"), dir.path().join("alias")).unwrap();
        let result = resolve_parent(dir.path(), "alias");
        assert!(matches!(result, Err(SafePathError::SymlinkEscape)));
    }

    #[test]
    fn test_resolve_parent_allows_new_file() {
        let dir = setup_test_dir();
        let result = resolve_parent(dir.path(), "brand_new.txt").unwrap();
        assert!(result.ends_with("brand_new.txt"));
    }

    #[test]
    fn test_validate_filename_rejects_empty() {
        assert!(matches!(
            validate_filename(""),
            Err(SafePathError::InvalidPath(_))
        ));
    }

    #[test]
    fn test_validate_filename_rejects_dot() {
        assert!(matches!(
            validate_filename("."),
            Err(SafePathError::Traversal)
        ));
    }

    #[test]
    fn test_validate_filename_rejects_dotdot() {
        assert!(matches!(
            validate_filename(".."),
            Err(SafePathError::Traversal)
        ));
    }

    #[test]
    fn test_validate_filename_rejects_control_chars() {
        assert!(matches!(
            validate_filename("file\x00.txt"),
            Err(SafePathError::InvalidPath(_))
        ));
    }

    #[test]
    fn test_validate_filename_rejects_slash() {
        assert!(matches!(
            validate_filename("path/file.txt"),
            Err(SafePathError::InvalidPath(_))
        ));
    }
}
