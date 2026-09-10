pub mod api;
pub mod auth;
pub mod cli;
pub mod commands;
pub mod constants;
pub mod core;
pub mod error;
pub mod i18n;
pub mod mcp;
pub mod output;
pub mod text;
pub mod ui;

use crate::auth::flow::{AuthOptions, validate_scope_options};
use crate::auth::oauth_client::AuthEndpoints;
use crate::auth::{CredentialStore, login, logout, show_authorized_user};
use crate::cli::*;
use crate::commands::shared::{SystemOpener, include_user_hint_in_url};
use crate::core::clasp::Clasp;
use crate::output::Output;
use crate::ui::{DemandAdapter, Ui};
use clap::CommandFactory;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static RUNTIME_COUNT: AtomicUsize = AtomicUsize::new(0);

pub use crate::cli::{Cli, Commands};
pub use crate::error::CrspError;

pub fn runtime_count() -> usize {
    RUNTIME_COUNT.load(Ordering::SeqCst)
}

pub fn run(cli: &Cli) -> Result<(), CrspError> {
    if matches!(cli.command, Some(Commands::ShowFileStatus)) {
        return run_status_sync(cli);
    }
    match &cli.command {
        None => {
            let help = Cli::command().render_help().to_string();
            Err(CrspError::Validation(help.trim_end().to_string()))
        }
        Some(Commands::External(args)) => {
            let command = args.first().map(String::as_str).unwrap_or_default();
            let mut program = Cli::command();
            program.print_help().map_err(CrspError::Io)?;
            Err(CrspError::Validation(crate::i18n::unknown_command(command)))
        }
        Some(_) => runtime().block_on(run_async(cli)),
    }
}

fn run_status_sync(cli: &Cli) -> Result<(), CrspError> {
    let cwd = std::env::current_dir()?;
    let config_path = cli
        .globals
        .project
        .as_deref()
        .map(Path::new)
        .map(|path| {
            if path.is_dir() {
                path.join(crate::constants::PROJECT_CONFIG_FILENAME)
            } else {
                path.to_path_buf()
            }
        })
        .unwrap_or_else(|| cwd.join(crate::constants::PROJECT_CONFIG_FILENAME));
    if !config_path.exists() {
        return Err(CrspError::Validation(
            crate::i18n::PROJECT_SETTINGS_NOT_FOUND.to_string(),
        ));
    }
    let content = std::fs::read_to_string(&config_path)?;
    let value: serde_json::Value =
        json5::from_str(&content).map_err(|error| CrspError::Config(error.to_string()))?;
    let root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let content_dir = root.join(
        value
            .get("srcDir")
            .or_else(|| value.get("rootDir"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("."),
    );
    let tracked = crate::commands::show_file_status::sync_status(&content_dir)?;
    let mut output = Output::stdout(cli.globals.json);
    if output.is_json() {
        output.print_json(&tracked)?;
    } else {
        output.message("Tracked files:");
        for file in &tracked.files_to_push {
            output.message(&format!("└─ {file}"));
        }
        output.message("Untracked files:");
        for file in &tracked.untracked_files {
            output.message(&format!("└─ {file}"));
        }
    }
    Ok(())
}

async fn run_async(cli: &Cli) -> Result<(), CrspError> {
    let _context = Clasp::init_context(
        cli.globals.project.as_deref().map(Path::new),
        cli.globals.ignore.as_deref().map(Path::new),
        cli.globals.auth.as_deref().map(Path::new),
        &cli.globals.user,
        cli.globals.adc,
        cli.globals.allow_symlinks,
    )
    .await?;
    match &cli.command {
        Some(Commands::StartMcpServer) => crate::mcp::start_server()
            .await
            .map_err(|error| CrspError::Io(std::io::Error::other(error.to_string()))),
        Some(Commands::Login(args)) => run_login(cli, args).await,
        Some(Commands::Logout) => run_logout(cli).await,
        Some(Commands::ShowAuthorizedUser) => run_show_user(cli).await,
        Some(command) => run_command(cli, command).await,
        None => unreachable!(),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    RUNTIME_COUNT.fetch_add(1, Ordering::SeqCst);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn run_login(cli: &Cli, args: &LoginArgs) -> Result<(), CrspError> {
    validate_scope_options(args.use_project_scopes, args.include_clasp_scopes)?;
    let store = CredentialStore::new(
        auth_path(cli.globals.auth.as_deref())?,
        cli.globals.allow_symlinks,
    );
    let options = AuthOptions {
        no_localhost: args.no_localhost,
        creds_file: args.creds.clone().map(Into::into),
        use_project_scopes: args.use_project_scopes,
        include_clasp_scopes: args.include_clasp_scopes,
        extra_scopes: args.extra_scopes.clone(),
        redirect_port: args.redirect_port,
        adc: cli.globals.adc,
    };
    let ui = Ui::new(DemandAdapter);
    let mut output = Output::stdout(cli.globals.json);
    let payload = login(
        &options,
        None,
        &store,
        &cli.globals.user,
        &ui,
        &mut output,
        &reqwest::Client::new(),
        &AuthEndpoints::default(),
        true,
    )
    .await?;
    if output.is_json() {
        output.print_json(&payload)?;
    }
    Ok(())
}

async fn run_logout(cli: &Cli) -> Result<(), CrspError> {
    let store = CredentialStore::new(
        auth_path(cli.globals.auth.as_deref())?,
        cli.globals.allow_symlinks,
    );
    let result = logout(&store, &cli.globals.user).await?;
    let mut output = Output::stdout(cli.globals.json);
    if output.is_json() {
        output.print_json(&crate::auth::flow::logout_payload())?;
    } else if result.deleted {
        output.message("Deleted credentials.");
    }
    Ok(())
}

async fn run_show_user(cli: &Cli) -> Result<(), CrspError> {
    let store = CredentialStore::new(
        auth_path(cli.globals.auth.as_deref())?,
        cli.globals.allow_symlinks,
    );
    let credentials =
        crate::auth::load_credentials(&store, &cli.globals.user, cli.globals.adc).await?;
    let payload = show_authorized_user(
        credentials,
        Some(&store),
        &cli.globals.user,
        &reqwest::Client::new(),
        &AuthEndpoints::default(),
    )
    .await?;
    let mut output = Output::stdout(cli.globals.json);
    if output.is_json() {
        output.print_json(&payload)?;
    } else if payload.logged_in {
        output.message(&format!(
            "Logged in as {}.",
            payload.email.unwrap_or_default()
        ));
    } else {
        output.message("Not logged in.");
    }
    Ok(())
}

async fn run_command(cli: &Cli, command: &Commands) -> Result<(), CrspError> {
    let context = Clasp::init(
        cli.globals.project.as_deref().map(Path::new),
        cli.globals.ignore.as_deref().map(Path::new),
        cli.globals.auth.as_deref().map(Path::new),
        &cli.globals.user,
        cli.globals.adc,
        cli.globals.allow_symlinks,
    )
    .await?;
    let ui = Ui::new(DemandAdapter);
    let opener = SystemOpener;
    let mut output = Output::stdout(cli.globals.json);
    let cwd = std::env::current_dir()?;
    let mut config = context.config.clone();
    match command {
        Commands::CloneScript(args) => {
            crate::commands::clone_script::clone_script(
                &context.client,
                &config,
                crate::commands::clone_script::CloneArgs {
                    script_id: args.script_id.as_deref(),
                    version_number: args.version_number.as_deref(),
                    root_dir: args.root_dir.as_deref(),
                    cwd: &cwd,
                },
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::CreateScript(args) => {
            crate::commands::create_script::create_script(
                &context.client,
                &config,
                crate::commands::create_script::CreateScriptArgs {
                    script_type: &args.script_type,
                    title: args.title.as_deref(),
                    parent_id: args.parent_id.as_deref(),
                    root_dir: args.root_dir.as_deref(),
                    cwd: &cwd,
                },
                &mut output,
            )
            .await?;
        }
        Commands::Push(args) => {
            crate::commands::push::push(
                &context.client,
                &config,
                args.force,
                args.watch,
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::Pull(args) => {
            let id = crate::core::project::assert_script_configured(&config).await?;
            let remote = crate::core::project::fetch_remote_files(
                &context.client,
                id,
                &config,
                &cwd,
                args.version_number
                    .as_deref()
                    .map(|v| {
                        v.parse().map_err(|_| {
                            CrspError::Validation(format!("'{v}' is not a valid integer."))
                        })
                    })
                    .transpose()?,
            )
            .await?;
            crate::commands::pull::pull(
                &context.client,
                &config,
                &remote.iter().map(|f| f.file.clone()).collect::<Vec<_>>(),
                args.delete_unused_files,
                args.force,
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::CreateDeployment(args) => {
            crate::commands::create_deployment::create_deployment(
                &context.client,
                &config,
                crate::commands::create_deployment::CreateDeploymentArgs {
                    version_number: args.version_number.as_deref(),
                    description: args.description.as_deref(),
                    deployment_id: args.deployment_id.as_deref(),
                },
                &mut output,
            )
            .await?;
        }
        Commands::UpdateDeployment(args) => {
            crate::commands::update_deployment::update_deployment(
                &context.client,
                &config,
                args.deployment_id.as_str(),
                crate::commands::update_deployment::UpdateDeploymentArgs {
                    version_number: args.version_number.as_deref(),
                    description: args.description.as_deref(),
                },
                &mut output,
            )
            .await?;
        }
        Commands::DeleteDeployment(args) => {
            crate::commands::delete_deployment::delete_deployment(
                &context.client,
                &config,
                args.deployment_id.as_deref(),
                args.all,
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::DeleteScript(args) => {
            crate::commands::delete_script::delete_script(
                &context.client,
                &config,
                args.script_id.as_deref(),
                args.force,
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::CreateVersion(args) => {
            crate::commands::create_version::create_version(
                &context.client,
                &config,
                args.description.as_deref(),
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::ListVersions(args) => {
            crate::commands::list_versions::list_versions(
                &context.client,
                &config,
                args.script_id.as_deref(),
                &mut output,
            )
            .await?;
        }
        Commands::ListDeployments(args) => {
            crate::commands::list_deployments::list_deployments(
                &context.client,
                &config,
                args.script_id.as_deref(),
                &mut output,
            )
            .await?;
        }
        Commands::ListScripts(args) => {
            crate::commands::list_scripts::list_scripts(
                &context.client,
                args.no_shorten,
                &mut output,
            )
            .await?;
        }
        Commands::RunFunction(args) => {
            crate::commands::run_function::run_function(
                &context.client,
                &config,
                crate::commands::run_function::RunFunctionArgs {
                    function_name: args.function_name.as_deref(),
                    nondev: args.nondev,
                    params: args.params.as_deref(),
                },
                &ui,
                &mut output,
            )
            .await?;
        }
        Commands::TailLogs(args) => {
            crate::commands::tail_logs::tail_logs(
                &context.client,
                &mut config,
                crate::commands::tail_logs::TailLogsArgs {
                    watch: args.watch,
                    simplified: args.simplified,
                    ..Default::default()
                },
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::SetupLogs => {
            crate::commands::setup_logs::setup_logs(&mut config, &ui, &opener, &mut output).await?;
        }
        Commands::ShowFileStatus => {
            crate::commands::show_file_status::show_file_status(&config, &mut output).await?;
        }
        Commands::ListApis => {
            crate::commands::list_apis::list_apis(
                &context.client,
                &mut config,
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::EnableApi(args) => {
            crate::commands::enable_api::enable_api(
                &context.client,
                &mut config,
                &args.api,
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::DisableApi(args) => {
            crate::commands::disable_api::disable_api(
                &context.client,
                &mut config,
                &args.api,
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenScript(args) => {
            crate::commands::open_script::open_script(
                &context.client,
                &config,
                crate::commands::open_script::OpenScriptArgs {
                    script_id: args.script_id.as_deref(),
                },
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenContainer => {
            crate::commands::open_container::open_container(
                &context.client,
                &config,
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenWebApp(args) => {
            crate::commands::open_web_app::open_web_app(
                &context.client,
                &config,
                crate::commands::open_web_app::OpenWebAppArgs {
                    deployment_id: args.deployment_id.as_deref(),
                },
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenLogs => {
            crate::commands::open_logs::open_logs(
                &context.client,
                &mut config,
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenApiConsole => {
            crate::commands::open_api_console::open_api_console(
                &context.client,
                &mut config,
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::OpenCredentialsSetup => {
            crate::commands::open_credentials_setup::open_credentials_setup(
                &context.client,
                &mut config,
                include_user_hint_in_url(),
                &ui,
                &opener,
                &mut output,
            )
            .await?;
        }
        Commands::Login(_)
        | Commands::Logout
        | Commands::ShowAuthorizedUser
        | Commands::StartMcpServer
        | Commands::External(_) => unreachable!(),
    }
    Ok(())
}

fn auth_path(auth: Option<&str>) -> Result<std::path::PathBuf, CrspError> {
    let path = auth
        .map(Path::new)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            home::home_dir()
                .unwrap_or_default()
                .join(crate::constants::CREDENTIALS_FILENAME)
        });
    Ok(if path.is_dir() {
        path.join(crate::constants::CREDENTIALS_FILENAME)
    } else {
        path
    })
}
