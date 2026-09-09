//! `open-container` — clasp commands/open-container.ts.
//!
//! Opens `https://drive.google.com/open?id={parentId}`; the parentId is
//! required (`Parent ID not set, unable to open document.`).

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

pub async fn open_container<A: PromptAdapter, O: UrlOpener>(
    client: &ApiClient,
    config: &ProjectConfig,
    user_hints: bool,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    let parent_id = config.parent_id.as_deref().ok_or(CrspError::Validation(
        i18n::PARENT_ID_NOT_SET_UNABLE_TO_OPEN.to_string(),
    ))?;

    let mut url = Url::parse("https://drive.google.com/open")
        .map_err(|error| CrspError::Validation(error.to_string()))?;
    url.query_pairs_mut().append_pair("id", parent_id);
    if user_hints {
        let hint = authorized_user_hint(client).await;
        url.query_pairs_mut().append_pair("authUser", &hint);
    }
    if output.is_json() {
        output.print_json(&json!({ "url": url.as_str() }))?;
    }
    open_url(ui, opener, url.as_str(), output)
}
