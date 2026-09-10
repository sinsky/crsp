use serde::Serialize;

use crate::core::config::ProjectConfig;
use crate::core::files::{LocalFile, collect_local_files};
use crate::core::path::relative_path;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

#[derive(Debug, Serialize)]
pub struct StatusPayload {
    #[serde(rename = "filesToPush")]
    pub files_to_push: Vec<String>,
    #[serde(rename = "untrackedFiles")]
    pub untracked_files: Vec<String>,
}

pub async fn show_file_status<A: PromptAdapter, W: std::io::Write, E: std::io::Write>(
    cwd: &std::path::Path,
    config: &ProjectConfig,
    ui: &Ui<A>,
    output: &mut Output<W, E>,
) -> Result<StatusPayload, CrspError> {
    let outcome = ui.with_spinner(i18n::ANALYZING_PROJECT_FILES, move || {
        crate::ui::drive_isolated(async move { collect_local_files(config).await })
    })?;
    let collected = outcome?;
    // clasp `localPath` values are `path.relative(cwd, …)`; the core
    // collection is contentDir-relative, so convert for every display path.
    let tracked = collected
        .files
        .iter()
        .map(|file| cwd_relative(cwd, &config.content_dir, &file.local_path))
        .collect::<Vec<_>>();
    let untracked = collect_untracked(cwd, config, &collected.files).await?;
    let payload = StatusPayload {
        files_to_push: tracked.clone(),
        untracked_files: untracked.clone(),
    };
    if output.is_json() {
        output.print_json(&payload)?;
    } else {
        output.message("Tracked files:");
        for file in &tracked {
            output.message(&format!("└─ {file}"));
        }
        output.message("Untracked files:");
        for file in &untracked {
            output.message(&format!("└─ {file}"));
        }
    }
    Ok(payload)
}

/// clasp `path.relative(cwd, path.join(contentDir, localPath))`.
pub fn cwd_relative(
    cwd: &std::path::Path,
    content_dir: &std::path::Path,
    local_path: &str,
) -> String {
    let absolute = content_dir.join(local_path.replace('\\', "/"));
    relative_path(cwd, &absolute)
}

/// clasp `getUntrackedFiles` (files.ts:507-549): every contentDir file not in
/// the tracked set, collapsed to the nearest untracked parent directory
/// (cwd-relative), sorted with clasp's `localeCompare`.
async fn collect_untracked(
    cwd: &std::path::Path,
    config: &ProjectConfig,
    tracked: &[LocalFile],
) -> Result<Vec<String>, CrspError> {
    let mut dirs_with_included_files = std::collections::HashSet::new();
    let tracked_paths: std::collections::HashSet<_> = tracked
        .iter()
        .map(|file| cwd_relative(cwd, &config.content_dir, &file.local_path))
        .collect();
    for path in &tracked_paths {
        for dir in parent_dirs(path) {
            dirs_with_included_files.insert(dir);
        }
    }
    let mut all = Vec::new();
    collect_paths(&config.content_dir, &config.content_dir, &mut all)?;
    let mut untracked = std::collections::BTreeSet::new();
    for path in all {
        let resolved = cwd_relative(cwd, &config.content_dir, &path);
        if tracked_paths.contains(&resolved) {
            continue;
        }
        let mut excluded = resolved.clone();
        for dir in parent_dirs(&resolved) {
            if dirs_with_included_files.contains(&dir) {
                break;
            }
            excluded = format!("{dir}/");
        }
        untracked.insert(excluded);
    }
    let mut sorted: Vec<String> = untracked.into_iter().collect();
    sorted.sort_by(|left, right| {
        crate::core::files::locale_key(left).cmp(&crate::core::files::locale_key(right))
    });
    Ok(sorted)
}

/// clasp `parentDirs` (files.ts): every ancestor directory of a relative
/// path, nearest first, stopping above the root.
fn parent_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut current = match path.rfind('/') {
        Some(index) => path[..index].to_string(),
        None => return dirs,
    };
    while !current.is_empty() {
        dirs.push(current.clone());
        current = match current.rfind('/') {
            Some(index) => current[..index].to_string(),
            None => break,
        };
    }
    dirs
}

fn collect_paths(
    root: &std::path::Path,
    current: &std::path::Path,
    result: &mut Vec<String>,
) -> Result<(), CrspError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if entry.file_type()?.is_dir() {
            collect_paths(root, &path, result)?;
        } else {
            result.push(relative);
        }
    }
    Ok(())
}
