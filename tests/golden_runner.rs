//! Golden compatibility harness runner (spec §9.2): discovers the cases in
//! `tests/golden/fixtures/`, runs the real binary against a fresh wiremock
//! server per case, and compares normalized stdout/stderr/exit/file-tree/
//! request snapshots. Includes the Step 5 live-test gate
//! (`CRSP_LIVE_TEST=1` + credentials), skipped by default.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use golden::{
    Case, ExpectedRequest, RunResult, Variant, compare, discover, normalize, register,
    snapshot_tree,
};
use tempfile::TempDir;
use wiremock::MockServer;

mod golden;

const FIXTURES_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/fixtures");
const HOME_DIR: &str = "_home";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_golden_cases_pass() {
    let cases = discover(Path::new(FIXTURES_ROOT));
    assert!(!cases.is_empty(), "no golden cases discovered");
    // Each case owns an isolated temp dir and wiremock server, so they are
    // independent. Run them with bounded concurrency (a semaphore of
    // CONCURRENCY permits) while spawning each on its own task: the spawn keeps
    // a wiremock drop-verification panic (e.g. a mocked endpoint never hit)
    // contained to that case instead of aborting the whole run. `run_binary`
    // uses the async `tokio::process` so blocked subprocess waits never starve
    // the worker threads that drive the wiremock servers.
    const CONCURRENCY: usize = 4;
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let mut handles = Vec::with_capacity(cases.len());
    for case in cases {
        let name = case.name.clone();
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("golden concurrency permit");
        let handle = tokio::spawn(async move {
            let _permit = permit;
            run_case(&case).await
        });
        handles.push((name, handle));
    }
    let mut failures = Vec::new();
    for (name, handle) in handles {
        let result = handle
            .await
            .map_err(|error| format!("case crashed: {error}"))
            .and_then(|result| result);
        if let Err(report) = result {
            failures.push(format!("[{name}] {report}"));
        }
    }
    assert!(
        failures.is_empty(),
        "golden cases failed:\n\n{}",
        failures.join("\n---\n")
    );
}

#[tokio::test]
async fn live_gate_skips_without_flag_or_credentials() {
    // Step 5: real Google API tests only run when CRSP_LIVE_TEST=1 AND the
    // credentials are present (a valid-token `.clasprc.json` in $HOME plus a
    // script id in CRSP_TEST_SCRIPT_ID); otherwise this test is a no-op and
    // CI stays hermetic.
    if std::env::var("CRSP_LIVE_TEST").as_deref() != Ok("1") {
        return;
    }
    let Some(script_id) = std::env::var("CRSP_TEST_SCRIPT_ID").ok() else {
        return;
    };
    let Some(home) = home::home_dir() else {
        return;
    };
    let clasprc = home.join(".clasprc.json");
    let credentials = std::fs::read_to_string(&clasprc).unwrap_or_default();
    if !credentials.contains("\"access_token\"") {
        return;
    }
    // Read-only live check: list the deployments of the configured script.
    let output = Command::new(env!("CARGO_BIN_EXE_crsp"))
        .arg("deployments")
        .arg(&script_id)
        .output()
        .expect("live crsp run");
    assert!(
        output.status.success(),
        "live deployments check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Copies `fixture/` into `project` (dropping `_home/`) and `fixture/_home/`
/// into `home`.
fn stage_fixture(case_dir: &Path, project: &Path, home: &Path) {
    let fixture = case_dir.join("fixture");
    if fixture.exists() {
        copy_tree(&fixture, project, Some(HOME_DIR));
        let fixture_home = fixture.join(HOME_DIR);
        if fixture_home.exists() {
            copy_tree(&fixture_home, home, None);
        }
    }
}

fn copy_tree(from: &Path, to: &Path, skip: Option<&str>) {
    if !from.exists() {
        return;
    }
    std::fs::create_dir_all(to).unwrap();
    let mut entries: Vec<_> = std::fs::read_dir(from)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if skip.is_some_and(|skip| entry.file_name() == skip) {
            continue;
        }
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target, None);
        } else {
            std::fs::copy(&source, &target).unwrap();
        }
    }
}

async fn run_case(case: &Case) -> Result<(), String> {
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    stage_fixture(&case.dir, project.path(), home.path());

    let server = MockServer::start().await;
    register(&server, &case.transcript).await;

    if !case.expected.mcp_steps.is_empty() {
        // MCP projectDir is jailed to $HOME and cwd (spec §8). macOS temp dirs
        // resolve `/var` → `/private/var` in getcwd, so stage the project
        // INSIDE the isolated HOME to keep the jail check lexical-clean.
        let project = home.path().join(&case.name);
        stage_fixture(&case.dir, &project, home.path());
        return run_mcp(case, &server, &project, home.path()).await;
    }

    let variants: Vec<(String, Variant)> = if case.expected.variants.is_empty() {
        vec![(
            "default".to_string(),
            Variant {
                command: case.expected.command.clone(),
                stdout: case.expected.stdout.clone(),
                stderr: case.expected.stderr.clone(),
                exit: case.expected.exit,
            },
        )]
    } else {
        case.expected
            .variants
            .iter()
            .enumerate()
            .map(|(index, variant)| {
                let label = if variant.command.is_empty() {
                    format!("variant {index}")
                } else {
                    variant.command.join(" ")
                };
                (label, variant.clone())
            })
            .collect()
    };

    let mut failures = Vec::new();
    let mut last: Option<RunResult> = None;
    for (label, variant) in &variants {
        let output = run_binary(case, &server, project.path(), home.path(), &variant.command).await;
        let stdout = normalize(
            String::from_utf8_lossy(&output.stdout).as_ref(),
            &case.secrets,
            &server.uri(),
            &project.path().to_string_lossy(),
            &home.path().to_string_lossy(),
        );
        let stderr = normalize(
            String::from_utf8_lossy(&output.stderr).as_ref(),
            &case.secrets,
            &server.uri(),
            &project.path().to_string_lossy(),
            &home.path().to_string_lossy(),
        );
        let exit = output.status.code().unwrap_or(-1);
        let stream = golden::compare_streams(
            &stdout,
            &stderr,
            exit,
            &variant.stdout,
            &variant.stderr,
            variant.exit,
        );
        if !stream.is_empty() {
            failures.push(format!("[{label}]\n{stream}"));
        }
        last = Some(RunResult {
            stdout,
            stderr,
            exit,
            files: snapshot_tree(project.path(), "project/", None)
                .into_iter()
                .chain(home_files(home.path()))
                .collect(),
            requests: observations(&server).await,
        });
    }
    let Some(last) = last else {
        return Err("no runs executed".to_string());
    };
    let tree = compare(
        &golden::RunResult {
            stdout: String::new(),
            stderr: String::new(),
            exit: 0,
            files: last.files.clone(),
            requests: last.requests.clone(),
        },
        &golden::Expected {
            command: Vec::new(),
            variants: Vec::new(),
            stdout: String::new(),
            stderr: String::new(),
            exit: 0,
            cwd: None,
            env: BTreeMap::new(),
            files: case.expected.files.clone(),
            requests: case.expected.requests.clone(),
            requests_unordered: case.expected.requests_unordered,
            mcp_steps: Vec::new(),
        },
    );
    if !tree.is_empty() {
        failures.push(tree);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        let report = failures.join("\n");
        eprintln!("GOLDEN CASE [{}] FAILED:\n{}", case.name, report);
        Err(report)
    }
}

fn home_files(home: &Path) -> BTreeMap<String, String> {
    snapshot_tree(home, "home/", None)
}

async fn run_binary(
    case: &Case,
    server: &MockServer,
    project_dir: &Path,
    home: &Path,
    argv: &[String],
) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_crsp"));
    let project_text = project_dir.to_string_lossy().to_string();
    let args: Vec<String> = argv
        .iter()
        .map(|arg| arg.replace(golden::TMP_TOKEN, &project_text))
        .collect();
    if let Some(cwd) = &case.expected.cwd {
        command.current_dir(project_dir.join(cwd));
    } else {
        command.current_dir(project_dir);
    }
    command
        .args(&args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("TZ", "UTC")
        .env("CRSP_API_BASE_URL", server.uri())
        .env("CRSP_SCRIPT_BASE_URL", server.uri())
        .env("CRSP_DRIVE_BASE_URL", server.uri())
        .env("CRSP_SERVICE_USAGE_BASE_URL", server.uri())
        .env("CRSP_DISCOVERY_BASE_URL", server.uri())
        .env("CRSP_LOGGING_BASE_URL", server.uri())
        .env("CRSP_OAUTH2_BASE_URL", server.uri())
        .env("CRSP_USERINFO_BASE_URL", server.uri())
        .env_remove("CLASP_ENABLE_USER_HINTS");
    for (key, value) in &case.expected.env {
        command.env(key, value);
    }
    command.output().await.expect("golden child process")
}

async fn observations(server: &MockServer) -> Vec<ExpectedRequest> {
    let mut out = Vec::new();
    for request in server.received_requests().await.unwrap_or_default().iter() {
        let mut req = ExpectedRequest {
            method: request.method.as_str().to_string(),
            path: request.url.path().to_string(),
            body: None,
        };
        if !request.body.is_empty() {
            req.body = serde_json::from_slice(&request.body).ok();
        }
        out.push(req);
    }
    out
}

/// MCP protocol case: speaks JSON-RPC over stdio, one `<TMP>`-templated
/// request per step, asserting the exact response object. The wiremock
/// server still serves any API calls the tools make.
async fn run_mcp(
    case: &Case,
    server: &MockServer,
    project_dir: &Path,
    home: &Path,
) -> Result<(), String> {
    use std::io::{BufRead, Read};

    let mut command = Command::new(env!("CARGO_BIN_EXE_crsp"));
    command
        .arg("start-mcp-server")
        .current_dir(project_dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("TZ", "UTC")
        .env("CRSP_API_BASE_URL", server.uri())
        .env("CRSP_SCRIPT_BASE_URL", server.uri())
        .env("CRSP_DRIVE_BASE_URL", server.uri())
        .env("CRSP_SERVICE_USAGE_BASE_URL", server.uri())
        .env("CRSP_DISCOVERY_BASE_URL", server.uri())
        .env("CRSP_LOGGING_BASE_URL", server.uri())
        .env("CRSP_OAUTH2_BASE_URL", server.uri())
        .env("CRSP_USERINFO_BASE_URL", server.uri())
        .env_remove("CLASP_ENABLE_USER_HINTS");
    for (key, value) in &case.expected.env {
        command.env(key, value);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp server");
    let mut stdin = child.stdin.take().expect("mcp stdin");
    let stdout = child.stdout.take().expect("mcp stdout");
    let mut stderr = String::new();
    let mut stderr_reader = std::io::BufReader::new(child.stderr.take().expect("mcp stderr"));

    // Bounded reads (T9 pattern): a worker thread feeds response lines into a
    // channel; every step waits at most 5s so a stalled server fails the case
    // instead of hanging the suite.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = line_tx.send(Err(std::io::Error::other("mcp stdout closed (EOF)")));
                    break;
                }
                Ok(_) => {
                    if line_tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = line_tx.send(Err(error));
                    break;
                }
            }
        }
    });

    let mut failures = Vec::new();
    let project_text = project_dir.to_string_lossy().to_string();
    'steps: for step in &case.expected.mcp_steps {
        let send = step.send.replace(golden::TMP_TOKEN, &project_text);
        if let Err(error) = stdin.write_all(format!("{send}\n").as_bytes()) {
            failures.push(format!("mcp write failed: {error}"));
            break;
        }
        let line = match line_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(line)) => line,
            Ok(Err(error)) => {
                failures.push(format!("mcp read failed: {error}"));
                break 'steps;
            }
            Err(_) => {
                failures.push("mcp read timed out after 5s".to_string());
                break 'steps;
            }
        };
        let actual_raw: serde_json::Value =
            serde_json::from_str(line.trim_end()).unwrap_or_else(|error| {
                panic!(
                    "case {}: response line is not JSON ({error}): {:?}",
                    case.name,
                    line.trim_end()
                )
            });
        // Normalize volatile values (the project path, the wiremock origin,
        // and any secrets) on BOTH sides before comparing.
        let actual_text = normalize(
            serde_json::to_string(&actual_raw).unwrap().as_str(),
            &case.secrets,
            &server.uri(),
            &project_text,
            &home.to_string_lossy(),
        );
        let actual: serde_json::Value = serde_json::from_str(&actual_text).unwrap();
        let expected_text = normalize(
            serde_json::to_string(&step.expect).unwrap().as_str(),
            &case.secrets,
            &server.uri(),
            &project_text,
            &home.to_string_lossy(),
        );
        let expected: serde_json::Value = serde_json::from_str(&expected_text).unwrap();
        if actual != expected {
            failures.push(format!(
                "mcp step {send:?} diverged:\n  expected: {}\n  actual:   {}",
                serde_json::to_string(&expected).unwrap(),
                serde_json::to_string(&actual).unwrap()
            ));
            break 'steps;
        }
    }
    drop(stdin);
    let _ = stderr_reader.read_to_string(&mut stderr);
    let status = child.wait().expect("wait for mcp server");
    if !status.success() {
        failures.push(format!("mcp server exited {status}: {stderr}"));
    }
    // The expected files/request data of MCP cases is asserted like every
    // other case (the MCP project lives inside $HOME, so the home walk skips
    // the project subtree).
    let files = snapshot_tree(project_dir, "project/", None)
        .into_iter()
        .chain(snapshot_tree(
            home,
            "home/",
            Some(
                project_dir
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or_default(),
            ),
        ))
        .collect();
    let state = compare(
        &golden::RunResult {
            stdout: String::new(),
            stderr: String::new(),
            exit: 0,
            files,
            requests: observations(server).await,
        },
        &golden::Expected {
            command: Vec::new(),
            variants: Vec::new(),
            stdout: String::new(),
            stderr: String::new(),
            exit: 0,
            cwd: None,
            env: BTreeMap::new(),
            files: case.expected.files.clone(),
            requests: case.expected.requests.clone(),
            requests_unordered: case.expected.requests_unordered,
            mcp_steps: Vec::new(),
        },
    );
    if !state.is_empty() {
        failures.push(state);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        let report = failures.join("\n");
        eprintln!("GOLDEN CASE [{}] FAILED:\n{}", case.name, report);
        Err(report)
    }
}
