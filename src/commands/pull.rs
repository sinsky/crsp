use std::path::Path;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{PullFile, PullResult, pull_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptConfirm, Ui};

pub async fn pull(
    _client: &ApiClient,
    config: &ProjectConfig,
    remote: &[PullFile],
    delete_unused: bool,
    force: bool,
    ui: &Ui,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PullResult, CrspError> {
    if delete_unused && !force && !ui.is_interactive() {
        output.warn(
            "You are not in an interactive terminal and --force not used. Skipping file deletion.",
        );
    }
    let result = pull_files(remote, &config.content_dir, config.allow_symlinks, 32).await?;
    if delete_unused && (force || ui.is_interactive()) {
        let confirmed = force
            || ui.confirm(PromptConfirm {
                prompt: "Delete unused files?".to_string(),
                default: false,
            })?;
        if !confirmed {
            return Ok(result);
        }
    }
    if output.is_json() {
        output.print_json(&result)?;
    }
    Ok(result)
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}
