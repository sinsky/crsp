use std::fs;
use std::path::{Path, PathBuf};

use google_clasp_rs::api::{ApiClient, ApiClientConfig, BaseUrls};
use google_clasp_rs::commands::show_file_status::show_file_status;
use google_clasp_rs::commands::{pull::pull as pull_command, push::push as push_command};
use google_clasp_rs::core::config::ProjectConfig;
use google_clasp_rs::core::files::{
    LocalExtensions, LocalFile, PullFile, PullFileFailure, SkipReason, WriteFault,
    collect_local_files, get_changed_files, pull_file, pull_file_with_fault, pull_files,
};
use google_clasp_rs::output::Output;
use google_clasp_rs::ui::{
    PromptAdapter, PromptConfirm, PromptDialog, PromptInput, PromptMultiSelect, PromptSelect,
    PromptSpinner, Ui,
};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    answer: bool,
    confirmations: std::cell::RefCell<Vec<String>>,
}
impl PromptAdapter for TestPrompt {
    fn is_interactive(&self) -> bool {
        self.interactive
    }
    fn input(&self, _: &PromptInput) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn select(&self, _: &PromptSelect) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn multi_select(&self, _: &PromptMultiSelect) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn confirm(&self, spec: &PromptConfirm) -> std::io::Result<bool> {
        self.confirmations.borrow_mut().push(spec.prompt.clone());
        Ok(self.answer)
    }
    fn dialog(&self, _: &PromptDialog) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn spinner<T, F: FnOnce() -> T>(&self, _: PromptSpinner, f: F) -> std::io::Result<T> {
        Ok(f())
    }
}

fn config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        config_file_path: root.join(".clasp.json"),
        project_root_dir: root.to_path_buf(),
        content_dir: root.join("src"),
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

#[tokio::test]
async fn push_zero_changed_issues_no_put_and_prints_up_to_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"files":[{"name":"Code","type":"SERVER_JS","source":"same"}]}),
        ))
        .mount(&server)
        .await;
    let put = Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200));
    put.mount(&server).await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "same").unwrap();
    let client = api_client(&server.uri());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let result = push_command(
        &client,
        &config(temp.path()),
        temp.path(),
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert!(result.up_to_date);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Script is already up to date.\n"
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "PUT")
            .count(),
        0
    );
}

#[tokio::test]
async fn push_changed_puts_all_files_with_normalized_names_and_manifest_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/projects/script/content")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files":[{"name":"nested/Keep","type":"SERVER_JS","source":"same"}]}))).mount(&server).await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("nested")).unwrap();
    fs::write(src.join("nested/Keep.js"), "same").unwrap();
    fs::write(src.join("nested/Changed.gs"), "changed").unwrap();
    fs::write(src.join("appsscript.json"), "{\"timeZone\":\"UTC\"}").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    push_command(
        &api_client(&server.uri()),
        &config(temp.path()),
        temp.path(),
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    let requests = server.received_requests().await.unwrap();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&put.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"files":[{"name":"appsscript","type":"JSON","source":"{\"timeZone\":\"UTC\"}"},{"name":"nested/Changed","type":"SERVER_JS","source":"changed"},{"name":"nested/Keep","type":"SERVER_JS","source":"same"}]})
    );
}

#[tokio::test]
async fn syntax_error_api_response_is_extracted_with_snippet() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files":[]})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"error":{"message":"Syntax error: Missing ; line: 2 file: Code"}}),
        ))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "one\ntwo\nthree").unwrap();
    let result =
        google_clasp_rs::core::files::push_files(&api_client(&server.uri()), &config(temp.path()))
            .await
            .unwrap_err();
    let message = result.to_string();
    let files = collect_local_files(&config(temp.path()))
        .await
        .unwrap()
        .files;
    let snippet = google_clasp_rs::core::files::syntax_error_snippet(&message, &files).unwrap();
    assert!(snippet.contains("Missing ;"));
    assert!(snippet.contains("=> two"));
}

#[tokio::test]
async fn prepare_push_rejects_missing_script_id_before_local_collection() {
    // The script-id assert must run before the local tree walk: collecting
    // first makes unconfigured runs (e.g. `crsp push` with no `.clasp.json`)
    // scan the whole content dir — including huge build outputs — before
    // failing. Observable: a broken ignore file must NOT surface when the
    // script id is missing.
    let server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let mut unconfigured = config(temp.path());
    unconfigured.script_id = None;
    unconfigured.ignore_file_path = Some(temp.path().join("missing.claspignore"));
    let error =
        google_clasp_rs::core::files::prepare_push(&api_client(&server.uri()), &unconfigured)
            .await
            .unwrap_err();
    assert_eq!(error.to_string(), "Project settings not found.");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no request must be sent"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn existing_target_mode_is_preserved_on_pull_overwrite() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("Code.js");
    fs::write(&target, "old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    pull_file(
        &src,
        false,
        &PullFile::new("Code", "SERVER_JS", "new"),
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn locale_sort_vector_matches_ruling() {
    let mut files = vec![
        LocalFile::new("Code.gs", "Code", "SERVER_JS", ""),
        LocalFile::new("appsscript.json", "appsscript", "JSON", ""),
        LocalFile::new("Bar.gs", "Bar", "SERVER_JS", ""),
    ];
    google_clasp_rs::core::files::sort_for_test(&mut files, &[]);
    assert_eq!(
        files
            .iter()
            .map(|file| file.local_path.as_str())
            .collect::<Vec<_>>(),
        ["appsscript.json", "Bar.gs", "Code.gs"]
    );
}

#[tokio::test]
async fn default_ignore_tracks_ts_then_marks_it_unsupported() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "function code() {}\n").unwrap();
    fs::write(src.join("types.ts"), "type X = string;\n").unwrap();
    fs::write(src.join("appsscript.json"), "{}\n").unwrap();

    let result = collect_local_files(&config(temp.path())).await.unwrap();
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].remote_path, "appsscript");
    assert!(result
        .skipped
        .iter()
        .any(|item| item.local_path == "types.ts" && item.reason == SkipReason::UnsupportedType));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_are_skipped_or_followed_by_configuration() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(temp.path().join("outside.js"), "outside").unwrap();
    symlink(temp.path().join("outside.js"), src.join("link.js")).unwrap();

    let denied = collect_local_files(&config(temp.path())).await.unwrap();
    assert!(
        denied
            .skipped
            .iter()
            .any(|item| item.local_path == "link.js" && item.reason == SkipReason::Symlink)
    );

    let mut allowed_config = config(temp.path());
    allowed_config.allow_symlinks = true;
    let allowed = collect_local_files(&allowed_config).await.unwrap();
    assert!(
        allowed
            .files
            .iter()
            .any(|file| file.local_path == "link.js")
    );
}

#[tokio::test]
async fn changed_files_ignore_remote_only_files() {
    let local = vec![
        LocalFile::new("Code.js", "Code", "SERVER_JS", "new"),
        LocalFile::new("same.html", "same", "HTML", "same"),
    ];
    let remote = vec![
        PullFile::new("Code", "SERVER_JS", "old"),
        PullFile::new("same", "HTML", "same"),
        PullFile::new("deleted", "SERVER_JS", "remote-only"),
    ];

    let changed = get_changed_files(&local, &remote);
    assert_eq!(
        changed
            .iter()
            .map(|file| file.local_path.as_str())
            .collect::<Vec<_>>(),
        ["Code.js"]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pull_writes_directly_with_mode_0644_and_jails_remote_paths() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let content = temp.path().join("src");
    fs::create_dir_all(&content).unwrap();
    let written = pull_file(
        &content,
        false,
        &PullFile::new("nested/Code", "SERVER_JS", "source"),
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert_eq!(written, Some("nested/Code.js".to_string()));
    let target = content.join("nested/Code.js");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "source");

    let skipped = pull_file(
        &content,
        false,
        &PullFile::new("../evil", "SERVER_JS", "bad"),
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert_eq!(skipped, None);
}

#[cfg(unix)]
#[tokio::test]
async fn pull_rejects_target_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let content = temp.path().join("src");
    fs::create_dir_all(&content).unwrap();
    fs::write(temp.path().join("outside"), "original").unwrap();
    symlink(temp.path().join("outside"), content.join("Code.js")).unwrap();

    let result = pull_file(
        &content,
        false,
        &PullFile::new("Code", "SERVER_JS", "changed"),
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert_eq!(result, None);
    assert_eq!(
        fs::read_to_string(temp.path().join("outside")).unwrap(),
        "original"
    );
}

#[tokio::test]
async fn server_js_collisions_are_rejected_and_push_order_is_preserved() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "js").unwrap();
    fs::write(src.join("Code.gs"), "gs").unwrap();
    let error = collect_local_files(&config(temp.path())).await.unwrap_err();
    assert!(error.to_string().contains("Conflicting files found"));

    fs::remove_file(src.join("Code.gs")).unwrap();
    fs::remove_file(src.join("Code.js")).unwrap();
    fs::write(src.join("z.js"), "z").unwrap();
    fs::write(src.join("a.js"), "a").unwrap();
    let mut configured = config(temp.path());
    configured.file_push_order = vec!["z.js".to_string()];
    let result = collect_local_files(&configured).await.unwrap();
    assert_eq!(
        result
            .files
            .iter()
            .map(|file| file.local_path.as_str())
            .collect::<Vec<_>>(),
        ["z.js", "a.js"]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn special_files_are_skipped_as_unsupported() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let fifo = src.join("pipe.js");
    std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    let result = collect_local_files(&config(temp.path())).await.unwrap();
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item.local_path == "pipe.js" && item.reason == SkipReason::UnsupportedType)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn parent_symlinks_are_skipped_without_writing_outside() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, src.join("nested")).unwrap();
    let result = pull_file(
        &src,
        false,
        &PullFile::new("nested/Code", "SERVER_JS", "bad"),
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert_eq!(result, None);
    assert!(!outside.join("Code.js").exists());
}

#[test]
fn syntax_error_extraction_includes_source_line() {
    let files = vec![LocalFile::new(
        "Code.js",
        "Code",
        "SERVER_JS",
        "one\ntwo\nthree",
    )];
    let snippet = google_clasp_rs::core::files::syntax_error_snippet(
        "Syntax error: Missing ; line: 2 file: Code",
        &files,
    )
    .unwrap();
    assert!(snippet.contains("Missing ; - \"Code:2\""));
    assert!(snippet.contains("=> two"));
}

#[tokio::test]
async fn concurrent_writers_share_deep_parent_safely() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let files = (0..64)
        .map(|index| PullFile::new(&format!("deep/shared/file{index}"), "SERVER_JS", "source"))
        .collect::<Vec<_>>();
    let result = pull_files(&files, &src, false, 32, &LocalExtensions::clasp_defaults())
        .await
        .unwrap();
    assert_eq!(result.written.len(), 64);
    assert!(src.join("deep/shared/file63.js").exists());
}

#[tokio::test]
async fn pull_uses_configured_extensions_and_round_trips() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let mut project = config(temp.path());
    // clasp getFileExtension resolves the FIRST configured extension.
    project.script_extensions = vec![".gs".to_string()];
    let files = vec![
        PullFile::new("Code", "SERVER_JS", "code"),
        PullFile::new("appsscript", "JSON", "{}"),
    ];
    let result = pull_files(
        &files,
        &src,
        false,
        32,
        &LocalExtensions::from_config(&project),
    )
    .await
    .unwrap();
    assert_eq!(
        result.written,
        vec!["Code.gs".to_string(), "appsscript.json".to_string()]
    );
    assert!(src.join("Code.gs").exists());
    assert!(!src.join("Code.js").exists());

    // Round-trip: the written tree is collected for push with the same
    // configuration (no data loss on the next full-replacement PUT).
    let collected = collect_local_files(&project).await.unwrap();
    assert_eq!(
        collected
            .files
            .iter()
            .map(|file| file.local_path.as_str())
            .collect::<Vec<_>>(),
        ["appsscript.json", "Code.gs"]
    );
    let code = collected
        .files
        .iter()
        .find(|f| f.local_path == "Code.gs")
        .unwrap();
    assert_eq!(code.remote_path, "Code");
    assert_eq!(code.file_type, "SERVER_JS");
}

#[tokio::test]
async fn pull_deletion_force_and_confirm_paths_report_deleted_files() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("old.js"), "old").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    let ui = Ui::new(TestPrompt {
        interactive: true,
        answer: true,
        confirmations: std::cell::RefCell::new(Vec::new()),
    });
    let result = pull_command(
        &config(temp.path()),
        temp.path(),
        &[],
        true,
        false,
        &ui,
        &mut output,
    )
    .await
    .unwrap();
    assert!(result.deleted.contains(&"old.js".to_string()));
    assert!(!src.join("old.js").exists());
    assert!(String::from_utf8(out).unwrap().contains("deleted"));
}

#[tokio::test]
async fn pull_deletion_prompts_once_per_file() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("first.js"), "first").unwrap();
    fs::write(src.join("second.js"), "second").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let prompt = TestPrompt {
        interactive: true,
        answer: true,
        confirmations: std::cell::RefCell::new(Vec::new()),
    };
    let ui = Ui::new(prompt);
    let result = pull_command(
        &config(temp.path()),
        temp.path(),
        &[],
        true,
        false,
        &ui,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(result.deleted.len(), 2);
    assert!(!src.join("first.js").exists());
    assert!(!src.join("second.js").exists());
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("Deleted src/first.js"));
    assert!(stdout.contains("Deleted src/second.js"));
    let confirmations = ui.into_adapter().confirmations.into_inner();
    assert_eq!(confirmations.len(), 2);
    assert!(confirmations.contains(&"Delete first.js?".to_string()));
    assert!(confirmations.contains(&"Delete second.js?".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn pull_deletion_fails_with_security_error_on_symlinked_content_dir() {
    use std::os::unix::fs::symlink;

    // clasp pull.ts:148-153: a symlinked content dir fails before any prompt
    // with `Security Error: Content directory is a symlink`.
    let temp = TempDir::new().unwrap();
    let real = temp.path().join("real");
    let content = temp.path().join("content");
    fs::create_dir_all(&real).unwrap();
    fs::write(real.join("old.js"), "old").unwrap();
    symlink(&real, &content).unwrap();
    let mut config = config(temp.path());
    config.content_dir = content.clone();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let error = pull_command(
        &config,
        temp.path(),
        &[],
        true,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .expect_err("clasp fails the command with a Security Error");
    assert!(error.to_string().contains("Content directory is a symlink"));
    assert!(real.join("old.js").exists());
}

#[tokio::test]
async fn pull_noninteractive_deletion_warns_and_preserves_files() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("old.js"), "old").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let result = pull_command(
        &config(temp.path()),
        temp.path(),
        &[],
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert!(result.deleted.is_empty());
    assert!(src.join("old.js").exists());
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "You are not in an interactive terminal and --force not used. Skipping file deletion.\n"
    );
}

#[tokio::test]
async fn noninteractive_manifest_change_is_rejected_before_put() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/projects/script/content")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files":[{"name":"appsscript","type":"JSON","source":"{\"old\":true}"}]}))).mount(&server).await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("appsscript.json"), "{\"new\":true}").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let result = push_command(
        &api_client(&server.uri()),
        &config(temp.path()),
        temp.path(),
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "Skipping push.\n");
    assert!(!result.up_to_date);
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "PUT")
            .count(),
        0
    );
}

#[tokio::test]
async fn public_pull_result_preserves_all_skip_reasons() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, src.join("parent")).unwrap();
    fs::write(src.join("target.js"), "before").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(src.join("target.js"), src.join("link.js")).unwrap();
    let files = vec![
        PullFile::new("../evil", "SERVER_JS", "x"),
        PullFile::new("parent/child", "SERVER_JS", "x"),
        PullFile::new("link", "SERVER_JS", "x"),
    ];
    let result = pull_files(&files, &src, false, 32, &LocalExtensions::clasp_defaults())
        .await
        .unwrap();
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item.reason == SkipReason::OutsideContentDir)
    );
    #[cfg(unix)]
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item.reason == SkipReason::ParentSymlink)
    );
    #[cfg(unix)]
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item.reason == SkipReason::TargetSymlink)
    );
    assert_eq!(
        pull_file_with_fault(
            &src,
            false,
            &PullFile::new("race", "SERVER_JS", "x"),
            Some(WriteFault::RaceCondition),
            &LocalExtensions::clasp_defaults(),
        ),
        Err(PullFileFailure::Skipped(SkipReason::RaceCondition))
    );
    assert_eq!(
        pull_file_with_fault(
            &src,
            false,
            &PullFile::new("loop", "SERVER_JS", "x"),
            Some(WriteFault::SymlinkLoop),
            &LocalExtensions::clasp_defaults(),
        ),
        Err(PullFileFailure::Skipped(SkipReason::SymlinkLoop))
    );
    assert_eq!(fs::read_to_string(src.join("target.js")).unwrap(), "before");
}

#[cfg(unix)]
#[tokio::test]
async fn real_eloop_is_reported_as_symlink_loop() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    symlink(src.join("x.js"), src.join("t.js")).unwrap();
    symlink(src.join("x.js"), src.join("x.js")).unwrap();
    let result = pull_files(
        &[PullFile::new("t", "SERVER_JS", "source")],
        &src,
        true,
        32,
        &LocalExtensions::clasp_defaults(),
    )
    .await
    .unwrap();
    assert!(
        result
            .skipped
            .iter()
            .any(|item| item.reason == SkipReason::SymlinkLoop)
    );
}

#[tokio::test]
async fn fixture_tree_matches_expected_snapshot() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/files/source");
    let expected = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/files/expected.txt"),
    )
    .unwrap();
    let mut configured = config(fixture.parent().unwrap());
    configured.content_dir = fixture;
    let result = collect_local_files(&configured).await.unwrap();
    let actual = result
        .files
        .iter()
        .map(|file| file.local_path.clone())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn status_compresses_untracked_files_to_common_parent() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("node_modules/lib")).unwrap();
    fs::write(src.join("tracked.js"), "tracked").unwrap();
    fs::write(src.join("node_modules/lib/a.txt"), "a").unwrap();
    fs::write(src.join("node_modules/lib/b.txt"), "b").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    show_file_status(
        temp.path(),
        &config(temp.path()),
        &Ui::new(google_clasp_rs::ui::DemandAdapter),
        &mut output,
    )
    .await
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        json["untrackedFiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "src/node_modules/"),
        "clasp collapses to the nearest untracked parent (cwd-relative): {:?}",
        json["untrackedFiles"]
    );
}

#[tokio::test]
async fn allow_symlinks_follows_directory_links() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("linked.js"), "linked").unwrap();
        symlink(&outside, src.join("linked")).unwrap();
        let mut configured = config(temp.path());
        configured.allow_symlinks = true;
        let result = collect_local_files(&configured).await.unwrap();
        assert!(
            result
                .files
                .iter()
                .any(|file| file.local_path == "linked/linked.js")
        );
    }
}

#[tokio::test]
async fn allow_symlinks_handles_circular_symlinks_without_infinite_loop() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let src = temp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("code.js"), "code").unwrap();
        symlink(&src, src.join("loop")).unwrap();
        let mut configured = config(temp.path());
        configured.allow_symlinks = true;
        let result = collect_local_files(&configured).await.unwrap();
        assert!(result.files.iter().any(|file| file.local_path == "code.js"));
    }
}

#[test]
fn remote_names_use_forward_slashes_in_local_and_payload_models() {
    let file = LocalFile::new("nested\\Code.gs", "nested/Code", "SERVER_JS", "source");
    assert_eq!(file.remote_path, "nested/Code");
    assert_eq!(
        PathBuf::from("nested/Code.gs").to_string_lossy(),
        "nested/Code.gs"
    );
}

// ---------------------------------------------------------------------------
// clasp display parity (clasp commands/pull.ts, push.ts, show-file-status.ts,
// core/files.ts cwd-relative localPath; golden-gate divergence fixes)
// ---------------------------------------------------------------------------

fn src_config(root: &Path) -> ProjectConfig {
    let mut configured = config(root);
    configured.content_dir = root.join("src");
    configured
}

#[tokio::test]
async fn push_json_no_change_prints_empty_array_like_clasp() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"files":[{"name":"Code","type":"SERVER_JS","source":"same"}]}),
        ))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "same").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    push_command(
        &api_client(&server.uri()),
        &src_config(temp.path()),
        temp.path(),
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp push.ts: JSON with no pending changes prints an empty array, not
    // the human up-to-date line.
    assert_eq!(String::from_utf8(out).unwrap(), "[]\n");
}

#[tokio::test]
async fn push_success_prints_pushed_line_and_cwd_relative_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"files":[{"name":"Code","type":"SERVER_JS","source":"old"}]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "new").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    push_command(
        &api_client(&server.uri()),
        &src_config(temp.path()),
        temp.path(),
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp push.ts: `Pushed {count, plural, ...} at {time}.` plus one
    // `└─ {cwd-relative path}` line per file.
    let stdout = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout:?}");
    assert!(
        lines[0].starts_with("Pushed one file at ") && lines[0].ends_with('.'),
        "first line: {}",
        lines[0]
    );
    let timestamp = lines[0]["Pushed one file at ".len()..lines[0].len() - 1].to_string();
    assert!(
        regex::Regex::new(r"^\d{1,2}:\d{2}:\d{2} (AM|PM)$")
            .unwrap()
            .is_match(&timestamp),
        "clasp `toLocaleTimeString()` en-US style: {timestamp:?}"
    );
    assert_eq!(lines[1], "└─ src/Code.js");
}

#[tokio::test]
async fn push_json_lists_cwd_relative_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"files":[{"name":"Code","type":"SERVER_JS","source":"old"}]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "new").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    push_command(
        &api_client(&server.uri()),
        &src_config(temp.path()),
        temp.path(),
        true,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "[\n  \"src/Code.js\"\n]\n");
}

#[tokio::test]
async fn status_paths_are_cwd_relative_like_clasp() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("node_modules/lib")).unwrap();
    fs::write(src.join("Code.js"), "code").unwrap();
    fs::write(src.join("appsscript.json"), "{}").unwrap();
    fs::write(src.join("node_modules/lib/a.txt"), "a").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    show_file_status(
        temp.path(),
        &src_config(temp.path()),
        &Ui::new(google_clasp_rs::ui::DemandAdapter),
        &mut output,
    )
    .await
    .unwrap();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains("└─ src/appsscript.json") && stdout.contains("└─ src/Code.js"),
        "cwd-relative display (clasp `path.relative(cwd, …)`): {stdout:?}"
    );
    assert!(
        !stdout.contains("└─ appsscript.json"),
        "contentDir-relative display is wrong: {stdout:?}"
    );
}

#[tokio::test]
async fn pull_prints_files_and_pulled_count_like_clasp() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {"name": "Code", "type": "SERVER_JS", "source": "// hello\n"},
                {"name": "appsscript", "type": "JSON", "source": "{}"}
            ]
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join(".clasp.json"),
        "{\"scriptId\":\"script\",\"rootDir\":\"src\"}",
    )
    .unwrap();
    let remote: Vec<google_clasp_rs::core::project::RemoteFile> = vec![
        google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("Code", "SERVER_JS", "// hello\n"),
            local_path: "src/Code.js".to_string(),
        },
        google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("appsscript", "JSON", "{}"),
            local_path: "src/appsscript.json".to_string(),
        },
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ src/Code.js\n└─ src/appsscript.json\nPulled 2 files.\n"
    );
}

#[tokio::test]
async fn pull_delete_prints_deleted_lines_and_cwd_relative_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [{"name": "Code", "type": "SERVER_JS", "source": "// hello\n"}]
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Unused.js"), "unused").unwrap();
    let remote: Vec<google_clasp_rs::core::project::RemoteFile> =
        vec![google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("Code", "SERVER_JS", "// hello\n"),
            local_path: "src/Code.js".to_string(),
        }];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        true,
        true,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp pull.ts: the Deleted line comes from the unused-file sweep before
    // the pulled-file listing.
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Deleted src/Unused.js\n└─ src/Code.js\nPulled one file.\n"
    );
    assert!(!src.join("Unused.js").exists());
}

#[tokio::test]
async fn pull_json_lists_cwd_relative_pulled_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [{"name": "Code", "type": "SERVER_JS", "source": "// hello\n"}]
        })))
        .mount(&server)
        .await;
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    let remote: Vec<google_clasp_rs::core::project::RemoteFile> =
        vec![google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("Code", "SERVER_JS", "// hello\n"),
            local_path: "src/Code.js".to_string(),
        }];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"pulledFiles\": [\n    \"src/Code.js\"\n  ],\n  \"deletedFiles\": []\n}\n"
    );
}

// ---------------------------------------------------------------------------
// pull skip warnings (clasp pull.ts:49-91; spec §9.4 stderr wording)
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn pull_warns_on_collect_time_symlinks_like_clasp() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(temp.path().join("outside.js"), "outside").unwrap();
    symlink(temp.path().join("outside.js"), src.join("link.js")).unwrap();
    let remote: Vec<google_clasp_rs::core::project::RemoteFile> =
        vec![google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("Code", "SERVER_JS", "// hello\n"),
            local_path: "src/Code.js".to_string(),
        }];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp pull.ts:57 — the collect-time symlink skip warns on stderr with
    // the cwd-relative path; the pull itself still succeeds.
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "Security Warning: Skipping symbolic link src/link.js. Symbolic links are not supported.\n"
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ src/Code.js\nPulled one file.\n"
    );
    assert!(src.join("Code.js").exists());

    // JSON mode never warns (clasp `if (!options.json && ...)`).
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(true, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(String::from_utf8(err).unwrap(), "");
}

#[cfg(unix)]
#[tokio::test]
async fn pull_warns_on_write_skips_like_clasp() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, src.join("nested")).unwrap();
    let remote: Vec<google_clasp_rs::core::project::RemoteFile> =
        vec![google_clasp_rs::core::project::RemoteFile {
            file: PullFile::new("nested/Code", "SERVER_JS", "bad"),
            local_path: "src/nested/Code.js".to_string(),
        }];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    pull_command(
        &src_config(temp.path()),
        temp.path(),
        &remote,
        false,
        false,
        &Ui::new(TestPrompt::default()),
        &mut output,
    )
    .await
    .unwrap();
    // clasp pull.ts:80-90 — every write skip warns with the clasp reason
    // text; nothing is written outside the jail.
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "Security Warning: Skipping write of src/nested/Code.js (parent directory contains a symbolic link).\n"
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "└─ src/nested/Code.js\nPulled one file.\n"
    );
    assert!(!outside.join("Code.js").exists());
}

#[test]
fn skip_reason_stderr_wording_matches_clasp() {
    // clasp pull.ts:81-90 / clone-script.ts:107-117 reason mapping (spec
    // §9.4: every skip reason's stderr wording pinned).
    use google_clasp_rs::commands::shared::write_skip_reason_text;
    use google_clasp_rs::core::files::SkipReason;
    assert_eq!(
        write_skip_reason_text(SkipReason::ParentSymlink),
        "parent directory contains a symbolic link"
    );
    assert_eq!(
        write_skip_reason_text(SkipReason::TargetSymlink),
        "target path is a symbolic link"
    );
    for reason in [
        SkipReason::Symlink,
        SkipReason::UnsupportedType,
        SkipReason::OutsideContentDir,
        SkipReason::RaceCondition,
        SkipReason::SymlinkLoop,
    ] {
        assert_eq!(
            write_skip_reason_text(reason),
            "outside project directory or unsafe race condition detected"
        );
    }
}
