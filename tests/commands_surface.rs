use assert_cmd::Command;
use predicates::str::contains;
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        ("show-file-status", &[], 1, "Project settings not found."),
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

#[tokio::test]
async fn show_authorized_user_human_output_matches_clasp_and_honors_base_url_overrides() {
    use serde_json::json;
    use tempfile::tempdir;
    let directory = tempdir().unwrap();
    let home = tempdir().unwrap();
    std::fs::write(
        home.path().join(".clasprc.json"),
        json!({"tokens": {"default": {
            "client_id": "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com",
            "client_secret": "v6V3fKV_zWU7iw1DrpO1rknX",
            "type": "authorized_user",
            "access_token": "ACCESS-PLACEHOLDER",
            "refresh_token": "REFRESH-PLACEHOLDER",
            "expiry_date": 99999999999999i64
        }}})
        .to_string(),
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/userinfo"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "u1", "email": "user@example.com"})),
        )
        .mount(&server)
        .await;

    let output = Command::cargo_bin("crsp")
        .unwrap()
        .arg("show-authorized-user")
        .current_dir(directory.path())
        .env("HOME", home.path())
        .env("CRSP_USERINFO_BASE_URL", server.uri())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "You are logged in as user@example.com.\nOAuth client ID: 1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com (google-provided).\n"
    );
    // The userinfo request was actually routed through the override.
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        1
    );
}

#[tokio::test]
async fn api_commands_fail_locally_without_credentials_like_clasp() {
    // clasp v3.4.1: google-auth-library `getRequestMetadataAsync` throws
    // before any HTTP request when the OAuth2 client has no credentials;
    // index.ts prints `error.message` on stderr with exit 1. crsp must match
    // (zero requests reach the API).
    use tempfile::TempDir;
    let directory = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let src = directory.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        directory.path().join(".clasp.json"),
        serde_json::json!({"scriptId": "script", "rootDir": "src"}).to_string(),
    )
    .unwrap();

    let server = MockServer::start().await;
    let output = Command::cargo_bin("crsp")
        .unwrap()
        .arg("push")
        .current_dir(directory.path())
        .env("HOME", home.path())
        .env("CRSP_API_BASE_URL", server.uri())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "No access, refresh token, API key or refresh handler callback is set.\n"
    );
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0,
        "the auth guard must fire before any request"
    );
}
