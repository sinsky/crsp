//! `create-version` (alias `version`) — clasp commands/create-version.ts.
//!
//! The description falls back to an interactive input prompt (`Give a
//! description:` with an empty default) when missing in an interactive
//! terminal; noninteractive runs send an empty description (clasp's
//! `description ?? ''`).

use std::io::Write;

use serde::Serialize;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::project::{assert_script_configured, version};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptInput, Ui};

/// The create-version result (clasp `{versionNumber}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CreateVersionResult {
    #[serde(rename = "versionNumber")]
    pub version_number: i32,
}

pub async fn create_version<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    description: Option<&str>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<CreateVersionResult, CrspError> {
    let script_id = assert_script_configured(config).await?.to_string();
    // clasp create-version.ts:35-48: an absent or empty description triggers
    // the interactive input; noninteractive runs keep the empty default.
    let mut description = description.map(str::to_string);
    if description.as_deref().is_none_or(str::is_empty) && ui.is_interactive() {
        description = Some(ui.input(PromptInput {
            prompt: i18n::GIVE_A_DESCRIPTION.to_string(),
            placeholder: None,
            default: Some(String::new()),
        })?);
    }
    let version_number = version(
        client,
        &script_id,
        description.as_deref().unwrap_or_default(),
    )
    .await?;
    if output.is_json() {
        output.print_json(&CreateVersionResult { version_number })?;
    } else {
        output.message(&i18n::created_version(version_number));
    }
    Ok(CreateVersionResult { version_number })
}
