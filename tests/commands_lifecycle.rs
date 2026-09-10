//! Integration tests for the project lifecycle and version/deployment
//! commands (Task 6): clone-script, create-script, create-version,
//! list-versions, create-deployment, update-deployment, delete-deployment,
//! list-deployments, list-scripts, and delete-script. HTTP contracts are
//! asserted against wiremock (exact paths, query parameters, bodies, and
//! request order); output contracts follow clasp v3.4.1 (spec §6.2, §5 #8).

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;

use google_clasp_rs::api::{ApiClient, ApiClientConfig, BaseUrls};
use google_clasp_rs::commands::clone_script::{CloneArgs, clone_script};
use google_clasp_rs::commands::create_deployment::{CreateDeploymentArgs, create_deployment};
use google_clasp_rs::commands::create_script::{CreateScriptArgs, create_script};
use google_clasp_rs::commands::create_version::create_version;
use google_clasp_rs::commands::delete_deployment::delete_deployment;
use google_clasp_rs::commands::delete_script::delete_script;
use google_clasp_rs::commands::list_deployments::list_deployments;
use google_clasp_rs::commands::list_scripts::list_scripts;
use google_clasp_rs::commands::list_versions::list_versions;
use google_clasp_rs::commands::shared::{ellipsize, extract_script_id};
use google_clasp_rs::commands::update_deployment::{UpdateDeploymentArgs, update_deployment};
use google_clasp_rs::core::config::ProjectConfig;
use google_clasp_rs::output::Output;
use google_clasp_rs::ui::{
    PromptAdapter, PromptConfirm, PromptDialog, PromptInput, PromptMultiSelect, PromptSelect,
    PromptSpinner, Ui,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn api_client(base: &str) -> ApiClient {
    let refresh: google_clasp_rs::api::RefreshFn =
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

#[derive(Default)]
struct TestPrompt {
    interactive: bool,
    input_answer: String,
    select_answer: String,
    confirm_answer: bool,
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
        Ok(self.confirm_answer)
    }
    fn dialog(&self, _: &PromptDialog) -> io::Result<String> {
        Ok(String::new())
    }
    fn spinner<T, F: FnOnce() -> T>(&self, _: PromptSpinner, f: F) -> io::Result<T> {
        Ok(f())
    }
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

/// The default (script-less) instance discovered from the temp root, like
/// `crsp clone-script` sees in an empty directory.
async fn default_config(root: &Path) -> ProjectConfig {
    ProjectConfig::discover(None, root).await.unwrap()
}

fn content_body() -> Value {
    json!({
        "files": [
            {"name": "Code", "type": "SERVER_JS", "source": "console.log('hi');"},
            {"name": "appsscript", "type": "JSON", "source": "{}"}
        ]
    })
}

/// Mounts the content GET used by clone/create initial pulls.
async fn mount_content(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(content_body()))
        .mount(server)
        .await;
}

async fn mount_content_for(server: &MockServer, script_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/projects/{script_id}/content")))
        .respond_with(ResponseTemplate::new(200).set_body_json(content_body()))
        .mount(server)
        .await;
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

// ---------------------------------------------------------------------------
// clone-script
// ---------------------------------------------------------------------------

#[tokio::test]
async fn clone_extracts_id_from_url_and_writes_config() {
    let server = MockServer::start().await;
    mount_content_for(&server, "the_script_id").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let result = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some(" https://script.google.com/d/the_script_id/edit#slide1 "),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(result.script_id, "the_script_id");
    assert_eq!(
        result.files,
        vec!["Code.js".to_string(), "appsscript.json".to_string()]
    );
    // The content fetch must target the extracted id (HEAD, no version).
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(url_of(&requests[0]).ends_with("/v1/projects/the_script_id/content"));
    assert!(!url_of(&requests[0]).contains("versionNumber"));

    let written = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let settings: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(settings["scriptId"], "the_script_id");
    assert_eq!(settings["rootDir"], "");
    assert!(settings.get("parentId").is_none());
    assert!(settings.get("projectId").is_none());
    assert_eq!(settings["filePushOrder"], json!([]));
    assert_eq!(settings["skipSubdirectories"], false);

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ Code.js\n└─ appsscript.json\nCloned 2 files.\n"
    );
    assert_eq!(String::from_utf8(err).unwrap(), "");
}

#[tokio::test]
async fn clone_with_version_fetches_versioned_content() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/abc/content"))
        .and(query_param("versionNumber", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(content_body()))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("abc"),
            version_number: Some("5"),
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(url_of(&requests[0]).ends_with("versionNumber=5"));
}

#[tokio::test]
async fn clone_json_prints_script_id_and_files() {
    let server = MockServer::start().await;
    mount_content(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"scriptId\": \"script\",\n  \"files\": [\n    \"Code.js\",\n    \"appsscript.json\"\n  ]\n}\n"
    );
}

#[tokio::test]
async fn clone_noninteractive_without_id_errors() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: None,
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "No script ID.");
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn clone_interactive_selects_from_drive_list() {
    let server = MockServer::start().await;
    mount_content(&server).await;
    Mock::given(method("GET"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                {"id": "script", "name": "My script"},
                {"id": "other", "name": "Another project"}
            ]
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut prompt = TestPrompt::interactive();
    prompt.select_answer.clone_from(&"script".to_string());
    let ui = Ui::new(prompt);
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let result = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: None,
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &ui,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(result.script_id, "script");
    let adapter = ui.into_adapter();
    let calls = adapter.select_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].prompt, "Clone which script?");
    assert_eq!(
        calls[0].options,
        vec![
            (
                "script".to_string(),
                "My script            - https://script.google.com/d/script/edit".to_string()
            ),
            (
                "other".to_string(),
                "Another project      - https://script.google.com/d/other/edit".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn clone_existing_project_is_rejected() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("other"),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Project file already exists.");
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn clone_invalid_version_argument_is_rejected() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: Some("abc"),
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "'abc' is not a valid integer.");
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn clone_root_dir_override_writes_files_and_config() {
    let server = MockServer::start().await;
    mount_content(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: None,
            root_dir: Some("src"),
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // Files are written under src/ and displayed relative to the cwd.
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ src/Code.js\n└─ src/appsscript.json\nCloned 2 files.\n"
    );
    assert!(temp.path().join("src").join("Code.js").exists());
    let written = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let settings: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(settings["rootDir"], "src");
}

#[tokio::test]
async fn clone_warns_on_skipped_write() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [{"name": "Code", "type": "SERVER_JS", "source": "x"}]
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", temp.path().join("Code.js")).unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "Security Warning: Skipping write of Code.js (target path is a symbolic link).\n"
    );
    // The skipped file is still listed as a remote file.
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ Code.js\nCloned one file.\n"
    );
}

#[tokio::test]
async fn clone_honors_configured_script_extensions() {
    let server = MockServer::start().await;
    mount_content(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let mut config = default_config(temp.path()).await;
    // clasp getFileExtension uses the FIRST configured script extension.
    config.script_extensions = vec![".gs".to_string()];
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ Code.gs\n└─ appsscript.json\nCloned 2 files.\n"
    );
    assert!(temp.path().join("Code.gs").exists());
    assert!(!temp.path().join("Code.js").exists());

    // Round-trip: a push on the cloned tree collects the written file.
    config.script_id = Some("script".to_string());
    let collected = google_clasp_rs::core::files::collect_local_files(&config)
        .await
        .unwrap();
    let code = collected
        .files
        .iter()
        .find(|file| file.local_path == "Code.gs")
        .unwrap();
    assert_eq!(code.file_type, "SERVER_JS");
    assert_eq!(code.remote_path, "Code");
}

#[tokio::test]
async fn clone_maps_invalid_argument_to_invalid_script_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": {"code": 400, "message": "Requested entity was not found."}}),
        ))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = clone_script(
        &client,
        &config,
        CloneArgs {
            script_id: Some("script"),
            version_number: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Invalid script ID.");
    // The failure happens before any pull or settings write.
    assert!(!temp.path().join(".clasp.json").exists());
}

// ---------------------------------------------------------------------------
// create-script
// ---------------------------------------------------------------------------

async fn mount_create(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"scriptId": "script"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_standalone_webapp_shows_deployment_tip() {
    let server = MockServer::start().await;
    mount_create(&server).await;
    let temp = TempDir::new().unwrap();
    let cwd = temp.path().join("my-project-folder");
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "WebApp",
            title: None,
            parent_id: None,
            root_dir: None,
            cwd: &cwd,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    let create_project = requests
        .iter()
        .find(|request| request.method == "POST" && url_of(request).ends_with("/v1/projects"))
        .unwrap();
    assert_eq!(
        body_of(create_project).await,
        json!({"title": "My-project-folder"})
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created new script: https://script.google.com/d/script/edit\n\
         Tip: to deploy this script as a web app, configure \"webApp\" in appsscript.json \
         and run `clasp create-deployment`.\n\
         Cloned no files.\n"
    );
    let written = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let settings: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(settings["scriptId"], "script");
    assert!(settings.get("parentId").is_none());
}

#[tokio::test]
async fn create_api_type_shows_api_tip() {
    let server = MockServer::start().await;
    mount_create(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "API",
            title: Some("My App"),
            parent_id: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created new script: https://script.google.com/d/script/edit\n\
         Tip: to deploy this script as a API executable, configure \"executionApi\" \
         in appsscript.json and run `clasp create-deployment`.\n\
         Cloned no files.\n"
    );
}

#[tokio::test]
async fn create_unknown_type_lists_valid_values() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "bogus",
            title: None,
            parent_id: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid script type \"bogus\". Valid types are: standalone, webapp, api, docs, forms, sheets, slides."
    );
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn create_container_docs_creates_drive_file_then_script() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "container1"})))
        .mount(&server)
        .await;
    mount_create(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "docs",
            title: Some("Docs title"),
            parent_id: Some("ignored"),
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    let drive_create = requests
        .iter()
        .find(|request| request.method == "POST" && url_of(request).ends_with("/v3/files"))
        .unwrap();
    assert_eq!(
        body_of(drive_create).await,
        json!({"mimeType": "application/vnd.google-apps.document", "name": "Docs title"})
    );
    let create_project = requests
        .iter()
        .find(|request| request.method == "POST" && url_of(request).ends_with("/v1/projects"))
        .unwrap();
    assert_eq!(
        body_of(create_project).await,
        json!({"parentId": "container1", "title": "Docs title"})
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Created new document: https://drive.google.com/open?id=container1\n\
         Created new script: https://script.google.com/d/script/edit\n\
         Cloned no files.\n"
    );
    let written = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let settings: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(settings["parentId"], "container1");
}

#[tokio::test]
async fn create_container_sheets_uses_spreadsheet_mime_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "container2"})))
        .mount(&server)
        .await;
    mount_create(&server).await;
    let temp = TempDir::new().unwrap();
    let cwd = temp.path().join("sheets-folder");
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let result = create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "sheets",
            title: None,
            parent_id: None,
            root_dir: None,
            cwd: &cwd,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    let drive_create = requests
        .iter()
        .find(|request| request.method == "POST" && url_of(request).ends_with("/v3/files"))
        .unwrap();
    // Default title: humanized basename of the cwd (the temp folder name).
    assert_eq!(
        body_of(drive_create).await,
        json!({"mimeType": "application/vnd.google-apps.spreadsheet", "name": "Sheets-folder"})
    );
    assert_eq!(result.parent_id.as_deref(), Some("container2"));
    // JSON mode: no success/tip lines, container parentId present.
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"scriptId\": \"script\",\n  \"parentId\": \"container2\",\n  \"files\": []\n}\n"
    );
}

#[tokio::test]
async fn create_standalone_json_omits_parent_id() {
    let server = MockServer::start().await;
    mount_create(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "standalone",
            title: None,
            parent_id: Some("p1"),
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"scriptId\": \"script\",\n  \"files\": []\n}\n"
    );
    // The --parentId option lands in .clasp.json (clasp createScript sets
    // options.project = {scriptId, parentId}).
    let written = fs::read_to_string(temp.path().join(".clasp.json")).unwrap();
    let settings: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(settings["parentId"], "p1");
}

#[tokio::test]
async fn create_existing_project_is_rejected() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "standalone",
            title: None,
            parent_id: None,
            root_dir: None,
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Project file already exists.");
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn create_root_dir_escape_is_rejected_before_type_check() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = create_script(
        &client,
        &config,
        CreateScriptArgs {
            script_type: "bogus",
            title: None,
            parent_id: None,
            root_dir: Some("../escape"),
            cwd: temp.path(),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Security Error: Content directory \"{}\" resolves outside the project root \"{}\". \
             This may indicate a path traversal attempt.",
            temp.path().parent().unwrap().join("escape").display(),
            temp.path().display()
        )
    );
    assert!(received(&server).await.is_empty());
}

// ---------------------------------------------------------------------------
// create-version
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_version_posts_description_and_prints_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 3})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let result = create_version(
        &client,
        &config,
        Some("first release"),
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(result.version_number, 3);
    let requests = received(&server).await;
    assert_eq!(
        body_of(&requests[0]).await,
        json!({"description": "first release"})
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"versionNumber\": 3\n}\n"
    );
}

#[tokio::test]
async fn create_version_human_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 42})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_version(
        &client,
        &config,
        Some("desc"),
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "Created version 42\n");
}

#[tokio::test]
async fn create_version_noninteractive_uses_empty_description() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 1})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_version(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(body_of(&requests[0]).await, json!({"description": ""}));
}

#[tokio::test]
async fn create_version_interactive_prompts_for_description() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 9})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut prompt = TestPrompt::interactive();
    prompt
        .input_answer
        .clone_from(&"typed description".to_string());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_version(&client, &config, None, &Ui::new(prompt), &mut output)
        .await
        .unwrap();
    let requests = received(&server).await;
    assert_eq!(
        body_of(&requests[0]).await,
        json!({"description": "typed description"})
    );
}

#[tokio::test]
async fn create_version_requires_configured_script() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = create_version(
        &client,
        &config,
        Some("desc"),
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Project settings not found.");
    assert!(received(&server).await.is_empty());
}

// ---------------------------------------------------------------------------
// list-versions
// ---------------------------------------------------------------------------

fn versions_body() -> Value {
    json!({
        "versions": [
            {"versionNumber": 1, "description": "first"},
            {"versionNumber": 2},
            {"versionNumber": 3, "description": "third"}
        ]
    })
}

#[tokio::test]
async fn list_versions_human_is_reversed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(versions_body()))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_versions(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Found 3 versions.\n3 - third\n2 - No description\n1 - first\n"
    );
}

#[tokio::test]
async fn list_versions_json_preserves_raw_api_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(versions_body()))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    list_versions(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "[\n  {\n    \"versionNumber\": 1,\n    \"description\": \"first\"\n  },\n  \
         {\n    \"versionNumber\": 2\n  },\n  \
         {\n    \"versionNumber\": 3,\n    \"description\": \"third\"\n  }\n]\n"
    );
}

#[tokio::test]
async fn list_versions_empty_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let versions = list_versions(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert!(versions.is_empty());
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "No deployed versions of script.\n"
    );
}

#[tokio::test]
async fn list_versions_explicit_script_id_overrides_config() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/other/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_versions(
        &client,
        &config,
        Some("other"),
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert!(url_of(&requests[0]).contains("/v1/projects/other/versions"));
}

// ---------------------------------------------------------------------------
// create-deployment / update-deployment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deploy_without_version_creates_version_then_deployment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 3})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep1",
            "deploymentConfig": {"versionNumber": 3, "description": "my deploy"}
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_deployment(
        &client,
        &config,
        CreateDeploymentArgs {
            version_number: None,
            description: Some("my deploy"),
            deployment_id: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(url_of(&requests[0]).ends_with("/v1/projects/script/versions"));
    assert_eq!(
        body_of(&requests[0]).await,
        json!({"description": "my deploy"})
    );
    assert!(url_of(&requests[1]).ends_with("/v1/projects/script/deployments"));
    assert_eq!(
        body_of(&requests[1]).await,
        json!({
            "description": "my deploy",
            "versionNumber": 3,
            "manifestFileName": "appsscript"
        })
    );
    assert_eq!(String::from_utf8(out).unwrap(), "Deployed dep1 @3\n");
}

#[tokio::test]
async fn deploy_explicit_version_skips_version_creation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep1",
            "deploymentConfig": {"versionNumber": 7}
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    create_deployment(
        &client,
        &config,
        CreateDeploymentArgs {
            version_number: Some("7"),
            description: None,
            deployment_id: None,
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
        json!({
            "description": "",
            "versionNumber": 7,
            "manifestFileName": "appsscript"
        })
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deploymentId\": \"dep1\",\n  \"versionNumber\": 7\n}\n"
    );
}

#[tokio::test]
async fn deploy_with_deployment_id_updates_it() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/deployments/dep1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep1",
            "deploymentConfig": {"versionNumber": 7, "description": "redep"}
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    create_deployment(
        &client,
        &config,
        CreateDeploymentArgs {
            version_number: Some("7"),
            description: Some("redep"),
            deployment_id: Some("dep1"),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        body_of(&requests[0]).await,
        json!({
            "deploymentConfig": {
                "description": "redep",
                "versionNumber": 7,
                "scriptId": "script",
                "manifestFileName": "appsscript"
            }
        })
    );
    assert_eq!(String::from_utf8(out).unwrap(), "Deployed dep1 @7\n");
}

#[tokio::test]
async fn deploy_head_response_prints_head() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 3})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deploymentId": "dep9"})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    create_deployment(
        &client,
        &config,
        CreateDeploymentArgs {
            version_number: None,
            description: None,
            deployment_id: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deploymentId\": \"dep9\"\n}\n"
    );
}

#[tokio::test]
async fn deploy_invalid_version_argument_is_rejected() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = create_deployment(
        &client,
        &config,
        CreateDeploymentArgs {
            version_number: Some("abc"),
            description: None,
            deployment_id: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "'abc' is not a valid integer.");
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn redeploy_without_version_creates_version_then_updates() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 4})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/deployments/dep1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep1",
            "deploymentConfig": {"versionNumber": 4}
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    update_deployment(
        &client,
        &config,
        "dep1",
        UpdateDeploymentArgs {
            version_number: None,
            description: None,
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(url_of(&requests[0]).ends_with("/v1/projects/script/versions"));
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(
        body_of(&requests[1]).await,
        json!({
            "deploymentConfig": {
                "description": "",
                "versionNumber": 4,
                "scriptId": "script",
                "manifestFileName": "appsscript"
            }
        })
    );
    assert_eq!(String::from_utf8(out).unwrap(), "Redeployed dep1 @4\n");
}

#[tokio::test]
async fn redeploy_json_prints_deployment_fields() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/deployments/dep1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep1",
            "deploymentConfig": {"versionNumber": 8, "description": "updated"}
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    update_deployment(
        &client,
        &config,
        "dep1",
        UpdateDeploymentArgs {
            version_number: Some("8"),
            description: Some("updated"),
        },
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deploymentId\": \"dep1\",\n  \"versionNumber\": 8,\n  \"description\": \"updated\"\n}\n"
    );
}

// ---------------------------------------------------------------------------
// delete-deployment
// ---------------------------------------------------------------------------

fn deployments_body() -> Value {
    json!({
        "deployments": [
            {"deploymentId": "dep1", "deploymentConfig": {"versionNumber": 2, "description": "v2"}},
            {"deploymentId": "dep2", "deploymentConfig": {}},
            {"deploymentId": "dep3", "deploymentConfig": {"versionNumber": 5, "description": "v5"}}
        ]
    })
}

async fn mount_deployments(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(deployments_body()))
        .mount(server)
        .await;
}

async fn mount_delete(server: &MockServer, id: &str) {
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/projects/script/deployments/{id}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
}

#[tokio::test]
async fn undeploy_single_versioned_deployment_is_auto_selected() {
    let server = MockServer::start().await;
    // dep1 is the only versioned deployment (dep2 is HEAD) -> auto-select.
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deployments": [
                {"deploymentId": "dep2", "deploymentConfig": {}},
                {"deploymentId": "dep1", "deploymentConfig": {"versionNumber": 2}}
            ]
        })))
        .mount(&server)
        .await;
    mount_delete(&server, "dep1").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let deleted = delete_deployment(
        &client,
        &config,
        None,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(deleted, vec!["dep1".to_string()]);
    let requests = received(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(url_of(&requests[1]).ends_with("/v1/projects/script/deployments/dep1"));
    assert_eq!(requests[1].method, "DELETE");
    assert_eq!(String::from_utf8(out).unwrap(), "Deleted deployment dep1\n");
}

#[tokio::test]
async fn undeploy_all_only_deletes_versioned_deployments() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    mount_delete(&server, "dep1").await;
    mount_delete(&server, "dep3").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let deleted = delete_deployment(
        &client,
        &config,
        None,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(deleted, vec!["dep1".to_string(), "dep3".to_string()]);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Deleted deployment dep1\nDeleted deployment dep3\nDeleted all deployments.\n"
    );
}

#[tokio::test]
async fn undeploy_all_json_suppresses_lines() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    mount_delete(&server, "dep1").await;
    mount_delete(&server, "dep3").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let deleted = delete_deployment(
        &client,
        &config,
        None,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(deleted.len(), 2);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deletedDeploymentIds\": [\n    \"dep1\",\n    \"dep3\"\n  ]\n}\n"
    );
}

#[tokio::test]
async fn undeploy_multiple_interactive_selects_deployment() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    mount_delete(&server, "dep3").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut prompt = TestPrompt::interactive();
    prompt.select_answer.clone_from(&"dep3".to_string());
    let ui = Ui::new(prompt);
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let deleted = delete_deployment(&client, &config, None, false, &ui, &mut output)
        .await
        .unwrap();
    assert_eq!(deleted, vec!["dep3".to_string()]);
    let adapter = ui.into_adapter();
    let calls = adapter.select_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].prompt, "Delete which deployment?");
    assert_eq!(
        calls[0].options,
        vec![
            ("dep1".to_string(), "dep1 - v2".to_string()),
            ("dep3".to_string(), "dep3 - v5".to_string()),
        ]
    );
}

#[tokio::test]
async fn undeploy_multiple_noninteractive_errors() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = delete_deployment(
        &client,
        &config,
        None,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "No deployments found.");
    // Only the list request happened; nothing was deleted.
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(url_of(&requests[0]).contains("/v1/projects/script/deployments"));
}

#[tokio::test]
async fn undeploy_multiple_noninteractive_json_emits_empty_list() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let deleted = delete_deployment(
        &client,
        &config,
        None,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert!(deleted.is_empty());
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deletedDeploymentIds\": []\n}\n"
    );
    // Only the list request happened; nothing was deleted.
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[tokio::test]
async fn undeploy_explicit_id_deletes_without_listing() {
    let server = MockServer::start().await;
    mount_delete(&server, "dep2").await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    let deleted = delete_deployment(
        &client,
        &config,
        Some("dep2"),
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(deleted, vec!["dep2".to_string()]);
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"deletedDeploymentIds\": [\n    \"dep2\"\n  ]\n}\n"
    );
}

// ---------------------------------------------------------------------------
// list-deployments
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_deployments_human_lines_and_json() {
    let server = MockServer::start().await;
    mount_deployments(&server).await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());

    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_deployments(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Found 3 deployments.\n- dep1 @2 - v2\n- dep2 @HEAD \n- dep3 @5 - v5\n"
    );

    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    list_deployments(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "[\n  {\n    \"deploymentId\": \"dep1\",\n    \"versionNumber\": 2,\n    \"description\": \"v2\"\n  },\n  \
         {\n    \"deploymentId\": \"dep2\"\n  },\n  \
         {\n    \"deploymentId\": \"dep3\",\n    \"versionNumber\": 5,\n    \"description\": \"v5\"\n  }\n]\n"
    );
}

#[tokio::test]
async fn list_deployments_empty_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_deployments(
        &client,
        &config,
        None,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "No deployments.\n");
}

// ---------------------------------------------------------------------------
// list-scripts
// ---------------------------------------------------------------------------

fn scripts_body() -> Value {
    json!({
        "files": [
            {"id": "s1", "name": "script 1"},
            {"id": "s2", "name": "abcdefghijklmnopqrst"},
            {"id": "s3", "name": "abcdefghijklmnopqrstu"},
            {"id": "s4", "name": "my super long script name here"}
        ]
    })
}

async fn mount_scripts(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(scripts_body()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn list_scripts_truncates_names_to_twenty_chars() {
    let server = MockServer::start().await;
    mount_scripts(&server).await;
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_scripts(&client, false, &Ui::new(TestPrompt::default()), &mut output)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Found 4 scripts.\n\
         script 1             - https://script.google.com/d/s1/edit\n\
         abcdefghijklmnopqrst - https://script.google.com/d/s2/edit\n\
         abcdefghijklmnopqrs… - https://script.google.com/d/s3/edit\n\
         my super long scrip… - https://script.google.com/d/s4/edit\n"
    );
}

#[tokio::test]
async fn list_scripts_no_shorten_prints_raw_names() {
    let server = MockServer::start().await;
    mount_scripts(&server).await;
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_scripts(&client, true, &Ui::new(TestPrompt::default()), &mut output)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Found 4 scripts.\n\
         script 1 - https://script.google.com/d/s1/edit\n\
         abcdefghijklmnopqrst - https://script.google.com/d/s2/edit\n\
         abcdefghijklmnopqrstu - https://script.google.com/d/s3/edit\n\
         my super long script name here - https://script.google.com/d/s4/edit\n"
    );
}

#[tokio::test]
async fn list_scripts_json_prints_raw_entries() {
    let server = MockServer::start().await;
    mount_scripts(&server).await;
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    list_scripts(&client, false, &Ui::new(TestPrompt::default()), &mut output)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "[\n  {\n    \"id\": \"s1\",\n    \"name\": \"script 1\"\n  },\n  \
         {\n    \"id\": \"s2\",\n    \"name\": \"abcdefghijklmnopqrst\"\n  },\n  \
         {\n    \"id\": \"s3\",\n    \"name\": \"abcdefghijklmnopqrstu\"\n  },\n  \
         {\n    \"id\": \"s4\",\n    \"name\": \"my super long script name here\"\n  }\n]\n"
    );
}

#[tokio::test]
async fn list_scripts_empty_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    list_scripts(&client, false, &Ui::new(TestPrompt::default()), &mut output)
        .await
        .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "No script files found.\n");
}

// ---------------------------------------------------------------------------
// delete-script
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_script_force_trashes_and_reports() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/files/script"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let result = delete_script(
        &client,
        &config,
        None,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert!(result.deleted);
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PATCH");
    assert!(url_of(&requests[0]).ends_with("/v3/files/script"));
    assert_eq!(body_of(&requests[0]).await, json!({"trashed": true}));
    assert_eq!(String::from_utf8(out).unwrap(), "Deleted script script\n");
}

#[tokio::test]
async fn delete_script_json_emits_success() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/files/script"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(true, &mut out, Vec::new());
    delete_script(
        &client,
        &config,
        None,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"success\": true\n}\n"
    );
}

#[tokio::test]
async fn delete_script_noninteractive_without_force_is_silent_noop() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    for json in [false, true] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut output = Output::new(json, &mut out, &mut err);
        let result = delete_script(
            &client,
            &config,
            None,
            false,
            &Ui::new(TestPrompt::default()),
            &mut output,
        )
        .await
        .unwrap();
        assert!(!result.deleted);
        assert_eq!(String::from_utf8(out).unwrap(), "");
        assert_eq!(String::from_utf8(err).unwrap(), "");
    }
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn delete_script_interactive_confirm_gates_deletion() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/files/script"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());

    // Confirmed: trashed.
    let mut prompt = TestPrompt::interactive();
    prompt.confirm_answer = true;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let result = delete_script(&client, &config, None, false, &Ui::new(prompt), &mut output)
        .await
        .unwrap();
    assert!(result.deleted);
    assert_eq!(String::from_utf8(out).unwrap(), "Deleted script script\n");

    // Declined: silent no-op.
    let mut prompt = TestPrompt::interactive();
    prompt.confirm_answer = false;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let result = delete_script(&client, &config, None, false, &Ui::new(prompt), &mut output)
        .await
        .unwrap();
    assert!(!result.deleted);
    assert_eq!(String::from_utf8(out).unwrap(), "");
}

#[tokio::test]
async fn delete_script_without_script_id_errors() {
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = default_config(temp.path()).await;
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    let error = delete_script(
        &client,
        &config,
        None,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Script ID not set, unable to delete the script."
    );
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn delete_script_explicit_script_id_overrides_config() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/files/other"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let client = api_client(&server.uri());
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = Output::new(false, &mut out, Vec::new());
    delete_script(
        &client,
        &config,
        Some("other"),
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "Deleted script other\n");
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

#[test]
fn extract_script_id_matches_clasp_regex() {
    assert_eq!(
        extract_script_id("https://script.google.com/d/1AbC_def-123/edit#slide1"),
        "1AbC_def-123"
    );
    assert_eq!(extract_script_id("my-id"), "my-id");
    assert_eq!(extract_script_id("  spaced-id  "), "spaced-id");
    assert_eq!(
        extract_script_id("https://script.google.com/d/xyz/u/1"),
        "xyz"
    );
    // A URL that does not match the pattern is treated as an ID.
    assert_eq!(
        extract_script_id("https://script.google.com/macros/d/exec"),
        "https://script.google.com/macros/d/exec"
    );
}

#[test]
fn ellipsize_truncates_on_space_and_pads() {
    // Longer than 20 with no space near the boundary: cut at 19 + ellipsis.
    assert_eq!(
        ellipsize("abcdefghijklmnopqrstu", 20),
        "abcdefghijklmnopqrs…"
    );
    // Exactly 20: unchanged.
    assert_eq!(
        ellipsize("abcdefghijklmnopqrst", 20),
        "abcdefghijklmnopqrst"
    );
    // Short names are padded to 20 characters (clasp's padEnd).
    assert_eq!(ellipsize("script 1", 20), "script 1            ");
    // Prefers truncation on a space within 3 positions left of the boundary.
    assert_eq!(
        ellipsize("aaaaaaaaaaaaaaaaa bbb", 20),
        "aaaaaaaaaaaaaaaaa…  "
    );
    // A space exactly at the boundary is replaced by the ellipsis.
    assert_eq!(
        ellipsize("this is a very long name", 20),
        "this is a very long…"
    );
    assert_eq!(
        ellipsize("my super long script name here", 20),
        "my super long scrip…"
    );
}
