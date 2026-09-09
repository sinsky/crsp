//! `open-credentials-setup` — clasp commands/open-credentials.ts.
//!
//! Opens the GCP credentials page for the configured (or prompted) project.

use std::io::Write;

use serde_json::json;
use url::Url;

use crate::api::ApiClient;
use crate::commands::shared::{
    UrlOpener, assert_gcp_project_configured, authorized_user_hint, maybe_prompt_for_project_id,
    open_url,
};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

pub async fn open_credentials_setup<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &mut ProjectConfig,
    user_hints: bool,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    let project_id = maybe_prompt_for_project_id(config, ui, opener, output).await?;
    assert_gcp_project_configured(config)?;

    let mut url = Url::parse("https://console.developers.google.com/apis/credentials")
        .map_err(|error| CrspError::Validation(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("project", project_id.as_deref().unwrap_or(""));
    if user_hints {
        let hint = authorized_user_hint(client).await;
        url.query_pairs_mut().append_pair("authUser", &hint);
    }
    if output.is_json() {
        output.print_json(&json!({ "url": url.as_str() }))?;
    }
    open_url(ui, opener, url.as_str(), output)
}
