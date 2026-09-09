use serde::Serialize;

use crate::core::config::ProjectConfig;
use crate::core::files::{LocalFile, collect_local_files};
use crate::error::CrspError;
use crate::output::Output;

#[derive(Debug, Serialize)]
pub struct StatusPayload {
    #[serde(rename = "filesToPush")]
    files_to_push: Vec<String>,
    #[serde(rename = "untrackedFiles")]
    untracked_files: Vec<String>,
}

pub async fn show_file_status<W: std::io::Write, E: std::io::Write>(
    config: &ProjectConfig,
    output: &mut Output<W, E>,
) -> Result<StatusPayload, CrspError> {
    let collected = collect_local_files(config).await?;
    let tracked = collected
        .files
        .iter()
        .map(|file| file.local_path.clone())
        .collect::<Vec<_>>();
    let untracked = collect_untracked(config, &collected.files).await?;
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

async fn collect_untracked(
    config: &ProjectConfig,
    tracked: &[LocalFile],
) -> Result<Vec<String>, CrspError> {
    let tracked: std::collections::HashSet<_> = tracked
        .iter()
        .map(|file| file.local_path.as_str())
        .collect();
    let matcher = config.ignore_matcher().await?;
    let mut all = Vec::new();
    collect_paths(&config.content_dir, &config.content_dir, &mut all)?;
    let mut untracked = std::collections::BTreeSet::new();
    for path in all {
        if tracked.contains(path.as_str()) || !matcher.is_tracked(&path) {
            continue;
        }
        let components: Vec<_> = path.split('/').collect();
        if components.len() > 1 {
            untracked.insert(format!("{}/", components[0]));
        } else {
            untracked.insert(path);
        }
    }
    Ok(untracked.into_iter().collect())
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
