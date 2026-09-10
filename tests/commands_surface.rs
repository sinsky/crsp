use assert_cmd::Command;
use clap::Parser;
use crsp::{Cli, Commands, run};
use predicates::str::contains;

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

const ALIAS_PAIRS: &[(&str, &str)] = &[
    ("clone", "clone-script"),
    ("create", "create-script"),
    ("deploy", "create-deployment"),
    ("redeploy", "update-deployment"),
    ("undeploy", "delete-deployment"),
    ("delete", "delete-script"),
    ("version", "create-version"),
    ("versions", "list-versions"),
    ("deployments", "list-deployments"),
    ("list", "list-scripts"),
    ("run", "run-function"),
    ("logs", "tail-logs"),
    ("status", "show-file-status"),
    ("apis", "list-apis"),
    ("mcp", "start-mcp-server"),
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
        "login" => vec![
            "crsp",
            command,
            "--creds",
            "/definitely/missing/client-secret.json",
        ],
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
fn aliases_and_canonical_commands_parse_to_equivalent_variants() {
    for (alias, canonical) in ALIAS_PAIRS {
        assert_eq!(
            std::mem::discriminant(&parse(alias).command.unwrap()),
            std::mem::discriminant(&parse(canonical).command.unwrap()),
            "alias {alias} differs from canonical {canonical}"
        );
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

fn binary() -> Command {
    Command::cargo_bin("crsp").unwrap()
}

#[test]
fn carried_validation_and_noninteractive_contracts_are_observable() {
    binary()
        .args(["login", "--extra-scopes", "a,,b"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains(
            "must be a comma-separated list of non-empty scopes.",
        ));
    binary()
        .args(["login", "--redirect-port", "65536"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("should be >= 0 and <= 65535"));
    binary()
        .args(["run-function", "--params", "{"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("EOF while parsing an object"));
    binary()
        .args(["delete-script", "fixture"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    binary()
        .args(["pull", "--deleteUnusedFiles"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("Project settings not found."));
}
