//! `setup-logs` — clasp commands/setup-logs.ts.
//!
//! Interactive projectId confirmation/setup only (no API call): when the
//! project ID is unset and the session is interactive, the shared
//! `maybePromptForProjectId` flow runs (instructions, settings page,
//! prompt, persisted via `updateSettings`).

use std::io::Write;

use crate::commands::shared::{
    UrlOpener, assert_gcp_project_configured, maybe_prompt_for_project_id,
};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

pub async fn setup_logs<A: PromptAdapter, O: UrlOpener>(
    config: &mut ProjectConfig,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    maybe_prompt_for_project_id(config, ui, opener, output).await?;
    assert_gcp_project_configured(config)?;

    if output.is_json() {
        output.print_json(&serde_json::json!({ "success": true }))?;
        return Ok(());
    }
    output.message(i18n::SETUP_LOGS_SUCCESS);
    Ok(())
}
