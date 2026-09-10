//! Integration tests for the run, API-management, logs, and open commands
//! (Task 7): run-function, list-apis, enable-api, disable-api, tail-logs,
//! setup-logs, and the open-* family. HTTP contracts are asserted against
//! wiremock (exact paths, query parameters, bodies, and request order);
//! output contracts follow clasp v3.4.1 (spec §2.5 rows 16-28, §5 #6,
//! §6.2 open-*/tail-logs rows).

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crsp::api::{ApiClient, ApiClientConfig, BaseUrls, SleepFn};
use crsp::commands::disable_api::disable_api;
use crsp::commands::enable_api::enable_api;
use crsp::commands::list_apis::list_apis;
use crsp::commands::run_function::{RunFunctionArgs, run_function};
use crsp::commands::setup_logs::setup_logs;
use crsp::commands::shared::UrlOpener;
use crsp::commands::tail_logs::{
    POLL_INTERVAL_MS, PollState, TailLogsArgs, format_local_time, local_utc_offset, tail_logs,
};
use crsp::core::config::ProjectConfig;
use crsp::error::CrspError;
use crsp::output::Output;
use crsp::ui::{
    PromptAdapter, PromptConfirm, PromptDialog, PromptInput, PromptMultiSelect, PromptSelect,
    PromptSpinner, Ui,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn api_client(base: &str) -> ApiClient {
    let refresh: crsp::api::RefreshFn =
        std::sync::Arc::new(|| Box::pin(async { Ok("token".to_string()) }));
    ApiClient::with_base_urls(
        ApiClientConfig::new("token", refresh),
        BaseUrls {
            script: base.to_string(),
            drive: base.to_string(),
            service_usage: base.to_string(),
            discovery: base.to_string(),
            logging: base.to_string(),
            oauth2: base.to_string(),
            userinfo: base.to_string(),
        },
    )
    .unwrap()
}

/// JS `String.prototype.padEnd` on character counts (ASCII fixtures).
fn pad(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}

/// A project configuration with a configured script id (`.clasp.json` in the
/// temp root).
fn configured_config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        config_file_path: root.join(".clasp.json"),
        project_root_dir: root.to_path_buf(),
        content_dir: root.to_path_buf(),
        script_id: Some("script".to_string()),
        project_id: None,
        parent_id: None,
        file_push_order: Vec::new(),
        script_extensions: vec![".js".to_string(), ".gs".to_string()],
        html_extensions: vec![".html".to_string()],
        json_extensions: vec![".json".to_string()],
        skip_subdirectories: false,
        allow_symlinks: false,
        ignore_file_path: None,
    }
}

/// A project configuration with both ids configured.
fn gcp_config(root: &Path, project_id: &str) -> ProjectConfig {
    let mut config = configured_config(root);
    config.project_id = Some(project_id.to_string());
    config
}

#[derive(Default)]
struct TestPrompt {
    interactive: bool,
    input_answer: String,
    select_answer: String,
    select_calls: RefCell<Vec<PromptSelect>>,
}

impl TestPrompt {
    fn interactive() -> Self {
        Self {
            interactive: true,
            ..Self::default()
        }
    }
}

impl PromptAdapter for TestPrompt {
    fn is_interactive(&self) -> bool {
        self.interactive
    }
    fn input(&self, _: &PromptInput) -> io::Result<String> {
        Ok(self.input_answer.clone())
    }
    fn select(&self, spec: &PromptSelect) -> io::Result<String> {
        self.select_calls.borrow_mut().push(spec.clone());
        Ok(self.select_answer.clone())
    }
    fn multi_select(&self, _: &PromptMultiSelect) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn confirm(&self, _: &PromptConfirm) -> io::Result<bool> {
        Ok(false)
    }
    fn dialog(&self, _: &PromptDialog) -> io::Result<String> {
        Ok(String::new())
    }
    fn spinner<T, F: FnOnce() -> T>(&self, _: PromptSpinner, f: F) -> io::Result<T> {
        Ok(f())
    }
}

/// A browser launcher that records opened URLs without spawning a process
/// (clasp `open` behind a testable seam).
#[derive(Default, Clone)]
struct RecordingOpener {
    opened: Rc<RefCell<Vec<String>>>,
}

impl UrlOpener for RecordingOpener {
    fn open(&self, url: &str) -> Result<(), CrspError> {
        self.opened.borrow_mut().push(url.to_string());
        Ok(())
    }
}

/// A browser launcher that always fails to spawn (clasp `open` failure path).
struct FailingOpener;

impl UrlOpener for FailingOpener {
    fn open(&self, _: &str) -> Result<(), CrspError> {
        Err(CrspError::Io(io::Error::other("xdg-open not found")))
    }
}

/// Sets `CLASP_ENABLE_USER_HINTS` for the guard's lifetime, unsetting it on
/// drop (clasp `experiments.ts` reads the variable per process).
#[allow(dead_code)]
struct HintsEnvGuard(MutexGuard<'static, ()>);

fn hints_env(value: Option<&str>) -> HintsEnvGuard {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Safety: serialized behind LOCK; tests are the only writers.
    unsafe {
        match value {
            Some(value) => std::env::set_var("CLASP_ENABLE_USER_HINTS", value),
            None => std::env::remove_var("CLASP_ENABLE_USER_HINTS"),
        }
    }
    HintsEnvGuard(guard)
}

impl Drop for HintsEnvGuard {
    fn drop(&mut self) {
        // Safety: serialized behind LOCK; tests are the only writers.
        unsafe { std::env::remove_var("CLASP_ENABLE_USER_HINTS") }
    }
}

async fn received(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn url_of(request: &Request) -> &str {
    request.url.as_str()
}

async fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn stdout_of(out: &[u8]) -> String {
    String::from_utf8(out.to_vec()).unwrap()
}

fn stderr_of(err: &[u8]) -> String {
    String::from_utf8(err.to_vec()).unwrap()
}

/// Mounts the userinfo endpoint returning a Google-style id/email.
async fn mount_userinfo(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v2/userinfo"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "1234567890", "email": "user@example.com"})),
        )
        .mount(server)
        .await;
}

/// Mounts the userinfo endpoint failing with a non-retryable status (clasp
/// `authorizedUser` catches any error).
async fn mount_userinfo_error(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v2/userinfo"))
        .respond_with(ResponseTemplate::new(400))
        .mount(server)
        .await;
}

// ---------------------------------------------------------------------------
// run-function
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_function_posts_params_and_prints_json_result() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .and(body_json(json!({
            "function": "myFunction",
            "parameters": [1, "a"],
            "devMode": true
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"response": {"result": "mock result"}, "done": true})),
        )
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("myFunction"),
            nondev: false,
            params: Some(r#"[1, "a"]"#),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_of(&requests[0]).await,
        json!({"function": "myFunction", "parameters": [1, "a"], "devMode": true})
    );
    assert_eq!(stdout_of(&out), "{\n  \"response\": \"mock result\"\n}\n");
    assert_eq!(stderr_of(&err), "");
}

#[tokio::test]
async fn run_function_nondev_defaults_empty_params() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .and(body_json(json!({
            "function": "f",
            "parameters": [],
            "devMode": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: true,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // No response and no error keys: both omitted by JSON.stringify.
    assert_eq!(stdout_of(&out), "{}\n");
}

#[tokio::test]
async fn run_function_forwards_non_array_and_null_params_verbatim() {
    let server = MockServer::start().await;
    for body in [
        json!({"function": "f", "parameters": 123, "devMode": true}),
        json!({"function": "f", "parameters": [], "devMode": true}),
        json!({"function": "f", "parameters": {"x": 1}, "devMode": true}),
    ] {
        Mock::given(method("POST"))
            .and(path("/v1/scripts/script/run"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
    }
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let ui = Ui::new(TestPrompt::default());
    for params in ["123", "null", "{\"x\": 1}"] {
        let mut out = Vec::new();
        let mut output = Output::new(false, &mut out, Vec::new());
        run_function(
            &client,
            &config,
            RunFunctionArgs {
                function_name: Some("f"),
                nondev: false,
                params: Some(params),
            },
            &ui,
            &mut output,
        )
        .await
        .unwrap();
    }
    let requests = received(&server).await;
    let mut bodies = Vec::new();
    for request in &requests {
        bodies.push(body_of(request).await);
    }
    assert_eq!(bodies[0]["parameters"], json!(123));
    // JSON.parse("null") is nullish, so `parameters ?? []` sends [].
    assert_eq!(bodies[1]["parameters"], json!([]));
    assert_eq!(bodies[2]["parameters"], json!({"x": 1}));
}

#[tokio::test]
async fn run_function_params_syntax_error_fails() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: Some("{bad"),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    // Pinned divergence (parked audit item 8): clasp surfaces V8's
    // `SyntaxError: Unexpected token 'b', "{bad" is not valid JSON.`; crsp
    // forwards serde_json's message verbatim. Accepted wrapper divergence.
    match error {
        CrspError::Validation(message) => {
            assert_eq!(message, "key must be a string at line 1 column 2")
        }
        other => panic!("expected Validation error, got {other:?}"),
    }
    assert!(received(&server).await.is_empty(), "no request is sent");
}

#[tokio::test]
async fn run_function_empty_params_string_is_treated_as_absent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .and(body_json(
            json!({"function": "f", "parameters": [], "devMode": true}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: Some(""),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(received(&server).await.len(), 1);
}

#[tokio::test]
async fn run_function_not_authorized_maps_to_deploy_notice() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error": {"code": 403, "message": "denied"}})),
        )
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains(
            "Unable to run script function. Please make sure you have permission to run the script function.",
        ),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn run_function_not_found_maps_to_api_executable_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"error": {"code": 404, "message": "nope"}})),
        )
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains(
            "Script function not found. Please make sure script is deployed as API executable.",
        ),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn run_function_interactive_selects_from_content_functions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                {"name": "Code", "type": "SERVER_JS", "functionSet": {"values": [
                    {"name": "alpha"}, {"name": "beta"}
                ]}}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .and(body_json(
            json!({"function": "beta", "parameters": [], "devMode": true}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"response": {"result": 42}})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut prompt = TestPrompt::interactive();
    prompt.select_answer = "beta".to_string();
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: None,
            nondev: false,
            params: None,
        },
        &Ui::new(prompt),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "42\n");
}

#[tokio::test]
async fn run_function_noninteractive_without_name_posts_without_function() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .and(body_json(json!({"parameters": [], "devMode": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: None,
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    let body = body_of(&requests[0]).await;
    assert!(
        body.get("function").is_none(),
        "clasp drops the undefined function key: {body}"
    );
}

#[tokio::test]
async fn run_function_exception_line_goes_to_stderr_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": {
                "code": 3,
                "message": "script error",
                "details": [{"errorMessage": "boom", "scriptStackTraceElements": ["Code:3"]}]
            }
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp: `console.error('Exception:', errorMessage, stack || [])` then a
    // normal return (exit 0). The stack array renders like util.inspect.
    assert_eq!(stderr_of(&err), "Exception: boom [ 'Code:3' ]\n");
    assert_eq!(stdout_of(&out), "");
}

#[tokio::test]
async fn run_function_no_response_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"done": true})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "No response.\n");
}

#[tokio::test]
async fn run_function_json_error_shape_omits_response_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/script/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": {"code": 3, "message": "script error", "details": [{"errorMessage": "boom"}]}
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    run_function(
        &client,
        &config,
        RunFunctionArgs {
            function_name: Some("f"),
            nondev: false,
            params: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "{\n  \"error\": {\n    \"code\": 3,\n    \"message\": \"script error\",\n    \"details\": [\n      {\n        \"errorMessage\": \"boom\"\n      }\n    ]\n  }\n}\n"
    );
}

// ---------------------------------------------------------------------------
// list-apis
// ---------------------------------------------------------------------------

/// Mounts the Service Usage list and the Discovery list with a mix of
/// advanced services and unrelated services. Both lists arrive in API order.
async fn mount_apis(server: &MockServer, project_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/projects/{project_id}/services")))
        .and(query_param("filter", "state:ENABLED"))
        .and(query_param("pageSize", "200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "services": [
                {"name": "projects/p/services/sheets.googleapis.com", "config": {
                    "name": "sheets.googleapis.com",
                    "documentation": {"summary": "Sheets API summary"}
                }},
                {"name": "projects/p/services/storage.googleapis.com", "config": {
                    "name": "storage.googleapis.com",
                    "documentation": {"summary": "Not an advanced service"}
                }},
                {"name": "projects/p/services/gmail.googleapis.com", "config": {
                    "name": "gmail.googleapis.com",
                    "documentation": {"summary": "Gmail API summary"}
                }}
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/discovery/v1/apis"))
        .and(query_param("preferred", "true"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "items": [
                    {"id": "sheets.googleapis.com", "name": "sheets", "description": "Sheets discovery"},
                    {"id": "docs.googleapis.com", "name": "docs", "description": "Docs discovery"},
                    {"id": "storage.googleapis.com", "name": "storage", "description": "Not advanced"},
                    {"id": "calendar-json.googleapis.com", "name": "calendar"}
                ]
            })),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_apis_filters_by_advanced_services_and_prints_json() {
    let server = MockServer::start().await;
    mount_apis(&server, "proj").await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    list_apis(
        &client,
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "{\n  \"enabledApis\": [\n    {\n      \"name\": \"sheets\",\n      \"description\": \"Sheets API summary\"\n    },\n    {\n      \"name\": \"gmail\",\n      \"description\": \"Gmail API summary\"\n    }\n  ],\n  \"availableApis\": [\n    {\n      \"name\": \"docs\",\n      \"description\": \"Docs discovery\"\n    },\n    {\n      \"name\": \"sheets\",\n      \"description\": \"Sheets discovery\"\n    }\n  ]\n}\n"
    );
}

#[tokio::test]
async fn list_apis_prints_padded_sections() {
    let server = MockServer::start().await;
    mount_apis(&server, "proj").await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_apis(
        &client,
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let stdout = stdout_of(&out);
    let expected_sheets = format!("{} - {}", pad("sheets", 25), pad("Sheets API summary", 60));
    let expected_docs = format!("{} - {}", pad("docs", 25), pad("Docs discovery", 60));
    assert!(
        stdout.contains("\n# Currently enabled APIs:\n"),
        "leading blank line + label expected: {stdout:?}"
    );
    assert!(
        stdout.contains(&expected_sheets),
        "padded row expected: {stdout:?}"
    );
    assert!(stdout.contains("\n# List of available APIs:\n"));
    assert!(stdout.contains(&expected_docs));
    assert!(
        !stdout.contains("storage"),
        "non-advanced services are filtered: {stdout}"
    );
}

#[tokio::test]
async fn list_apis_requires_project_id_when_noninteractive() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = list_apis(
        &client,
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GCP project ID is not set, unable to continue."
    );
    assert_eq!(received(&server).await.len(), 0);
}

#[tokio::test]
async fn list_apis_requires_script_id_when_project_id_set() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    config.script_id = None;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = list_apis(
        &client,
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    // clasp's core `getEnabledServices` asserts script configuration first.
    assert_eq!(error.to_string(), "Project settings not found.");
}

#[tokio::test]
async fn list_apis_prompts_and_saves_project_id_interactively() {
    let server = MockServer::start().await;
    mount_apis(&server, "proj-from-prompt").await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    let mut prompt = TestPrompt::interactive();
    prompt.input_answer = "proj-from-prompt".to_string();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let opener = RecordingOpener::default();
    list_apis(&client, &mut config, &Ui::new(prompt), &opener, &mut output)
        .await
        .unwrap();
    // clasp maybePromptForProjectId: instructions, then openUrl(settings).
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains(
            "The script is not bound to a GCP project. To view or configure the GCP project for this\n      script, open https://script.google.com/home/projects/script/settings in your browser and follow instructions for setting up a GCP project. If a project is already\n      configured, open the GCP project to get the project ID value."
        ),
        "instructions expected: {stdout:?}"
    );
    assert!(
        stdout.contains(
            "Opening https://script.google.com/home/projects/script/settings in your browser."
        ),
        "opening line expected (interactive adapter)"
    );
    let saved = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["projectId"], "proj-from-prompt");
    // The API calls target the saved project id.
    let requests = received(&server).await;
    assert!(
        url_of(&requests[0]).contains("/v1/projects/proj-from-prompt/services"),
        "service usage uses the prompted id: {}",
        url_of(&requests[0])
    );
}

// ---------------------------------------------------------------------------
// enable-api / disable-api
// ---------------------------------------------------------------------------

fn manifest_body(dependencies: Value) -> Value {
    json!({
        "timeZone": "America/New_York",
        "dependencies": dependencies,
        "exceptionLogging": "STACKDRIVER",
        "runtimeVersion": "V8"
    })
}

async fn write_manifest(root: &Path, dependencies: Value) {
    let manifest = manifest_body(dependencies);
    fs::write(
        root.join("appsscript.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

async fn mount_enable(server: &MockServer, project_id: &str, api: &str, status: u16) {
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/projects/{project_id}/services/{api}.googleapis.com:enable"
        )))
        .respond_with(ResponseTemplate::new(status).set_body_json(json!({})))
        .mount(server)
        .await;
}

async fn mount_disable(server: &MockServer, project_id: &str, api: &str, status: u16) {
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1/projects/{project_id}/services/{api}.googleapis.com:disable"
        )))
        .respond_with(ResponseTemplate::new(status).set_body_json(json!({})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn enable_api_appends_entry_and_calls_service_usage() {
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(
        temp.path(),
        json!({"enabledAdvancedServices": [
            {"userSymbol": "Drive", "version": "v3", "serviceId": "drive"}
        ]}),
    )
    .await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "Enabled sheets API.\n");
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    // Key order of the manifest and of the appended entry is preserved.
    assert_eq!(
        saved,
        "{\n  \"timeZone\": \"America/New_York\",\n  \"dependencies\": {\n    \"enabledAdvancedServices\": [\n      {\n        \"userSymbol\": \"Drive\",\n        \"version\": \"v3\",\n        \"serviceId\": \"drive\"\n      },\n      {\n        \"userSymbol\": \"Sheets\",\n        \"version\": \"v4\",\n        \"serviceId\": \"sheets\"\n      }\n    ]\n  },\n  \"exceptionLogging\": \"STACKDRIVER\",\n  \"runtimeVersion\": \"V8\"\n}"
    );
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(
        url_of(&requests[0]).ends_with("/v1/projects/proj/services/sheets.googleapis.com:enable")
    );
}

#[tokio::test]
async fn enable_api_json_prints_success() {
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), json!({})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "{\n  \"success\": true\n}\n");
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        saved["dependencies"]["enabledAdvancedServices"],
        json!([{"userSymbol": "Sheets", "version": "v4", "serviceId": "sheets"}])
    );
}

#[tokio::test]
async fn enable_api_skips_duplicates_by_user_symbol() {
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "drive", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(
        temp.path(),
        json!({"enabledAdvancedServices": [
            {"userSymbol": "Drive", "version": "v3", "serviceId": "drive"}
        ]}),
    )
    .await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    enable_api(
        &client,
        &mut config,
        "drive",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    assert_eq!(
        saved.matches("\"userSymbol\"").count(),
        1,
        "no duplicate entry: {saved}"
    );
}

#[tokio::test]
async fn enable_api_rejects_unknown_service() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), json!({})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = enable_api(
        &client,
        &mut config,
        "notaservice",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Service is not a valid advanced service."
    );
    assert_eq!(received(&server).await.len(), 0);
}

#[tokio::test]
async fn enable_api_requires_manifest_file() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Manifest file does not exist.");
}

#[tokio::test]
async fn enable_api_not_authorized_keeps_manifest_update() {
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "sheets", 403).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), json!({})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Not authorized to enable sheets or it does not exist."
    );
    // clasp updates the manifest before the Service Usage call, so the entry
    // survives even when the API call fails.
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    assert!(saved.contains("\"serviceId\": \"sheets\""));
}

#[tokio::test]
async fn disable_api_removes_entry_and_calls_service_usage() {
    let server = MockServer::start().await;
    mount_disable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(
        temp.path(),
        json!({"enabledAdvancedServices": [
            {"userSymbol": "Drive", "version": "v3", "serviceId": "drive"},
            {"userSymbol": "Sheets", "version": "v4", "serviceId": "sheets"}
        ]}),
    )
    .await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    disable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "{\n  \"success\": true,\n  \"disabledService\": \"sheets\"\n}\n"
    );
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    assert_eq!(
        saved,
        "{\n  \"timeZone\": \"America/New_York\",\n  \"dependencies\": {\n    \"enabledAdvancedServices\": [\n      {\n        \"userSymbol\": \"Drive\",\n        \"version\": \"v3\",\n        \"serviceId\": \"drive\"\n      }\n    ]\n  },\n  \"exceptionLogging\": \"STACKDRIVER\",\n  \"runtimeVersion\": \"V8\"\n}"
    );
    let requests = received(&server).await;
    assert!(
        url_of(&requests[0]).ends_with("/v1/projects/proj/services/sheets.googleapis.com:disable")
    );
}

#[tokio::test]
async fn disable_api_keeps_manifest_when_dependencies_missing() {
    let server = MockServer::start().await;
    mount_disable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let manifest_before = serde_json::to_string_pretty(&manifest_body(json!({}))).unwrap();
    write_manifest(temp.path(), json!({})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    disable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "Disabled sheets API.\n");
    // clasp skips the manifest write when the structure is missing.
    let saved = fs::read_to_string(temp.path().join("appsscript.json")).unwrap();
    assert_eq!(saved, manifest_before);
    // But the Service Usage disable is still attempted.
    assert_eq!(received(&server).await.len(), 1);
}

#[tokio::test]
async fn disable_api_requires_manifest_file() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = disable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Manifest file does not exist.");
}

// ---------------------------------------------------------------------------
// tail-logs
// ---------------------------------------------------------------------------

fn log_entry(
    insert_id: &str,
    severity: &str,
    timestamp: &str,
    function_name: Option<&str>,
) -> Value {
    let mut entry = json!({
        "insertId": insert_id,
        "severity": severity,
        "timestamp": timestamp,
        "resource": {"labels": {}}
    });
    if let Some(name) = function_name {
        entry["resource"]["labels"]["function_name"] = json!(name);
    }
    entry
}

fn text_entry(
    insert_id: &str,
    severity: &str,
    timestamp: &str,
    text: &str,
    function_name: Option<&str>,
) -> Value {
    let mut entry = log_entry(insert_id, severity, timestamp, function_name);
    entry["textPayload"] = json!(text);
    entry
}

/// Mounts the entries:list endpoint with clasp's `timestamp desc` order
/// (newest first) and no page token.
async fn mount_logs_desc(server: &MockServer, entries: Value) {
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": entries})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn tail_logs_formats_text_and_json_payloads_oldest_first() {
    let server = MockServer::start().await;
    let tz = local_utc_offset();
    mount_logs_desc(
        &server,
        json!([
            // Newest first (orderBy: timestamp desc).
            text_entry("i3", "DEBUG", "2026-01-01T02:00:00Z", "{\"a\":1}", None),
            text_entry(
                "i2",
                "ERROR",
                "2026-01-01T01:00:00Z",
                "boom",
                Some("doStuff")
            ),
            text_entry("i1", "INFO", "2026-01-01T00:00:00Z", "hello", Some("greet")),
        ]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let stdout = stdout_of(&out);
    // Oldest first (clasp `results.reverse()`), severity/function padded,
    // local time rendering.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "three formatted lines: {stdout:?}");
    assert_eq!(
        lines[0],
        format!(
            "{} {} {} {}",
            pad("INFO", 20),
            format_local_time("2026-01-01T00:00:00Z", tz).unwrap(),
            pad("greet", 15),
            pad("hello", 20)
        )
    );
    assert_eq!(
        lines[1],
        format!(
            "{} {} {} {}",
            pad("ERROR", 20),
            format_local_time("2026-01-01T01:00:00Z", tz).unwrap(),
            pad("doStuff", 15),
            pad("boom", 20)
        )
    );
    // The DEBUG entry has a JSON string as text payload here (compact).
    assert_eq!(
        lines[2],
        format!(
            "{} {} {} {}",
            pad("DEBUG", 20),
            format_local_time("2026-01-01T02:00:00Z", tz).unwrap(),
            pad("N/A", 15),
            pad("{\"a\":1}", 20)
        )
    );
    assert!(
        !stdout.contains("PAST"),
        "the debug line is removed (spec §5 #6): {stdout:?}"
    );
}

#[tokio::test]
async fn tail_logs_first_request_has_empty_filter_and_dedupes() {
    let server = MockServer::start().await;
    // Page 1 (desc order): newest first, with a page token.
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(json!({
            "resourceNames": ["projects/proj"],
            "filter": "",
            "orderBy": "timestamp desc",
            "pageSize": 100
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [
                text_entry("only", "INFO", "2026-01-01T00:00:01Z", "keep", Some("f")),
                text_entry("dup", "INFO", "2026-01-01T00:00:00Z", "first", Some("f"))
            ],
            "nextPageToken": "t1"
        })))
        .mount(&server)
        .await;
    // Page 2 (same poll): the filter is fixed per getLogEntries call (clasp
    // fetchWithPages closes over it); only the page token changes.
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(json!({
            "resourceNames": ["projects/proj"],
            "filter": "",
            "orderBy": "timestamp desc",
            "pageSize": 100,
            "pageToken": "t1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [text_entry("dup", "INFO", "2026-01-01T00:00:00Z", "first", Some("f"))]
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let stdout = stdout_of(&out);
    // Oldest first; the duplicate insertId prints exactly once even across
    // pagination pages.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "dedupe keeps one line per insertId: {stdout:?}"
    );
    assert!(lines[0].contains("first"), "oldest first: {stdout:?}");
    assert!(lines[1].contains("keep"));
}

#[tokio::test]
async fn tail_logs_watch_sleeps_interval_and_dedupes_across_polls() {
    let server = MockServer::start().await;
    // Poll 1 (empty filter): one entry.
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(json!({
            "resourceNames": ["projects/proj"],
            "filter": "",
            "orderBy": "timestamp desc",
            "pageSize": 100
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [text_entry("id-1", "INFO", "2026-01-01T00:00:00Z", "x", Some("f"))]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Poll 2 (since filter from poll 1): the same insertId again.
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(json!({
            "resourceNames": ["projects/proj"],
            "filter": "timestamp >= \"2026-01-01T00:00:00.000Z\"",
            "orderBy": "timestamp desc",
            "pageSize": 100
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [text_entry("id-1", "INFO", "2026-01-01T00:00:00Z", "x", Some("f"))]
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());

    let durations: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = durations.clone();
    let sleep: SleepFn = Arc::new(move |duration| {
        recorded.lock().unwrap().push(duration);
        Box::pin(async {})
    });
    let polls = RefCell::new(0);
    let mut state = PollState::default();
    let mut poller = crsp::commands::tail_logs::LogPoller::new(
        &client,
        "proj",
        false,
        false,
        &mut state,
        &mut output,
    );
    poller
        .watch(Duration::from_millis(POLL_INTERVAL_MS), &sleep, || {
            let mut polls = polls.borrow_mut();
            *polls += 1;
            *polls > 2
        })
        .await
        .unwrap();
    // Two polls happen, each preceded by the 6000ms interval sleep, and the
    // repeated insertId prints once.
    assert_eq!(
        *durations.lock().unwrap(),
        vec![Duration::from_millis(6000); 2]
    );
    assert_eq!(received(&server).await.len(), 2);
    assert_eq!(stdout_of(&out).lines().count(), 1, "dedupe across polls");
}

#[tokio::test]
async fn tail_logs_simplified_omits_timestamp() {
    let server = MockServer::start().await;
    mount_logs_desc(
        &server,
        json!([text_entry(
            "i1",
            "WARNING",
            "2026-01-01T00:00:00Z",
            "careful",
            Some("f")
        )]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs {
            watch: false,
            simplified: true,
            poll_interval: Duration::from_millis(6000),
        },
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out).trim_end_matches('\n'),
        format!(
            "{} {} {}",
            pad("WARNING", 20),
            pad("f", 15),
            pad("careful", 20)
        )
    );
}

#[tokio::test]
async fn tail_logs_json_mode_embeds_multiline_payload() {
    let server = MockServer::start().await;
    let tz = local_utc_offset();
    mount_logs_desc(
        &server,
        // No payload at all: JSON mode does not require one (timestamp and
        // resource only).
        json!([log_entry("i1", "INFO", "2026-01-01T00:00:00Z", Some("f"))]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let stdout = stdout_of(&out);
    // The formatted line embeds JSON.stringify(entry, null, 2) multiline; the
    // first line carries the prefix, subsequent lines are raw.
    let expected_entry = log_entry("i1", "INFO", "2026-01-01T00:00:00Z", Some("f"));
    let pretty = serde_json::to_string_pretty(&expected_entry).unwrap();
    let mut json_lines = pretty.lines();
    let expected_first = format!(
        "{} {} {} {}",
        pad("INFO", 20),
        format_local_time("2026-01-01T00:00:00Z", tz).unwrap(),
        pad("f", 15),
        json_lines.next().unwrap()
    );
    let rendered: Vec<&str> = stdout.lines().collect();
    assert_eq!(rendered[0], expected_first);
    assert_eq!(&rendered[1..], json_lines.collect::<Vec<_>>().as_slice());
}

#[tokio::test]
async fn tail_logs_skips_entries_without_payload_or_required_fields() {
    let server = MockServer::start().await;
    let mut no_timestamp = text_entry("i2", "INFO", "", "x", Some("f"));
    no_timestamp.as_object_mut().unwrap().remove("timestamp");
    let mut no_insert_id = text_entry("i3", "INFO", "2026-01-01T03:00:00Z", "no id", Some("f"));
    no_insert_id.as_object_mut().unwrap().remove("insertId");
    mount_logs_desc(
        &server,
        json!([
            // No payload -> skipped in human mode.
            log_entry("i1", "INFO", "2026-01-01T00:00:00Z", Some("f")),
            no_timestamp,
            no_insert_id,
        ]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "");
}

#[tokio::test]
async fn tail_logs_requires_project_id_noninteractive() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GCP project ID is not set, unable to continue."
    );
}

#[tokio::test]
async fn tail_logs_requires_script_id_when_project_id_set() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    config.script_id = None;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    // clasp's core `getLogEntries` asserts script configuration first.
    assert_eq!(error.to_string(), "Project settings not found.");
}

// ---------------------------------------------------------------------------
// setup-logs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setup_logs_prints_success_with_existing_project() {
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    setup_logs(
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Script logs are now available in Cloud Logging.\n"
    );
}

#[tokio::test]
async fn setup_logs_json_success() {
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    setup_logs(
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(stdout_of(&out), "{\n  \"success\": true\n}\n");
}

#[tokio::test]
async fn setup_logs_prompts_and_persists_project_id() {
    let server = MockServer::start().await;
    let _client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    let mut prompt = TestPrompt::interactive();
    prompt.input_answer = "logs-proj".to_string();
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    setup_logs(&mut config, &Ui::new(prompt), &opener, &mut output)
        .await
        .unwrap();
    let stdout = stdout_of(&out);
    assert!(stdout.contains(
        "Opening https://script.google.com/home/projects/script/settings in your browser."
    ));
    assert!(stdout.contains("Script logs are now available in Cloud Logging."));
    let saved = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["projectId"], "logs-proj");
    assert_eq!(received(&server).await.len(), 0, "no API call");
}

#[tokio::test]
async fn setup_logs_requires_project_id_noninteractive() {
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = setup_logs(
        &mut config,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GCP project ID is not set, unable to continue."
    );
}

// ---------------------------------------------------------------------------
// open commands
// ---------------------------------------------------------------------------

#[tokio::test]
async fn open_script_non_tty_prints_manual_line_without_launching() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        false,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Open https://script.google.com/d/script/edit in your browser to continue.\n"
    );
    assert!(
        opener.opened.borrow().is_empty(),
        "no browser launch on non-TTY"
    );
}

#[tokio::test]
async fn open_script_json_prints_url_and_keeps_the_line() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs {
            script_id: Some("abc123"),
        },
        false,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "{\n  \"url\": \"https://script.google.com/d/abc123/edit\"\n}\nOpen https://script.google.com/d/abc123/edit in your browser to continue.\n"
    );
    assert!(opener.opened.borrow().is_empty());
}

#[tokio::test]
async fn open_script_tty_launches_browser_and_prints_opening_line() {
    let server = MockServer::start().await;
    mount_userinfo(&server).await;
    let _env = hints_env(Some("1"));
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        true,
        &Ui::new(TestPrompt::interactive()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Opening https://script.google.com/d/script/edit?authUser=1234567890 in your browser.\n"
    );
    assert_eq!(
        *opener.opened.borrow(),
        vec!["https://script.google.com/d/script/edit?authUser=1234567890".to_string()]
    );

    // JSON + TTY: the URL document AND the opening line are both printed
    // (preserved open-* quirk), then the browser launches.
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        true,
        &Ui::new(TestPrompt::interactive()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "{\n  \"url\": \"https://script.google.com/d/script/edit?authUser=1234567890\"\n}\nOpening https://script.google.com/d/script/edit?authUser=1234567890 in your browser.\n"
    );
    assert_eq!(opener.opened.borrow().len(), 1);
}

#[tokio::test]
async fn open_script_tty_launch_failure_propagates() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let result = crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        false,
        &Ui::new(TestPrompt::interactive()),
        &FailingOpener,
        &mut output,
    )
    .await;
    // The line was already printed; the launch error propagates (exit 1).
    assert_eq!(
        stdout_of(&out),
        "Opening https://script.google.com/d/script/edit in your browser.\n"
    );
    assert!(result.is_err(), "browser-launch failure must propagate");
}

#[tokio::test]
async fn open_script_requires_script_id() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    config.script_id = None;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Script ID not set, unable to open IDE.");
}

#[tokio::test]
async fn open_container_requires_parent_id_and_builds_drive_url() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = crsp::commands::open_container::open_container(
        &client,
        &config,
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Parent ID not set, unable to open document."
    );

    let mut config = config.clone();
    config.parent_id = Some("doc123".to_string());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    crsp::commands::open_container::open_container(
        &client,
        &config,
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Open https://drive.google.com/open?id=doc123 in your browser to continue.\n"
    );
}

#[tokio::test]
async fn open_logs_builds_cloud_console_url() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "gcp-proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    crsp::commands::open_logs::open_logs(
        &client,
        &mut config,
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Open https://console.cloud.google.com/logs/viewer?project=gcp-proj&resource=app_script_function in your browser to continue.\n"
    );
}

#[tokio::test]
async fn open_api_console_builds_dashboard_url() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "gcp-proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    crsp::commands::open_api_console::open_api_console(
        &client,
        &mut config,
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Open https://console.developers.google.com/apis/dashboard?project=gcp-proj in your browser to continue.\n"
    );
}

#[tokio::test]
async fn open_credentials_setup_builds_credentials_url() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "gcp-proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    crsp::commands::open_credentials_setup::open_credentials_setup(
        &client,
        &mut config,
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        stdout_of(&out),
        "Open https://console.developers.google.com/apis/credentials?project=gcp-proj in your browser to continue.\n"
    );
}

/// Mounts the deployments list used by open-web-app interactive selection
/// (API order, unsorted by update time).
async fn mount_deployments(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "deployments": [
                    {"deploymentId": "dep-late", "updateTime": "2026-03-01T00:00:00Z", "deploymentConfig": {"description": "later deploy", "versionNumber": 4}},
                    {"deploymentId": "dep-early", "updateTime": "2026-01-01T00:00:00Z", "deploymentConfig": {"description": "early deploy", "versionNumber": 2}},
                    {"deploymentId": "dep-head", "deploymentConfig": {}}
                ]
            })),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn open_web_app_sorts_choices_by_update_time_ascending() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments/dep-early"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "entryPoints": [{"entryPointType": "WEB_APP", "webApp": {"url": "https://script.google.com/macros/s/AKfy/exec"}}]
            })),
        )
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut prompt = TestPrompt::interactive();
    prompt.select_answer = "dep-early".to_string();
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    let ui = Ui::new(prompt);
    crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: None,
        },
        false,
        &ui,
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    let prompt = ui.into_adapter();
    let select_calls = prompt.select_calls.borrow();
    assert!(!select_calls.is_empty(), "a select prompt must be shown");
    let options: Vec<(String, String)> = select_calls[0]
        .options
        .iter()
        .map(|(value, label)| (value.clone(), label.clone()))
        .collect();
    // updateTime ascending: dep-early, dep-late, then the no-updateTime entry
    // keeps its relative position (stable sort).
    let order: Vec<&str> = options.iter().map(|(value, _)| value.as_str()).collect();
    assert_eq!(order, vec!["dep-early", "dep-late", "dep-head"]);
    // Choice labels: ellipsize(description, 30)@version(padEnd 4) - id.
    assert_eq!(
        options[0].1,
        format!("{}@{} - dep-early", pad("early deploy", 30), pad("2", 4))
    );
    assert_eq!(
        options[1].1,
        format!("{}@{} - dep-late", pad("later deploy", 30), pad("4", 4))
    );
    assert_eq!(
        options[2].1,
        format!("{}@{} - dep-head", pad("", 30), pad("HEAD", 4))
    );
    assert_eq!(
        stdout_of(&out),
        "Opening https://script.google.com/macros/s/AKfy/exec in your browser.\n"
    );
    assert_eq!(
        *opener.opened.borrow(),
        vec!["https://script.google.com/macros/s/AKfy/exec".to_string()]
    );
}

#[tokio::test]
async fn open_web_app_argument_skips_selection_and_gets_entry_points() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments/dep-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "entryPoints": [
                    {"entryPointType": "EXECUTION"},
                    {"entryPointType": "WEB_APP", "webApp": {"url": "https://script.google.com/macros/s/XYZ/exec"}}
                ]
            })),
        )
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: Some("dep-1"),
        },
        false,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(url_of(&requests[0]).ends_with("/v1/projects/script/deployments/dep-1"));
    assert_eq!(
        stdout_of(&out),
        "Open https://script.google.com/macros/s/XYZ/exec in your browser to continue.\n"
    );
}

#[tokio::test]
async fn open_web_app_errors_when_no_web_app_entry_point() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entryPoints": [{"entryPointType": "EXECUTION"}]
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: Some("dep-1"),
        },
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "No web app entry point found.");
}

#[tokio::test]
async fn open_web_app_requires_deployment_id_noninteractive() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: None,
        },
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Deployment ID is required.");
}

#[tokio::test]
async fn open_web_app_requires_script_id() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = configured_config(temp.path());
    config.script_id = None;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: Some("dep-1"),
        },
        false,
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Script ID not set, unable to open web app."
    );
}

/// clasp `experiments.ts` `isEnabled`: unset → false; 'true' (case
/// insensitive) or '1' → true; every other value → false.
#[test]
fn include_user_hint_in_url_parses_env_values() {
    for (value, expected) in [
        (None, false),
        (Some("1"), true),
        (Some("true"), true),
        (Some("TRUE"), true),
        (Some("True"), true),
        (Some("yes"), false),
        (Some("0"), false),
        (Some(""), false),
    ] {
        let _env = hints_env(value);
        assert_eq!(
            crsp::commands::shared::include_user_hint_in_url(),
            expected,
            "CLASP_ENABLE_USER_HINTS={value:?}"
        );
    }
}

#[tokio::test]
async fn open_hints_disabled_makes_no_userinfo_request() {
    let server = MockServer::start().await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "gcp-proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_logs::open_logs(
        &client,
        &mut config,
        false,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert!(
        requests.is_empty(),
        "no userinfo fetch when hints are disabled"
    );
    assert_eq!(
        stdout_of(&out),
        "Open https://console.cloud.google.com/logs/viewer?project=gcp-proj&resource=app_script_function in your browser to continue.\n"
    );
}

#[tokio::test]
async fn open_hints_userinfo_failure_sets_empty_auth_user() {
    let server = MockServer::start().await;
    mount_userinfo_error(&server).await;
    let _env = hints_env(Some("true"));
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_script::open_script(
        &client,
        &config,
        crsp::commands::open_script::OpenScriptArgs { script_id: None },
        true,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    // clasp: `url.searchParams.set('authUser', userHint ?? '')`.
    assert_eq!(
        stdout_of(&out),
        "Open https://script.google.com/d/script/edit?authUser= in your browser to continue.\n"
    );
}

// ---------------------------------------------------------------------------
// Degenerate-input divergences from clasp (parked audit item 10) and pinned
// wrapper/parse divergences. Each test pins crsp's CURRENT behavior and
// documents what clasp does instead; these are accepted divergences, not
// bugs to fix.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tail_logs_empty_text_payload_prints_a_blank_payload_line_divergence() {
    // Degenerate-input divergence (accepted): clasp's
    // `if (entry.textPayload)` treats an empty string as falsy and falls
    // through to the jsonPayload branch — with no jsonPayload the entry is
    // skipped entirely. crsp's `text_payload()` yields Some("") and prints
    // the padded-blank payload column instead of dropping the line.
    let server = MockServer::start().await;
    mount_logs_desc(
        &server,
        json!([text_entry(
            "i1",
            "INFO",
            "2026-01-01T00:00:00Z",
            "",
            Some("greet")
        )]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let tz = local_utc_offset();
    let stdout = stdout_of(&out);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![format!(
            "{} {} {} {}",
            pad("INFO", 20),
            format_local_time("2026-01-01T00:00:00Z", tz).unwrap(),
            pad("greet", 15),
            pad("", 20)
        )]
    );
}

#[tokio::test]
async fn tail_logs_null_resource_prints_the_entry_divergence() {
    // Degenerate-input divergence (accepted): clasp's
    // `if (!entry.resource || !entry.timestamp) return` drops entries whose
    // resource is explicitly null (falsy). crsp checks presence only, so the
    // entry prints with an N/A function column.
    let server = MockServer::start().await;
    mount_logs_desc(
        &server,
        json!([{
            "insertId": "i1",
            "severity": "INFO",
            "timestamp": "2026-01-01T00:00:00Z",
            "resource": null,
            "textPayload": "hello"
        }]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let tz = local_utc_offset();
    let stdout = stdout_of(&out);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![format!(
            "{} {} {} {}",
            pad("INFO", 20),
            format_local_time("2026-01-01T00:00:00Z", tz).unwrap(),
            pad("N/A", 15),
            pad("hello", 20)
        )]
    );
}

#[tokio::test]
async fn tail_logs_non_string_json_message_compact_stringifies_divergence() {
    // Degenerate-input divergence (accepted): a truthy NON-string
    // `jsonPayload.message` (e.g. 123) is printed verbatim by clasp
    // (`padEnd(entry.jsonPayload?.message, 20)`), while crsp's String
    // pattern misses it and compact-stringifies the whole payload.
    let server = MockServer::start().await;
    mount_logs_desc(
        &server,
        json!([{
            "insertId": "i1",
            "severity": "INFO",
            "timestamp": "2026-01-01T00:00:00Z",
            "resource": {"labels": {"function_name": "greet"}},
            "jsonPayload": {"message": 123}
        }]),
    )
    .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    tail_logs(
        &client,
        &mut config,
        TailLogsArgs::default(),
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap();
    let tz = local_utc_offset();
    let stdout = stdout_of(&out);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![format!(
            "{} {} {} {}",
            pad("INFO", 20),
            format_local_time("2026-01-01T00:00:00Z", tz).unwrap(),
            pad("greet", 15),
            pad("{\"message\":123}", 20)
        )]
    );
}

#[tokio::test]
async fn open_web_app_pre_existing_auth_user_param_is_appended_divergence() {
    // Degenerate-input divergence (accepted): when the API-provided web app
    // URL already carries an authUser parameter, clasp's
    // `url.searchParams.set('authUser', ...)` REPLACES it, while crsp's
    // append_pair adds a second authUser pair.
    let server = MockServer::start().await;
    mount_userinfo(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entryPoints": [{"entryPointType": "WEB_APP", "webApp": {"url": "https://script.google.com/macros/s/XYZ/exec?authUser=OLD"}}]
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = gcp_config(temp.path(), "proj");
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let opener = RecordingOpener::default();
    crsp::commands::open_web_app::open_web_app(
        &client,
        &config,
        crsp::commands::open_web_app::OpenWebAppArgs {
            deployment_id: Some("dep-1"),
        },
        true,
        &Ui::new(TestPrompt::default()),
        &opener,
        &mut output,
    )
    .await
    .unwrap();
    // clasp set() would render `?authUser=1234567890`; crsp keeps both pairs.
    assert_eq!(
        stdout_of(&out),
        "Open https://script.google.com/macros/s/XYZ/exec?authUser=OLD&authUser=1234567890 in your browser to continue.\n"
    );
}

#[tokio::test]
async fn enable_api_null_enabled_advanced_services_fails_divergence() {
    // Degenerate-input divergence (accepted): clasp replaces a null
    // `enabledAdvancedServices` with a one-entry array and proceeds to
    // enable the service; crsp treats the explicit null as a non-array and
    // fails before any request.
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), json!({"enabledAdvancedServices": null})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let error = enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    match error {
        CrspError::Config(message) => assert_eq!(
            message,
            "Manifest dependencies.enabledAdvancedServices is not an array."
        ),
        other => panic!("expected Config error, got {other:?}"),
    }
    assert!(
        received(&server).await.is_empty(),
        "no Service Usage request is sent"
    );
}

#[tokio::test]
async fn disable_api_null_enabled_advanced_services_fails_divergence() {
    // Degenerate-input divergence (accepted): clasp skips the manifest write
    // for a null `enabledAdvancedServices` (falsy) and still disables the
    // service at GCP; crsp fails before any request.
    let server = MockServer::start().await;
    mount_disable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    write_manifest(temp.path(), json!({"enabledAdvancedServices": null})).await;
    let mut config = gcp_config(temp.path(), "proj");
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let error = disable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    match error {
        CrspError::Config(message) => assert_eq!(
            message,
            "Manifest dependencies.enabledAdvancedServices is not an array."
        ),
        other => panic!("expected Config error, got {other:?}"),
    }
    assert!(
        received(&server).await.is_empty(),
        "no Service Usage request is sent"
    );
}

#[tokio::test]
async fn enable_api_top_level_array_manifest_fails_divergence() {
    // Degenerate-input divergence (accepted): clasp assigns `dependencies`
    // onto the array object and JSON.stringify drops it, so the manifest is
    // unchanged and the service is still enabled (silent no-op); crsp fails
    // with a Config error before any request.
    let server = MockServer::start().await;
    mount_enable(&server, "proj", "sheets", 200).await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("appsscript.json"), "[1, 2]").unwrap();
    let mut config = gcp_config(temp.path(), "proj");
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let error = enable_api(
        &client,
        &mut config,
        "sheets",
        &Ui::new(TestPrompt::default()),
        &RecordingOpener::default(),
        &mut output,
    )
    .await
    .unwrap_err();
    match error {
        CrspError::Config(message) => assert_eq!(message, "Manifest is not an object."),
        other => panic!("expected Config error, got {other:?}"),
    }
    assert!(
        received(&server).await.is_empty(),
        "no Service Usage request is sent"
    );
    // The array manifest is left untouched.
    assert_eq!(
        fs::read_to_string(temp.path().join("appsscript.json")).unwrap(),
        "[1, 2]"
    );
}
