//! `list-deployments` (alias `deployments`) — clasp
//! commands/list-deployments.ts.
//!
//! Human output prints `- {id} @{version|HEAD} {- description}` lines (with
//! clasp's trailing space when the description is empty); `--json` prints
//! `{deploymentId, versionNumber, description}` entries with undefined keys
//! omitted.

use std::io::Write;

use crate::api::{ApiClient, Deployment};
use crate::commands::shared::DeploymentJson;
use crate::core::config::ProjectConfig;
use crate::core::project::assert_script_configured;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

pub async fn list_deployments<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    script_id: Option<&str>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<Vec<Deployment>, CrspError> {
    let script_id = match script_id {
        Some(script_id) => script_id.to_string(),
        None => assert_script_configured(config).await?.to_string(),
    };
    let outcome = ui.with_spinner(i18n::FETCHING_DEPLOYMENTS, move || {
        crate::ui::drive_isolated(async move { client.script().list_deployments(&script_id).await })
    })?;
    let deployments = (outcome?).results;
    if output.is_json() {
        let entries: Vec<_> = deployments.iter().map(DeploymentJson::of).collect();
        output.print_json(&entries)?;
    } else if deployments.is_empty() {
        output.message(i18n::NO_DEPLOYMENTS);
    } else {
        output.message(&i18n::found_items(
            deployments.len(),
            "deployment",
            "deployments",
        ));
        for deployment in deployments.iter().filter(|deployment| {
            deployment.deployment_config.is_some() && deployment.deployment_id.is_some()
        }) {
            // clasp: `versionNumber ? `@${n}` : '@HEAD'` (0 reads as HEAD) and
            // a truthy description renders as `- {description}`.
            let version = match deployment.version_number() {
                Some(number) if number != 0 => format!("@{number}"),
                _ => "@HEAD".to_string(),
            };
            let description = deployment
                .deployment_config
                .as_ref()
                .and_then(|config| config.description.as_deref())
                .filter(|description| !description.is_empty())
                .map(|description| format!("- {description}"))
                .unwrap_or_default();
            output.message(&format!(
                "- {} {version} {description}",
                deployment.deployment_id.as_deref().unwrap_or_default()
            ));
        }
    }
    Ok(deployments)
}
