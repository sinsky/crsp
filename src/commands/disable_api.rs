//! `disable-api` — clasp commands/disable-api.ts.
//!
//! Updates the manifest and disables the Service Usage service. Unlike
//! enable-api there is no NOT_AUTHORIZED special case (clasp rethrows).

use std::io::Write;

use serde::Serialize;

use crate::api::ApiClient;
use crate::commands::shared::{
    UrlOpener, assert_gcp_project_configured, maybe_prompt_for_project_id,
};
use crate::core::config::ProjectConfig;
use crate::core::services::disable_service;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// `--json` payload (clasp `JSON.stringify({success: true,
/// disabledService}, null, 2)`).
#[derive(Serialize)]
struct DisableJson<'a> {
    success: bool,
    #[serde(rename = "disabledService")]
    disabled_service: &'a str,
}

pub async fn disable_api<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &mut ProjectConfig,
    api: &str,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    maybe_prompt_for_project_id(config, ui, opener, output).await?;
    assert_gcp_project_configured(config)?;

    let config_ref: &crate::core::config::ProjectConfig = config;
    let outcome = ui.with_spinner(i18n::DISABLING_SERVICE, move || {
        crate::ui::drive_isolated(async move { disable_service(client, config_ref, api).await })
    })?;
    outcome?;

    if output.is_json() {
        output.print_json(&DisableJson {
            success: true,
            disabled_service: api,
        })?;
        return Ok(());
    }
    output.message(&i18n::disabled_api(api));
    Ok(())
}
