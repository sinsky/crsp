//! `list-versions` (alias `versions`) — clasp commands/list-versions.ts.
//!
//! Human output reverses the API order (`N - description` lines after a
//! `Found N versions.` count); `--json` preserves the raw API order.

use std::io::Write;

use serde::Serialize;

use crate::api::{ApiClient, Version};
use crate::core::config::ProjectConfig;
use crate::core::project::assert_script_configured;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;

/// `--json` entry (clasp `{versionNumber, description}` per version, raw API
/// order; undefined keys are omitted by `JSON.stringify`).
#[derive(Serialize)]
struct VersionJson<'a> {
    #[serde(rename = "versionNumber", skip_serializing_if = "Option::is_none")]
    version_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

pub async fn list_versions(
    client: &ApiClient,
    config: &ProjectConfig,
    script_id: Option<&str>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<Vec<Version>, CrspError> {
    let script_id = match script_id {
        Some(script_id) => script_id.to_string(),
        None => assert_script_configured(config).await?.to_string(),
    };
    let versions = client.script().list_versions(&script_id).await?.results;
    if output.is_json() {
        let entries: Vec<_> = versions
            .iter()
            .map(|version| VersionJson {
                version_number: version.version_number,
                description: version.description.as_deref(),
            })
            .collect();
        output.print_json(&entries)?;
    } else if versions.is_empty() {
        output.message(i18n::NO_DEPLOYED_VERSIONS);
    } else {
        output.message(&i18n::found_items(versions.len(), "version", "versions"));
        for version in versions.iter().rev() {
            output.message(&i18n::version_line(
                version.version_number.unwrap_or(0),
                version.description.as_deref(),
            ));
        }
    }
    Ok(versions)
}
