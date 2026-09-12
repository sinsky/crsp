//! `open-web-app` — clasp commands/open-webapp.ts.
//!
//! With a deployment argument the entry points are fetched for it;
//! interactively the deployments are listed and sorted by `updateTime`
//! ascending (stable, missing keys keep their relative order) for selection.
//! The `WEB_APP` entry point's `webApp.url` is opened; anything else is clasp's
//! `No web app entry point found.` error.

use std::io::Write;

use serde_json::json;
use url::Url;

use crate::api::ApiClient;
use crate::commands::shared::{UrlOpener, authorized_user_hint, ellipsize, open_url, pad_end};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptSelect, Ui};

/// Arguments for [`open_web_app`] (clasp `open-web-app [deploymentId]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenWebAppArgs<'a> {
    pub deployment_id: Option<&'a str>,
}

pub async fn open_web_app<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: OpenWebAppArgs<'_>,
    user_hints: bool,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    let script_id = config.script_id.as_deref().ok_or(CrspError::Validation(
        i18n::OPEN_WEB_APP_SCRIPT_ID_NOT_SET.to_string(),
    ))?;

    let mut deployment_id = args.deployment_id.map(str::to_string);
    if deployment_id.is_none() && ui.is_interactive() {
        let mut deployments = ui
            .with_async_spinner(i18n::FETCHING_DEPLOYMENTS, async move {
                Ok::<_, CrspError>(client.script().list_deployments(script_id).await?.results)
            })
            .await??;
        // Order deployments by update time (clasp sorts with localeCompare
        // when both keys exist; the stable sort keeps the rest in place).
        deployments.sort_by(
            |a, b| match (a.update_time.as_deref(), b.update_time.as_deref()) {
                (Some(a), Some(b)) => a.cmp(b),
                _ => std::cmp::Ordering::Equal,
            },
        );
        let options = deployments
            .iter()
            .map(|deployment| {
                let id = deployment.deployment_id.as_deref().unwrap_or("undefined");
                let description = ellipsize(
                    deployment
                        .deployment_config
                        .as_ref()
                        .and_then(|config| config.description.as_deref())
                        .unwrap_or_default(),
                    30,
                );
                let version_number = match deployment.version_number() {
                    Some(number) => pad_end(&number.to_string(), 4),
                    None => pad_end("HEAD", 4),
                };
                (
                    id.to_string(),
                    format!("{description}@{version_number} - {id}"),
                )
            })
            .collect();
        deployment_id = Some(ui.select(PromptSelect {
            prompt: i18n::OPEN_WHICH_DEPLOYMENT.to_string(),
            options,
            default: None,
        })?);
    }

    let Some(deployment_id) = deployment_id else {
        return Err(CrspError::Validation(
            i18n::DEPLOYMENT_ID_REQUIRED.to_string(),
        ));
    };

    let deployment = client
        .script()
        .get_deployment(script_id, &deployment_id)
        .await?;
    let web_app_url = deployment
        .entry_points
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|entry_point| {
            entry_point.entry_point_type.as_deref() == Some("WEB_APP")
                && entry_point
                    .web_app
                    .as_ref()
                    .and_then(|web_app| web_app.url.as_deref())
                    .is_some()
        })
        .and_then(|entry_point| entry_point.web_app.as_ref().and_then(|web| web.url.clone()))
        .ok_or(CrspError::Validation(
            i18n::NO_WEB_APP_ENTRY_POINT.to_string(),
        ))?;

    let mut url =
        Url::parse(&web_app_url).map_err(|error| CrspError::Validation(error.to_string()))?;
    if user_hints {
        let hint = authorized_user_hint(client).await;
        url.query_pairs_mut().append_pair("authUser", &hint);
    }
    if output.is_json() {
        output.print_json(&json!({ "url": url.as_str() }))?;
    }
    open_url(ui, opener, url.as_str(), output)
}
