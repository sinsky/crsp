use assert_cmd::Command;
use predicates::str::contains;

const ALIAS_PAIRS: &[(&str, &str, &[&str])] = &[
    ("clone", "clone-script", &[]),
    ("create", "create-script", &["--type", "invalid"]),
    ("deploy", "create-deployment", &[]),
    ("redeploy", "update-deployment", &["fixture"]),
    ("undeploy", "delete-deployment", &[]),
    ("delete", "delete-script", &[]),
    ("version", "create-version", &[]),
    ("versions", "list-versions", &[]),
    ("deployments", "list-deployments", &[]),
    ("list", "list-scripts", &["--noShorten"]),
    ("run", "run-function", &["--params", "{"]),
    ("logs", "tail-logs", &[]),
    ("status", "show-file-status", &[]),
    ("apis", "list-apis", &[]),
];

fn binary() -> Command {
    Command::cargo_bin("crsp").unwrap()
}

fn run_binary(name: &str, args: &[&str]) -> std::process::Output {
    binary().arg(name).args(args).output().unwrap()
}

#[test]
fn canonical_commands_have_command_specific_binary_outcomes() {
    let cases: &[(&str, &[&str], i32, &str)] = &[
        (
            "login",
            &["--creds", "/missing/client.json"],
            1,
            "No such file",
        ),
        ("logout", &[], 0, ""),
        ("show-authorized-user", &[], 0, ""),
        ("clone-script", &[], 1, "No script ID."),
        (
            "create-script",
            &["--type", "invalid"],
            1,
            "Invalid script type",
        ),
        ("push", &[], 1, "Project settings not found."),
        ("pull", &[], 1, "Project settings not found."),
        ("create-deployment", &[], 1, "Project settings not found."),
        (
            "update-deployment",
            &["fixture"],
            1,
            "Project settings not found.",
        ),
        ("delete-deployment", &[], 1, "Project settings not found."),
        ("delete-script", &[], 1, "Script ID not set"),
        ("create-version", &[], 1, "Project settings not found."),
        ("list-versions", &[], 1, "Project settings not found."),
        ("list-deployments", &[], 1, "Project settings not found."),
        (
            "run-function",
            &["--params", "{"],
            1,
            "EOF while parsing an object",
        ),
        ("tail-logs", &[], 1, "GCP project ID is not set"),
        ("setup-logs", &[], 1, "GCP project ID is not set"),
        ("show-file-status", &[], 0, "Tracked files:"),
        ("list-apis", &[], 1, "GCP project ID is not set"),
        ("enable-api", &["drive"], 1, "GCP project ID is not set"),
        ("disable-api", &["drive"], 1, "GCP project ID is not set"),
        ("open-script", &[], 1, "Script ID not set"),
        ("open-container", &[], 1, "Parent ID not set"),
        ("open-web-app", &[], 1, "Script ID not set"),
        ("open-logs", &[], 1, "GCP project ID is not set"),
        ("open-api-console", &[], 1, "GCP project ID is not set"),
        (
            "open-credentials-setup",
            &[],
            1,
            "GCP project ID is not set",
        ),
    ];
    for (command, args, code, expected) in cases {
        let output = run_binary(command, args);
        assert_eq!(output.status.code(), Some(*code), "{command}");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains(expected), "{command}: {text}");
    }
}

#[test]
fn aliases_match_canonical_binary_streams_and_exit_codes() {
    for (alias, canonical, args) in ALIAS_PAIRS {
        let alias_output = run_binary(alias, args);
        let canonical_output = run_binary(canonical, args);
        assert_eq!(
            alias_output.status.code(),
            canonical_output.status.code(),
            "{alias}"
        );
        assert_eq!(
            alias_output.stdout, canonical_output.stdout,
            "{alias} stdout"
        );
        assert_eq!(
            alias_output.stderr, canonical_output.stderr,
            "{alias} stderr"
        );
    }
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
