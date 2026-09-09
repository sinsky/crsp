//! `clone-script` (alias `clone`) — clasp commands/clone-script.ts.
//!
//! Accepts a script URL or id (URL extraction per clasp's regex), an optional
//! version number (HEAD when absent), and `--rootDir`. Without a script id it
//! offers an interactive Drive list (skipped noninteractively, producing the
//! clasp `No script ID.` error). The pull writes files through the secure
//! pipeline, then `.clasp.json` is created via
//! [`ProjectConfig::update_settings`].

use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::api::ApiClient;
use crate::api::error::ApiErrorKind;
use crate::commands::shared::{
    extract_script_id, pad_end, parse_version, pull_initial_files, write_skip_reason_text,
};
use crate::core::config::ProjectConfig;
use crate::core::files::SkippedFile;
use crate::core::project::{list_scripts, with_content_dir, with_script_id};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptSelect, Ui};

/// Arguments for [`clone_script`] (clasp `clone-script [scriptId]
/// [versionNumber] --rootDir`).
#[derive(Debug, Clone, Copy)]
pub struct CloneArgs<'a> {
    pub script_id: Option<&'a str>,
    pub version_number: Option<&'a str>,
    pub root_dir: Option<&'a str>,
    /// Process working directory (clasp resolves pulled-file display paths
    /// against `process.cwd()`).
    pub cwd: &'a Path,
}

/// The clone result: the cloned script id, the pulled files' display paths,
/// and the write-skip report (clasp `{files, writeResult}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloneResult {
    #[serde(rename = "scriptId")]
    pub script_id: String,
    /// Remote files' local display paths (clasp `files.map(f => f.localPath)`).
    pub files: Vec<String>,
    #[serde(skip)]
    pub skipped: Vec<SkippedFile>,
}

/// `--json` payload (clasp `JSON.stringify({scriptId, files}, null, 2)`).
#[derive(Serialize)]
struct CloneJson<'a> {
    #[serde(rename = "scriptId")]
    script_id: &'a str,
    files: &'a [String],
}

pub async fn clone_script<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: CloneArgs<'_>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<CloneResult, CrspError> {
    if config.script_id.is_some() {
        return Err(CrspError::Validation(
            i18n::PROJECT_FILE_ALREADY_EXISTS.to_string(),
        ));
    }
    let version_number = parse_version(args.version_number)?;
    let config = with_content_dir(config, args.root_dir)?;

    // Priority (clasp clone-script.ts:48-83): 1. the given scriptId (URL or
    // id), 2. the interactive Drive list, 3. the `No script ID.` error.
    let script_id = match args.script_id {
        Some(argument) => extract_script_id(argument),
        None if ui.is_interactive() => {
            let scripts = list_scripts(client).await?.results;
            let options = scripts
                .iter()
                .filter_map(|file| {
                    let id = file.id.as_deref()?;
                    let label = format!(
                        "{} - https://script.google.com/d/{id}/edit",
                        pad_end(file.name.as_deref().unwrap_or(""), 20),
                    );
                    Some((id.to_string(), label))
                })
                .collect::<Vec<_>>();
            if options.is_empty() {
                return Err(CrspError::Validation(i18n::NO_SCRIPT_ID.to_string()));
            }
            ui.select(PromptSelect {
                prompt: i18n::CLONE_WHICH_SCRIPT.to_string(),
                options,
                default: None,
            })?
        }
        None => return Err(CrspError::Validation(i18n::NO_SCRIPT_ID.to_string())),
    };
    if script_id.is_empty() {
        return Err(CrspError::Validation(i18n::NO_SCRIPT_ID.to_string()));
    }

    // clasp clone-script.ts:142-152: a 400 from the content fetch surfaces
    // as `Invalid script ID.`; other errors propagate.
    let pulled =
        match pull_initial_files(client, &script_id, &config, args.cwd, version_number).await {
            Ok(pulled) => pulled,
            Err(CrspError::Api {
                kind: ApiErrorKind::InvalidArgument,
                ..
            }) => return Err(CrspError::Validation(i18n::INVALID_SCRIPT_ID.to_string())),
            Err(error) => return Err(error),
        };
    with_script_id(&config, &script_id)
        .update_settings()
        .await?;

    if !output.is_json() {
        for (display, reason) in &pulled.skipped {
            output.warn(&i18n::security_warning_skipping_write(
                display,
                write_skip_reason_text(*reason),
            ));
        }
        for path in &pulled.files {
            output.message(&format!("└─ {path}"));
        }
        output.message(&i18n::cloned_files(pulled.files.len()));
    } else {
        output.print_json(&CloneJson {
            script_id: &script_id,
            files: &pulled.files,
        })?;
    }
    Ok(CloneResult {
        script_id,
        files: pulled.files,
        skipped: pulled
            .skipped
            .into_iter()
            .map(|(local_path, reason)| SkippedFile { local_path, reason })
            .collect(),
    })
}
