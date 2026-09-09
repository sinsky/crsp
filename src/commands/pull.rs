use std::fs;
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{PullFile, PullResult, pull_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

pub async fn pull<A: PromptAdapter>(
    _client: &ApiClient,
    config: &ProjectConfig,
    remote: &[PullFile],
    delete_unused: bool,
    force: bool,
    ui: &Ui<A>,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PullResult, CrspError> {
    if delete_unused && !force && !ui.is_interactive() {
        output.warn(
            "You are not in an interactive terminal and --force not used. Skipping file deletion.",
        );
    }
    let mut result = pull_files(remote, &config.content_dir, config.allow_symlinks, 32).await?;
    if delete_unused && (force || ui.is_interactive()) {
        let confirmed = force
            || ui.confirm(PromptConfirm {
                prompt: "Delete unused files?".to_string(),
                default: false,
            })?;
        if confirmed {
            let remote_names: std::collections::HashSet<_> =
                remote.iter().map(local_name).collect();
            let mut candidates = Vec::new();
            collect_files(&config.content_dir, &config.content_dir, &mut candidates)?;
            for (relative, target) in candidates {
                if !remote_names.contains(&relative)
                    && safe_delete(&config.content_dir, &target, config.allow_symlinks)?
                {
                    fs::remove_file(&target)?;
                    result.deleted.push(relative);
                }
            }
        }
    }
    if output.is_json() {
        #[derive(serde::Serialize)]
        struct PullOutput<'a> {
            #[serde(rename = "pulledFiles")]
            pulled_files: &'a [String],
            #[serde(rename = "deletedFiles")]
            deleted_files: &'a [String],
        }
        output.print_json(&PullOutput {
            pulled_files: &result.written,
            deleted_files: &result.deleted,
        })?;
    }
    Ok(result)
}

fn local_name(file: &PullFile) -> String {
    if file.file_type == "JSON" && file.remote_path == "appsscript" {
        "appsscript.json".to_string()
    } else {
        format!(
            "{}{}",
            file.remote_path,
            match file.file_type.as_str() {
                "SERVER_JS" => ".js",
                "HTML" => ".html",
                "JSON" => ".json",
                _ => "",
            }
        )
    }
}

fn collect_files(
    root: &Path,
    current: &Path,
    result: &mut Vec<(String, PathBuf)>,
) -> Result<(), CrspError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if entry.file_type()?.is_dir() {
            collect_files(root, &path, result)?;
        } else {
            result.push((relative, path));
        }
    }
    Ok(())
}

fn safe_delete(content_dir: &Path, target: &Path, allow_symlinks: bool) -> Result<bool, CrspError> {
    if !Path::new(target).starts_with(content_dir)
        || (!allow_symlinks && fs::symlink_metadata(target)?.file_type().is_symlink())
    {
        return Ok(false);
    }
    if !allow_symlinks {
        let parent = fs::canonicalize(target.parent().unwrap_or(content_dir))?;
        let root = fs::canonicalize(content_dir)?;
        if parent != root && !crate::core::path::PathJail::is_inside(&root, &parent) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}
