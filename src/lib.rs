//! crsp - The Apps Script CLI (Rust reimplementation of clasp, spec
//! docs/superpowers/specs/2026-09-09-crsp-design.md).

pub mod api;
pub mod auth;
pub mod cli;
pub mod constants;
pub mod core;
pub mod error;
pub mod i18n;
pub mod output;
pub mod text;
pub mod ui;

use clap::CommandFactory;

use crate::error::CrspError;

pub use crate::cli::{Cli, Commands};

/// Dispatches a parsed command line (spec §3.2 flow). Command handlers are
/// wired by later tasks; every canonical command currently reports
/// [`CrspError::NotImplemented`]. Unknown commands render clasp's diagnostic
/// plus help on stdout, and a missing subcommand renders help on stderr.
pub fn run(cli: &Cli) -> Result<(), CrspError> {
    match &cli.command {
        None => {
            let help = Cli::command().render_help().to_string();
            Err(CrspError::Validation(help.trim_end().to_string()))
        }
        Some(Commands::External(args)) => {
            let command = args.first().map(String::as_str).unwrap_or_default();
            let mut program = Cli::command();
            program.print_help().map_err(CrspError::Io)?;
            Err(CrspError::Validation(i18n::unknown_command(command)))
        }
        Some(command) => dispatch_not_wired(command),
    }
}

fn dispatch_not_wired(command: &Commands) -> Result<(), CrspError> {
    Err(CrspError::NotImplemented(
        canonical_name(command).to_string(),
    ))
}

fn canonical_name(command: &Commands) -> &'static str {
    match command {
        Commands::Login(_) => "login",
        Commands::Logout => "logout",
        Commands::ShowAuthorizedUser => "show-authorized-user",
        Commands::CloneScript(_) => "clone-script",
        Commands::CreateScript(_) => "create-script",
        Commands::Push(_) => "push",
        Commands::Pull(_) => "pull",
        Commands::CreateDeployment(_) => "create-deployment",
        Commands::UpdateDeployment(_) => "update-deployment",
        Commands::DeleteDeployment(_) => "delete-deployment",
        Commands::DeleteScript(_) => "delete-script",
        Commands::CreateVersion(_) => "create-version",
        Commands::ListVersions(_) => "list-versions",
        Commands::ListDeployments(_) => "list-deployments",
        Commands::ListScripts(_) => "list-scripts",
        Commands::RunFunction(_) => "run-function",
        Commands::TailLogs(_) => "tail-logs",
        Commands::SetupLogs => "setup-logs",
        Commands::ShowFileStatus => "show-file-status",
        Commands::ListApis => "list-apis",
        Commands::EnableApi(_) => "enable-api",
        Commands::DisableApi(_) => "disable-api",
        Commands::OpenScript(_) => "open-script",
        Commands::OpenContainer => "open-container",
        Commands::OpenWebApp(_) => "open-web-app",
        Commands::OpenLogs => "open-logs",
        Commands::OpenApiConsole => "open-api-console",
        Commands::OpenCredentialsSetup => "open-credentials-setup",
        Commands::StartMcpServer => "start-mcp-server",
        Commands::External(_) => "external",
    }
}
