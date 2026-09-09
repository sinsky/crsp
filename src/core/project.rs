use std::path::Path;

use crate::api::drive::DriveFile;
use crate::api::{ApiClient, Deployment, PagedResults};
use crate::core::config::ProjectConfig;
use crate::core::files::{
    CollectLocalFilesResult, LocalExtensions, LocalFile, PullFile, collect_local_files,
};
use crate::core::path::{PathJail, normalize_slashes, relative_path};
use crate::error::CrspError;
use crate::i18n;

pub async fn assert_script_configured(config: &ProjectConfig) -> Result<&str, CrspError> {
    config
        .script_id
        .as_deref()
        .ok_or_else(|| CrspError::Config(i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))
}

pub async fn collect_for_status(
    config: &ProjectConfig,
) -> Result<CollectLocalFilesResult, CrspError> {
    collect_local_files(config).await
}

pub fn tracked_names(files: &[LocalFile]) -> Vec<String> {
    files.iter().map(|file| file.local_path.clone()).collect()
}

pub fn project_path(config: &ProjectConfig, relative: &str) -> std::path::PathBuf {
    config.content_dir.join(Path::new(relative))
}

/// Concurrent disk-write limit for the clone/create initial pulls (spec §2.6
/// Layer C; matches the pull command pipeline).
pub const PULL_WRITE_LIMIT: usize = 32;

/// clasp `withContentDir` (clasp.ts:129-142): overrides the content directory
/// with the `--rootDir` option (relative paths resolve against the project
/// root; the result must stay inside it).
pub fn with_content_dir(
    config: &ProjectConfig,
    root_dir: Option<&str>,
) -> Result<ProjectConfig, CrspError> {
    match root_dir {
        None => Ok(config.clone()),
        Some(raw) => {
            let mut config = config.clone();
            config.content_dir = PathJail::resolve_content_dir(&config.project_root_dir, raw)?;
            Ok(config)
        }
    }
}

/// clasp `withScriptId` (clasp.ts:113): replaces the project settings, so any
/// existing parentId/projectId are dropped.
pub fn with_script_id(config: &ProjectConfig, script_id: &str) -> ProjectConfig {
    let mut config = config.clone();
    config.script_id = Some(script_id.to_string());
    config.parent_id = None;
    config.project_id = None;
    config
}

/// clasp `createScript` (project.ts:81-110): POSTs the project and returns
/// the new script id.
pub async fn create_script(
    client: &ApiClient,
    title: &str,
    parent_id: Option<&str>,
) -> Result<String, CrspError> {
    let script = client.script().create_project(title, parent_id).await?;
    script
        .script_id
        .ok_or_else(|| CrspError::Validation(i18n::UNEXPECTED_SCRIPT_ID_MISSING.to_string()))
}

/// clasp `createWithContainer` (project.ts:146-183): creates the Drive
/// container file, then the Apps Script project bound to it. Returns
/// `(scriptId, parentId)`.
pub async fn create_with_container(
    client: &ApiClient,
    name: &str,
    mime_type: &str,
) -> Result<(String, String), CrspError> {
    let container = client.drive().create_file(mime_type, name).await?;
    let parent_id = container
        .id
        .ok_or_else(|| CrspError::Validation(i18n::UNEXPECTED_CONTAINER_ID_MISSING.to_string()))?;
    let script_id = create_script(client, name, Some(&parent_id)).await?;
    Ok((script_id, parent_id))
}

/// clasp `listScripts` (project.ts:192-216): Drive files list filtered to
/// Apps Script projects.
pub async fn list_scripts(client: &ApiClient) -> Result<PagedResults<DriveFile>, CrspError> {
    client.drive().list_files().await
}

/// clasp `trashScript` (project.ts:116-136): Drive PATCH `{trashed: true}`.
pub async fn trash_script(client: &ApiClient, script_id: &str) -> Result<(), CrspError> {
    client.drive().trash_file(script_id).await
}

/// clasp `version` (project.ts:224-249): POST versions; the response's
/// missing version number reads as 0.
pub async fn version(
    client: &ApiClient,
    script_id: &str,
    description: &str,
) -> Result<i32, CrspError> {
    let created = client
        .script()
        .create_version(script_id, description)
        .await?;
    Ok(created.version_number.unwrap_or(0))
}

/// clasp `deploy` (project.ts:329-382): creates an implicit version when no
/// version number is given, then creates a new deployment or updates the
/// existing one identified by `deployment_id`.
pub async fn deploy(
    client: &ApiClient,
    script_id: &str,
    description: &str,
    deployment_id: Option<&str>,
    version_number: Option<i32>,
) -> Result<Deployment, CrspError> {
    let version_number = match version_number {
        Some(number) => number,
        None => version(client, script_id, description).await?,
    };
    match deployment_id {
        None => {
            client
                .script()
                .create_deployment(script_id, description, version_number)
                .await
        }
        Some(deployment_id) => {
            client
                .script()
                .update_deployment(script_id, deployment_id, description, version_number)
                .await
        }
    }
}

/// clasp `undeploy` (project.ts:415-435): DELETE deployments/{id}.
pub async fn undeploy(
    client: &ApiClient,
    script_id: &str,
    deployment_id: &str,
) -> Result<(), CrspError> {
    client
        .script()
        .delete_deployment(script_id, deployment_id)
        .await
}

/// A remote project file prepared for the clone/create initial pull: the
/// pipeline input plus clasp's cwd-relative display path (`files.ts`
/// `fetchRemote`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub file: PullFile,
    /// `path.relative(cwd, path.resolve(contentDir, name + extension))`.
    pub local_path: String,
}

/// clasp `fetchRemote` (files.ts:264-327): GETs the project content and maps
/// every remote file to its local display path, rejecting names that would
/// escape the content directory before any write happens. Naming uses the
/// project's configured extensions ([`LocalExtensions`]).
pub async fn fetch_remote_files(
    client: &ApiClient,
    script_id: &str,
    config: &ProjectConfig,
    cwd: &Path,
    version_number: Option<i32>,
) -> Result<Vec<RemoteFile>, CrspError> {
    let content = client
        .script()
        .get_content(script_id, version_number)
        .await?;
    let extensions = LocalExtensions::from_config(config);
    content
        .files()
        .iter()
        .map(|file| {
            let file_type = file.file_type.clone().unwrap_or_default();
            let name = file.name.clone().unwrap_or_default();
            let remote = PullFile::new(&name, &file_type, file.source.as_deref().unwrap_or(""));
            let local_name = extensions.local_name(&remote);
            let resolved = PathJail::resolve_lexical(&config.content_dir, &local_name);
            if !PathJail::is_inside(&config.content_dir, &resolved) {
                return Err(CrspError::Validation(
                    i18n::remote_file_attempts_outside_write(&name),
                ));
            }
            let local_path = normalize_slashes(&relative_path(cwd, &resolved)).into_owned();
            Ok(RemoteFile {
                file: remote,
                local_path,
            })
        })
        .collect()
}

/// Applies clasp's post-create project settings (`createScript` sets
/// `options.project = {scriptId, parentId}`, dropping any projectId).
pub fn with_created_project(
    config: &ProjectConfig,
    script_id: &str,
    parent_id: Option<String>,
) -> ProjectConfig {
    let mut config = config.clone();
    config.script_id = Some(script_id.to_string());
    config.parent_id = parent_id;
    config.project_id = None;
    config
}
