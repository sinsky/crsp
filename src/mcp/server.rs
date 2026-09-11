use std::path::{Path, PathBuf};
use std::sync::Arc;

use home::home_dir;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::JsonSchema;
use rmcp::service::ServiceExt;
use rmcp::transport::io::stdio;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::{ApiClient, ApiClientConfig, BaseUrls};
use crate::commands::shared::pull_initial_files;
use crate::core::config::ProjectConfig;
use crate::core::files::{LocalExtensions, pull_files};
use crate::core::path::{PathJail, normalize_slashes};
use crate::core::project::{create_script, fetch_remote_files, list_scripts};
use crate::error::CrspError;

const SCRIPT_ID_REQUIRED: &str = "Script ID is required.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushProjectArgs {
    #[schemars(
        description = "The local directory of the Apps Script project to push. Must contain a .clasp.json file containing the project info."
    )]
    pub project_dir: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullProjectArgs {
    #[schemars(
        description = "The local directory of the Apps Script project to update. Must contain a .clasp.json file containing the project info."
    )]
    pub project_dir: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectArgs {
    #[schemars(description = "The local directory where the Apps Script project will be created.")]
    pub project_dir: String,
    #[schemars(
        description = "Local directory relative to projectDir where the Apps Script source files are located. If not specified, files are placed in the project directory."
    )]
    pub source_dir: Option<String>,
    #[schemars(
        description = "Name of the project. If not provided, the project name will be inferred from the directory."
    )]
    pub project_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CloneProjectArgs {
    #[schemars(description = "The local directory where the Apps Script project will be created.")]
    pub project_dir: String,
    #[schemars(
        description = "Local directory relative to projectDir where the Apps Script source files are located. If not specified, files are placed in the project directory."
    )]
    pub source_dir: Option<String>,
    #[schemars(description = "ID of the Apps Script project to clone.")]
    pub script_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct FilesOutput {
    script_id: String,
    project_dir: String,
    files: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ScriptsOutput {
    scripts: Vec<ScriptOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ScriptOutput {
    script_id: String,
    name: String,
}

#[derive(Clone)]
pub struct McpServer {
    /// The tool API client: the user's credentials (clasp preAction
    /// `initAuth`) or an unauthenticated client whose API calls fail locally
    /// with clasp's auth error (the ApiClient no-credentials guard).
    client: ApiClient,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    pub fn new() -> Self {
        let refresh: crate::api::RefreshFn = Arc::new(|| {
            Box::pin(async { Err(CrspError::Auth(crate::i18n::NO_CREDENTIALS.to_string())) })
        });
        Self {
            client: ApiClient::with_base_urls(
                ApiClientConfig::new(String::new(), refresh),
                BaseUrls::from_env(),
            )
            .expect("mcp api client"),
        }
    }

    pub fn new_for_tests() -> Self {
        Self::with_base_urls_and_token(BaseUrls::default(), "")
    }

    /// Test constructor: explicit base URLs and bearer token (the production
    /// path loads credentials from `$HOME`, exercised by the golden cases).
    pub fn with_base_urls_and_token(base_urls: BaseUrls, access_token: &str) -> Self {
        let refresh: crate::api::RefreshFn = Arc::new(|| {
            Box::pin(async { Err(CrspError::Auth(crate::i18n::NO_CREDENTIALS.to_string())) })
        });
        Self::with_base_urls_and_refresh(base_urls, access_token, refresh)
    }

    /// Test plumbing: like [`Self::with_base_urls_and_token`] with a custom
    /// refresh closure (used to pin the per-call token refresh re-issue).
    pub fn with_base_urls_and_refresh(
        base_urls: BaseUrls,
        access_token: &str,
        refresh: crate::api::RefreshFn,
    ) -> Self {
        Self {
            client: ApiClient::with_base_urls(
                ApiClientConfig::new(access_token.to_string(), refresh),
                base_urls,
            )
            .expect("mcp api client"),
        }
    }

    /// clasp `start-mcp.ts`: the MCP server reuses the preAction-initialized
    /// clasp instance, so its tool API calls carry the user's OAuth2 client
    /// (bearer token + 401 refresh), exactly like the CLI commands.
    pub fn with_context(context: &crate::core::clasp::Clasp) -> Self {
        Self {
            client: context.client.clone(),
        }
    }

    fn client(&self) -> ApiClient {
        self.client.clone()
    }

    fn validate_project(&self, project_dir: &str) -> Result<PathBuf, String> {
        if project_dir.trim().is_empty() {
            return Err("Project directory is required.".to_string());
        }
        let home =
            home_dir().ok_or_else(|| "Unable to determine the user home directory.".to_string())?;
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        validate_project_dir(Path::new(project_dir), &home, &cwd).map_err(|error| error.to_string())
    }
}

pub fn validate_project_dir(
    candidate: &Path,
    home: &Path,
    cwd: &Path,
) -> Result<PathBuf, CrspError> {
    PathJail::validate_project_dir(candidate, home, cwd)
        .map(|path| path.to_string_lossy().into_owned().into())
}

pub fn validate_source_dir(project_dir: &Path, source_dir: &str) -> Result<PathBuf, CrspError> {
    PathJail::resolve_content_dir(project_dir, source_dir)
}

fn error_result(prefix: &str, error: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("{prefix}: {error}"))])
}

fn success_with_files(
    message: String,
    script_id: String,
    project_dir_raw: &str,
    resolved_dir: &Path,
    files: Vec<String>,
) -> CallToolResult {
    let absolute_files: Vec<String> = files
        .iter()
        .map(|file| normalize_slashes(&resolved_dir.join(file).to_string_lossy()).into_owned())
        .collect();
    let mut content = vec![ContentBlock::text(message)];
    content.extend(
        absolute_files
            .iter()
            .map(|file| ContentBlock::text(format!("Updated file: {file}"))),
    );
    let structured = json!(FilesOutput {
        script_id,
        // clasp parity (mcp/server.ts:146, 233, 346, 462): structuredContent
        // echoes the raw tool-input argument; path resolution applies only to
        // the `files` entries (and the jail error text).
        project_dir: project_dir_raw.to_string(),
        files: absolute_files,
    });
    let mut result = CallToolResult::structured(structured);
    result.content = content;
    result
}

#[tool_router]
impl McpServer {
    #[tool(
        name = "push_files",
        description = "Pushes the local Apps Script project to the remote server.",
        annotations(title = "Push project files to Apps Script", open_world_hint = false, destructive_hint = true, idempotent_hint = false, read_only_hint = false),
        output_schema = rmcp::handler::server::tool::schema_for_type::<FilesOutput>()
    )]
    async fn push_files(&self, Parameters(args): Parameters<PushProjectArgs>) -> CallToolResult {
        let project_dir = match self.validate_project(&args.project_dir) {
            Ok(path) => path,
            Err(error) => return CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        let config = match ProjectConfig::discover(Some(&project_dir), &project_dir).await {
            Ok(config) => config,
            Err(error) => return error_result("Error pushing project", error),
        };
        let script_id = match config.script_id.clone() {
            Some(id) => id,
            None => return error_result("Error pushing project", "Project settings not found."),
        };
        let client = self.client();
        match crate::core::files::push_files(&client, &config).await {
            Ok(result) => success_with_files(
                format!(
                    "Pushed project in {} to remote server successfully.",
                    args.project_dir
                ),
                script_id,
                &args.project_dir,
                &project_dir,
                result
                    .files
                    .iter()
                    .map(|file| file.local_path.clone())
                    .collect(),
            ),
            Err(error) => error_result("Error pushing project", error),
        }
    }

    #[tool(
        name = "pull_files",
        description = "Pulls files from Apps Script project to local file system.",
        annotations(title = "Pull project files from Apps Script", open_world_hint = false, destructive_hint = true, idempotent_hint = false, read_only_hint = false),
        output_schema = rmcp::handler::server::tool::schema_for_type::<FilesOutput>()
    )]
    async fn pull_files(&self, Parameters(args): Parameters<PullProjectArgs>) -> CallToolResult {
        let project_dir = match self.validate_project(&args.project_dir) {
            Ok(path) => path,
            Err(error) => return CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        let config = match ProjectConfig::discover(Some(&project_dir), &project_dir).await {
            Ok(config) => config,
            Err(error) => return error_result("Error pulling project", error),
        };
        let script_id = match config.script_id.clone() {
            Some(id) => id,
            None => return error_result("Error pulling project", "Project settings not found."),
        };
        let client = self.client();
        match fetch_remote_files(&client, &script_id, &config, &project_dir, None).await {
            Ok(remote) => {
                let inputs = remote
                    .iter()
                    .map(|file| file.file.clone())
                    .collect::<Vec<_>>();
                match pull_files(
                    &inputs,
                    &config.content_dir,
                    config.allow_symlinks,
                    32,
                    &LocalExtensions::from_config(&config),
                )
                .await
                {
                    Ok(_) => success_with_files(
                        format!(
                            "Pulled project in {} to local filesystem successfully.",
                            args.project_dir
                        ),
                        script_id,
                        &args.project_dir,
                        &project_dir,
                        remote.into_iter().map(|file| file.local_path).collect(),
                    ),
                    Err(error) => error_result("Error pulling project", error),
                }
            }
            Err(error) => error_result("Error pulling project", error),
        }
    }

    #[tool(
        name = "create_project",
        description = "Create a new apps script project.",
        annotations(title = "Create Apps Script project", open_world_hint = false, destructive_hint = true, idempotent_hint = false, read_only_hint = false),
        output_schema = rmcp::handler::server::tool::schema_for_type::<FilesOutput>()
    )]
    async fn create_project(
        &self,
        Parameters(args): Parameters<CreateProjectArgs>,
    ) -> CallToolResult {
        let project_dir = match self.validate_project(&args.project_dir) {
            Ok(path) => path,
            Err(error) => return CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        if let Err(error) = tokio::fs::create_dir_all(&project_dir).await {
            return error_result("Error creating project", error);
        }
        let source_dir = match args.source_dir.as_deref() {
            Some(source) => match validate_source_dir(&project_dir, source) {
                Ok(path) => Some(path),
                Err(error) => return error_result("Error creating project", error),
            },
            None => None,
        };
        let mut config = match ProjectConfig::discover(Some(&project_dir), &project_dir).await {
            Ok(config) => config,
            Err(error) => return error_result("Error creating project", error),
        };
        if let Some(source_dir) = source_dir {
            config.content_dir = source_dir;
        }
        let client = self.client();
        // clasp create_project (mcp/server.ts:335 + create-script.ts:70,201):
        // an omitted projectName humanizes the project directory basename
        // (inflection.humanize), exactly like `create-script`'s default title.
        let name = args
            .project_name
            .clone()
            .unwrap_or_else(|| crate::commands::create_script::default_project_name(&project_dir));
        match create_script(&client, &name, config.parent_id.as_deref()).await {
            Ok(script_id) => {
                match pull_initial_files(&client, &script_id, &config, &project_dir, None).await {
                    Ok(pulled) => {
                        let mut settings = config.clone();
                        settings.script_id = Some(script_id.clone());
                        if let Err(error) = settings.update_settings().await {
                            return error_result("Error creating project", error);
                        }
                        success_with_files(
                            format!(
                                "Created project {script_id} in {} successfully.",
                                args.project_dir
                            ),
                            script_id,
                            &args.project_dir,
                            &project_dir,
                            pulled.files,
                        )
                    }
                    Err(error) => error_result("Error creating project", error),
                }
            }
            Err(error) => error_result("Error creating project", error),
        }
    }

    #[tool(
        name = "clone_project",
        description = "Clones and pulls an existing Apps Script project to a local directory.",
        annotations(title = "Clone Apps Script project", open_world_hint = false, destructive_hint = true, idempotent_hint = false, read_only_hint = false),
        output_schema = rmcp::handler::server::tool::schema_for_type::<FilesOutput>()
    )]
    async fn clone_project(
        &self,
        Parameters(args): Parameters<CloneProjectArgs>,
    ) -> CallToolResult {
        let project_dir = match self.validate_project(&args.project_dir) {
            Ok(path) => path,
            Err(error) => return CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        if let Err(error) = tokio::fs::create_dir_all(&project_dir).await {
            return error_result("Error cloning project", error);
        }
        let script_id = match args.script_id.filter(|id| !id.is_empty()) {
            Some(id) => id,
            None => return CallToolResult::error(vec![ContentBlock::text(SCRIPT_ID_REQUIRED)]),
        };
        let source_dir = match args.source_dir.as_deref() {
            Some(source) => match validate_source_dir(&project_dir, source) {
                Ok(path) => Some(path),
                Err(error) => return error_result("Error cloning project", error),
            },
            None => None,
        };
        let mut config = match ProjectConfig::discover(Some(&project_dir), &project_dir).await {
            Ok(config) => config,
            Err(error) => return error_result("Error cloning project", error),
        };
        if let Some(source_dir) = source_dir {
            config.content_dir = source_dir;
        }
        config.script_id = Some(script_id.clone());
        let client = self.client();
        match pull_initial_files(&client, &script_id, &config, &project_dir, None).await {
            Ok(pulled) => {
                if let Err(error) = config.update_settings().await {
                    return error_result("Error cloning project", error);
                }
                success_with_files(
                    format!(
                        "Cloned project {script_id} in {} successfully.",
                        args.project_dir
                    ),
                    script_id,
                    &args.project_dir,
                    &project_dir,
                    pulled.files,
                )
            }
            Err(error) => error_result("Error cloning project", error),
        }
    }

    #[tool(
        name = "list_projects",
        description = "List Apps Script projects",
        annotations(title = "List Apps Script projects", open_world_hint = false, destructive_hint = false, idempotent_hint = false, read_only_hint = true),
        output_schema = rmcp::handler::server::tool::schema_for_type::<ScriptsOutput>()
    )]
    async fn list_projects(&self, Parameters(_args): Parameters<EmptyArgs>) -> CallToolResult {
        let client = self.client();
        let scripts = match list_scripts(&client).await {
            Ok(scripts) => scripts.results,
            Err(error) => return error_result("Error listing projects", error),
        };
        let entries = scripts
            .into_iter()
            .filter_map(|script| {
                Some(ScriptOutput {
                    script_id: script.id?,
                    name: script.name.unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        let mut content = vec![ContentBlock::text(format!(
            "Found {} Apps Script projects (script ID in parentheses):",
            entries.len()
        ))];
        content.extend(
            entries
                .iter()
                .map(|entry| ContentBlock::text(format!("{} ({})", entry.name, entry.script_id))),
        );
        let mut result = CallToolResult::structured(json!(ScriptsOutput { scripts: entries }));
        result.content = content;
        result
    }
}

#[tool_handler(name = "Crsp")]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("Crsp", env!("CARGO_PKG_VERSION")))
    }
}

pub async fn start_server(
    context: &Arc<crate::core::clasp::Clasp>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // clasp start-mcp.ts: the MCP server shares the preAction-initialized
    // auth context (user credentials from `$HOME`), so tool API calls carry
    // the user's bearer token and 401-refresh exactly like the CLI commands.
    let service = McpServer::with_context(context).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
