use assert_cmd::Command;
use predicates::str::contains;
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use tempfile::tempdir;

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
        ("logout", &["--json"], 0, "\"success\": true"),
        (
            "show-authorized-user",
            &["--json"],
            0,
            "\"loggedIn\": false",
        ),
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
        ("start-mcp-server", &[], 0, ""),
    ];
    for (command, args, code, expected) in cases {
        if *command == "start-mcp-server" {
            continue;
        }
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
fn start_mcp_server_returns_initialize_response() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_crsp"))
        .arg("start-mcp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"surface\",\"version\":\"1\"}}}\n")
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["result"]["serverInfo"]["name"], "Crsp");
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn auth_surface_uses_command_specific_fixture_observables() {
    let directory = tempdir().unwrap();
    let auth = directory.path().join(".clasprc.json");
    std::fs::write(
        &auth,
        r#"{"tokens":{"default":{"access_token":"ACCESS-PLACEHOLDER"}}}"#,
    )
    .unwrap();
    let output = run_binary("logout", &["--auth", auth.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Deleted credentials."));
    let output = run_binary("show-authorized-user", &["--auth", auth.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Not logged in."));
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
