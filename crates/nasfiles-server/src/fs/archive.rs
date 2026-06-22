use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::Instant;

use crate::fs::roots;
use crate::state::AppState;

const EXTRACT_TIMEOUT: Duration = Duration::from_secs(60 * 30);
const MAX_ARCHIVE_ENTRIES: u64 = 100_000;
const MAX_EXTRACTED_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_EXTRACTED_FILE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 1_000;
const MAX_EXTRACTOR_ERROR_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractMode {
    Here,
    HereRemove,
    Subfolder,
}

pub async fn extract_archive(
    state: &AppState,
    user: &nasfiles_core::models::AuthUser,
    root_key: &str,
    archive_path: &str,
    mode: ExtractMode,
) -> Result<(), ArchiveError> {
    if state.config.no_server_side_execution {
        return Err(ArchiveError::ServerSideExecutionDisabled);
    }

    let root_path = roots::resolve_root(&state.config, user, root_key, roots::RequiredCap::Write)
        .map_err(|e| ArchiveError::Root(e.to_string()))?;
    let archive = nasfiles_core::safe_path::resolve(&root_path, archive_path)
        .map_err(|e| ArchiveError::Path(e.to_string()))?;

    if !archive.is_file() {
        return Err(ArchiveError::InvalidArchive);
    }
    if !is_supported_archive_path(&archive) {
        return Err(ArchiveError::UnsupportedArchive);
    }

    let dest_dir = archive.parent().ok_or(ArchiveError::InvalidPath)?;
    let staging = dest_dir.join(format!(".extract-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&staging).await.map_err(ArchiveError::io)?;

    let result = async {
        let plan = validate_archive_listing(&archive).await?;
        plan.enforce()?;
        run_extractor(&archive, &staging).await?;
        validate_extracted_tree(&staging).await?;

        match mode {
            ExtractMode::Here | ExtractMode::HereRemove => publish_flat(&staging, dest_dir).await?,
            ExtractMode::Subfolder => {
                let folder_name = archive_folder_name(&archive)?;
                let target = dest_dir.join(folder_name);
                if target.exists() {
                    return Err(ArchiveError::AlreadyExists);
                }
                fs::rename(&staging, &target)
                    .await
                    .map_err(ArchiveError::io)?;
            }
        }

        if mode == ExtractMode::HereRemove {
            remove_archive_parts(&archive).await?;
        }

        Ok(())
    }
    .await;

    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging).await;
    }

    result
}

async fn run_extractor(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let kind = archive_kind(archive).ok_or(ArchiveError::UnsupportedArchive)?;

    let mut command = match kind {
        ArchiveKind::Tar => {
            let mut cmd = Command::new("tar");
            cmd.arg("-xf").arg(archive).arg("-C").arg(staging);
            cmd
        }
        ArchiveKind::BzipFile => return extract_bzip_file(archive, staging).await,
        ArchiveKind::Rar => return extract_rar_archive(archive, staging).await,
        ArchiveKind::SevenZipCompatible => seven_zip_extract_command(archive, staging),
    };

    run_extraction_command_with_limits(&mut command, staging, required_tool(kind)).await
}

async fn extract_rar_archive(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let primary_archive = rar_primary_archive_path(archive)?;
    let mut command = Command::new("unar");
    command
        .arg("-q")
        .arg("-D")
        .arg("-f")
        .arg("-o")
        .arg(staging)
        .arg(&primary_archive);

    match run_extraction_command_with_limits(&mut command, staging, "unar").await {
        Err(ArchiveError::Unavailable(tool)) if tool == "unar" => {
            let mut fallback = seven_zip_extract_command(&primary_archive, staging);
            run_extraction_command_with_limits(&mut fallback, staging, "7z").await
        }
        result => result,
    }
}

fn seven_zip_extract_command(archive: &Path, staging: &Path) -> Command {
    let mut cmd = Command::new("7z");
    cmd.arg("x")
        .arg("-y")
        .arg(format!("-o{}", staging.display()))
        .arg(archive);
    cmd
}

async fn run_extraction_command_with_limits(
    command: &mut Command,
    staging: &Path,
    tool: &str,
) -> Result<(), ArchiveError> {
    let mut child = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ArchiveError::Unavailable(tool.to_string())
            } else {
                ArchiveError::Io(e.to_string())
            }
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ArchiveError::Process("failed to capture extractor stderr".to_string()))?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        let mut buffer = [0u8; 1024];

        loop {
            let read = match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break,
            };

            if output.len() < MAX_EXTRACTOR_ERROR_BYTES {
                let remaining = MAX_EXTRACTOR_ERROR_BYTES - output.len();
                output.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }

        String::from_utf8_lossy(&output).trim().to_string()
    });
    let deadline = Instant::now() + EXTRACT_TIMEOUT;
    let mut interval = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            status = child.wait() => {
                let status = status.map_err(ArchiveError::io)?;
                if status.success() {
                    return Ok(());
                }
                let stderr = stderr_task.await.unwrap_or_default();
                let message = if stderr.is_empty() {
                    format!("extractor exited with status {status}")
                } else {
                    format!("extractor exited with status {status}: {stderr}")
                };
                return Err(ArchiveError::Process(message));
            }
            _ = interval.tick() => {
                if Instant::now() >= deadline {
                    let _ = child.kill().await;
                    stderr_task.abort();
                    return Err(ArchiveError::Process("archive extraction timed out".to_string()));
                }
                if let Err(err @ ArchiveError::LimitExceeded(_)) = check_tree_limits(staging).await {
                    let _ = child.kill().await;
                    stderr_task.abort();
                    return Err(err);
                }
            }
        }
    }
}

async fn extract_bzip_file(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    tokio::time::timeout(EXTRACT_TIMEOUT, extract_bzip_file_inner(archive, staging))
        .await
        .map_err(|_| ArchiveError::Process("bzip2 extraction timed out".to_string()))?
}

async fn extract_bzip_file_inner(archive: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let output_name = single_bzip_output_name(archive)?;
    let output_path = staging.join(output_name);
    let mut child = Command::new("bzip2")
        .arg("-dc")
        .arg(archive)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ArchiveError::Unavailable("bzip2".to_string())
            } else {
                ArchiveError::Io(e.to_string())
            }
        })?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ArchiveError::Process("failed to capture bzip2 stdout".to_string()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ArchiveError::Process("failed to capture bzip2 stderr".to_string()))?;
    let mut output = fs::File::create(&output_path)
        .await
        .map_err(ArchiveError::io)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut written = 0u64;

    loop {
        let read = stdout.read(&mut buffer).await.map_err(ArchiveError::io)?;
        if read == 0 {
            break;
        }

        written = written.saturating_add(read as u64);
        if written > MAX_EXTRACTED_FILE_BYTES || written > MAX_EXTRACTED_BYTES {
            let _ = child.kill().await;
            return Err(ArchiveError::LimitExceeded(format!(
                "extracted file exceeds {} bytes",
                MAX_EXTRACTED_FILE_BYTES
            )));
        }

        output
            .write_all(&buffer[..read])
            .await
            .map_err(ArchiveError::io)?;
    }

    output.flush().await.map_err(ArchiveError::io)?;

    let status = child.wait().await.map_err(ArchiveError::io)?;
    if status.success() {
        return Ok(());
    }

    let mut error = String::new();
    let _ = stderr.read_to_string(&mut error).await;
    Err(ArchiveError::Process(if error.trim().is_empty() {
        format!("bzip2 exited with status {status}")
    } else {
        error.trim().to_string()
    }))
}

async fn validate_archive_listing(archive: &Path) -> Result<ArchivePlan, ArchiveError> {
    let kind = archive_kind(archive).ok_or(ArchiveError::UnsupportedArchive)?;
    let packed_bytes = archive_input_size(archive).await?;

    match kind {
        ArchiveKind::Tar => validate_tar_listing(archive, packed_bytes).await,
        ArchiveKind::Rar => {
            validate_7z_listing(&rar_primary_archive_path(archive)?, packed_bytes).await
        }
        ArchiveKind::SevenZipCompatible => validate_7z_listing(archive, packed_bytes).await,
        ArchiveKind::BzipFile => Ok(ArchivePlan {
            entries: 1,
            total_unpacked_bytes: 0,
            largest_file_bytes: 0,
            packed_bytes,
            has_unknown_unpacked_size: true,
        }),
    }
}

async fn validate_tar_listing(
    archive: &Path,
    packed_bytes: u64,
) -> Result<ArchivePlan, ArchiveError> {
    let names = run_listing(CommandSpec::new("tar").args(["-tf"]).path_arg(archive)).await?;
    let mut entries = 0u64;
    for name in names.lines() {
        validate_archive_member_path(name)?;
        entries = entries.saturating_add(1);
    }

    let verbose = run_listing(CommandSpec::new("tar").args(["-tvf"]).path_arg(archive)).await?;
    let mut total_unpacked_bytes = 0u64;
    let mut largest_file_bytes = 0u64;
    for line in verbose.lines().filter(|line| !line.trim().is_empty()) {
        match line.as_bytes().first() {
            Some(b'-' | b'd') => {}
            _ => {
                return Err(ArchiveError::UnsafeEntry(
                    "archives containing links or special files are not supported".to_string(),
                ));
            }
        }

        if line.as_bytes().first() == Some(&b'-') {
            let size = parse_tar_verbose_size(line)?;
            total_unpacked_bytes = total_unpacked_bytes.saturating_add(size);
            largest_file_bytes = largest_file_bytes.max(size);
        }
    }

    Ok(ArchivePlan {
        entries,
        total_unpacked_bytes,
        largest_file_bytes,
        packed_bytes,
        has_unknown_unpacked_size: false,
    })
}

async fn validate_7z_listing(
    archive: &Path,
    packed_bytes: u64,
) -> Result<ArchivePlan, ArchiveError> {
    let listing = run_listing(CommandSpec::new("7z").args(["l", "-slt"]).path_arg(archive)).await?;
    let mut in_entries = false;
    let mut plan = ArchivePlan {
        entries: 0,
        total_unpacked_bytes: 0,
        largest_file_bytes: 0,
        packed_bytes,
        has_unknown_unpacked_size: false,
    };
    let mut current = SevenZipEntry::default();

    for line in listing.lines() {
        if line == "----------" {
            in_entries = true;
            continue;
        }
        if !in_entries {
            continue;
        }

        if let Some(path) = line.strip_prefix("Path = ") {
            current.finish_into(&mut plan)?;
            validate_archive_member_path(path)?;
            current = SevenZipEntry {
                seen: true,
                ..SevenZipEntry::default()
            };
        } else if let Some(folder) = line.strip_prefix("Folder = ") {
            current.is_dir = folder.trim() == "+";
        } else if let Some(size) = line.strip_prefix("Size = ") {
            current.size = Some(parse_u64_field(size, "archive entry size")?);
        } else if let Some(attributes) = line.strip_prefix("Attributes = ") {
            if attributes.contains('l') {
                return Err(ArchiveError::UnsafeEntry(
                    "archives containing symlinks are not supported".to_string(),
                ));
            }
        } else if line.starts_with("Symbolic Link = ") {
            return Err(ArchiveError::UnsafeEntry(
                "archives containing symlinks are not supported".to_string(),
            ));
        }
    }
    current.finish_into(&mut plan)?;

    Ok(plan)
}

async fn run_listing(spec: CommandSpec<'_>) -> Result<String, ArchiveError> {
    let mut command = Command::new(spec.program);
    for arg in spec.args {
        command.arg(arg);
    }
    if let Some(path) = spec.path_arg {
        command.arg(path);
    }

    let output = tokio::time::timeout(EXTRACT_TIMEOUT, command.output())
        .await
        .map_err(|_| ArchiveError::Process("archive listing timed out".to_string()))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ArchiveError::Unavailable(spec.program.to_string())
            } else {
                ArchiveError::Io(e.to_string())
            }
        })?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(ArchiveError::Process(if stderr.trim().is_empty() {
        format!("archive listing failed with status {}", output.status)
    } else {
        stderr.trim().to_string()
    }))
}

fn validate_archive_member_path(path: &str) -> Result<(), ArchiveError> {
    let member = Path::new(path);
    if path.is_empty() || member.is_absolute() {
        return Err(ArchiveError::UnsafeEntry(
            "archive contains an absolute or empty path".to_string(),
        ));
    }

    for component in member.components() {
        match component {
            std::path::Component::Normal(name) if !name.is_empty() => {}
            std::path::Component::CurDir => {}
            _ => {
                return Err(ArchiveError::UnsafeEntry(
                    "archive contains a path outside the destination".to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn parse_tar_verbose_size(line: &str) -> Result<u64, ArchiveError> {
    let fields: Vec<_> = line.split_whitespace().collect();
    let size_field = if fields
        .get(1)
        .is_some_and(|value| value.chars().all(|c| c.is_ascii_digit()))
    {
        fields.get(4)
    } else {
        fields.get(2)
    };

    size_field
        .ok_or_else(|| ArchiveError::Process("could not read tar entry size".to_string()))?
        .parse::<u64>()
        .map_err(|_| ArchiveError::Process("could not parse tar entry size".to_string()))
}

fn parse_u64_field(value: &str, label: &str) -> Result<u64, ArchiveError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| ArchiveError::Process(format!("could not parse {label}")))
}

async fn archive_input_size(archive: &Path) -> Result<u64, ArchiveError> {
    let archive = archive.to_path_buf();
    tokio::task::spawn_blocking(move || archive_input_size_sync(&archive))
        .await
        .map_err(|e| ArchiveError::Io(e.to_string()))?
}

fn archive_input_size_sync(archive: &Path) -> Result<u64, ArchiveError> {
    let parent = archive.parent().ok_or(ArchiveError::InvalidPath)?;
    let mut total = 0u64;

    for part in archive_parts(archive)? {
        let metadata = std::fs::metadata(parent.join(part)).map_err(ArchiveError::io)?;
        if !metadata.is_file() {
            return Err(ArchiveError::InvalidArchive);
        }
        total = total.saturating_add(metadata.len());
    }

    Ok(total)
}

fn rar_primary_archive_path(archive: &Path) -> Result<std::path::PathBuf, ArchiveError> {
    let parent = archive.parent().ok_or(ArchiveError::InvalidPath)?;
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ArchiveError::InvalidPath)?;
    let lower_name = name.to_ascii_lowercase();

    if let Some(prefix) = numbered_rar_part_prefix(&lower_name) {
        let primary = lowest_numbered_rar_part(parent, prefix)?;
        if let Some(primary) = primary {
            return Ok(parent.join(primary));
        }
    }

    if let Some(prefix) = old_style_rar_prefix(&lower_name) {
        let primary = matching_siblings(parent, |s| {
            s.to_ascii_lowercase() == format!("{prefix}.rar")
        })?
        .into_iter()
        .next();

        if let Some(primary) = primary {
            return Ok(parent.join(primary));
        }
    }

    Ok(archive.to_path_buf())
}

fn lowest_numbered_rar_part(parent: &Path, prefix: &str) -> Result<Option<OsString>, ArchiveError> {
    let mut lowest: Option<(u64, OsString)> = None;

    for part in matching_siblings(parent, |s| numbered_part_rar_number(s, prefix).is_some())? {
        let Some(part_name) = part.to_str() else {
            continue;
        };
        let Some(part_number) = numbered_part_rar_number(part_name, prefix) else {
            continue;
        };

        if lowest
            .as_ref()
            .is_none_or(|(lowest_number, _)| part_number < *lowest_number)
        {
            lowest = Some((part_number, part));
        }
    }

    Ok(lowest.map(|(_, name)| name))
}

#[derive(Default)]
struct SevenZipEntry {
    seen: bool,
    is_dir: bool,
    size: Option<u64>,
}

impl SevenZipEntry {
    fn finish_into(&mut self, plan: &mut ArchivePlan) -> Result<(), ArchiveError> {
        if !self.seen {
            return Ok(());
        }

        plan.entries = plan.entries.saturating_add(1);
        if !self.is_dir {
            let Some(size) = self.size else {
                plan.has_unknown_unpacked_size = true;
                return Ok(());
            };
            plan.total_unpacked_bytes = plan.total_unpacked_bytes.saturating_add(size);
            plan.largest_file_bytes = plan.largest_file_bytes.max(size);
        }

        Ok(())
    }
}

struct ArchivePlan {
    entries: u64,
    total_unpacked_bytes: u64,
    largest_file_bytes: u64,
    packed_bytes: u64,
    has_unknown_unpacked_size: bool,
}

impl ArchivePlan {
    fn enforce(&self) -> Result<(), ArchiveError> {
        if self.entries > MAX_ARCHIVE_ENTRIES {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive contains more than {} entries",
                MAX_ARCHIVE_ENTRIES
            )));
        }

        if self.largest_file_bytes > MAX_EXTRACTED_FILE_BYTES {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive contains a file larger than {} bytes",
                MAX_EXTRACTED_FILE_BYTES
            )));
        }

        if self.total_unpacked_bytes > MAX_EXTRACTED_BYTES {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive expands beyond {} bytes",
                MAX_EXTRACTED_BYTES
            )));
        }

        if !self.has_unknown_unpacked_size
            && self.packed_bytes == 0
            && self.total_unpacked_bytes > 0
        {
            return Err(ArchiveError::LimitExceeded(
                "archive has invalid packed size metadata".to_string(),
            ));
        }

        if !self.has_unknown_unpacked_size
            && self.packed_bytes > 0
            && self.total_unpacked_bytes / self.packed_bytes > MAX_COMPRESSION_RATIO
        {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive compression ratio exceeds {}:1",
                MAX_COMPRESSION_RATIO
            )));
        }

        Ok(())
    }
}

struct CommandSpec<'a> {
    program: &'a str,
    args: Vec<&'a str>,
    path_arg: Option<&'a Path>,
}

impl<'a> CommandSpec<'a> {
    fn new(program: &'a str) -> Self {
        Self {
            program,
            args: Vec::new(),
            path_arg: None,
        }
    }

    fn args(mut self, args: impl IntoIterator<Item = &'a str>) -> Self {
        self.args.extend(args);
        self
    }

    fn path_arg(mut self, path: &'a Path) -> Self {
        self.path_arg = Some(path);
        self
    }
}

async fn publish_flat(staging: &Path, dest_dir: &Path) -> Result<(), ArchiveError> {
    let mut entries = fs::read_dir(staging).await.map_err(ArchiveError::io)?;
    let mut moves = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(ArchiveError::io)? {
        let name = entry.file_name();
        let target = dest_dir.join(&name);
        if target.exists() {
            return Err(ArchiveError::AlreadyExists);
        }
        moves.push((entry.path(), target));
    }

    for (source, target) in moves {
        fs::rename(source, target).await.map_err(ArchiveError::io)?;
    }

    fs::remove_dir_all(staging)
        .await
        .map_err(ArchiveError::io)?;
    Ok(())
}

async fn validate_extracted_tree(root: &Path) -> Result<(), ArchiveError> {
    check_tree_limits(root).await
}

async fn check_tree_limits(root: &Path) -> Result<(), ArchiveError> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || check_tree_limits_sync(&root))
        .await
        .map_err(|e| ArchiveError::Io(e.to_string()))?
}

fn check_tree_limits_sync(root: &Path) -> Result<(), ArchiveError> {
    let mut pending = vec![root.to_path_buf()];
    let mut entries = 0u64;
    let mut total_bytes = 0u64;

    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(ArchiveError::io)? {
            let entry = entry.map_err(ArchiveError::io)?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(ArchiveError::io)?;
            let file_type = metadata.file_type();
            entries = entries.saturating_add(1);

            if entries > MAX_ARCHIVE_ENTRIES {
                return Err(ArchiveError::LimitExceeded(format!(
                    "extracted archive contains more than {} entries",
                    MAX_ARCHIVE_ENTRIES
                )));
            }

            if file_type.is_symlink() {
                return Err(ArchiveError::UnsafeEntry(
                    "archives containing symlinks are not supported".to_string(),
                ));
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if !file_type.is_file() {
                return Err(ArchiveError::UnsafeEntry(
                    "archives containing special files are not supported".to_string(),
                ));
            } else {
                let size = metadata.len();
                if size > MAX_EXTRACTED_FILE_BYTES {
                    return Err(ArchiveError::LimitExceeded(format!(
                        "extracted file exceeds {} bytes",
                        MAX_EXTRACTED_FILE_BYTES
                    )));
                }
                total_bytes = total_bytes.saturating_add(size);
                if total_bytes > MAX_EXTRACTED_BYTES {
                    return Err(ArchiveError::LimitExceeded(format!(
                        "extracted archive exceeds {} bytes",
                        MAX_EXTRACTED_BYTES
                    )));
                }
            }
        }
    }

    Ok(())
}

async fn remove_archive_parts(archive: &Path) -> Result<(), ArchiveError> {
    let Some(parent) = archive.parent() else {
        return Err(ArchiveError::InvalidPath);
    };
    let parts = archive_parts(archive)?;

    for part in parts {
        let target = parent.join(part);
        if target.exists() {
            fs::remove_file(&target).await.map_err(ArchiveError::io)?;
        }
    }

    Ok(())
}

fn archive_parts(archive: &Path) -> Result<Vec<OsString>, ArchiveError> {
    let parent = archive.parent().ok_or(ArchiveError::InvalidPath)?;
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ArchiveError::InvalidPath)?;
    let lower_name = name.to_ascii_lowercase();

    let mut parts = vec![OsString::from(name)];

    if let Some(prefix) = lower_name.strip_suffix(".zip") {
        parts.extend(matching_siblings(parent, |s| {
            let lower = s.to_ascii_lowercase();
            lower.len() == prefix.len() + 4
                && lower.starts_with(prefix)
                && lower.as_bytes().get(prefix.len()) == Some(&b'.')
                && lower.as_bytes().get(prefix.len() + 1) == Some(&b'z')
                && lower[prefix.len() + 2..]
                    .chars()
                    .all(|c| c.is_ascii_digit())
        })?);
    } else if let Some(prefix) = numbered_rar_part_prefix(&lower_name) {
        parts = matching_siblings(parent, |s| numbered_part_rar_number(s, prefix).is_some())?;
    } else if let Some(prefix) = lower_name.strip_suffix(".rar") {
        parts.extend(matching_siblings(parent, |s| {
            let lower = s.to_ascii_lowercase();
            lower.len() == prefix.len() + 4
                && lower.starts_with(prefix)
                && lower.as_bytes().get(prefix.len()) == Some(&b'.')
                && lower.as_bytes().get(prefix.len() + 1) == Some(&b'r')
                && lower[prefix.len() + 2..]
                    .chars()
                    .all(|c| c.is_ascii_digit())
        })?);
    } else if let Some(prefix) = old_style_rar_prefix(&lower_name) {
        parts = matching_siblings(parent, |s| {
            let lower = s.to_ascii_lowercase();
            lower == format!("{prefix}.rar")
                || (lower.len() == prefix.len() + 4
                    && lower.starts_with(prefix)
                    && lower.as_bytes().get(prefix.len()) == Some(&b'.')
                    && lower.as_bytes().get(prefix.len() + 1) == Some(&b'r')
                    && lower[prefix.len() + 2..]
                        .chars()
                        .all(|c| c.is_ascii_digit()))
        })?;
    } else if let Some(prefix) = lower_name.strip_suffix(".7z.001") {
        parts = matching_siblings(parent, |s| {
            let lower = s.to_ascii_lowercase();
            lower.len() == prefix.len() + 7
                && lower.starts_with(prefix)
                && lower[prefix.len()..].starts_with(".7z.")
                && lower[prefix.len() + 4..]
                    .chars()
                    .all(|c| c.is_ascii_digit())
        })?;
    }

    parts.sort();
    parts.dedup();
    Ok(parts)
}

fn numbered_rar_part_prefix(name: &str) -> Option<&str> {
    multipart_prefix(name, ".part", ".rar").or_else(|| multipart_prefix(name, " part", ".rar"))
}

fn numbered_part_rar_number(name: &str, prefix: &str) -> Option<u64> {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with(prefix) || !lower.ends_with(".rar") {
        return None;
    }

    let part_suffix = &lower[prefix.len()..lower.len() - 4];
    part_suffix
        .strip_prefix(".part")
        .or_else(|| part_suffix.strip_prefix(" part"))
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .and_then(|n| n.parse().ok())
}

fn old_style_rar_prefix(name: &str) -> Option<&str> {
    if name.len() < 5 {
        return None;
    }

    let (prefix, suffix) = name.rsplit_once('.')?;
    let suffix = suffix.strip_prefix('r')?;
    if suffix.len() == 2 && suffix.chars().all(|c| c.is_ascii_digit()) {
        Some(prefix)
    } else {
        None
    }
}

fn matching_siblings(
    parent: &Path,
    matches: impl Fn(&str) -> bool,
) -> Result<Vec<OsString>, ArchiveError> {
    let mut found = Vec::new();

    for entry in std::fs::read_dir(parent).map_err(ArchiveError::io)? {
        let entry = entry.map_err(ArchiveError::io)?;
        if let Some(name) = entry.file_name().to_str()
            && matches(name)
        {
            found.push(entry.file_name());
        }
    }

    Ok(found)
}

fn archive_folder_name(archive: &Path) -> Result<String, ArchiveError> {
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ArchiveError::InvalidPath)?;
    let folder_name = strip_archive_extension(name).ok_or(ArchiveError::UnsupportedArchive)?;
    validate_filename(folder_name)?;
    Ok(folder_name.to_string())
}

fn single_bzip_output_name(archive: &Path) -> Result<String, ArchiveError> {
    let name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ArchiveError::InvalidPath)?;
    let output_name = name
        .strip_suffix(".bz2")
        .or_else(|| name.strip_suffix(".BZ2"))
        .or_else(|| name.strip_suffix(".bz"))
        .or_else(|| name.strip_suffix(".BZ"))
        .ok_or(ArchiveError::UnsupportedArchive)?;
    validate_filename(output_name)?;
    Ok(output_name.to_string())
}

fn validate_filename(name: &str) -> Result<(), ArchiveError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.chars().any(|c| c.is_control())
    {
        return Err(ArchiveError::InvalidPath);
    }

    Ok(())
}

fn is_supported_archive_path(path: &Path) -> bool {
    archive_kind(path).is_some()
}

fn archive_kind(path: &Path) -> Option<ArchiveKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();

    if name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz")
        || name.ends_with(".tbz2")
    {
        Some(ArchiveKind::Tar)
    } else if name.ends_with(".bz") || name.ends_with(".bz2") {
        Some(ArchiveKind::BzipFile)
    } else if name.ends_with(".rar") || old_style_rar_prefix(&name).is_some() {
        Some(ArchiveKind::Rar)
    } else if name.ends_with(".zip") || name.ends_with(".7z") || name.ends_with(".7z.001") {
        Some(ArchiveKind::SevenZipCompatible)
    } else {
        None
    }
}

fn strip_archive_extension(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    if let Some(prefix) = numbered_rar_part_prefix(&lower) {
        return name.get(..prefix.len());
    }
    if let Some(prefix) = old_style_rar_prefix(&lower) {
        return name.get(..prefix.len());
    }

    for suffix in [
        ".tar.gz", ".tar.bz2", ".7z.001", ".tgz", ".tbz2", ".tbz", ".zip", ".rar", ".tar", ".7z",
        ".bz2", ".bz",
    ] {
        if lower.ends_with(suffix) {
            return name.get(..name.len() - suffix.len());
        }
    }

    None
}

fn multipart_prefix<'a>(name: &'a str, marker: &str, suffix: &str) -> Option<&'a str> {
    if !name.ends_with(suffix) {
        return None;
    }
    let without_suffix = &name[..name.len() - suffix.len()];
    let marker_pos = without_suffix.rfind(marker)?;
    let number = &without_suffix[marker_pos + marker.len()..];
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(&name[..marker_pos])
}

fn required_tool(kind: ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::Tar => "tar",
        ArchiveKind::BzipFile => "bzip2",
        ArchiveKind::Rar => "unar",
        ArchiveKind::SevenZipCompatible => "7z",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveKind {
    Tar,
    BzipFile,
    Rar,
    SevenZipCompatible,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("path error: {0}")]
    Path(String),
    #[error("root error: {0}")]
    Root(String),
    #[error("invalid path")]
    InvalidPath,
    #[error("archive format is not supported")]
    UnsupportedArchive,
    #[error("selected item is not an archive file")]
    InvalidArchive,
    #[error("target already exists")]
    AlreadyExists,
    #[error("archive contains unsafe entry: {0}")]
    UnsafeEntry(String),
    #[error("archive extraction limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("{0} is not available on the server")]
    Unavailable(String),
    #[error("server-side execution is disabled")]
    ServerSideExecutionDisabled,
    #[error("extractor failed: {0}")]
    Process(String),
    #[error("I/O error: {0}")]
    Io(String),
}

impl ArchiveError {
    fn io(error: impl std::fmt::Display) -> Self {
        Self::Io(error.to_string())
    }
}

impl IntoResponse for ArchiveError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            ArchiveError::Root(_) => (axum::http::StatusCode::FORBIDDEN, self.to_string()),
            ArchiveError::AlreadyExists => (axum::http::StatusCode::CONFLICT, self.to_string()),
            ArchiveError::LimitExceeded(_) => {
                (axum::http::StatusCode::PAYLOAD_TOO_LARGE, self.to_string())
            }
            ArchiveError::Unavailable(_) => (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                self.to_string(),
            ),
            ArchiveError::ServerSideExecutionDisabled => (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                self.to_string(),
            ),
            ArchiveError::Io(_) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
            _ => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
        };

        (status, axum::Json(serde_json::json!({"error": msg}))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, AuthMode};
    use crate::state::AppState;
    use nasfiles_core::models::AuthUser;
    use sqlx::any::AnyPoolOptions;
    use std::collections::HashMap;

    fn disabled_test_state() -> AppState {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .expect("lazy sqlite pool");
        AppState::new(
            AppConfig {
                bind_addr: String::new(),
                base_url: String::new(),
                session_secret: Vec::new(),
                data_dir: std::path::PathBuf::new(),
                dev_mode: false,
                auth_mode: AuthMode::Sso,
                no_server_side_execution: true,
                csp_extra_img_src: Vec::new(),
                csp_extra_media_src: Vec::new(),
                db_url: String::new(),
                common_folders: HashMap::new(),
                home_folder_root: None,
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
                thumbnail_cache_dir: std::path::PathBuf::new(),
                thumbnail_max_source_file_size: 0,
                thumbnail_max_image_width: 0,
                thumbnail_max_image_height: 0,
                thumbnail_max_image_alloc: 0,
                thumbnail_max_concurrent_generations: 1,
                media_preview_max_concurrent_transcodes: 1,
                share_token_bytes: 24,
                sftp_enabled: false,
                sftp_bind_addr: String::new(),
                sftp_host_key_path: std::path::PathBuf::new(),
                max_upload_file_size: 0,
                max_upload_request_size: 0,
                log_level: String::new(),
                trusted_proxy_depth: 1,
            },
            pool,
        )
        .expect("test app state")
    }

    fn test_user() -> AuthUser {
        AuthUser {
            user_id: "user".to_string(),
            external_id: "user".to_string(),
            username: "user".to_string(),
            display_name: "User".to_string(),
            picture_url: None,
            folder_permissions: HashMap::new(),
            has_home: false,
            is_admin: false,
        }
    }

    #[test]
    fn tar_verbose_size_parses_bsd_tar_format() {
        let line = "-rw-r--r--  0 keller staff       2 Jun 13 01:00 file.txt";
        assert_eq!(parse_tar_verbose_size(line).unwrap(), 2);
    }

    #[test]
    fn tar_verbose_size_parses_gnu_tar_format() {
        let line = "-rw-r--r-- keller/staff       2 2026-06-13 01:00 file.txt";
        assert_eq!(parse_tar_verbose_size(line).unwrap(), 2);
    }

    #[test]
    fn archive_plan_rejects_too_many_entries() {
        let plan = ArchivePlan {
            entries: MAX_ARCHIVE_ENTRIES + 1,
            total_unpacked_bytes: 1,
            largest_file_bytes: 1,
            packed_bytes: 1,
            has_unknown_unpacked_size: false,
        };

        assert!(matches!(
            plan.enforce(),
            Err(ArchiveError::LimitExceeded(_))
        ));
    }

    #[test]
    fn archive_plan_rejects_zip_bomb_ratio() {
        let plan = ArchivePlan {
            entries: 1,
            total_unpacked_bytes: MAX_COMPRESSION_RATIO + 1,
            largest_file_bytes: MAX_COMPRESSION_RATIO + 1,
            packed_bytes: 1,
            has_unknown_unpacked_size: false,
        };

        assert!(matches!(
            plan.enforce(),
            Err(ArchiveError::LimitExceeded(_))
        ));
    }

    #[test]
    fn archive_kind_routes_rar_to_rar_extractor() {
        assert_eq!(archive_kind(Path::new("movie.rar")), Some(ArchiveKind::Rar));
        assert_eq!(archive_kind(Path::new("movie.r00")), Some(ArchiveKind::Rar));
        assert_eq!(
            archive_kind(Path::new("movie.zip")),
            Some(ArchiveKind::SevenZipCompatible)
        );
        assert_eq!(
            archive_kind(Path::new("movie.7z")),
            Some(ArchiveKind::SevenZipCompatible)
        );
    }

    #[test]
    fn disabled_error_maps_to_service_unavailable() {
        let response = ArchiveError::ServerSideExecutionDisabled.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn disabled_guard_runs_before_root_path_or_archive_work() {
        let state = disabled_test_state();
        let user = test_user();

        let result = extract_archive(
            &state,
            &user,
            "missing-root",
            "missing.zip",
            ExtractMode::Here,
        )
        .await;

        assert!(matches!(
            result,
            Err(ArchiveError::ServerSideExecutionDisabled)
        ));
    }

    #[test]
    fn rar_primary_archive_path_uses_lowest_numbered_part() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(dir.path().join("show.part1.rar"));
        write_empty(dir.path().join("show.part2.rar"));
        write_empty(dir.path().join("show.part10.rar"));

        assert_eq!(
            rar_primary_archive_path(&dir.path().join("show.part2.rar")).unwrap(),
            dir.path().join("show.part1.rar")
        );
    }

    #[test]
    fn rar_primary_archive_path_uses_rar_for_old_style_volume() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(dir.path().join("show.rar"));
        write_empty(dir.path().join("show.r00"));
        write_empty(dir.path().join("show.r01"));

        assert_eq!(
            rar_primary_archive_path(&dir.path().join("show.r00")).unwrap(),
            dir.path().join("show.rar")
        );
    }

    #[test]
    fn archive_parts_discovers_numbered_rar_siblings_from_any_part() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(dir.path().join("show.part1.rar"));
        write_empty(dir.path().join("show.part2.rar"));
        write_empty(dir.path().join("show.part10.rar"));
        write_empty(dir.path().join("show.rar"));

        let parts = archive_part_names(&dir.path().join("show.part2.rar"));

        assert_eq!(
            parts,
            vec!["show.part1.rar", "show.part10.rar", "show.part2.rar"]
        );
    }

    #[test]
    fn archive_parts_discovers_space_part_rar_siblings_from_any_part() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part01.rar"),
        );
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar"),
        );
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part49.rar"),
        );
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Vol00+01.par2"),
        );
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags.par2"),
        );

        let parts = archive_part_names(
            &dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar"),
        );

        assert_eq!(
            parts,
            vec![
                "Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part01.rar",
                "Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar",
                "Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part49.rar"
            ]
        );
    }

    #[test]
    fn rar_primary_archive_path_uses_lowest_space_part_volume() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part01.rar"),
        );
        write_empty(
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar"),
        );

        assert_eq!(
            rar_primary_archive_path(
                &dir.path()
                    .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar")
            )
            .unwrap(),
            dir.path()
                .join("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part01.rar")
        );
    }

    #[test]
    fn strip_archive_extension_removes_space_part_marker() {
        assert_eq!(
            strip_archive_extension("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags Part02.rar"),
            Some("Stv 1X01 Caretaker Dvdrip Xvid Ac3-Bags")
        );
    }

    #[test]
    fn archive_parts_discovers_old_style_rar_siblings_from_secondary_volume() {
        let dir = tempfile::tempdir().unwrap();
        write_empty(dir.path().join("show.rar"));
        write_empty(dir.path().join("show.r00"));
        write_empty(dir.path().join("show.r01"));

        let parts = archive_part_names(&dir.path().join("show.r00"));

        assert_eq!(parts, vec!["show.r00", "show.r01", "show.rar"]);
    }

    fn archive_part_names(path: &Path) -> Vec<String> {
        archive_parts(path)
            .unwrap()
            .into_iter()
            .map(|name| name.into_string().unwrap())
            .collect()
    }

    fn write_empty(path: impl AsRef<Path>) {
        std::fs::write(path, []).unwrap();
    }
}
