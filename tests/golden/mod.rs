//! Golden compatibility harness (spec §9.2, §9.1): pins crsp's observable
//! behavior against clasp v3.4.1 as files under `tests/golden/fixtures/`.
//!
//! Each case directory contains:
//!   - `fixture/`      — the project tree (`.clasp.json`, source files); a
//!     `_home/` subdirectory holds the isolated `HOME` (`.clasprc.json`).
//!   - `mock-transcript.toml` — wiremock request/response pairs to register.
//!   - `expected.json` — `{command, env, stdout, stderr, exit, files,
//!     requests}` snapshot.
//!
//! The runner copies the fixture to a tempdir, starts wiremock, registers the
//! transcript, points the real binary at it via `CRSP_*_BASE_URL` (plus an
//! isolated HOME), captures stdout/stderr/exit, normalizes (mock origin, temp
//! paths, push timestamps, and any known token/secret values), and compares
//! against the snapshot. Exact request bodies and the full request sequence
//! pin the HTTP contract (§9.2, §7.1 retry counts). The live-test gate is
//! separate (`live_gate` in the runner).
//!
//! SECURITY: every credential in the fixtures is a placeholder string; the
//! captured-output masker additionally scrubs the fixture's token/secret
//! values as defense in depth so a leaked real-looking secret can never be
//! committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A `mock-transcript.toml` entry (one wiremock mock).
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptEntry {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    /// Exact request-header matchers (e.g. the bearer authorization header).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    pub status: u16,
    /// Optional exact response body as JSON text. Omitted = empty body.
    #[serde(default)]
    pub body_json: Option<String>,
    /// Expected request count; wiremock fails the case when the final count
    /// differs (pins the §7.1 retry contract without wall-clock timing).
    #[serde(default = "default_times")]
    pub times: usize,
}

fn default_times() -> usize {
    1
}

/// `expected.json` — one asserted request observation.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedRequest {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Option<Value>,
}

/// One command run within a case (single `command` or `variants`).
#[derive(Debug, Clone, Deserialize)]
pub struct Variant {
    pub command: Vec<String>,
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub exit: i32,
}

/// `expected.json` — the desired snapshot.
#[derive(Debug, Clone, Deserialize)]
pub struct Expected {
    /// The binary arguments (argv after `crsp`). A literal `<TMP>` is
    /// replaced with the copied fixture's project directory. Ignored when
    /// `variants` is present.
    #[serde(default)]
    pub command: Vec<String>,
    /// Alternative: multiple command runs over the same fixture/transcript
    /// (each with its own stdout/stderr/exit).
    #[serde(default)]
    pub variants: Vec<Variant>,
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub exit: i32,
    /// Working directory relative to the copied fixture's project root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Extra environment for the child (merged over the harness defaults).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Snapshotted files: `project/<rel>` (project tree) and `home/<rel>`
    /// (isolated `$HOME`). Every file in the tree must be listed.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Optional exact full request sequence (method + path + exact body when
    /// given). When present, an empty array asserts ZERO requests; when
    /// absent the sequence is not asserted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests: Option<Vec<ExpectedRequest>>,
    /// Compare the request list as a multiset instead of an ordered
    /// sequence (clasp fires some requests concurrently — e.g. list-apis'
    /// `Promise.all` — so their relative order is nondeterministic).
    #[serde(default)]
    pub requests_unordered: bool,
    /// MCP protocol case: `[{"send": "...", "expect": {...}}]` JSON-RPC
    /// request/response pairs over the binary's stdin/stdout. `send` and
    /// `expect` may embed the literal `<TMP>` (replaced with the copied
    /// fixture's project directory).
    #[serde(default)]
    pub mcp_steps: Vec<McpStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpStep {
    pub send: String,
    pub expect: Value,
}

/// Normalized placeholders (documented in each case's expected data).
pub const MOCK_ORIGIN_TOKEN: &str = "<MOCK>";
pub const TMP_TOKEN: &str = "<TMP>";
pub const TIME_TOKEN: &str = "<TIME>";
pub const MASKED_TOKEN: &str = "<MASKED>";

#[derive(Clone)]
pub struct Case {
    pub name: String,
    pub dir: PathBuf,
    pub transcript: Vec<TranscriptEntry>,
    pub expected: Expected,
    /// Secret values gathered from the fixture `.clasprc.json` and the
    /// default OAuth client, used by the output masker.
    pub secrets: Vec<String>,
}

/// Top-level transcript document (`[[request]]` array).
#[derive(Debug, Clone, Deserialize)]
struct Transcript {
    request: Vec<TranscriptEntry>,
}

/// Loads every case directory under `tests/golden/fixtures`.
pub fn discover(fixtures_root: &Path) -> Vec<Case> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures_root)
        .expect("golden fixtures directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry
                .file_type()
                .ok()?
                .is_dir()
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let dir = fixtures_root.join(&name);
            let transcript: Vec<TranscriptEntry> = {
                let content = std::fs::read_to_string(dir.join("mock-transcript.toml"))
                    .unwrap_or_else(|error| panic!("{name}: mock-transcript.toml: {error}"));
                if content.trim().is_empty() {
                    Vec::new()
                } else {
                    let parsed: Transcript = toml::from_str(&content).unwrap_or_else(|error| {
                        panic!("{name}: parse mock-transcript.toml: {error}")
                    });
                    parsed.request
                }
            };
            let expected: Expected = serde_json::from_str(
                &std::fs::read_to_string(dir.join("expected.json"))
                    .unwrap_or_else(|error| panic!("{name}: expected.json: {error}")),
            )
            .unwrap_or_else(|error| panic!("{name}: parse expected.json: {error}"));
            let secrets = secret_values(&dir);
            Case {
                name,
                dir,
                transcript,
                expected,
                secrets,
            }
        })
        .collect()
}

/// Registers every transcript entry on the mock server. `path` is always
/// matched; the query map and optional body tighten the match. `times`
/// becomes the expected hit count so wiremock fails the case if the request
/// count diverges.
pub async fn register(server: &MockServer, transcript: &[TranscriptEntry]) {
    for entry in transcript {
        let mut mock = Mock::given(method(entry.method.as_str())).and(path(entry.path.as_str()));
        for (key, value) in &entry.query {
            mock = mock.and(query_param(key, value));
        }
        for (key, value) in &entry.headers {
            mock = mock.and(header(key, value));
        }
        if let Some(body) = &entry.body {
            let body: Value = serde_json::from_str(body)
                .unwrap_or_else(|error| panic!("transcript body {body}: {error}"));
            mock = mock.and(body_json(body));
        }
        let mut template = ResponseTemplate::new(entry.status);
        if let Some(body_json) = &entry.body_json {
            let body: Value = serde_json::from_str(body_json)
                .unwrap_or_else(|error| panic!("transcript body_json {body_json}: {error}"));
            template = template.set_body_json(body);
        }
        let mock = mock.respond_with(template);
        // `expect` requires exactly `times` hits so retry-pinned transcripts
        // fail on any divergence (spec §7.1 request counts).
        mock.expect(entry.times as u64).mount(server).await;
    }
}

/// Secret values used by the masker: the default OAuth client *secret* and
/// any token-like values in the fixture `.clasprc.json`. The client ID is not
/// secret (it is published by clasp); the client secret is masked as hygiene.
fn secret_values(dir: &Path) -> Vec<String> {
    let mut secrets = Vec::new();
    secrets.push(google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET.to_string());
    for key in [
        "access_token",
        "refresh_token",
        "id_token",
        "client_secret",
        "token",
    ] {
        if let Some(value) = clasprc_value(dir, key).filter(|value| value.len() >= 8) {
            secrets.push(value);
        }
    }
    secrets.push("ACCESS-PLACEHOLDER".to_string());
    secrets.push("REFRESH-PLACEHOLDER".to_string());
    secrets.push("TOKEN-PLACEHOLDER".to_string());
    secrets
}

fn clasprc_value(dir: &Path, key: &str) -> Option<String> {
    let path = dir.join("fixture/_home/.clasprc.json");
    let content = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    value
        .get("tokens")
        .and_then(|tokens| tokens.get("default"))
        .and_then(|tokens| tokens.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A captured run against a case.
pub struct RunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
    /// `project/<rel>` + `home/<rel>` → file content.
    pub files: BTreeMap<String, String>,
    /// Recorded request observations in order.
    pub requests: Vec<ExpectedRequest>,
}

/// Normalizes volatile output: the wiremock origin, the temp project/home
/// paths, the push `at h:mm:ss AM/PM` timestamp, and any known secret value.
pub fn normalize(
    text: &str,
    secrets: &[String],
    mock_origin: &str,
    project_dir: &str,
    home_dir: &str,
) -> String {
    let mut replaced = text.replace(mock_origin, MOCK_ORIGIN_TOKEN);
    // Replace paths in both raw and JSON-escaped form: inside serialized JSON a
    // Windows `\` appears as `\\`, so the raw path alone would not match.
    // Longer (project) paths go first so a project nested under $HOME is not
    // partially replaced by the home prefix.
    let project_escaped = project_dir.replace('\\', "\\\\");
    let home_escaped = home_dir.replace('\\', "\\\\");
    let mut replacements: Vec<(&str, &str)> = vec![
        (project_dir, TMP_TOKEN),
        (home_dir, TMP_TOKEN),
        (&project_escaped, TMP_TOKEN),
        (&home_escaped, TMP_TOKEN),
    ];
    replacements.sort_by_key(|(pattern, _)| std::cmp::Reverse(pattern.len()));
    for (pattern, token) in replacements {
        if !pattern.is_empty() {
            replaced = replaced.replace(pattern, token);
        }
    }
    for secret in secrets {
        replaced = replaced.replace(secret, MASKED_TOKEN);
    }
    // push.ts `toLocaleTimeString()` rendering (spec clause 5 of §4 notes the
    // time is locale/en-US style, 12-hour `h:mm:ss AM/PM`).
    let time_pattern = regex::Regex::new(r"at \d{1,2}:\d{2}:\d{2} (AM|PM)\.").expect("time regex");
    replaced = time_pattern
        .replace_all(&replaced, format!("at {TIME_TOKEN}."))
        .to_string();
    replaced
}

/// Compares the captured streams against expectations; returns a failure
/// report (empty = match).
pub fn compare_streams(
    stdout: &str,
    stderr: &str,
    exit: i32,
    expected_stdout: &str,
    expected_stderr: &str,
    expected_exit: i32,
) -> String {
    let mut failures = Vec::new();
    if stdout != expected_stdout {
        failures.push(format!(
            "stdout diverged:\n--- expected ---\n{}\n--- actual ---\n{}",
            expected_stdout, stdout
        ));
    }
    if stderr != expected_stderr {
        failures.push(format!(
            "stderr diverged:\n--- expected ---\n{}\n--- actual ---\n{}",
            expected_stderr, stderr
        ));
    }
    if exit != expected_exit {
        failures.push(format!(
            "exit diverged: expected {}, actual {}",
            expected_exit, exit
        ));
    }
    failures.join("\n\n")
}

/// Compares the captured run against the expected snapshot; a non-empty
/// return is the failure report.
pub fn compare(actual: &RunResult, expected: &Expected) -> String {
    let stream_failures = compare_streams(
        &actual.stdout,
        &actual.stderr,
        actual.exit,
        &expected.stdout,
        &expected.stderr,
        expected.exit,
    );
    let mut failures = Vec::new();
    if !stream_failures.is_empty() {
        failures.push(stream_failures);
    }
    compare_files(actual, expected, &mut failures);
    compare_requests(actual, expected, &mut failures);
    failures.join("\n\n")
}

fn compare_files(actual: &RunResult, expected: &Expected, failures: &mut Vec<String>) {
    let mut missing: Vec<&String> = expected
        .files
        .keys()
        .filter(|path| !actual.files.contains_key(*path))
        .collect();
    missing.sort();
    if !missing.is_empty() {
        failures.push(format!(
            "files missing:\n{}",
            missing
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let mut extra: Vec<&String> = actual
        .files
        .keys()
        .filter(|path| !expected.files.contains_key(*path))
        .collect();
    extra.sort();
    if !extra.is_empty() {
        failures.push(format!(
            "unexpected files:\n{}",
            extra
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let mut content_diff = Vec::new();
    for (path, expected_content) in &expected.files {
        let Some(actual_content) = actual.files.get(path) else {
            continue;
        };
        if actual_content != expected_content {
            content_diff.push(format!(
                "file {path} diverged:\n{}",
                textual_diff(expected_content, actual_content)
            ));
        }
    }
    if !content_diff.is_empty() {
        failures.push(content_diff.join("\n"));
    }
    // Show the actual file tree (with contents) whenever the snapshot does
    // not match, so a failure message pinpoints exactly what diverged.
    if !missing.is_empty() || !extra.is_empty() || !content_diff.is_empty() {
        let mut tree: Vec<(String, &String)> =
            actual.files.iter().map(|(k, v)| (k.clone(), v)).collect();
        tree.sort();
        failures.push(format!(
            "actual file tree:\n{}",
            tree.iter()
                .map(|(path, content)| format!("  {path}: {content:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
}

fn compare_requests(actual: &RunResult, expected: &Expected, failures: &mut Vec<String>) {
    let Some(expected_requests) = &expected.requests else {
        return;
    };
    let mut actual_requests = actual.requests.clone();
    if expected.requests_unordered {
        let sort_key = |request: &ExpectedRequest| {
            (
                request.method.clone(),
                request.path.clone(),
                request
                    .body
                    .as_ref()
                    .map(|body| serde_json::to_string(body).unwrap_or_default())
                    .unwrap_or_default(),
            )
        };
        actual_requests.sort_by_key(sort_key);
        let mut expected_sorted = expected_requests.clone();
        expected_sorted.sort_by_key(sort_key);
        if actual_requests.len() != expected_sorted.len() {
            failures.push(format!(
                "request count diverged: expected {}, actual {}",
                expected_sorted.len(),
                actual_requests.len()
            ));
            return;
        }
        for (index, (actual, expected)) in actual_requests.iter().zip(&expected_sorted).enumerate()
        {
            compare_request_entry(actual, expected, index, failures);
        }
        return;
    }
    if actual_requests.len() != expected_requests.len() {
        failures.push(format!(
            "request count diverged: expected {}, actual {}",
            expected_requests.len(),
            actual_requests.len()
        ));
        return;
    }
    for (index, (actual, expected)) in actual_requests.iter().zip(expected_requests).enumerate() {
        compare_request_entry(actual, expected, index, failures);
    }
}

fn compare_request_entry(
    actual: &ExpectedRequest,
    expected: &ExpectedRequest,
    index: usize,
    failures: &mut Vec<String>,
) {
    if actual.method != expected.method || actual.path != expected.path {
        failures.push(format!(
            "request[{index}] diverged: expected {} {}, actual {} {}",
            expected.method, expected.path, actual.method, actual.path
        ));
    }
    if let Some(expected_body) = &expected.body
        && actual.body.as_ref() != Some(expected_body)
    {
        failures.push(format!(
            "request[{index}] {} {} body diverged:\n  expected: {expected_body}\n  actual:   {}",
            actual.method,
            actual.path,
            actual.body.as_ref().unwrap_or(&Value::Null)
        ));
    }
}

/// Minimal line-based diff for failure messages.
fn textual_diff(expected: &str, actual: &str) -> String {
    let expected: Vec<&str> = expected.split('\n').collect();
    let actual: Vec<&str> = actual.split('\n').collect();
    let mut out = Vec::new();
    let max = expected.len().max(actual.len());
    for index in 0..max {
        let left = expected.get(index).copied().unwrap_or("");
        let right = actual.get(index).copied().unwrap_or("");
        if left != right {
            out.push(format!(
                "  line {index}: expected {left:?}  actual {right:?}"
            ));
        }
    }
    out.join("\n")
}

/// Recursively walks `root`, returning `relative path → content` maps for the
/// given keyspace prefix. `skip_home` drops the fixture's `_home/` subtree
/// from the project walk (it is moved to `$HOME`).
pub fn snapshot_tree(root: &Path, prefix: &str, skip: Option<&str>) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    walk(root, root, prefix, skip, &mut files);
    files
}

fn walk(
    root: &Path,
    current: &Path,
    prefix: &str,
    skip: Option<&str>,
    files: &mut BTreeMap<String, String>,
) {
    let mut entries: Vec<_> = std::fs::read_dir(current)
        .unwrap_or_else(|error| panic!("snapshot read_dir {current:?}: {error}"))
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if skip.is_some_and(|skip| entry.file_name() == skip) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, prefix, skip, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("inside root")
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("snapshot read {path:?}: {error}"));
            files.insert(format!("{prefix}{relative}"), content);
        }
    }
}
