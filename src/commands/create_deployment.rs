//! `create-deployment` (alias `deploy`) — clasp commands/create-deployment.ts
//! and core/project.ts `deploy`.
//!
//! Without `-V` an immutable version is created first (inheriting the
//! deployment description), then the deployment is created. With
//! `-i/--deploymentId` the existing deployment is updated instead (clasp's
//! update path).

use std::io::Write;

use crate::api::{ApiClient, Deployment};
use crate::commands::shared::{parse_version, print_deployment_result};
use crate::core::config::ProjectConfig;
use crate::core::project::{assert_script_configured, deploy};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// Arguments for [`create_deployment`] (clasp `create-deployment -V -d -i`).
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateDeploymentArgs<'a> {
    pub version_number: Option<&'a str>,
    pub description: Option<&'a str>,
    pub deployment_id: Option<&'a str>,
}

pub async fn create_deployment<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: CreateDeploymentArgs<'_>,
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
                args.deployment_id,
                version_number,
            )
            .await
        })
        .await??;
    print_deployment_result(&deployment, false, output)?;
    Ok(deployment)
}
