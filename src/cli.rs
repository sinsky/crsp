//! clap definition of the crsp CLI surface (spec §2.5): global options,
//! all 29 canonical commands with exact names, aliases, positional arguments,
//! option names, short flags, and defaults, plus external-command capture so
//! unknown commands can be reported the clasp way (spec §5 #1).
//!
//! Help strings are given as explicit `help`/`about` attributes (not doc
//! comments) because clap rewrites doc comments (stripping trailing periods),
//! while attribute strings are used verbatim — required for clasp-identical
//! output.

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

use crate::i18n;

// The root parser. Built by clap's `try_parse`; not meant to be constructed
// by hand. Doc comments are avoided because clap renders them as long_about.
#[derive(Parser, Debug)]
#[allow(clippy::manual_non_exhaustive)]
#[command(
    name = crate::constants::PROJECT_NAME,
    version = env!("CARGO_PKG_VERSION"),
    disable_version_flag = true,
    // clap attributes require literals; keep in sync with constants::PROJECT_NAME.
    about = "crsp - The Apps Script CLI",
    override_usage = "crsp <command> [options]",
    allow_external_subcommands = true
)]
pub struct Cli {
    #[arg(short = 'v', long, action = ArgAction::Version, help = "output the current version")]
    version: (),

    #[command(flatten)]
    pub globals: GlobalOptions,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Global options; every one is also accepted after the subcommand, matching
/// commander's ancestor option lookup in clasp.
#[derive(Args, Debug)]
pub struct GlobalOptions {
    #[arg(
        short = 'A',
        long,
        global = true,
        env = "clasp_config_auth",
        value_name = "file",
        help = "path to an auth file or a folder with a '.clasprc.json' file."
    )]
    pub auth: Option<String>,

    #[arg(
        short = 'u',
        long,
        global = true,
        default_value = "default",
        value_name = "name",
        help = "Store named credentials. If unspecified, the \"default\" user is used."
    )]
    pub user: String,

    #[arg(
        long,
        global = true,
        help = "Use the application default credentials from the environment."
    )]
    pub adc: bool,

    #[arg(long, global = true, help = "Show output in JSON format")]
    pub json: bool,

    #[arg(
        long,
        global = true,
        help = "Allow symlinked local files and directories to be pushed or pulled"
    )]
    pub allow_symlinks: bool,

    #[arg(
        short = 'I',
        long,
        global = true,
        env = "clasp_config_ignore",
        value_name = "file",
        help = "path to an ignore file or a folder with a '.claspignore' file."
    )]
    pub ignore: Option<String>,

    #[arg(
        short = 'P',
        long,
        global = true,
        env = "clasp_config_project",
        value_name = "file",
        help = "path to a project file or to a folder with a '.clasp.json' file."
    )]
    pub project: Option<String>,
}

/// All 29 canonical commands from spec §2.5 (canonical long names, short
/// forms registered as aliases).
#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(name = "login", about = "Log in to script.google.com")]
    Login(LoginArgs),

    #[command(name = "logout", about = "Logout of clasp")]
    Logout,

    #[command(
        name = "show-authorized-user",
        about = "Show information about the current authorizations state."
    )]
    ShowAuthorizedUser,

    #[command(
        name = "clone-script",
        alias = "clone",
        about = "Clone an existing script"
    )]
    CloneScript(CloneScriptArgs),

    #[command(name = "create-script", alias = "create", about = "Create a script")]
    CreateScript(CreateScriptArgs),

    #[command(name = "push", about = "Update the remote project")]
    Push(PushArgs),

    #[command(name = "pull", about = "Fetch a remote project")]
    Pull(PullArgs),

    #[command(
        name = "create-deployment",
        alias = "deploy",
        about = "Deploy a project"
    )]
    CreateDeployment(CreateDeploymentArgs),

    #[command(
        name = "update-deployment",
        alias = "redeploy",
        about = "Updates a deployment for a project to a new version"
    )]
    UpdateDeployment(UpdateDeploymentArgs),

    #[command(
        name = "delete-deployment",
        alias = "undeploy",
        about = "Delete a deployment of a project"
    )]
    DeleteDeployment(DeleteDeploymentArgs),

    #[command(name = "delete-script", alias = "delete", about = "Delete a project")]
    DeleteScript(DeleteScriptArgs),

    #[command(
        name = "create-version",
        alias = "version",
        about = "Creates an immutable version of the script"
    )]
    CreateVersion(CreateVersionArgs),

    #[command(
        name = "list-versions",
        alias = "versions",
        about = "List versions of a script"
    )]
    ListVersions(ListVersionsArgs),

    #[command(
        name = "list-deployments",
        alias = "deployments",
        about = "List deployment ids of a script"
    )]
    ListDeployments(ListDeploymentsArgs),

    #[command(
        name = "list-scripts",
        alias = "list",
        about = "List Apps Script projects"
    )]
    ListScripts(ListScriptsArgs),

    #[command(
        name = "run-function",
        alias = "run",
        about = "Run a function in your Apps Scripts project"
    )]
    RunFunction(RunFunctionArgs),

    #[command(
        name = "tail-logs",
        alias = "logs",
        about = "Print the most recent log entries"
    )]
    TailLogs(TailLogsArgs),

    #[command(name = "setup-logs", about = "Setup Cloud Logging")]
    SetupLogs,

    #[command(
        name = "show-file-status",
        alias = "status",
        about = "Lists files that will be pushed by clasp"
    )]
    ShowFileStatus,

    #[command(
        name = "list-apis",
        alias = "apis",
        about = "List enabled APIs for the current project"
    )]
    ListApis,

    #[command(
        name = "enable-api",
        about = "Enable a service for the current project."
    )]
    EnableApi(EnableApiArgs),

    #[command(
        name = "disable-api",
        about = "Disable a service for the current project."
    )]
    DisableApi(DisableApiArgs),

    #[command(
        name = "open-script",
        about = "Open the Apps Script IDE for the current project."
    )]
    OpenScript(OpenScriptArgs),

    #[command(
        name = "open-container",
        about = "Open the Apps Script IDE for the current project."
    )]
    OpenContainer,

    #[command(
        name = "open-web-app",
        about = "Open a deployed web app in the browser."
    )]
    OpenWebApp(OpenWebAppArgs),

    #[command(name = "open-logs", about = "Open logs in the developer console")]
    OpenLogs,

    #[command(
        name = "open-api-console",
        about = "Open the API console for the current project."
    )]
    OpenApiConsole,

    #[command(
        name = "open-credentials-setup",
        about = "Open credentials page for the script's GCP project"
    )]
    OpenCredentialsSetup,

    #[command(
        name = "start-mcp-server",
        alias = "mcp",
        about = "Starts an MCP server for interacting with apps script."
    )]
    StartMcpServer,

    #[command(name = "completion", about = "Generate shell completion scripts")]
    Completion(CompletionArgs),

    /// Catch-all for unknown commands so `run` can report them clasp-style.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    #[arg(value_name = "shell", help = "The shell to generate completions for")]
    pub shell: CompletionShell,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "zsh")]
    Zsh,
    #[value(name = "fish")]
    Fish,
    #[value(name = "powershell")]
    Powershell,
}

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long, help = "Do not run a local server, manually enter code instead")]
    pub no_localhost: bool,

    #[arg(
        long,
        value_name = "file",
        help = "Relative path to OAuth client secret file (from GCP)."
    )]
    pub creds: Option<String>,

    #[arg(
        long,
        help = "Use the scopes from the current project manifest. Used only when authorizing access for the run command."
    )]
    pub use_project_scopes: bool,

    #[arg(
        long,
        help = "Include default clasp scopes in addition to project scopes. Can only be used with --use-project-scopes."
    )]
    pub include_clasp_scopes: bool,

    #[arg(
        long,
        value_name = "scopes",
        value_delimiter = ',',
        value_parser = parse_scope_element,
        help = "Include additional OAuth scopes as a comma-separated list."
    )]
    pub extra_scopes: Vec<String>,

    #[arg(
        long,
        value_name = "port",
        value_parser = parse_redirect_port,
        help = "Specify a custom port for the redirect URL."
    )]
    pub redirect_port: Option<u16>,
}

#[derive(Args, Debug)]
pub struct CloneScriptArgs {
    #[arg(value_name = "scriptId")]
    pub script_id: Option<String>,

    #[arg(value_name = "versionNumber")]
    pub version_number: Option<String>,

    #[arg(
        long = "rootDir",
        value_name = "rootDir",
        help = "Local root directory in which clasp will store your project files."
    )]
    pub root_dir: Option<String>,
}

#[derive(Args, Debug)]
pub struct CreateScriptArgs {
    #[arg(
        long = "type",
        value_name = "type",
        default_value = "standalone",
        help = "Creates a new Apps Script project attached to a new Document, Spreadsheet, Presentation, Form, or as a standalone script, web app, or API."
    )]
    pub script_type: String,

    #[arg(long, value_name = "title", help = "The project title.")]
    pub title: Option<String>,

    #[arg(long = "parentId", value_name = "id", help = "A project parent Id.")]
    pub parent_id: Option<String>,

    #[arg(
        long = "rootDir",
        value_name = "rootDir",
        help = "Local root directory in which clasp will store your project files."
    )]
    pub root_dir: Option<String>,
}

#[derive(Args, Debug)]
pub struct PushArgs {
    #[arg(short = 'f', long, help = "Forcibly overwrites the remote manifest.")]
    pub force: bool,

    #[arg(
        short = 'w',
        long,
        help = "Watches for local file changes. Pushes when a non-ignored file changes."
    )]
    pub watch: bool,
}

#[derive(Args, Debug)]
pub struct PullArgs {
    #[arg(
        long = "versionNumber",
        value_name = "version",
        help = "The version number of the project to retrieve."
    )]
    pub version_number: Option<String>,

    #[arg(
        short = 'd',
        long = "deleteUnusedFiles",
        help = "Delete local files that are not in the remote project. Use with caution."
    )]
    pub delete_unused_files: bool,

    #[arg(
        short = 'f',
        long,
        help = "Forcibly delete local files that are not in the remote project without prompting."
    )]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct CreateDeploymentArgs {
    #[arg(
        short = 'V',
        long = "versionNumber",
        value_name = "version",
        help = "The project version"
    )]
    pub version_number: Option<String>,

    #[arg(
        short = 'd',
        long,
        value_name = "description",
        help = "The deployment description"
    )]
    pub description: Option<String>,

    #[arg(
        short = 'i',
        long = "deploymentId",
        value_name = "id",
        help = "The deployment ID to redeploy"
    )]
    pub deployment_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct UpdateDeploymentArgs {
    #[arg(value_name = "deploymentId")]
    pub deployment_id: String,

    #[arg(
        short = 'V',
        long = "versionNumber",
        value_name = "version",
        help = "The project version"
    )]
    pub version_number: Option<String>,

    #[arg(
        short = 'd',
        long,
        value_name = "description",
        help = "The deployment description"
    )]
    pub description: Option<String>,
}

#[derive(Args, Debug)]
pub struct DeleteDeploymentArgs {
    #[arg(value_name = "deploymentId")]
    pub deployment_id: Option<String>,

    #[arg(short = 'a', long, help = "Undeploy all deployments")]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct DeleteScriptArgs {
    #[arg(
        value_name = "scriptId",
        help = "Apps Script ID to list deployments for"
    )]
    pub script_id: Option<String>,

    #[arg(
        short = 'f',
        long,
        help = "Bypass any confirmation messages. It's not a good idea to do this unless you want to run clasp from a script."
    )]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct CreateVersionArgs {
    #[arg(value_name = "description")]
    pub description: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListVersionsArgs {
    #[arg(
        value_name = "scriptId",
        help = "Apps Script ID to list deployments for"
    )]
    pub script_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListDeploymentsArgs {
    #[arg(
        value_name = "scriptId",
        help = "Apps Script ID to list deployments for"
    )]
    pub script_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListScriptsArgs {
    #[arg(long = "noShorten", help = "Do not shorten long names")]
    pub no_shorten: bool,
}

#[derive(Args, Debug)]
pub struct RunFunctionArgs {
    #[arg(value_name = "functionName", help = "The name of the function to run")]
    pub function_name: Option<String>,

    #[arg(long, help = "Run script function in non-devMode")]
    pub nondev: bool,

    #[arg(
        short = 'p',
        long,
        value_name = "value",
        help = "Parameters to pass to the function, as a JSON-encoded array"
    )]
    pub params: Option<String>,
}

#[derive(Args, Debug)]
pub struct TailLogsArgs {
    #[arg(long, help = "Watch and print new logs")]
    pub watch: bool,

    #[arg(long, help = "Hide timestamps with logs")]
    pub simplified: bool,
}

#[derive(Args, Debug)]
pub struct EnableApiArgs {
    #[arg(value_name = "api", help = "Service to enable")]
    pub api: String,
}

#[derive(Args, Debug)]
pub struct DisableApiArgs {
    #[arg(value_name = "api", help = "Service to disable")]
    pub api: String,
}

#[derive(Args, Debug)]
pub struct OpenScriptArgs {
    #[arg(value_name = "scriptId")]
    pub script_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct OpenWebAppArgs {
    #[arg(value_name = "deploymentId")]
    pub deployment_id: Option<String>,
}

/// clasp's `parseExtraScopes` element rule: trimmed, never empty.
fn parse_scope_element(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(i18n::EXTRA_SCOPES_INVALID.to_string());
    }
    Ok(trimmed.to_string())
}

/// clasp's `validateOptionInt` for `--redirect-port`: integer within 0–65535.
fn parse_redirect_port(value: &str) -> Result<u16, String> {
    let parsed: i64 = value
        .parse()
        .map_err(|_| i18n::not_a_valid_integer(value))?;
    if (0..=65535).contains(&parsed) {
        Ok(parsed as u16)
    } else {
        Err(i18n::integer_out_of_range(value, 0, 65535))
    }
}
