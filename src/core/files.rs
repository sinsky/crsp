use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

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
    pub written: Vec<String>,
    pub skipped: Vec<SkippedFile>,
    pub deleted: Vec<String>,
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

fn sort_push_order(files: &mut [LocalFile], order: &[String]) {
    files.sort_by_key(|file| {
        let index = order
            .iter()
            .position(|item| normalize_slashes(item) == file.local_path);
        (
            index.is_none(),
            index.unwrap_or(usize::MAX),
            file.local_path.clone(),
        )
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

pub async fn pull_file(
    content_dir: &Path,
    allow_symlinks: bool,
    file: &PullFile,
) -> Result<Option<String>, CrspError> {
    let extension = extension_for_type(&file.file_type);
    let name = if file.file_type == "JSON" && file.remote_path == "appsscript" {
        "appsscript.json".to_string()
    } else {
        format!("{}{}", file.remote_path, extension)
    };
    let Some(target) = PathJail::remote_target_path(content_dir, &name) else {
        return Ok(None);
    };
    if !write_remote_file(content_dir, &target, allow_symlinks, &file.source)? {
        return Ok(None);
    }
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

fn extension_for_type(file_type: &str) -> &'static str {
    match file_type {
        "SERVER_JS" => ".js",
        "HTML" => ".html",
        "JSON" => ".json",
        _ => "",
    }
}

fn write_remote_file(
    content_dir: &Path,
    target: &Path,
    allow_symlinks: bool,
    source: &str,
) -> Result<bool, CrspError> {
    if !allow_symlinks
        && fs::symlink_metadata(content_dir)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Ok(false);
    }
    fs::create_dir_all(content_dir)?;
    let real_content = fs::canonicalize(content_dir).unwrap_or_else(|_| content_dir.to_path_buf());
    let parent = target.parent().unwrap_or(content_dir);
    if !verify_or_create_parent(parent, &real_content, allow_symlinks)? {
        return Ok(false);
    }
    if !allow_symlinks
        && fs::symlink_metadata(target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Ok(false);
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
        if !allow_symlinks {
            options.custom_flags(libc::O_NOFOLLOW);
        }
    }
    let mut file = match options.open(target) {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    file.write_all(source.as_bytes())?;
    Ok(true)
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
) -> Result<PullResult, CrspError> {
    let semaphore = Semaphore::new(max_writes.max(1));
    let mut result = PullResult::default();
    for file in files {
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|_| CrspError::Validation("write semaphore closed".to_string()))?;
        if let Some(path) = pull_file(content_dir, allow_symlinks, file).await? {
            result.written.push(path);
        }
    }
    Ok(result)
}

pub async fn push_files(
    client: &ApiClient,
    config: &ProjectConfig,
) -> Result<PushResult, CrspError> {
    let collected = collect_local_files(config).await?;
    let script_id = config
        .script_id
        .as_deref()
        .ok_or_else(|| CrspError::Config(i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))?;
    let remote = client.script().get_content(script_id, None).await?;
    let remote = remote
        .files()
        .iter()
        .filter_map(script_file_to_pull)
        .collect::<Vec<_>>();
    let changed = get_changed_files(&collected.files, &remote);
    if changed.is_empty() {
        return Ok(PushResult {
            files: collected.files,
            changed,
            skipped: collected.skipped,
            up_to_date: true,
        });
    }
    let body = collected
        .files
        .iter()
        .map(|file| PushFile {
            name: file.remote_path.clone(),
            file_type: file.file_type.clone(),
            source: file.source.clone(),
        })
        .collect::<Vec<_>>();
    client.script().update_content(script_id, &body).await?;
    Ok(PushResult {
        files: collected.files,
        changed,
        skipped: collected.skipped,
        up_to_date: false,
    })
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
