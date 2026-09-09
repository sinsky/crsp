use std::fs;
use std::path::{Path, PathBuf};

use crsp::core::config::ProjectConfig;
use crsp::core::files::{
    LocalFile, PullFile, SkipReason, collect_local_files, get_changed_files, pull_file,
};
use tempfile::TempDir;

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
async fn default_ignore_tracks_ts_then_marks_it_unsupported() {
    let temp = TempDir::new().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("Code.js"), "function code() {}\n").unwrap();
    fs::write(src.join("types.ts"), "type X = string;\n").unwrap();
    fs::write(src.join("appsscript.json"), "{}\n").unwrap();

    let result = collect_local_files(&config(temp.path())).await.unwrap();
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].remote_path, "Code");
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
    let snippet = crsp::core::files::syntax_error_snippet(
        "Syntax error: Missing ; line: 2 file: Code",
        &files,
    )
    .unwrap();
    assert!(snippet.contains("Missing ; - \"Code:2\""));
    assert!(snippet.contains("=> two"));
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
