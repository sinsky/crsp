//! `delete-deployment` (alias `undeploy`) — clasp
//! commands/delete-deployment.ts.
//!
//! Versioned deployments are those with
//! `deploymentConfig?.versionNumber !== undefined`. `--all` deletes every
//! versioned deployment; otherwise a single versioned deployment is
//! auto-selected when exactly one exists, chosen interactively when several
//! exist, or — noninteractively — the command emits `[]` in JSON mode and
//! errors with `No deployments found.` otherwise.

use std::io::Write;

use serde::Serialize;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::project::{assert_script_configured, undeploy};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptSelect, Ui};

/// `--json` payload (clasp `JSON.stringify({deletedDeploymentIds}, null, 2)`).
#[derive(Serialize)]
struct DeletedJson<'a> {
    #[serde(rename = "deletedDeploymentIds")]
    deleted_deployment_ids: &'a [String],
}

pub async fn delete_deployment<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    deployment_id: Option<&str>,
    all: bool,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<Vec<String>, CrspError> {
    let script_id = assert_script_configured(config).await?.to_string();
    let mut deleted: Vec<String> = Vec::new();

    if all {
        let deployments = client.script().list_deployments(&script_id).await?.results;
        for deployment in deployments
            .iter()
            .filter(|deployment| deployment.version_number().is_some())
        {
            let Some(id) = deployment.deployment_id.as_deref() else {
                continue;
            };
            undeploy(client, &script_id, id).await?;
            deleted.push(id.to_string());
            if !output.is_json() {
                output.message(&i18n::deleted_deployment(id));
            }
        }
        if output.is_json() {
            output.print_json(&DeletedJson {
                deleted_deployment_ids: &deleted,
            })?;
        } else {
            output.message(i18n::DELETED_ALL_DEPLOYMENTS);
        }
        return Ok(deleted);
    }

    let mut target = deployment_id.map(str::to_string);
    if target.is_none() {
        let deployments = client.script().list_deployments(&script_id).await?.results;
        let versioned: Vec<_> = deployments
            .iter()
            .filter(|deployment| deployment.version_number().is_some())
            .collect();
        if versioned.len() == 1 {
            target = versioned[0].deployment_id.clone();
        } else if ui.is_interactive() {
            let options = versioned
                .iter()
                .filter_map(|deployment| {
                    let id = deployment.deployment_id.as_deref()?;
                    let description = deployment
                        .deployment_config
                        .as_ref()
                        .and_then(|config| config.description.as_deref())
                        .unwrap_or_default();
                    Some((id.to_string(), format!("{id} - {description}")))
                })
                .collect::<Vec<_>>();
            target = Some(ui.select(PromptSelect {
                prompt: i18n::DELETE_WHICH_DEPLOYMENT.to_string(),
                options,
                default: None,
            })?);
        }
    }

    match target {
        Some(id) => {
            undeploy(client, &script_id, &id).await?;
            deleted.push(id.clone());
            if !output.is_json() {
                output.message(&i18n::deleted_deployment(&id));
            } else {
                output.print_json(&DeletedJson {
                    deleted_deployment_ids: &deleted,
                })?;
            }
            Ok(deleted)
        }
        None if output.is_json() => {
            output.print_json(&DeletedJson {
                deleted_deployment_ids: &[],
            })?;
            Ok(deleted)
        }
        None => Err(CrspError::Validation(
            i18n::NO_DEPLOYMENTS_FOUND.to_string(),
        )),
    }
}
