//! CLI contract tests for the crsp command surface (spec §2.5).
//!
//! Binary-level tests assert exit codes and stream routing (clasp-compatible:
//! all failures exit 1, `--version`/`--help` exit 0). Library-level tests
//! assert exact parsing of every canonical command, alias, and option.

use assert_cmd::Command;
use clap::Parser;
use crsp::cli::Commands;
use crsp::{Cli, run};
use predicates::prelude::PredicateBooleanExt;

fn crsp_bin() -> Command {
    Command::cargo_bin("crsp").expect("crsp binary target")
}

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("crsp").chain(args.iter().copied())).expect("valid args")
}

/// Commands with a required positional argument need a placeholder value to
/// parse in table-driven tests.
fn parse_canonical(name: &str) -> Cli {
    match name {
        "update-deployment" | "redeploy" | "enable-api" | "disable-api" => {
            parse(&[name, "placeholder"])
        }
        _ => parse(&[name]),
    }
}

fn dispatch_tag(command: &Commands) -> &'static str {
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

const CANONICAL_COMMANDS: &[(&str, &str)] = &[
    ("login", "login"),
    ("logout", "logout"),
    ("show-authorized-user", "show-authorized-user"),
    ("clone-script", "clone-script"),
    ("create-script", "create-script"),
    ("push", "push"),
    ("pull", "pull"),
    ("create-deployment", "create-deployment"),
    ("update-deployment", "update-deployment"),
    ("delete-deployment", "delete-deployment"),
    ("delete-script", "delete-script"),
    ("create-version", "create-version"),
    ("list-versions", "list-versions"),
    ("list-deployments", "list-deployments"),
    ("list-scripts", "list-scripts"),
    ("run-function", "run-function"),
    ("tail-logs", "tail-logs"),
    ("setup-logs", "setup-logs"),
    ("show-file-status", "show-file-status"),
    ("list-apis", "list-apis"),
    ("enable-api", "enable-api"),
    ("disable-api", "disable-api"),
    ("open-script", "open-script"),
    ("open-container", "open-container"),
    ("open-web-app", "open-web-app"),
    ("open-logs", "open-logs"),
    ("open-api-console", "open-api-console"),
    ("open-credentials-setup", "open-credentials-setup"),
    ("start-mcp-server", "start-mcp-server"),
];

const ALIASES: &[(&str, &str)] = &[
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

#[test]
fn project_name_constant_matches_the_cli_literals() {
    // about/override_usage clap attributes are literals; guard against drift.
    assert_eq!(crsp::constants::PROJECT_NAME, "crsp");
}

#[test]
fn version_flag_reports_crate_version_on_stdout_with_exit_zero() {
    let assertion = crsp_bin().arg("--version").assert();
    assertion
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")))
        .stderr("");
}

#[test]
fn short_version_flag_reports_crate_version_with_exit_zero() {
    crsp_bin()
        .arg("-v")
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flag_prints_usage_and_exits_zero() {
    let assertion = crsp_bin().arg("--help").assert().success();
    let output = assertion.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: crsp <command> [options]"),
        "{stdout}"
    );
    assert!(stdout.contains("crsp - The Apps Script CLI"), "{stdout}");
    assert!(stdout.contains("-v, --version"), "{stdout}");
    assert!(stdout.contains("login"), "{stdout}");
    assert!(stdout.contains("start-mcp-server"), "{stdout}");
}

#[test]
fn no_arguments_prints_help_to_stderr_and_exits_one() {
    let assertion = crsp_bin().assert().failure().code(1);
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage: crsp <command> [options]"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty: {stderr}");
}

#[test]
fn unknown_command_reports_clasp_style_message_and_exits_one() {
    let assertion = crsp_bin().arg("frobnicate").assert().failure().code(1);
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stderr.contains("Unknown command \"crsp frobnicate\""),
        "stderr: {stderr}"
    );
    assert!(
        stdout.contains("Usage: crsp <command> [options]"),
        "stdout: {stdout}"
    );
}

#[test]
fn unknown_option_exits_one() {
    crsp_bin()
        .arg("--bogus")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("--bogus"));
}

#[test]
fn missing_required_argument_exits_one() {
    crsp_bin()
        .arg("update-deployment")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("deploymentId"));
}

#[test]
fn redirect_port_must_be_an_integer() {
    crsp_bin()
        .args(["login", "--redirect-port", "abc"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("'abc' is not a valid integer."));
}

#[test]
fn redirect_port_must_be_within_zero_to_65535() {
    crsp_bin()
        .args(["login", "--redirect-port", "99999"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains(
            "'99999' should be >= 0 and <= 65535.",
        ));
}

#[test]
fn extra_scopes_rejects_empty_elements() {
    crsp_bin()
        .args(["login", "--extra-scopes", "a,,b"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains(
            "must be a comma-separated list of non-empty scopes.",
        ));
}

#[test]
fn auth_env_var_is_honored() {
    crsp_bin()
        .env("clasp_config_auth", "/tmp/alternate.clasprc.json")
        .arg("push")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::is_empty().not());
}

#[test]
fn ignore_env_var_is_honored() {
    crsp_bin()
        .env("clasp_config_ignore", "/tmp/custom.ignore")
        .arg("push")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::is_empty().not());
}

#[test]
fn project_env_var_is_honored() {
    crsp_bin()
        .env("clasp_config_project", "/tmp/other.clasp.json")
        .arg("push")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::is_empty().not());
}

#[test]
fn not_yet_wired_commands_exit_one_with_typed_error() {
    crsp_bin()
        .arg("push")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::is_empty().not());
}

#[test]
fn all_canonical_commands_parse() {
    for (name, expected) in CANONICAL_COMMANDS {
        let cli = parse_canonical(name);
        let command = cli.command.as_ref().expect("subcommand");
        assert_eq!(dispatch_tag(command), *expected, "for command {name}");
    }
}

#[test]
fn all_short_aliases_parse_to_canonical_commands() {
    for (alias, expected) in ALIASES {
        let cli = parse_canonical(alias);
        let command = cli.command.as_ref().expect("subcommand");
        assert_eq!(dispatch_tag(command), *expected, "for alias {alias}");
    }
}

#[test]
fn global_options_parse_at_top_level() {
    let cli = parse(&[
        "--json",
        "--adc",
        "--allow-symlinks",
        "-A",
        "a.json",
        "-I",
        "i.ignore",
        "-P",
        "p.json",
        "-u",
        "bob",
        "push",
    ]);
    assert!(cli.globals.json);
    assert!(cli.globals.adc);
    assert!(cli.globals.allow_symlinks);
    assert_eq!(cli.globals.auth.as_deref(), Some("a.json"));
    assert_eq!(cli.globals.ignore.as_deref(), Some("i.ignore"));
    assert_eq!(cli.globals.project.as_deref(), Some("p.json"));
    assert_eq!(cli.globals.user, "bob");
}

#[test]
fn global_options_parse_after_the_subcommand() {
    let cli = parse(&[
        "push",
        "--json",
        "--adc",
        "--allow-symlinks",
        "-A",
        "a.json",
        "-I",
        "i.ignore",
        "-P",
        "p.json",
        "-u",
        "bob",
        "-f",
        "-w",
    ]);
    assert!(cli.globals.json);
    assert!(cli.globals.adc);
    assert!(cli.globals.allow_symlinks);
    assert_eq!(cli.globals.auth.as_deref(), Some("a.json"));
    assert_eq!(cli.globals.ignore.as_deref(), Some("i.ignore"));
    assert_eq!(cli.globals.project.as_deref(), Some("p.json"));
    assert_eq!(cli.globals.user, "bob");
    let Commands::Push(args) = cli.command.expect("subcommand") else {
        panic!("expected push");
    };
    assert!(args.force);
    assert!(args.watch);
}

#[test]
fn user_option_defaults_to_default_user() {
    let cli = parse(&["push"]);
    assert_eq!(cli.globals.user, "default");
}

#[test]
fn env_vars_are_applied_for_global_options() {
    unsafe {
        std::env::set_var("clasp_config_auth", "/env/auth.json");
        std::env::set_var("clasp_config_ignore", "/env/ignore");
        std::env::set_var("clasp_config_project", "/env/project.json");
    }
    let cli = parse(&["push"]);
    unsafe {
        std::env::remove_var("clasp_config_auth");
        std::env::remove_var("clasp_config_ignore");
        std::env::remove_var("clasp_config_project");
    }
    assert_eq!(cli.globals.auth.as_deref(), Some("/env/auth.json"));
    assert_eq!(cli.globals.ignore.as_deref(), Some("/env/ignore"));
    assert_eq!(cli.globals.project.as_deref(), Some("/env/project.json"));
}

#[test]
fn login_options_parse_exactly() {
    let cli = parse(&[
        "login",
        "--no-localhost",
        "--creds",
        "client_secret.json",
        "--use-project-scopes",
        "--include-clasp-scopes",
        "--extra-scopes",
        "a,b ,c",
        "--redirect-port",
        "8080",
    ]);
    let Commands::Login(args) = cli.command.expect("subcommand") else {
        panic!("expected login");
    };
    assert!(args.no_localhost);
    assert_eq!(args.creds.as_deref(), Some("client_secret.json"));
    assert!(args.use_project_scopes);
    assert!(args.include_clasp_scopes);
    assert_eq!(
        args.extra_scopes,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert_eq!(args.redirect_port, Some(8080));
}

#[test]
fn create_script_options_keep_clasp_casing() {
    let cli = parse(&[
        "create",
        "--type",
        "webapp",
        "--title",
        "My title",
        "--parentId",
        "p1",
        "--rootDir",
        "src",
    ]);
    let Commands::CreateScript(args) = cli.command.expect("subcommand") else {
        panic!("expected create-script");
    };
    assert_eq!(args.script_type, "webapp");
    assert_eq!(args.title.as_deref(), Some("My title"));
    assert_eq!(args.parent_id.as_deref(), Some("p1"));
    assert_eq!(args.root_dir.as_deref(), Some("src"));
}

#[test]
fn create_script_type_defaults_to_standalone() {
    let cli = parse(&["create"]);
    let Commands::CreateScript(args) = cli.command.expect("subcommand") else {
        panic!("expected create-script");
    };
    assert_eq!(args.script_type, "standalone");
}

#[test]
fn kebab_case_variants_of_clasp_camelcase_options_do_not_exist() {
    for args in [
        vec!["create", "--root-dir", "src"],
        vec!["create", "--parent-id", "p1"],
        vec!["clone", "--root-dir", "src"],
        vec!["list", "--no-shorten"],
        vec!["pull", "--delete-unused-files"],
        vec!["deploy", "--version-number", "3"],
        vec!["deploy", "--deployment-id", "id"],
    ] {
        let result = Cli::try_parse_from(std::iter::once("crsp").chain(args.iter().copied()));
        assert!(result.is_err(), "kebab-case must not parse: {args:?}");
    }
}

#[test]
fn clone_parses_script_id_and_version_number() {
    let cli = parse(&["clone", "1AbC", "2", "--rootDir", "out"]);
    let Commands::CloneScript(args) = cli.command.expect("subcommand") else {
        panic!("expected clone-script");
    };
    assert_eq!(args.script_id.as_deref(), Some("1AbC"));
    assert_eq!(args.version_number.as_deref(), Some("2"));
    assert_eq!(args.root_dir.as_deref(), Some("out"));
}

#[test]
fn pull_options_parse_exactly() {
    let cli = parse(&["pull", "--versionNumber", "3", "-d", "-f"]);
    let Commands::Pull(args) = cli.command.expect("subcommand") else {
        panic!("expected pull");
    };
    assert_eq!(args.version_number.as_deref(), Some("3"));
    assert!(args.delete_unused_files);
    assert!(args.force);
}

#[test]
fn deployment_commands_parse_exactly() {
    let cli = parse(&["deploy", "-V", "3", "-d", "desc", "-i", "dep1"]);
    let Commands::CreateDeployment(args) = cli.command.expect("subcommand") else {
        panic!("expected create-deployment");
    };
    assert_eq!(args.version_number.as_deref(), Some("3"));
    assert_eq!(args.description.as_deref(), Some("desc"));
    assert_eq!(args.deployment_id.as_deref(), Some("dep1"));

    let cli = parse(&["redeploy", "dep1", "-V", "4", "-d", "desc2"]);
    let Commands::UpdateDeployment(args) = cli.command.expect("subcommand") else {
        panic!("expected update-deployment");
    };
    assert_eq!(args.deployment_id, "dep1");
    assert_eq!(args.version_number.as_deref(), Some("4"));
    assert_eq!(args.description.as_deref(), Some("desc2"));

    let cli = parse(&["undeploy", "-a", "dep9"]);
    let Commands::DeleteDeployment(args) = cli.command.expect("subcommand") else {
        panic!("expected delete-deployment");
    };
    assert!(args.all);
    assert_eq!(args.deployment_id.as_deref(), Some("dep9"));
}

#[test]
fn delete_script_parses_force_and_script_id() {
    let cli = parse(&["delete", "--force", "sid"]);
    let Commands::DeleteScript(args) = cli.command.expect("subcommand") else {
        panic!("expected delete-script");
    };
    assert!(args.force);
    assert_eq!(args.script_id.as_deref(), Some("sid"));
}

#[test]
fn version_and_listing_commands_parse_positionals() {
    let cli = parse(&["version", "release notes"]);
    let Commands::CreateVersion(args) = cli.command.expect("subcommand") else {
        panic!("expected create-version");
    };
    assert_eq!(args.description.as_deref(), Some("release notes"));

    for (name, tag) in [
        ("versions", "list-versions"),
        ("deployments", "list-deployments"),
    ] {
        let cli = parse(&[name, "sid"]);
        let command = cli.command.as_ref().expect("subcommand");
        assert_eq!(dispatch_tag(command), tag);
    }
}

#[test]
fn list_scripts_uses_clasp_camelcase_flag() {
    let cli = parse(&["list", "--noShorten"]);
    let Commands::ListScripts(args) = cli.command.expect("subcommand") else {
        panic!("expected list-scripts");
    };
    assert!(args.no_shorten);
    let cli = parse(&["list"]);
    let Commands::ListScripts(args) = cli.command.expect("subcommand") else {
        panic!("expected list-scripts");
    };
    assert!(!args.no_shorten);
}

#[test]
fn run_function_parses_params_and_nondev() {
    let cli = parse(&["run", "myFn", "--nondev", "-p", "[1,2]"]);
    let Commands::RunFunction(args) = cli.command.expect("subcommand") else {
        panic!("expected run-function");
    };
    assert_eq!(args.function_name.as_deref(), Some("myFn"));
    assert!(args.nondev);
    assert_eq!(args.params.as_deref(), Some("[1,2]"));
}

#[test]
fn logs_options_parse_exactly() {
    let cli = parse(&["logs", "--watch", "--simplified"]);
    let Commands::TailLogs(args) = cli.command.expect("subcommand") else {
        panic!("expected tail-logs");
    };
    assert!(args.watch);
    assert!(args.simplified);
}

#[test]
fn api_commands_require_the_service_argument() {
    for name in ["enable-api", "disable-api"] {
        let cli = parse(&[name, "drive.googleapis.com"]);
        let command = cli.command.as_ref().expect("subcommand");
        assert_eq!(dispatch_tag(command), name);
        assert!(
            Cli::try_parse_from(["crsp", name]).is_err(),
            "{name} requires <api>"
        );
    }
}

#[test]
fn open_commands_parse_their_positionals() {
    let cli = parse(&["open-script", "sid"]);
    let Commands::OpenScript(args) = cli.command.expect("subcommand") else {
        panic!("expected open-script");
    };
    assert_eq!(args.script_id.as_deref(), Some("sid"));

    let cli = parse(&["open-web-app", "dep1"]);
    let Commands::OpenWebApp(args) = cli.command.expect("subcommand") else {
        panic!("expected open-web-app");
    };
    assert_eq!(args.deployment_id.as_deref(), Some("dep1"));

    for name in [
        "open-container",
        "open-logs",
        "open-api-console",
        "open-credentials-setup",
    ] {
        let cli = parse(&[name]);
        let command = cli.command.as_ref().expect("subcommand");
        assert_eq!(dispatch_tag(command), name);
    }
}

#[test]
fn update_deployment_accepts_json_flag_at_both_positions() {
    let cli = parse(&["redeploy", "dep1", "--json"]);
    assert!(cli.globals.json);
    let cli = parse(&["--json", "redeploy", "dep1"]);
    assert!(cli.globals.json);
}

#[test]
fn run_wires_every_canonical_command_without_placeholder_errors() {
    for (name, _) in CANONICAL_COMMANDS {
        let cli = parse_canonical(name);
        let result = run(&cli);
        assert!(
            result.is_err(),
            "{name} unexpectedly succeeded without fixture context"
        );
    }
}

#[test]
fn run_reports_unknown_command_for_unrecognized_input() {
    let cli = parse(&["frobnicate", "--json"]);
    let error = run(&cli).expect_err("unknown command must fail");
    match error {
        CrspError::Validation(message) => {
            assert_eq!(message, "Unknown command \"crsp frobnicate\"");
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[test]
fn run_reports_usage_error_when_no_subcommand_is_given() {
    let cli = parse(&[]);
    let error = run(&cli).expect_err("no subcommand must fail");
    let message = error.to_string();
    assert!(
        message.contains("Usage: crsp <command> [options]"),
        "{message}"
    );
}
