//! `open-script` — clasp commands/open-script.ts.
//!
//! Opens `https://script.google.com/d/{id}/edit` for the given or configured
//! script id, with the clasp open-* output quirk: JSON mode still prints the
//! human open line, and a non-TTY stdout skips the browser launch.

use std::io::Write;

use serde_json::json;
use url::Url;

use crate::api::ApiClient;
use crate::commands::shared::{UrlOpener, authorized_user_hint, open_url};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// Arguments for [`open_script`] (clasp `open-script [scriptId]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenScriptArgs<'a> {
    pub script_id: Option<&'a str>,
}

pub async fn open_script<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: OpenScriptArgs<'_>,
    user_hints: bool,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    let script_id = args
        .script_id
        .or(config.script_id.as_deref())
        .ok_or(CrspError::Validation(
            i18n::OPEN_IDE_SCRIPT_ID_NOT_SET.to_string(),
        ))?;

    let mut url = Url::parse(&format!("https://script.google.com/d/{script_id}/edit"))
        .map_err(|error| CrspError::Validation(error.to_string()))?;
    if user_hints {
        let hint = authorized_user_hint(client).await;
        url.query_pairs_mut().append_pair("authUser", &hint);
    }
    if output.is_json() {
        output.print_json(&json!({ "url": url.as_str() }))?;
    }
    open_url(ui, opener, url.as_str(), output)
}
