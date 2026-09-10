use clap::Parser;
use crsp::{Cli, Commands, run};

const COMMANDS: &[&str] = &[
    "login",
    "logout",
    "show-authorized-user",
    "clone-script",
    "create-script",
    "push",
    "pull",
    "create-deployment",
    "update-deployment",
    "delete-deployment",
    "delete-script",
    "create-version",
    "list-versions",
    "list-deployments",
    "list-scripts",
    "run-function",
    "tail-logs",
    "setup-logs",
    "show-file-status",
    "list-apis",
    "enable-api",
    "disable-api",
    "open-script",
    "open-container",
    "open-web-app",
    "open-logs",
    "open-api-console",
    "open-credentials-setup",
    "start-mcp-server",
];

const ALIASES: &[&str] = &[
    "clone",
    "create",
    "deploy",
    "redeploy",
    "undeploy",
    "delete",
    "version",
    "versions",
    "deployments",
    "list",
    "run",
    "logs",
    "status",
    "apis",
    "mcp",
];

fn parse(command: &str) -> Cli {
    let args = match command {
        "login" => vec!["crsp", command, "--include-clasp-scopes"],
        "update-deployment" | "redeploy" | "enable-api" | "disable-api" => {
            vec!["crsp", command, "fixture"]
        }
        _ => vec!["crsp", command],
    };
    Cli::try_parse_from(args).expect("surface command parses")
}

fn assert_wired(command: &str) {
    let cli = parse(command);
    let result = run(&cli);
    assert!(
        !format!("{result:?}").contains("NotImplemented"),
        "{command} still uses placeholder dispatch"
    );
}

#[test]
fn every_canonical_command_dispatches_without_placeholder_error() {
    for command in COMMANDS {
        assert_wired(command);
    }
}

#[test]
fn every_alias_dispatches_without_placeholder_error() {
    for command in ALIASES {
        assert_wired(command);
    }
}

#[test]
fn command_variants_remain_real_parser_variants() {
    for command in COMMANDS.iter().chain(ALIASES) {
        assert!(!matches!(
            parse(command).command,
            Some(Commands::External(_))
        ));
    }
}
