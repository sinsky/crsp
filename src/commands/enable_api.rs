//! `enable-api` — clasp commands/enable-api.ts.
//!
//! Updates the manifest and enables the Service Usage service; a 403 maps to
//! clasp's `Not authorized to enable … or it does not exist.` notice.

use std::io::Write;

use crate::api::ApiClient;
use crate::api::error::ApiErrorKind;
use crate::commands::shared::{
    UrlOpener, assert_gcp_project_configured, maybe_prompt_for_project_id,
};
use crate::core::config::ProjectConfig;
use crate::core::services::enable_service;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

pub async fn enable_api<A: PromptAdapter, O: UrlOpener>(
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
    let outcome = ui.with_spinner(i18n::ENABLING_SERVICE, move || {
        crate::ui::drive_isolated(async move { enable_service(client, config_ref, api).await })
    })?;
    if let Err(error) = outcome {
        if matches!(
            &error,
            CrspError::Api {
                kind: ApiErrorKind::NotAuthorized,
                ..
            }
        ) {
            return Err(CrspError::Validation(i18n::not_authorized_to_enable(api)));
        }
        return Err(error);
    }

    if output.is_json() {
        output.print_json(&serde_json::json!({ "success": true }))?;
        return Ok(());
    }
    output.message(&i18n::enabled_api(api));
    Ok(())
}
