//! `list-scripts` (alias `list`) — clasp commands/list-scripts.ts.
//!
//! Drive script list; human output truncates names with clasp's `ellipsize`
//! to 20 display columns (padded to 20 characters) unless `--noShorten` is
//! given; `--json` prints the raw `{id, name}` entries.

use std::io::Write;

use serde::Serialize;

use crate::api::{ApiClient, DriveFile};
use crate::commands::shared::ellipsize;
use crate::core::project::list_scripts as fetch_scripts;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// `--json` entry (clasp `{id, name}`).
#[derive(Serialize)]
struct ScriptJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

pub async fn list_scripts<A: PromptAdapter>(
    client: &ApiClient,
    no_shorten: bool,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<Vec<DriveFile>, CrspError> {
    let files = ui
        .with_async_spinner(i18n::FINDING_YOUR_SCRIPTS, async move {
            Ok::<_, CrspError>(fetch_scripts(client).await?.results)
        })
        .await??;
    if output.is_json() {
        let entries: Vec<_> = files
            .iter()
            .map(|file| ScriptJson {
                id: file.id.as_deref(),
                name: file.name.as_deref(),
            })
            .collect();
        output.print_json(&entries)?;
    } else if files.is_empty() {
        output.message(i18n::NO_SCRIPT_FILES_FOUND);
    } else {
        output.message(&i18n::found_items(files.len(), "script", "scripts"));
        for file in &files {
            let name = if no_shorten {
                file.name.clone().unwrap_or_default()
            } else {
                ellipsize(file.name.as_deref().unwrap_or_default(), 20)
            };
            // clasp renders a missing id as `undefined` in the URL template.
            output.message(&format!(
                "{name} - https://script.google.com/d/{}/edit",
                file.id.as_deref().unwrap_or("undefined")
            ));
        }
    }
    Ok(files)
}
