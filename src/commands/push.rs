use std::path::Path;
use std::time::Duration;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{PushResult, prepare_push, put_push_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

pub async fn push<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    mut force: bool,
    watch: bool,
    ui: &Ui<A>,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PushResult, CrspError> {
    loop {
        let result = prepare_push(client, config).await?;
        if !force
            && result
                .changed
                .iter()
                .any(|file| file.local_path == "appsscript.json")
        {
            let confirmed = ui.confirm(PromptConfirm {
                prompt: "Manifest file has been updated. Do you want to push and overwrite?"
                    .to_string(),
                default: false,
            })?;
            if !confirmed {
                output.message("Skipping push.");
                return Ok(result);
            }
            force = true;
        }
        put_push_files(client, config, &result).await?;
        if result.up_to_date {
            output.message("Script is already up to date.");
        } else if output.is_json() {
            output.print_json(
                &result
                    .files
                    .iter()
                    .map(|file| file.local_path.clone())
                    .collect::<Vec<_>>(),
            )?;
        }
        if !watch {
            return Ok(result);
        }
        output.message("Waiting for changes...");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}
