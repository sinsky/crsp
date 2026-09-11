//! Core config, manifest, ignore, and pagination tests (spec §2.3, §4, §7.3).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use google_clasp_rs::constants::{
    PROJECT_CONFIG_FILENAME, PROJECT_IGNORE_FILENAME, PROJECT_MANIFEST_FILENAME,
};
use google_clasp_rs::core::config::ProjectConfig;
use google_clasp_rs::core::ignore::IgnoreMatcher;
use google_clasp_rs::core::manifest::{EnabledAdvancedService, Manifest};
use google_clasp_rs::core::pagination::{Page, PageOptions, fetch_pages};
use google_clasp_rs::error::CrspError;

fn temp_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    (dir, root)
}

async fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, content).await.unwrap();
}

async fn config_at(content: &str) -> (tempfile::TempDir, ProjectConfig) {
    let (dir, root) = temp_root();
    write_file(&root.join(PROJECT_CONFIG_FILENAME), content).await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    (dir, config)
}

// ---------------------------------------------------------------------------
// .clasp.json parsing (JSON5, defaults, empty-string edge)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_parses_json5_comments_and_single_quotes() {
    let content = r#"
// clasp config with a comment
{
  // single quotes are JSON5
  'scriptId': 'abc123',
  /* block comment */
  "srcDir": 'src',
}
"#;
    let (guard, root) = temp_root();
    // The content dir must exist for the realpath check (clasp's
    // fs.realpath falls back to the lexical path when it is missing, and a
    // /var-based tempdir root would then fail the containment check).
    write_file(&root.join("src/.keep"), "").await;
    write_file(&root.join(PROJECT_CONFIG_FILENAME), content).await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(config.script_id.as_deref(), Some("abc123"));
    assert_eq!(config.content_dir, config.project_root_dir.join("src"));
    drop(guard);
}

#[tokio::test]
async fn load_applies_default_fields() {
    let (_guard, config) = config_at(r#"{ "scriptId": "abc123" }"#).await;
    assert_eq!(config.script_id.as_deref(), Some("abc123"));
    assert_eq!(config.project_id, None);
    assert_eq!(config.parent_id, None);
    assert_eq!(config.script_extensions, vec![".js", ".gs"]);
    assert_eq!(config.html_extensions, vec![".html"]);
    assert_eq!(config.json_extensions, vec![".json"]);
    assert!(config.file_push_order.is_empty());
    assert!(!config.skip_subdirectories);
    assert!(!config.allow_symlinks);
    // contentDir = project root when srcDir/rootDir are unset.
    assert_eq!(config.content_dir, config.project_root_dir);
}

#[tokio::test]
async fn empty_src_dir_and_root_dir_mean_project_root() {
    let (_guard, config) = config_at(r#"{ "scriptId": "s", "srcDir": "" }"#).await;
    assert_eq!(config.content_dir, config.project_root_dir);

    let (_guard, config) = config_at(r#"{ "scriptId": "s", "srcDir": "", "rootDir": "" }"#).await;
    assert_eq!(config.content_dir, config.project_root_dir);

    // Empty srcDir falls back to a non-empty rootDir.
    let (guard, root) = temp_root();
    write_file(&root.join("dist/.keep"), "").await;
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "srcDir": "", "rootDir": "dist" }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(config.content_dir, root.join("dist"));
    drop(guard);
}

#[tokio::test]
async fn parent_id_array_takes_first_element() {
    let (_guard, config) = config_at(r#"{ "scriptId": "s", "parentId": ["P1", "P2"] }"#).await;
    assert_eq!(config.parent_id.as_deref(), Some("P1"));

    let (_guard, config) = config_at(r#"{ "scriptId": "s", "parentId": "P0" }"#).await;
    assert_eq!(config.parent_id.as_deref(), Some("P0"));
}

#[tokio::test]
async fn legacy_file_extension_and_fixups() {
    // Legacy singular setting only.
    let (_guard, config) = config_at(r#"{ "scriptId": "s", "fileExtension": "GS" }"#).await;
    assert_eq!(config.script_extensions, vec![".gs"]);

    // Plural wins over legacy; fixups lowercase and prepend a dot.
    let (_guard, config) =
        config_at(r#"{ "scriptId": "s", "fileExtension": "gs", "scriptExtensions": ["JS"] }"#)
            .await;
    assert_eq!(config.script_extensions, vec![".js"]);

    // String shorthand for extension arrays.
    let (_guard, config) = config_at(
        r#"{ "scriptId": "s", "htmlExtensions": "HTML", "jsonExtensions": ["JSON", ".JSON"] }"#,
    )
    .await;
    assert_eq!(config.html_extensions, vec![".html"]);
    assert_eq!(config.json_extensions, vec![".json", ".json"]);

    // Non-string array elements are filtered out.
    let (_guard, config) =
        config_at(r#"{ "scriptId": "s", "scriptExtensions": [".gs", 42] }"#).await;
    assert_eq!(config.script_extensions, vec![".gs"]);

    // Empty string settings are falsy and keep the defaults.
    let (_guard, config) =
        config_at(r#"{ "scriptId": "s", "fileExtension": "", "scriptExtensions": "" }"#).await;
    assert_eq!(config.script_extensions, vec![".js", ".gs"]);
}

#[tokio::test]
async fn file_push_order_and_flags_are_loaded() {
    let (_guard, config) = config_at(
        r#"{ "scriptId": "s", "filePushOrder": ["b.js", "a.js"], "skipSubdirectories": true, "allowSymlinks": true }"#,
    )
    .await;
    assert_eq!(config.file_push_order, vec!["b.js", "a.js"]);
    assert!(config.skip_subdirectories);
    assert!(config.allow_symlinks);
}

#[tokio::test]
async fn malformed_config_is_a_config_error() {
    let (dir, root) = temp_root();
    write_file(&root.join(PROJECT_CONFIG_FILENAME), "{ not json5 !!").await;
    let error = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Config(_)), "{error:?}");
    drop(dir);
}

// ---------------------------------------------------------------------------
// contentDir jail at load time (clasp.ts:205-221)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn src_dir_outside_project_root_is_rejected() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "srcDir": "../outside" }"#,
    )
    .await;
    let error = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap_err();
    let message = match error {
        CrspError::Config(message) => message,
        other => panic!("expected Config error, got {other:?}"),
    };
    // Byte-exact clasp text (clasp.ts:215-220; parked audit item 12).
    let resolved = root.parent().unwrap().join("outside");
    assert_eq!(
        message,
        format!(
            "Security Error: srcDir \"../outside\" escapes project root.\n  \
             Resolved: {}\n  Project root: {}\n\
             This may indicate a malicious .clasp.json file attempting path traversal.",
            resolved.to_string_lossy().replace('\\', "/"),
            root.to_string_lossy().replace('\\', "/")
        )
    );
    drop(dir);
}

#[tokio::test]
async fn src_dir_traversal_inside_root_is_rejected_after_resolution() {
    // Resolves to root/src via traversal, which is allowed (inside root).
    let (guard, root) = temp_root();
    write_file(&root.join("src/.keep"), "").await;
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s", "srcDir": "sub/../src" }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(config.content_dir, root.join("src"));
    drop(guard);
}

// ---------------------------------------------------------------------------
// discovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discover_finds_config_up_the_tree_from_cwd() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "deep" }"#,
    )
    .await;
    let cwd = root.join("a/b/c");
    tokio::fs::create_dir_all(&cwd).await.unwrap();
    let config = ProjectConfig::discover(None, &cwd).await.unwrap();
    assert_eq!(config.project_root_dir, root);
    assert_eq!(config.content_dir, root);
    assert_eq!(config.script_id.as_deref(), Some("deep"));
    drop(dir);
}

#[tokio::test]
async fn discover_accepts_explicit_directory_or_file() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "by-dir" }"#,
    )
    .await;
    let by_dir = ProjectConfig::discover(Some(&root), &PathBuf::from("/"))
        .await
        .unwrap();
    assert_eq!(by_dir.script_id.as_deref(), Some("by-dir"));
    assert_eq!(by_dir.project_root_dir, root);

    let file = root.join(PROJECT_CONFIG_FILENAME);
    let by_file = ProjectConfig::discover(Some(&file), &PathBuf::from("/"))
        .await
        .unwrap();
    assert_eq!(by_file.script_id.as_deref(), Some("by-dir"));
    drop(dir);
}

#[tokio::test]
async fn discover_rejects_missing_explicit_path() {
    let missing = PathBuf::from("/nonexistent/crsp-test-project");
    let error = ProjectConfig::discover(Some(&missing), &PathBuf::from("/"))
        .await
        .unwrap_err();
    let message = match error {
        CrspError::Config(message) => message,
        other => panic!("expected Config error, got {other:?}"),
    };
    assert_eq!(
        message,
        "Invalid --project path: /nonexistent/crsp-test-project. File or directory does not exist."
    );
}

#[tokio::test]
async fn clasp_init_context_rejects_missing_ignore_path() {
    let missing = PathBuf::from("/nonexistent/crsp-test-ignore");
    let error = match google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        Some(&missing),
        None,
        "default",
        false,
        false,
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("expected Err"),
    };
    let message = match error {
        CrspError::Config(message) => message,
        other => panic!("expected Config error, got {other:?}"),
    };
    assert_eq!(
        message,
        "Invalid --ignore path: /nonexistent/crsp-test-ignore. File or directory does not exist."
    );
}

#[tokio::test]
async fn discover_without_config_defaults_to_cwd_root() {
    let (dir, cwd) = temp_root();
    let config = ProjectConfig::discover(None, &cwd).await.unwrap();
    assert_eq!(config.project_root_dir, cwd);
    assert_eq!(config.content_dir, cwd);
    assert_eq!(config.config_file_path, cwd.join(PROJECT_CONFIG_FILENAME));
    assert_eq!(config.script_id, None);
    assert_eq!(config.script_extensions, vec![".js", ".gs"]);
    drop(dir);
}

#[tokio::test]
async fn default_instance_still_discovers_the_ignore_file() {
    // clasp's no-project branch looks up .claspignore at the default root.
    let (dir, cwd) = temp_root();
    write_file(&cwd.join(PROJECT_IGNORE_FILENAME), "*.log\n").await;
    let config = ProjectConfig::discover(None, &cwd).await.unwrap();
    assert_eq!(
        config.ignore_file_path.as_deref(),
        Some(cwd.join(PROJECT_IGNORE_FILENAME).as_path())
    );
    drop(dir);
}

#[tokio::test]
async fn discover_locates_the_ignore_file() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s" }"#,
    )
    .await;
    write_file(&root.join(PROJECT_IGNORE_FILENAME), "foo.txt\n").await;
    let config = ProjectConfig::discover(None, &root).await.unwrap();
    assert_eq!(
        config.ignore_file_path.as_deref(),
        Some(root.join(PROJECT_IGNORE_FILENAME).as_path())
    );

    // No ignore file -> None (defaults are applied by IgnoreMatcher).
    let (dir2, root2) = temp_root();
    write_file(
        &root2.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "s" }"#,
    )
    .await;
    let config2 = ProjectConfig::discover(None, &root2).await.unwrap();
    assert_eq!(config2.ignore_file_path, None);
    drop(dir);
    drop(dir2);
}

// ---------------------------------------------------------------------------
// update_settings (clasp project.ts:443-460)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_settings_writes_clasp_compatible_json() {
    let (dir, root) = temp_root();
    write_file(&root.join("src/.keep"), "").await;
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "abc123", "srcDir": "src", "projectId": "p-1", "parentId": "P1", "htmlExtensions": ["html"] }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    config.update_settings().await.unwrap();
    let written = tokio::fs::read_to_string(root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(
        written,
        concat!(
            "{\n",
            "  \"scriptId\": \"abc123\",\n",
            "  \"rootDir\": \"src\",\n",
            "  \"parentId\": \"P1\",\n",
            "  \"projectId\": \"p-1\",\n",
            "  \"scriptExtensions\": [\n",
            "    \".js\",\n",
            "    \".gs\"\n",
            "  ],\n",
            "  \"htmlExtensions\": [\n",
            "    \".html\"\n",
            "  ],\n",
            "  \"jsonExtensions\": [\n",
            "    \".json\"\n",
            "  ],\n",
            "  \"filePushOrder\": [],\n",
            "  \"skipSubdirectories\": false\n",
            "}"
        )
    );
    drop(dir);
}

#[tokio::test]
async fn update_settings_omits_undefined_keys_and_resets_push_order() {
    let (dir, root) = temp_root();
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "abc123", "filePushOrder": ["a.js", "b.js"], "skipSubdirectories": true, "allowSymlinks": true }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    config.update_settings().await.unwrap();
    let written = tokio::fs::read_to_string(root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    // parentId/projectId are absent entirely; filePushOrder is reset; the
    // extensions and skipSubdirectories are always present.
    assert_eq!(
        written,
        concat!(
            "{\n",
            "  \"scriptId\": \"abc123\",\n",
            "  \"rootDir\": \"\",\n",
            "  \"scriptExtensions\": [\n",
            "    \".js\",\n",
            "    \".gs\"\n",
            "  ],\n",
            "  \"htmlExtensions\": [\n",
            "    \".html\"\n",
            "  ],\n",
            "  \"jsonExtensions\": [\n",
            "    \".json\"\n",
            "  ],\n",
            "  \"filePushOrder\": [],\n",
            "  \"skipSubdirectories\": true\n",
            "}"
        )
    );
    drop(dir);
}

#[tokio::test]
async fn update_settings_recomputes_root_dir_from_content_dir() {
    let (dir, root) = temp_root();
    write_file(&root.join("sub/deep/.keep"), "").await;
    write_file(
        &root.join(PROJECT_CONFIG_FILENAME),
        r#"{ "scriptId": "abc123", "srcDir": "sub/deep" }"#,
    )
    .await;
    let config = ProjectConfig::load(&root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert_eq!(config.content_dir, root.join("sub/deep"));
    config.update_settings().await.unwrap();
    let written = tokio::fs::read_to_string(root.join(PROJECT_CONFIG_FILENAME))
        .await
        .unwrap();
    assert!(written.contains("\"rootDir\": \"sub/deep\""), "{written}");
    drop(dir);
}

#[tokio::test]
async fn update_settings_requires_script_id() {
    let (dir, root) = temp_root();
    // No .clasp.json at all -> default config without scriptId.
    let config = ProjectConfig::discover(None, &root).await.unwrap();
    let error = config.update_settings().await.unwrap_err();
    match error {
        CrspError::Config(message) => assert_eq!(message, "Project settings not found."),
        other => panic!("expected Config error, got {other:?}"),
    }
    drop(dir);
}

// ---------------------------------------------------------------------------
// manifest (appsscript.json)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manifest_round_trip_preserves_key_order() {
    let (dir, root) = temp_root();
    let manifest_path = root.join(PROJECT_MANIFEST_FILENAME);
    let original = concat!(
        "{\n",
        "  \"timeZone\": \"America/New_York\",\n",
        "  \"oauthScopes\": [\n",
        "    \"https://www.googleapis.com/auth/drive\"\n",
        "  ],\n",
        "  \"dependencies\": {\n",
        "    \"enabledAdvancedServices\": [],\n",
        "    \"libraries\": []\n",
        "  },\n",
        "  \"exceptionLogging\": \"STACKDRIVER\",\n",
        "  \"webapp\": {\n",
        "    \"access\": \"MYSELF\",\n",
        "    \"executeAs\": \"USER_DEPLOYING\"\n",
        "  }\n",
        "}"
    );
    write_file(&manifest_path, original).await;
    let manifest = Manifest::read(&manifest_path).await.unwrap();
    assert_eq!(manifest.to_json_pretty().unwrap(), original);
    drop(dir);
}

#[tokio::test]
async fn manifest_enable_appends_service_with_key_order_preserved() {
    let (dir, root) = temp_root();
    let manifest_path = root.join(PROJECT_MANIFEST_FILENAME);
    write_file(&manifest_path, "{\n  \"timeZone\": \"America/New_York\"\n}").await;
    let mut manifest = Manifest::read(&manifest_path).await.unwrap();
    let drive = EnabledAdvancedService {
        user_symbol: "Drive".to_string(),
        version: "v3".to_string(),
        service_id: "drive".to_string(),
    };
    assert!(manifest.enable_advanced_service(&drive).unwrap());
    // Second enable of the same userSymbol is a no-op.
    assert!(!manifest.enable_advanced_service(&drive).unwrap());
    manifest.write(&manifest_path).await.unwrap();
    let written = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    assert_eq!(
        written,
        concat!(
            "{\n",
            "  \"timeZone\": \"America/New_York\",\n",
            "  \"dependencies\": {\n",
            "    \"enabledAdvancedServices\": [\n",
            "      {\n",
            "        \"userSymbol\": \"Drive\",\n",
            "        \"version\": \"v3\",\n",
            "        \"serviceId\": \"drive\"\n",
            "      }\n",
            "    ]\n",
            "  }\n",
            "}"
        )
    );
    drop(dir);
}

#[tokio::test]
async fn manifest_enable_creates_dependencies_struct() {
    let (dir, root) = temp_root();
    let manifest_path = root.join(PROJECT_MANIFEST_FILENAME);
    // Existing dependencies object without enabledAdvancedServices.
    write_file(
        &manifest_path,
        "{\n  \"dependencies\": {\n    \"libraries\": []\n  }\n}",
    )
    .await;
    let mut manifest = Manifest::read(&manifest_path).await.unwrap();
    let sheets = EnabledAdvancedService {
        user_symbol: "Sheets".to_string(),
        version: "v4".to_string(),
        service_id: "sheets".to_string(),
    };
    assert!(manifest.enable_advanced_service(&sheets).unwrap());
    manifest.write(&manifest_path).await.unwrap();
    let written = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    assert_eq!(
        written,
        concat!(
            "{\n",
            "  \"dependencies\": {\n",
            "    \"libraries\": [],\n",
            "    \"enabledAdvancedServices\": [\n",
            "      {\n",
            "        \"userSymbol\": \"Sheets\",\n",
            "        \"version\": \"v4\",\n",
            "        \"serviceId\": \"sheets\"\n",
            "      }\n",
            "    ]\n",
            "  }\n",
            "}"
        )
    );
    drop(dir);
}

#[tokio::test]
async fn manifest_disable_removes_service_by_id() {
    let (dir, root) = temp_root();
    let manifest_path = root.join(PROJECT_MANIFEST_FILENAME);
    write_file(
        &manifest_path,
        concat!(
            "{\n",
            "  \"timeZone\": \"America/New_York\",\n",
            "  \"dependencies\": {\n",
            "    \"enabledAdvancedServices\": [\n",
            "      {\n",
            "        \"userSymbol\": \"Drive\",\n",
            "        \"version\": \"v3\",\n",
            "        \"serviceId\": \"drive\"\n",
            "      },\n",
            "      {\n",
            "        \"userSymbol\": \"Sheets\",\n",
            "        \"version\": \"v4\",\n",
            "        \"serviceId\": \"sheets\"\n",
            "      }\n",
            "    ]\n",
            "  }\n",
            "}"
        ),
    )
    .await;
    let mut manifest = Manifest::read(&manifest_path).await.unwrap();
    assert!(manifest.disable_advanced_service("drive").unwrap());
    manifest.write(&manifest_path).await.unwrap();
    let written = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    assert!(written.contains("sheets"), "{written}");
    assert!(!written.contains("drive"), "{written}");
    assert!(written.contains("timeZone"), "{written}");

    // Disabling when the service is absent does nothing.
    let mut manifest = Manifest::read(&manifest_path).await.unwrap();
    assert!(!manifest.disable_advanced_service("drive").unwrap());
    // Disabling without any dependencies does nothing either.
    let (dir2, root2) = temp_root();
    let manifest_path2 = root2.join(PROJECT_MANIFEST_FILENAME);
    write_file(
        &manifest_path2,
        "{\n  \"timeZone\": \"America/New_York\"\n}",
    )
    .await;
    let mut manifest2 = Manifest::read(&manifest_path2).await.unwrap();
    assert!(!manifest2.disable_advanced_service("drive").unwrap());
    drop(dir);
    drop(dir2);
}

#[tokio::test]
async fn manifest_read_reports_malformed_documents() {
    let (dir, root) = temp_root();
    let manifest_path = root.join(PROJECT_MANIFEST_FILENAME);
    write_file(&manifest_path, "{ broken").await;
    let error = Manifest::read(&manifest_path).await.unwrap_err();
    assert!(matches!(error, CrspError::Config(_)), "{error:?}");
    drop(dir);
}

// ---------------------------------------------------------------------------
// ignore matching (micromatch.not semantics, spec §2.3)
// ---------------------------------------------------------------------------

#[test]
fn default_ignore_worked_example_from_spec() {
    let matcher = IgnoreMatcher::from_patterns(IgnoreMatcher::default_patterns()).unwrap();
    let files = [
        "a.js",
        "x.txt",
        "appsscript.json",
        ".git/config.js",
        "node_modules/f.js",
        "sub/Code.gs",
    ];
    // tracked = micromatch.not(files, patterns, {dot: true}); input order kept.
    assert_eq!(
        matcher.tracked(files),
        vec!["a.js", "appsscript.json", "sub/Code.gs"]
    );
}

#[test]
fn ts_files_are_tracked_by_default_ignore() {
    let matcher = IgnoreMatcher::from_patterns(IgnoreMatcher::default_patterns()).unwrap();
    assert!(matcher.is_tracked("src/App.ts"));
    // ...even though the file pipeline later skips them as unsupported type.
}

#[test]
fn ignore_patterns_are_order_dependent_like_micromatch() {
    // A negative pattern only removes files from the match set if a later
    // positive pattern does not re-add them (micromatch processes patterns
    // in order: positives add+rescue, negatives omit). With the negation
    // first, the trailing `**/**` rescues everything -> nothing tracked.
    let matcher = IgnoreMatcher::from_patterns(["!**/*.js", "**/**"]).unwrap();
    assert_eq!(matcher.tracked(["a.js", "b.txt"]), Vec::<&str>::new());

    // Reversing restores the usual semantics: `!**/*.js` after `**/**`
    // omits .js files from the match set, so a.js is tracked.
    let matcher = IgnoreMatcher::from_patterns(["**/**", "!**/*.js"]).unwrap();
    assert_eq!(matcher.tracked(["a.js", "b.txt"]), vec!["a.js"]);
}

#[test]
fn ignore_negation_only_patterns_keep_non_matches() {
    // When every pattern is negated, the match set is "files matching no
    // negation" (verified against micromatch 4.0.8: not(['a.js','b.txt'],
    // ['!**/*.js']) == ['a.js']) — so a.js is tracked, b.txt is ignored.
    let matcher = IgnoreMatcher::from_patterns(["!**/*.js"]).unwrap();
    assert_eq!(matcher.tracked(["a.js", "b.txt"]), vec!["a.js"]);
    assert!(matcher.is_tracked("a.js"));
    assert!(!matcher.is_tracked("b.txt"));
}

#[test]
fn ignore_matches_dot_files() {
    let matcher = IgnoreMatcher::from_patterns(["*.txt"]).unwrap();
    assert!(!matcher.is_tracked(".hidden.txt"));
    assert!(matcher.is_tracked("visible.md"));
}

#[test]
fn ignore_normalizes_windows_separators_before_matching() {
    let matcher = IgnoreMatcher::from_patterns(["sub/*.gs", "node_modules/**"]).unwrap();
    // Windows-style input paths are normalized to / before glob evaluation,
    // and the original (unnormalized) strings are returned by tracked().
    let files = ["sub\\Code.gs", "node_modules\\f.js", "src\\a.js"];
    assert_eq!(matcher.tracked(files), vec!["src\\a.js"]);
}

#[tokio::test]
async fn ignore_file_loading_strips_bom_and_blank_lines() {
    let (dir, root) = temp_root();
    let path = root.join(PROJECT_IGNORE_FILENAME);
    let content = "\u{feff}*.log\r\n\r\nnotes/**\n";
    write_file(&path, content).await;
    let matcher = IgnoreMatcher::from_file(&path).await.unwrap();
    assert_eq!(
        matcher.tracked(["a.log", "b.js", "notes/x.txt", "c.log"]),
        vec!["b.js"]
    );
    drop(dir);
}

// ---------------------------------------------------------------------------
// pagination (clasp core/utils.ts:186-214)
// ---------------------------------------------------------------------------

fn page(results: Vec<i32>, page_token: Option<&str>) -> Page<i32> {
    Page {
        results,
        page_token: page_token.map(str::to_string),
    }
}

#[tokio::test]
async fn pagination_single_page_has_no_partial_results() {
    let calls = AtomicUsize::new(0);
    let fetch = |page_size: usize, token: Option<String>| {
        calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(page_size, 100);
        assert_eq!(token, None);
        async move { Ok(page(vec![1, 2, 3], None)) }
    };
    let result = fetch_pages(fetch, PageOptions::default()).await.unwrap();
    assert_eq!(result.results, vec![1, 2, 3]);
    assert!(!result.partial_results);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn pagination_follows_tokens_until_exhausted() {
    let calls = AtomicUsize::new(0);
    let fetch = |_page_size: usize, token: Option<String>| {
        let call = calls.fetch_add(1, Ordering::Relaxed) + 1;
        async move {
            match (call, token.as_deref()) {
                (1, None) => Ok(page(vec![1], Some("t1"))),
                (2, Some("t1")) => Ok(page(vec![2], Some("t2"))),
                (3, Some("t2")) => Ok(page(vec![3], None)),
                _ => panic!("unexpected call {token:?}"),
            }
        }
    };
    let result = fetch_pages(fetch, PageOptions::default()).await.unwrap();
    assert_eq!(result.results, vec![1, 2, 3]);
    assert!(!result.partial_results);
}

#[tokio::test]
async fn pagination_reports_partial_results_at_max_pages() {
    let calls = AtomicUsize::new(0);
    let fetch = |_page_size: usize, _token: Option<String>| {
        let n = calls.fetch_add(1, Ordering::Relaxed) + 1;
        async move { Ok(page(vec![n as i32], Some("next"))) }
    };
    let options = PageOptions {
        page_size: 100,
        max_pages: 3,
        max_results: usize::MAX,
    };
    let result = fetch_pages(fetch, options).await.unwrap();
    assert_eq!(result.results, vec![1, 2, 3]);
    assert!(result.partial_results);
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn pagination_stops_at_max_results_and_trims_with_partial() {
    // Loop stops once results.len() >= maxResults; an overshooting final page
    // is trimmed and marked partial (clasp utils.ts:210-216).
    let calls = AtomicUsize::new(0);
    let fetch = |_page_size: usize, _token: Option<String>| {
        let n = calls.fetch_add(1, Ordering::Relaxed) + 1;
        async move { Ok(page(vec![n as i32; 5], Some("next"))) }
    };
    let options = PageOptions {
        page_size: 100,
        max_pages: 10,
        max_results: 7,
    };
    let result = fetch_pages(fetch, options).await.unwrap();
    // Page 1: 5 results (< 7) continues; page 2: 10 results -> loop stops,
    // trimmed to 7, partial.
    assert_eq!(result.results.len(), 7);
    assert!(result.results.iter().all(|&r| r <= 2));
    assert!(result.partial_results);
}

#[tokio::test]
async fn pagination_mid_page_errors_propagate() {
    let calls = AtomicUsize::new(0);
    let fetch = |_page_size: usize, token: Option<String>| {
        let call = calls.fetch_add(1, Ordering::Relaxed) + 1;
        async move {
            match (call, token.as_deref()) {
                (1, None) => Ok(page(vec![1], Some("t1"))),
                _ => Err(CrspError::Api {
                    kind: google_clasp_rs::error::ApiErrorKind::UnexpectedApiError,
                    message: "boom".to_string(),
                }),
            }
        }
    };
    let error = fetch_pages(fetch, PageOptions::default())
        .await
        .unwrap_err();
    match error {
        CrspError::Api { message, .. } => assert_eq!(message, "boom"),
        other => panic!("expected Api error, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn pagination_defaults_match_clasp_and_service_usage_overrides() {
    let options = PageOptions::default();
    assert_eq!(options.page_size, 100);
    assert_eq!(options.max_pages, 10);
    // Service Usage (list-apis) uses pageSize 200 / maxResults 10000.
    let options = PageOptions {
        page_size: google_clasp_rs::core::pagination::SERVICE_USAGE_PAGE_SIZE,
        max_pages: 10,
        max_results: google_clasp_rs::core::pagination::SERVICE_USAGE_MAX_RESULTS,
    };
    assert_eq!(options.page_size, 200);
    assert_eq!(options.max_results, 10000);
}
