use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use unicode_width::UnicodeWidthChar;

use crate::api::ApiClient;
use crate::api::Deployment;
use crate::core::config::ProjectConfig;
use crate::core::files::{PullFile, SkipReason, pull_files};
use crate::core::project::{PULL_WRITE_LIMIT, fetch_remote_files};
use crate::error::CrspError;
use crate::output::Output;

pub fn parse_version(value: Option<&str>) -> Result<Option<i32>, CrspError> {
    value
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|_| CrspError::Validation(format!("'{value}' is not a valid integer.")))
        })
        .transpose()
}

/// clasp clone-script.ts:52: extracts the script id from a
/// `script.google.com/d/{id}/…` URL, otherwise trims the argument and treats
/// it as an id.
pub fn extract_script_id(raw: &str) -> String {
    static SCRIPT_URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"https://script\.google\.com/d/([^/]+)/.*").unwrap());
    SCRIPT_URL
        .captures(raw)
        .and_then(|captures| captures.get(1))
        .map(|group| group.as_str().to_string())
        .unwrap_or_else(|| raw.trim().to_string())
}

/// JS `String.prototype.padEnd` on character counts.
pub fn pad_end(value: &str, length: usize) -> String {
    let missing = length.saturating_sub(value.chars().count());
    let mut padded = value.to_string();
    padded.extend(std::iter::repeat_n(' ', missing));
    padded
}

/// clasp `ellipsize` (commands/utils.ts:147-149): `cli-truncate` v4 with
/// `position: 'end'` and `preferTruncationOnSpace`, then `padEnd(length)`.
///
/// cli-truncate finds a space at most three characters left of the boundary
/// (`getIndexOfNearestSpace` on character indices), slices to that many
/// display columns (`slice-ansi`), and appends `…`; the result is padded to
/// `length` characters (`padEnd` counts characters, not columns).
pub fn ellipsize(value: &str, length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    if length == 1 {
        return pad_end("…", 1);
    }
    let chars: Vec<char> = value.chars().collect();
    let width: usize = chars.iter().map(|c| c.width().unwrap_or(0)).sum();
    let truncated = if width <= length {
        value.to_string()
    } else {
        // getIndexOfNearestSpace(text, columns - 1): the boundary character
        // index, then up to three positions to the left.
        let wanted = length - 1;
        let nearest = if chars.get(wanted) == Some(&' ') {
            wanted
        } else {
            (1..=3)
                .map(|index| wanted.saturating_sub(index))
                .find(|index| chars.get(*index) == Some(&' '))
                .unwrap_or(wanted)
        };
        // sliceAnsi(text, 0, nearest): a prefix of `nearest` display columns.
        let mut sliced = String::new();
        let mut used = 0;
        for character in &chars {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > nearest {
                break;
            }
            sliced.push(*character);
            used += character_width;
        }
        format!("{sliced}…")
    };
    pad_end(&truncated, length)
}

/// clasp clone-script.ts:107-117: human-readable reason for a skipped write.
pub fn write_skip_reason_text(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::ParentSymlink => "parent directory contains a symbolic link",
        SkipReason::TargetSymlink => "target path is a symbolic link",
        SkipReason::OutsideContentDir
        | SkipReason::RaceCondition
        | SkipReason::SymlinkLoop
        | SkipReason::Symlink
        | SkipReason::UnsupportedType => {
            "outside project directory or unsafe race condition detected"
        }
    }
}

/// The clone/create initial pull (clasp `files.pull`): the remote files' cwd
/// relative display paths plus the write skips (clasp `{files,
/// writeResult.skipped}`).
pub struct InitialPull {
    pub files: Vec<String>,
    /// Skip entries with their clasp display path.
    pub skipped: Vec<(String, SkipReason)>,
}

/// Fetches the remote project content and writes it through the secure
/// pipeline (clasp `fetchRemote` + `WriteFiles`; files with an empty source
/// are skipped by clasp's `WriteFiles` without a skip entry).
pub async fn pull_initial_files(
    client: &ApiClient,
    script_id: &str,
    config: &ProjectConfig,
    cwd: &Path,
    version_number: Option<i32>,
) -> Result<InitialPull, CrspError> {
    let remote =
        fetch_remote_files(client, script_id, &config.content_dir, cwd, version_number).await?;
    let pull_inputs: Vec<PullFile> = remote
        .iter()
        .filter(|file| !file.file.source.is_empty())
        .map(|file| file.file.clone())
        .collect();
    let write_result = pull_files(
        &pull_inputs,
        &config.content_dir,
        config.allow_symlinks,
        PULL_WRITE_LIMIT,
    )
    .await?;
    let files: Vec<String> = remote.iter().map(|file| file.local_path.clone()).collect();
    let skipped = write_result
        .skipped
        .iter()
        .map(|skipped| {
            let display = remote
                .iter()
                .find(|file| file.file.remote_path == skipped.local_path)
                .map(|file| file.local_path.clone())
                .unwrap_or_else(|| skipped.local_path.clone());
            (display, skipped.reason)
        })
        .collect();
    Ok(InitialPull { files, skipped })
}

/// `--json` deployment payload (clasp create-deployment.ts:51-55,
/// update-deployment.ts:58-62; undefined keys are omitted by
/// `JSON.stringify`).
#[derive(Serialize)]
struct DeploymentJson<'a> {
    #[serde(rename = "deploymentId", skip_serializing_if = "Option::is_none")]
    deployment_id: Option<&'a str>,
    #[serde(rename = "versionNumber", skip_serializing_if = "Option::is_none")]
    version_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

/// Prints a create/update deployment result (clasp create-deployment.ts /
/// update-deployment.ts): `--json` prints the deployment fields, human mode
/// prints `Deployed …`/`Redeployed …` with `@HEAD` for unversioned
/// deployments.
pub fn print_deployment_result(
    deployment: &Deployment,
    redeploy: bool,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    if output.is_json() {
        output.print_json(&DeploymentJson {
            deployment_id: deployment.deployment_id.as_deref(),
            version_number: deployment.version_number(),
            description: deployment
                .deployment_config
                .as_ref()
                .and_then(|config| config.description.as_deref()),
        })?;
    } else {
        let message = if redeploy {
            crate::i18n::redeployed(
                deployment.deployment_id.as_deref().unwrap_or_default(),
                deployment.version_number(),
            )
        } else {
            crate::i18n::deployed(
                deployment.deployment_id.as_deref().unwrap_or_default(),
                deployment.version_number(),
            )
        };
        output.message(&message);
    }
    Ok(())
}
