//! `create-script` (alias `create`) — clasp commands/create-script.ts.
//!
//! `--type` accepts the standalone family (`standalone|webapp|api`,
//! lowercased) and the Drive container types (`docs|forms|sheets|slides`
//! mapped to their Google MIME types). The default title humanizes the cwd
//! folder name. After creation the initial files are pulled and
//! `.clasp.json` is written via [`ProjectConfig::update_settings`].

use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::api::ApiClient;
use crate::commands::shared::pull_initial_files;
use crate::core::config::ProjectConfig;
use crate::core::project::{
    create_script as create_project_script, create_with_container, with_content_dir,
    with_created_project,
};
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::text::humanize_title;
use crate::ui::{PromptAdapter, Ui};

/// Standalone script types; `webapp`/`api` are aliases of `standalone`
/// because deployment happens via create-deployment (clasp
/// STANDALONE_SCRIPT_TYPES).
const STANDALONE_SCRIPT_TYPES: [&str; 3] = ["standalone", "webapp", "api"];

/// Valid `--type` values in clasp's error message order (clasp
/// create-script.ts:84: the standalone set first, then the container keys).
const VALID_TYPES: &str = "standalone, webapp, api, docs, forms, sheets, slides";

/// Drive MIME types per container type (clasp DRIVE_FILE_MIMETYPES).
fn container_mime_type(script_type: &str) -> Option<&'static str> {
    match script_type {
        "docs" => Some("application/vnd.google-apps.document"),
        "forms" => Some("application/vnd.google-apps.form"),
        "sheets" => Some("application/vnd.google-apps.spreadsheet"),
        "slides" => Some("application/vnd.google-apps.presentation"),
        _ => None,
    }
}

/// Arguments for [`create_script`] (clasp `create-script --type --title
/// --parentId --rootDir`).
#[derive(Debug, Clone, Copy)]
pub struct CreateScriptArgs<'a> {
    pub script_type: &'a str,
    pub title: Option<&'a str>,
    pub parent_id: Option<&'a str>,
    pub root_dir: Option<&'a str>,
    /// Process working directory (the default title source).
    pub cwd: &'a Path,
}

/// The create result: the new script id, the bound parent (the container file
/// id for `--type docs|forms|sheets|slides`), and pulled files' display
/// paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreateScriptResult {
    #[serde(rename = "scriptId")]
    pub script_id: String,
    #[serde(rename = "parentId", skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub files: Vec<String>,
}

/// `--json` payload (clasp `JSON.stringify({scriptId, parentId, files},
/// null, 2)`; `parentId` is undefined for standalone scripts, so the key is
/// omitted).
#[derive(Serialize)]
struct CreateJson<'a> {
    #[serde(rename = "scriptId")]
    script_id: &'a str,
    #[serde(rename = "parentId", skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
    files: &'a [String],
}

/// clasp `getDefaultProjectName`: humanize the cwd folder name.
pub fn default_project_name(cwd: &Path) -> String {
    humanize_title(
        cwd.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_str(),
    )
}

pub async fn create_script<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    args: CreateScriptArgs<'_>,
    ui: &Ui<A>,
    output: &mut Output<impl Write, impl Write>,
) -> Result<CreateScriptResult, CrspError> {
    if config.script_id.is_some() {
        return Err(CrspError::Validation(
            i18n::PROJECT_FILE_ALREADY_EXISTS.to_string(),
        ));
    }
    let name = match args.title {
        Some(title) => title.to_string(),
        None => default_project_name(args.cwd),
    };
    let script_type = args.script_type.to_lowercase();
    let config = with_content_dir(config, args.root_dir)?;

    let (script_id, created_parent_id) = if STANDALONE_SCRIPT_TYPES.contains(&script_type.as_str())
    {
        let script_id = ui
            .with_async_spinner(i18n::CREATING_SCRIPT, async move {
                create_project_script(client, &name, args.parent_id).await
            })
            .await??;
        if !output.is_json() {
            output.message(&i18n::created_standalone_script(
                &format!("https://script.google.com/d/{script_id}/edit"),
                args.parent_id,
            ));
            // Surface the next step for web app / API executable intents
            // (clasp create-script.ts:147-158); the script itself is always
            // created as standalone.
            match script_type.as_str() {
                "webapp" => {
                    output.message(&i18n::deployment_tip("web app", "webApp"));
                }
                "api" => {
                    output.message(&i18n::deployment_tip("API executable", "executionApi"));
                }
                _ => {}
            }
        }
        (script_id, None)
    } else {
        let Some(mime_type) = container_mime_type(&script_type) else {
            return Err(CrspError::Validation(i18n::invalid_script_type(
                &script_type,
                VALID_TYPES,
            )));
        };
        let (script_id, parent_id) = ui
            .with_async_spinner(i18n::CREATING_SCRIPT, async move {
                create_with_container(client, &name, mime_type).await
            })
            .await??;
        if !output.is_json() {
            output.message(&i18n::created_container_script(
                &format!("https://drive.google.com/open?id={parent_id}"),
                &format!("https://script.google.com/d/{script_id}/edit"),
            ));
        }
        (script_id, Some(parent_id))
    };

    // Initial pull + settings write (clasp create-script.ts:162-170: both the
    // pull and `updateSettings` run inside the single `Cloning script...`
    // spinner). clasp createScript sets `options.project = {scriptId,
    // parentId}`.
    let config_ref: &crate::core::config::ProjectConfig = &config;
    let script_id_ref: &str = &script_id;
    let created_parent_ref: &Option<String> = &created_parent_id;
    let pulled = ui
        .with_async_spinner(i18n::CLONING_SCRIPT, async move {
            let pulled =
                pull_initial_files(client, script_id_ref, config_ref, args.cwd, None).await?;
            let parent_for_settings = created_parent_ref
                .clone()
                .or_else(|| args.parent_id.map(str::to_string));
            with_created_project(config_ref, script_id_ref, parent_for_settings)
                .update_settings()
                .await?;
            Ok::<_, CrspError>(pulled)
        })
        .await??;

    if output.is_json() {
        output.print_json(&CreateJson {
            script_id: &script_id,
            parent_id: created_parent_id.as_deref(),
            files: &pulled.files,
        })?;
    } else {
        for path in &pulled.files {
            output.message(&format!("└─ {path}"));
        }
        output.message(&i18n::cloned_files(pulled.files.len()));
    }
    Ok(CreateScriptResult {
        script_id,
        parent_id: created_parent_id,
        files: pulled.files,
    })
}
