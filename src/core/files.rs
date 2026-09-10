use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use futures::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::api::{ApiClient, PushFile, ScriptFile};
use crate::core::config::ProjectConfig;
use crate::core::path::{PathJail, normalize_slashes};
use crate::error::CrspError;
use crate::i18n;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Symlink,
    UnsupportedType,
    OutsideContentDir,
    ParentSymlink,
    TargetSymlink,
    RaceCondition,
    SymlinkLoop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedFile {
    pub local_path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalFile {
    pub local_path: String,
    pub remote_path: String,
    pub file_type: String,
    pub source: String,
}

impl LocalFile {
    pub fn new(local_path: &str, remote_path: &str, file_type: &str, source: &str) -> Self {
        Self {
            local_path: normalize_slashes(local_path).into_owned(),
            remote_path: normalize_slashes(remote_path).into_owned(),
            file_type: file_type.to_string(),
            source: source.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFile {
    pub remote_path: String,
    pub file_type: String,
    pub source: String,
}

impl PullFile {
    pub fn new(remote_path: &str, file_type: &str, source: &str) -> Self {
        Self {
            remote_path: normalize_slashes(remote_path).into_owned(),
            file_type: file_type.to_string(),
            source: source.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CollectLocalFilesResult {
    pub files: Vec<LocalFile>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PullResult {
    #[serde(rename = "pulledFiles")]
    pub written: Vec<String>,
    pub skipped: Vec<SkippedFile>,
    #[serde(rename = "deletedFiles")]
    pub deleted: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFault {
    RaceCondition,
    SymlinkLoop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PushResult {
    pub files: Vec<LocalFile>,
    pub changed: Vec<LocalFile>,
    pub skipped: Vec<SkippedFile>,
    pub up_to_date: bool,
}

pub async fn collect_local_files(
    config: &ProjectConfig,
) -> Result<CollectLocalFilesResult, CrspError> {
    let matcher = config.ignore_matcher().await?;
    let root = config.content_dir.clone();
    let allow = config.allow_symlinks;
    if !allow
        && fs::symlink_metadata(&root)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Ok(CollectLocalFilesResult {
            skipped: vec![SkippedFile {
                local_path: ".".to_string(),
                reason: SkipReason::Symlink,
            }],
            ..Default::default()
        });
    }
    let mut entries = Vec::new();
    collect_entries(
        &root,
        &root,
        config.skip_subdirectories,
        allow,
        &mut entries,
    )?;
    entries.sort();
    let mut result = CollectLocalFilesResult::default();
    let mut collisions = HashSet::new();
    for (relative, path, symlink) in entries {
        let relative = normalize_slashes(&relative).into_owned();
        if !matcher.is_tracked(&relative) {
            continue;
        }
        if symlink && !allow {
            result.skipped.push(SkippedFile {
                local_path: relative,
                reason: SkipReason::Symlink,
            });
            continue;
        }
        let metadata = if allow {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        }?;
        if !metadata.file_type().is_file() {
            result.skipped.push(SkippedFile {
                local_path: relative,
                reason: SkipReason::UnsupportedType,
            });
            continue;
        }
        let extension = Path::new(&relative)
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| format!(".{x}"));
        let file_type = match extension.as_deref() {
            Some(ext) if config.script_extensions.iter().any(|x| x == ext) => "SERVER_JS",
            Some(ext) if config.html_extensions.iter().any(|x| x == ext) => "HTML",
            Some(ext)
                if config.json_extensions.iter().any(|x| x == ext)
                    && Path::new(&relative).file_stem().and_then(|x| x.to_str())
                        == Some("appsscript") =>
            {
                "JSON"
            }
            _ => {
                result.skipped.push(SkippedFile {
                    local_path: relative,
                    reason: SkipReason::UnsupportedType,
                });
                continue;
            }
        };
        let key = Path::new(&relative)
            .with_extension("")
            .to_string_lossy()
            .to_string();
        if file_type == "SERVER_JS" && !collisions.insert(key.clone()) {
            return Err(CrspError::Validation(format!(
                "Conflicting files found: {key}"
            )));
        }
        let source = fs::read_to_string(&path)?;
        let remote = if file_type == "JSON" {
            "appsscript".to_string()
        } else {
            let mut value = relative.clone();
            if let Some(ext) = extension {
                value.truncate(value.len() - ext.len());
            }
            value
        };
        result
            .files
            .push(LocalFile::new(&relative, &remote, file_type, &source));
    }
    sort_push_order(&mut result.files, &config.file_push_order);
    Ok(result)
}

fn collect_entries(
    root: &Path,
    current: &Path,
    shallow: bool,
    allow: bool,
    output: &mut Vec<(String, PathBuf, bool)>,
) -> Result<(), CrspError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            if allow {
                let metadata = fs::metadata(&path);
                if let Ok(metadata) = metadata {
                    if metadata.is_dir() && !shallow {
                        collect_entries(root, &path, false, true, output)?;
                    } else if metadata.is_file() {
                        output.push((relative, path, true));
                    }
                }
            } else {
                output.push((relative, path, true));
            }
        } else if file_type.is_dir() {
            if !shallow {
                collect_entries(root, &path, false, allow, output)?;
            }
        } else {
            output.push((relative, path, false));
        }
    }
    Ok(())
}

pub fn sort_for_test(files: &mut [LocalFile], order: &[String]) {
    sort_push_order(files, order);
}

/// JS `localeCompare` approximation used for display sorts (clasp sorts
/// untracked files with `a.localeCompare(b)`).
pub fn locale_key(value: &str) -> (String, String, String) {
    let folded = value.to_lowercase();
    let case_tie = value
        .chars()
        .map(|ch| if ch.is_lowercase() { '0' } else { '1' })
        .collect();
    (folded, case_tie, value.to_string())
}

fn sort_push_order(files: &mut [LocalFile], order: &[String]) {
    files.sort_by(|left, right| {
        let left_index = order
            .iter()
            .position(|item| normalize_slashes(item) == left.local_path);
        let right_index = order
            .iter()
            .position(|item| normalize_slashes(item) == right.local_path);
        left_index
            .is_none()
            .cmp(&right_index.is_none())
            .then_with(|| {
                left_index
                    .unwrap_or(usize::MAX)
                    .cmp(&right_index.unwrap_or(usize::MAX))
            })
            .then_with(|| locale_key(&left.local_path).cmp(&locale_key(&right.local_path)))
    });
}

pub fn get_changed_files(local: &[LocalFile], remote: &[PullFile]) -> Vec<LocalFile> {
    local
        .iter()
        .filter(|file| {
            remote
                .iter()
                .find(|other| other.remote_path == file.remote_path)
                .is_none_or(|other| other.source != file.source)
        })
        .cloned()
        .collect()
}

/// The local naming view of the project extension settings (clasp
/// `getFileExtension`, files.ts:186-211): the FIRST configured extension per
/// type, falling back to clasp's defaults (`.js`/`.html`/`.json`) when the
/// list is empty. Single source of truth for remote→local naming in the pull
/// pipeline, clone/create initial pulls, and the `--deleteUnusedFiles`
/// matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExtensions {
    script: String,
    html: String,
    json: String,
}

impl LocalExtensions {
    /// Reads the first entry of the project's fixed-up extension lists.
    pub fn from_config(config: &ProjectConfig) -> Self {
        Self {
            script: config
                .script_extensions
                .first()
                .cloned()
                .unwrap_or_else(|| ".js".to_string()),
            html: config
                .html_extensions
                .first()
                .cloned()
                .unwrap_or_else(|| ".html".to_string()),
            json: config
                .json_extensions
                .first()
                .cloned()
                .unwrap_or_else(|| ".json".to_string()),
        }
    }

    /// clasp `readFileExtensions({})` defaults.
    pub fn clasp_defaults() -> Self {
        Self {
            script: ".js".to_string(),
            html: ".html".to_string(),
            json: ".json".to_string(),
        }
    }

    /// clasp `getFileExtension`: SERVER_JS/HTML/JSON map to their first
    /// configured extension; unknown types carry no extension.
    pub fn extension_for_type(&self, file_type: &str) -> &str {
        match file_type {
            "SERVER_JS" => &self.script,
            "HTML" => &self.html,
            "JSON" => &self.json,
            _ => "",
        }
    }

    /// The local file name for a remote file (clasp `fetchRemote`:
    /// `{remotePath}{extension}`; the manifest's remote name is `appsscript`).
    pub fn local_name(&self, file: &PullFile) -> String {
        format!(
            "{}{}",
            file.remote_path,
            self.extension_for_type(&file.file_type)
        )
    }
}

pub async fn pull_file(
    content_dir: &Path,
    allow_symlinks: bool,
    file: &PullFile,
    extensions: &LocalExtensions,
) -> Result<Option<String>, CrspError> {
    match pull_file_with_fault(content_dir, allow_symlinks, file, None, extensions) {
        Ok(value) => Ok(value),
        Err(_) => Ok(None),
    }
}

pub fn pull_file_with_fault(
    content_dir: &Path,
    allow_symlinks: bool,
    file: &PullFile,
    fault: Option<WriteFault>,
    extensions: &LocalExtensions,
) -> Result<Option<String>, SkipReason> {
    let name = extensions.local_name(file);
    let Some(target) = PathJail::remote_target_path(content_dir, &name) else {
        return Err(SkipReason::OutsideContentDir);
    };
    if let Some(fault) = fault {
        return Err(match fault {
            WriteFault::RaceCondition => SkipReason::RaceCondition,
            WriteFault::SymlinkLoop => SkipReason::SymlinkLoop,
        });
    }
    if !allow_symlinks
        && fs::symlink_metadata(&target)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Err(SkipReason::TargetSymlink);
    }
    if !allow_symlinks {
        let mut current = target.parent().unwrap_or(content_dir);
        while current != content_dir && PathJail::is_inside(content_dir, current) {
            if fs::symlink_metadata(current)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(SkipReason::ParentSymlink);
            }
            current = current.parent().unwrap_or(content_dir);
        }
    }
    write_remote_file(content_dir, &target, allow_symlinks, &file.source)?;
    Ok(Some(
        normalize_slashes(
            target
                .strip_prefix(content_dir)
                .unwrap_or(&target)
                .to_string_lossy()
                .as_ref(),
        )
        .into_owned(),
    ))
}

fn write_remote_file(
    content_dir: &Path,
    target: &Path,
    allow_symlinks: bool,
    source: &str,
) -> Result<(), SkipReason> {
    if !allow_symlinks
        && fs::symlink_metadata(content_dir)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Err(SkipReason::ParentSymlink);
    }
    fs::create_dir_all(content_dir).map_err(|_| SkipReason::RaceCondition)?;
    let real_content = fs::canonicalize(content_dir).map_err(|_| SkipReason::OutsideContentDir)?;
    let parent = target.parent().unwrap_or(content_dir);
    if !verify_or_create_parent(parent, &real_content, allow_symlinks)
        .map_err(|_| SkipReason::ParentSymlink)?
    {
        return Err(SkipReason::ParentSymlink);
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if !allow_symlinks {
            options.custom_flags(libc::O_NOFOLLOW);
        }
        options.mode(0o644);
    }
    let mut file = match options.open(target) {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            return Err(SkipReason::SymlinkLoop);
        }
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            return Err(SkipReason::RaceCondition);
        }
        Err(_) => return Err(SkipReason::RaceCondition),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let fd_metadata = file.metadata().map_err(|_| SkipReason::RaceCondition)?;
        let path_metadata = fs::metadata(target).map_err(|_| SkipReason::RaceCondition)?;
        if fd_metadata.dev() != path_metadata.dev() || fd_metadata.ino() != path_metadata.ino() {
            return Err(SkipReason::RaceCondition);
        }
        let real_target = fs::canonicalize(target).map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                SkipReason::SymlinkLoop
            } else {
                SkipReason::ParentSymlink
            }
        })?;
        if !allow_symlinks && !PathJail::is_inside(&real_content, &real_target) {
            return Err(SkipReason::ParentSymlink);
        }
    }
    file.set_len(0).map_err(|_| SkipReason::RaceCondition)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| SkipReason::RaceCondition)?;
    file.write_all(source.as_bytes())
        .map_err(|_| SkipReason::RaceCondition)?;
    Ok(())
}

fn verify_or_create_parent(
    parent: &Path,
    content_dir: &Path,
    allow_symlinks: bool,
) -> Result<bool, CrspError> {
    if !allow_symlinks {
        let mut current = parent;
        while current != content_dir && PathJail::is_inside(content_dir, current) {
            if let Ok(metadata) = fs::symlink_metadata(current)
                && metadata.file_type().is_symlink()
            {
                return Ok(false);
            }
            current = current.parent().unwrap_or(content_dir);
        }
    }
    match fs::create_dir_all(parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let resolved = fs::canonicalize(parent)?;
    if !allow_symlinks && resolved != *content_dir && !PathJail::is_inside(content_dir, &resolved) {
        return Ok(false);
    }
    Ok(true)
}

pub async fn pull_files(
    files: &[PullFile],
    content_dir: &Path,
    allow_symlinks: bool,
    max_writes: usize,
    extensions: &LocalExtensions,
) -> Result<PullResult, CrspError> {
    let semaphore = std::sync::Arc::new(Semaphore::new(max_writes.clamp(1, 32)));
    let results = stream::iter(files.iter().cloned().enumerate().map(|(index, file)| {
        let semaphore = std::sync::Arc::clone(&semaphore);
        let content_dir = content_dir.to_path_buf();
        let extensions = extensions.clone();
        async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|_| CrspError::Validation("write semaphore closed".to_string()))?;
            let outcome = tokio::task::spawn_blocking(move || {
                match pull_file_with_fault(&content_dir, allow_symlinks, &file, None, &extensions) {
                    Ok(Some(path)) => (Some(path), None),
                    Ok(None) => (None, None),
                    Err(reason) => (
                        None,
                        Some(SkippedFile {
                            local_path: file.remote_path,
                            reason,
                        }),
                    ),
                }
            })
            .await
            .map_err(|error| CrspError::Validation(error.to_string()))?;
            Ok::<_, CrspError>((index, outcome))
        }
    }))
    .buffer_unordered(32)
    .try_collect::<Vec<_>>()
    .await?;
    let mut result = PullResult::default();
    let mut results = results;
    results.sort_by_key(|(index, _)| *index);
    for (_, (written, skipped)) in results {
        if let Some(path) = written {
            result.written.push(path);
        }
        if let Some(skipped) = skipped {
            result.skipped.push(skipped);
        }
    }
    Ok(result)
}

pub async fn prepare_push(
    client: &ApiClient,
    config: &ProjectConfig,
) -> Result<PushResult, CrspError> {
    // The script-id assert runs before the local tree walk so unconfigured
    // runs fail fast instead of scanning the whole content dir first.
    let script_id = config
        .script_id
        .as_deref()
        .ok_or_else(|| CrspError::Config(i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))?;
    let collected = collect_local_files(config).await?;
    let remote = client.script().get_content(script_id, None).await?;
    let remote = remote
        .files()
        .iter()
        .filter_map(script_file_to_pull)
        .collect::<Vec<_>>();
    let changed = get_changed_files(&collected.files, &remote);
    Ok(PushResult {
        files: collected.files,
        changed: changed.clone(),
        skipped: collected.skipped,
        up_to_date: changed.is_empty(),
    })
}

pub async fn put_push_files(
    client: &ApiClient,
    config: &ProjectConfig,
    result: &PushResult,
) -> Result<(), CrspError> {
    if result.up_to_date {
        return Ok(());
    }
    let script_id = config
        .script_id
        .as_deref()
        .ok_or_else(|| CrspError::Config(i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))?;
    let body = result
        .files
        .iter()
        .map(|file| PushFile {
            name: file.remote_path.clone(),
            file_type: file.file_type.clone(),
            source: file.source.clone(),
        })
        .collect::<Vec<_>>();
    match client.script().update_content(script_id, &body).await {
        Ok(()) => Ok(()),
        Err(error) => {
            if let CrspError::Api { message, .. } = &error
                && let Some(snippet) = syntax_error_snippet(message, &result.files)
            {
                return Err(CrspError::Validation(format!("{message}\n{snippet}")));
            }
            Err(error)
        }
    }
}

pub async fn push_files(
    client: &ApiClient,
    config: &ProjectConfig,
) -> Result<PushResult, CrspError> {
    let result = prepare_push(client, config).await?;
    put_push_files(client, config, &result).await?;
    Ok(result)
}

fn script_file_to_pull(file: &ScriptFile) -> Option<PullFile> {
    Some(PullFile::new(
        file.name.as_deref()?,
        file.file_type.as_deref()?,
        file.source.as_deref().unwrap_or_default(),
    ))
}

pub fn syntax_error_snippet(message: &str, files: &[LocalFile]) -> Option<String> {
    let re = regex::Regex::new(r"Syntax error: (.+) line: (\d+) file: (.+)").ok()?;
    let captures = re.captures(message)?;
    let remote_name = captures.get(3)?.as_str();
    let file = files.iter().find(|file| file.remote_path == remote_name)?;
    let line: usize = captures.get(2)?.as_str().parse().ok()?;
    let lines: Vec<_> = file.source.lines().collect();
    let index = line.saturating_sub(1);
    lines.get(index).map(|source| {
        format!(
            "{} - \"{}:{}\"\n=> {}",
            &captures[1], remote_name, line, source
        )
    })
}
