//! Regression tests for `cargo mutants` missed mutants in `src/core`
//! (apis.rs, clasp.rs, config.rs).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use google_clasp_rs::constants::PROJECT_CONFIG_FILENAME;
use google_clasp_rs::core::apis::AdvancedService;
use google_clasp_rs::core::clasp::Clasp;
use google_clasp_rs::core::config::ProjectConfig;
use google_clasp_rs::error::CrspError;

fn temp_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    (dir, root)
}

static API_BASE_ENV_LOCK: Mutex<()> = Mutex::new(());
const API_ENV_VARS: [&str; 3] = [
    "CRSP_API_BASE_URL",
    "CRSP_SCRIPT_API_BASE_URL",
    "CRSP_OAUTH2_BASE_URL",
];

#[allow(dead_code)]
struct ApiBaseEnvGuard(std::sync::MutexGuard<'static, ()>);

impl Drop for ApiBaseEnvGuard {
    fn drop(&mut self) {
        for name in API_ENV_VARS {
            unsafe { std::env::remove_var(name) };
        }
    }
}

fn api_base_env_guard(url: &str) -> ApiBaseEnvGuard {
    let guard = API_BASE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for name in API_ENV_VARS {
        unsafe { std::env::set_var(name, url) };
    }
    ApiBaseEnvGuard(guard)
}

async fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, content).await.unwrap();
}

// ---------------------------------------------------------------------------
// apis.rs:187,189 — by_service_id must return None / use ==
// ---------------------------------------------------------------------------

#[test]
fn by_service_id_returns_none_for_unknown_id() {
    assert!(AdvancedService::by_service_id("no-such-service").is_none());
    assert!(AdvancedService::by_service_id("").is_none());
}

#[test]
fn by_service_id_matches_exact_id_only() {
    let sheets = AdvancedService::by_service_id("sheets").unwrap();
    assert_eq!(sheets.user_symbol, "Sheets");
    // Case differs -> no match (==, not a case-insensitive compare).
    assert!(AdvancedService::by_service_id("Sheets").is_none());
    assert!(AdvancedService::by_service_id("SHEETS").is_none());
}

// ---------------------------------------------------------------------------
// clasp.rs:51 — allow_symlinks is OR, not AND
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_context_or_combines_cli_and_config_allow_symlinks() {
    // Config true + CLI false -> true (|| keeps it; && would drop it).
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "allowSymlinks": true }"#,
    )
    .await;
    let context = Clasp::init_context(
        Some(&root.join(PROJECT_CONFIG_FILENAME)),
        None,
        None,
        "u",
        false,
        false,
    )
    .await
    .unwrap();
    assert!(context.config.allow_symlinks);
    drop(dir);

    // Config false + CLI true -> true.
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s" }"#,
    )
    .await;
    let context = Clasp::init_context(
        Some(&root.join(PROJECT_CONFIG_FILENAME)),
        None,
        None,
        "u",
        false,
        true,
    )
    .await
    .unwrap();
    assert!(context.config.allow_symlinks);
    drop(dir);

    // Both false -> false.
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s" }"#,
    )
    .await;
    let context = Clasp::init_context(
        Some(&root.join(PROJECT_CONFIG_FILENAME)),
        None,
        None,
        "u",
        false,
        false,
    )
    .await
    .unwrap();
    assert!(!context.config.allow_symlinks);
    drop(dir);
}

// ---------------------------------------------------------------------------
// clasp.rs:63 — credentials kept unless BOTH tokens are missing/empty
// ---------------------------------------------------------------------------

fn store_with(tokens_json: &str) -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = temp_root();
    let path = root.join(".clasprc.json");
    std::fs::write(
        &path,
        format!("{{\"tokens\": {{\"default\": {tokens_json}}}}}"),
    )
    .unwrap();
    (dir, path)
}

#[tokio::test]
async fn init_context_keeps_entry_missing_access_but_having_refresh() {
    // access missing + refresh present -> kept (&& requires both missing).
    let (_dir, store) = store_with(r#"{"refresh_token": "R"}"#);
    let context = Clasp::init_context(None, None, Some(&store), "default", false, false)
        .await
        .unwrap();
    let credentials = context.credentials.as_ref().expect("entry kept");
    assert_eq!(credentials.refresh_token.as_deref(), Some("R"));
}

#[tokio::test]
async fn init_context_discards_entry_missing_both_tokens() {
    // access missing + refresh missing -> discarded.
    let (_dir, store) = store_with(r#"{"type": "authorized_user"}"#);
    let context = Clasp::init_context(None, None, Some(&store), "default", false, false)
        .await
        .unwrap();
    assert!(context.credentials.is_none());
}

#[tokio::test]
async fn init_context_keeps_entry_with_access_but_no_refresh() {
    // access present + refresh missing -> kept (|| would discard it).
    let (_dir, store) = store_with(r#"{"access_token": "A"}"#);
    let context = Clasp::init_context(None, None, Some(&store), "default", false, false)
        .await
        .unwrap();
    let credentials = context.credentials.as_ref().expect("entry kept");
    assert_eq!(credentials.access_token.as_deref(), Some("A"));
}

// ---------------------------------------------------------------------------
// clasp.rs:111 — token_expiry cell must be populated from stored expiry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_context_wires_stored_expiry_into_pre_send_refresh() {
    // A stored access token with an already-past expiry must trigger a
    // pre-send refresh on first use (expiry cell wired via token_expiry).
    // Deleting `token_expiry: Some(...)` would send the stale token instead.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"access_token": "FRESH", "expires_in": 3600})),
        )
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"files": [{"name": "Code", "type": "SERVER_JS", "source": "// ok\n"}]}),
        ))
        .mount(&server)
        .await;
    let _env = api_base_env_guard(&server.uri());
    let past = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 60_000;
    let (_dir, store) = store_with(&format!(
        r#"{{"access_token": "STALE", "refresh_token": "R", "expiry_date": {past}}}"#
    ));
    let context = Clasp::init_context(None, None, Some(&store), "default", false, false)
        .await
        .unwrap();
    context
        .client
        .script()
        .get_content("script", None)
        .await
        .expect("expired token refreshes before send");
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.iter().any(|r| r.method == "POST"),
        "expected a pre-send refresh POST, got {} request(s)",
        requests.len()
    );
}

// ---------------------------------------------------------------------------
// clasp.rs:135 (auth_path) — dir gets filename appended, file passes through
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_context_resolves_auth_dir_and_file_paths() {
    // Auth path pointing at a directory -> .clasprc.json inside it is used.
    let (dir, root) = temp_root();
    std::fs::write(
        root.join(".clasprc.json"),
        r#"{"tokens": {"default": {"access_token": "DIR-A"}}}"#,
    )
    .unwrap();
    let context = Clasp::init_context(None, None, Some(&root), "default", false, false)
        .await
        .unwrap();
    assert_eq!(
        context
            .credentials
            .as_ref()
            .and_then(|c| c.access_token.as_deref()),
        Some("DIR-A"),
        "directory auth path must resolve to .clasprc.json inside it"
    );

    // Auth path pointing at a file -> used as-is.
    let file = root.join("custom-auth.json");
    std::fs::write(
        &file,
        r#"{"tokens": {"default": {"access_token": "FILE-A"}}}"#,
    )
    .unwrap();
    let context = Clasp::init_context(None, None, Some(&file), "default", false, false)
        .await
        .unwrap();
    assert_eq!(
        context
            .credentials
            .as_ref()
            .and_then(|c| c.access_token.as_deref()),
        Some("FILE-A"),
        "file auth path must be used directly, not defaulted"
    );
    drop(dir);
}

// ---------------------------------------------------------------------------
// clasp.rs:151 (resolve_file_or_dir) — dir gets filename, file passes through
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_context_resolves_ignore_dir_and_file_paths() {
    let (dir, root) = temp_root();
    // Directory ignore path -> .claspignore inside it is honored.
    let sub = root.join("ign");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(".claspignore"), "*.log\n").unwrap();
    std::fs::write(sub.join("keep.js"), "").unwrap();
    let context = Clasp::init_context(None, Some(&sub), None, "u", false, false)
        .await
        .unwrap();
    assert_eq!(
        context.config.ignore_file_path.as_deref(),
        Some(sub.join(".claspignore").as_path()),
        "directory ignore path must resolve to .claspignore inside it"
    );

    // File ignore path -> used as-is.
    let file = root.join("custom.ignore");
    std::fs::write(&file, "*.log\n").unwrap();
    let context = Clasp::init_context(None, Some(&file), None, "u", false, false)
        .await
        .unwrap();
    assert_eq!(
        context.config.ignore_file_path.as_deref(),
        Some(file.as_path()),
        "file ignore path must be used directly, not defaulted"
    );
    drop(dir);
}

// ---------------------------------------------------------------------------
// config.rs:63 — non-NotFound IO errors must not become "invalid path"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_explicit_dir_without_config_still_discovers() {
    // An existing directory arg joins .clasp.json; a missing file inside it
    // is NotFound -> default instance, not an error.
    let (dir, root) = temp_root();
    let config = ProjectConfig::discover(Some(&root), &root).await.unwrap();
    assert_eq!(config.project_root_dir, root);
    drop(dir);
}

#[tokio::test]
async fn discover_missing_explicit_path_reports_invalid_path() {
    let missing = PathBuf::from("/nonexistent/crsp-mutants-project");
    let error = ProjectConfig::discover(Some(&missing), Path::new("/"))
        .await
        .unwrap_err();
    match error {
        CrspError::Config(message) => assert!(
            message.starts_with("Invalid --project path:"),
            "unexpected message: {message}"
        ),
        other => panic!("expected Config error, got {other:?}"),
    }
}

#[tokio::test]
async fn discover_non_notfound_io_error_is_not_invalid_path() {
    // A permission-denied metadata failure must surface as an IO error, not
    // the NotFound "Invalid --project path" (guard uses is_path_not_found).
    let (dir, root) = temp_root();
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    let target = locked.join(PROJECT_CONFIG_FILENAME);
    std::fs::write(&target, r#"{ "scriptId": "s" }"#).unwrap();
    let mut perms = std::fs::metadata(&locked).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&locked, perms).unwrap();
    // Remove all access bits (readonly alone still allows metadata read).
    std::process::Command::new("chmod")
        .args(["000", locked.to_str().unwrap()])
        .status()
        .unwrap();
    let error = ProjectConfig::discover(Some(&target), &root)
        .await
        .unwrap_err();
    std::process::Command::new("chmod")
        .args(["755", locked.to_str().unwrap()])
        .status()
        .unwrap();
    match error {
        CrspError::Io(_) => {}
        other => panic!("expected Io error, got {other:?}"),
    }
    drop(dir);
}

// ---------------------------------------------------------------------------
// config.rs:97 — parent fallback uses || (empty parent OR "." -> cwd)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_relative_config_file_resolves_root_at_cwd() {
    // A bare filename has an empty parent; load must fall back to the
    // process cwd (||), not treat "" as a real directory (&&).
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s" }"#,
    )
    .await;
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&root).unwrap();
    let config = ProjectConfig::load(Path::new(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    std::env::set_current_dir(&previous).unwrap();
    assert_eq!(
        config.project_root_dir,
        root.canonicalize().unwrap_or(root.clone())
    );
    drop(dir);
}

// ---------------------------------------------------------------------------
// config.rs:134 — canonicalize failure falls back to lexical check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_allows_missing_content_dir_when_canonicalize_fails() {
    // A not-yet-existing content dir (e.g. clone target) fails canonicalize;
    // the lexical check governs and the load succeeds.
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "rootDir": "new-dir" }"#,
    )
    .await;
    assert!(!root.join("new-dir").exists());
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(config.content_dir, root.join("new-dir"));
    drop(dir);
}

#[tokio::test]
async fn load_rejects_escaping_content_dir() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "rootDir": ".." }"#,
    )
    .await;
    let error = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Config(_)), "{error:?}");
    drop(dir);
}

#[tokio::test]
async fn load_rejects_symlink_escape_when_canonicalize_succeeds() {
    // Both paths canonicalize here (dir + content exist); a symlink content
    // dir pointing outside the project must be rejected by the (Ok, Ok)
    // match arm. Deleting that arm (`_ => true`) would wrongly accept it.
    let (dir, root) = temp_root();
    let sibling = root.join("sibling-outside");
    std::fs::create_dir_all(&sibling).unwrap();
    // Symlink root/linked -> root/sibling-outside/.. escapes to root's parent.
    let escape = sibling.join("up");
    std::os::unix::fs::symlink(root.parent().unwrap(), &escape).unwrap();
    let link = root.join("linked");
    std::os::unix::fs::symlink(&escape, &link).unwrap();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "rootDir": "linked" }"#,
    )
    .await;
    let error = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Config(_)), "{error:?}");
    drop(dir);
}

// ---------------------------------------------------------------------------
// config.rs:311 — truthy treats numeric 0 as falsy, nonzero as truthy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_treats_zero_and_nonzero_numbers_like_js_truthiness() {
    // 0 is falsy -> allowSymlinks off; 1 is truthy -> on.
    // (!= vs == would flip this.)
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "allowSymlinks": 0 }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert!(!config.allow_symlinks);

    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "allowSymlinks": 2 }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert!(config.allow_symlinks);
    drop(dir);
}
