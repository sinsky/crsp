//! `update-deployment` (alias `redeploy`) — clasp
//! commands/update-deployment.ts and core/project.ts `deploy`.
//!
//! Requires the `<deploymentId>` positional; without `-V` an immutable
//! version is created first, then the deployment is updated (clasp funnels
//! both commands through `project.deploy`).

use std::io::Write;

use crate::api::error::ApiErrorKind;
use crate::api::{ApiClient, Deployment};
use crate::commands::shared::{parse_version, print_deployment_result};
use crate::core::config::ProjectConfig;
use crate::core::project::{assert_script_configured, deploy};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// Arguments for [`update_deployment`] (clasp `update-deployment
/// <deploymentId> -V -d`).
#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateDeploymentArgs<'a> {
    pub version_number: Option<&'a str>,
    pub description: Option<&'a str>,
}

pub async fn update_deployment<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    deployment_id: &str,
    args: UpdateDeploymentArgs<'_>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<Deployment, CrspError> {
    let script_id = assert_script_configured(config).await?.to_string();
    let version_number = parse_version(args.version_number)?;
    let description = args.description.unwrap_or_default();
    let deployment = ui
        .with_async_spinner(i18n::DEPLOYING_PROJECT, async move {
            deploy(
                client,
                &script_id,
                description,
                Some(deployment_id),
                version_number,
            )
            .await
        })
        .await?;
    // clasp update-deployment.ts:80-85: an INVALID_ARGUMENT failure surfaces
    // only `error.cause.message` (no `API error (…)` prefix); other errors
    // propagate with the standard rendering.
    let deployment = match deployment {
        Ok(deployment) => deployment,
        Err(CrspError::Api {
            kind: ApiErrorKind::InvalidArgument,
            message,
        }) => return Err(CrspError::Validation(message)),
        Err(error) => return Err(error),
    };
    print_deployment_result(&deployment, true, output)?;
    Ok(deployment)
}
