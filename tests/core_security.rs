//! Path jail and MCP jail security tests (spec §5 #5, §8, §2.6).

use std::path::{Path, PathBuf};

use google_clasp_rs::core::path::{PathJail, normalize_slashes, relative_path};
use google_clasp_rs::error::CrspError;

// ---------------------------------------------------------------------------
// component-aware containment
// ---------------------------------------------------------------------------

#[test]
fn is_inside_never_uses_raw_string_prefixes() {
    // /app_fake is a SIBLING of /app, not a child. A naive string prefix
    // check would wrongly accept it.
    assert!(!PathJail::is_inside(
        Path::new("/app"),
        Path::new("/app_fake")
    ));
    assert!(!PathJail::is_inside(
        Path::new("/app"),
        Path::new("/application/x.js")
    ));
    assert!(PathJail::is_inside(
        Path::new("/app"),
        Path::new("/app/sub/x.js")
    ));
}

#[test]
fn is_inside_rejects_equal_paths_and_traversal() {
    // Equal paths are not "inside" (clasp isInside: relative !== '').
    assert!(!PathJail::is_inside(Path::new("/app"), Path::new("/app")));
    // Unnormalized traversal resolves outside.
    assert!(!PathJail::is_inside(
        Path::new("/app"),
        Path::new("/app/../evil")
    ));
    // Traversal that lands back inside is accepted (resolved view).
    assert!(PathJail::is_inside(
        Path::new("/app"),
        Path::new("/app/sub/../x.js")
    ));
    // Parent-of-base is not inside.
    assert!(!PathJail::is_inside(
        Path::new("/app/sub"),
        Path::new("/app")
    ));
}

// ---------------------------------------------------------------------------
// lexical resolution
// ---------------------------------------------------------------------------

#[test]
fn resolve_lexical_joins_resolves_and_never_escapes_semantically() {
    assert_eq!(
        PathJail::resolve_lexical(Path::new("/app"), "src"),
        PathBuf::from("/app/src")
    );
    assert_eq!(
        PathJail::resolve_lexical(Path::new("/app"), "./src/./x"),
        PathBuf::from("/app/src/x")
    );
    assert_eq!(
        PathJail::resolve_lexical(Path::new("/app"), "sub/../src"),
        PathBuf::from("/app/src")
    );
    assert_eq!(
        PathJail::resolve_lexical(Path::new("/app"), ".."),
        PathBuf::from("/")
    );
    // Absolute candidates replace the base entirely (path.resolve semantics).
    assert_eq!(
        PathJail::resolve_lexical(Path::new("/app"), "/etc/x.js"),
        PathBuf::from("/etc/x.js")
    );
}

#[test]
fn normalize_slashes_copies_only_when_needed() {
    let plain = "sub/Code.gs";
    match normalize_slashes(plain) {
        std::borrow::Cow::Borrowed(s) => assert_eq!(s, "sub/Code.gs"),
        other => panic!("expected borrowed Cow, got {other:?}"),
    }
    assert_eq!(normalize_slashes("sub\\Code.gs").as_ref(), "sub/Code.gs");
    assert_eq!(normalize_slashes("a\\b\\c.js").as_ref(), "a/b/c.js");
}

#[test]
fn relative_path_matches_javascript_path_relative() {
    assert_eq!(
        relative_path(Path::new("/a/b"), Path::new("/a/b/src")),
        "src"
    );
    assert_eq!(relative_path(Path::new("/a/b"), Path::new("/a/b")), "");
    assert_eq!(relative_path(Path::new("/a/b"), Path::new("/a/c")), "../c");
    assert_eq!(relative_path(Path::new("/a/b/c"), Path::new("/a/b")), "..");
    assert_eq!(
        relative_path(Path::new("/a/b/c"), Path::new("/a/x/y")),
        "../../x/y"
    );
    assert_eq!(relative_path(Path::new("/"), Path::new("/a/b")), "a/b");
}

// ---------------------------------------------------------------------------
// contentDir override resolution (clasp withContentDir, clasp.ts:129-142;
// unified MCP sourceDir rule per spec §5 #5)
// ---------------------------------------------------------------------------

#[test]
fn resolve_content_dir_accepts_relative_and_absolute_inside_paths() {
    let base = Path::new("/home/alice/work/app");
    assert_eq!(
        PathJail::resolve_content_dir(base, "src").unwrap(),
        base.join("src")
    );
    assert_eq!(PathJail::resolve_content_dir(base, ".").unwrap(), base);
    // Absolute path inside the base is accepted.
    assert_eq!(
        PathJail::resolve_content_dir(base, "/home/alice/work/app/src").unwrap(),
        base.join("src")
    );
}

#[test]
fn resolve_content_dir_rejects_escapes_with_clasp_message() {
    let base = Path::new("/home/alice/work/app");
    let error = PathJail::resolve_content_dir(base, "../outside").unwrap_err();
    match error {
        CrspError::Validation(message) => assert_eq!(
            message,
            "Security Error: Content directory \"/home/alice/work/outside\" resolves outside the project root \"/home/alice/work/app\". This may indicate a path traversal attempt."
        ),
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Absolute escape is rejected too.
    let error = PathJail::resolve_content_dir(base, "/etc/x").unwrap_err();
    assert!(matches!(error, CrspError::Validation(_)), "{error:?}");
}

#[test]
fn mcp_source_dir_jail_resolves_relative_to_project_dir() {
    // Spec §5 #5: sourceDir resolves RELATIVE TO projectDir and must stay
    // inside projectDir (not relative to the process cwd).
    let project_dir = Path::new("/home/alice/work/app");
    assert_eq!(
        PathJail::resolve_content_dir(project_dir, "sub/src").unwrap(),
        project_dir.join("sub/src")
    );
    let error = PathJail::resolve_content_dir(project_dir, "../evil").unwrap_err();
    assert!(matches!(error, CrspError::Validation(_)), "{error:?}");
}

// ---------------------------------------------------------------------------
// MCP projectDir jail (clasp server.ts:40-51)
// ---------------------------------------------------------------------------

#[test]
fn mcp_project_dir_allows_home_and_cwd_descendants() {
    let home = Path::new("/home/alice");
    let cwd = Path::new("/home/alice/work");
    assert_eq!(
        PathJail::validate_project_dir(Path::new("/home/alice/work/app"), home, cwd).unwrap(),
        PathBuf::from("/home/alice/work/app")
    );
    // The home dir itself is allowed (resolved === base).
    assert_eq!(
        PathJail::validate_project_dir(home, home, cwd).unwrap(),
        PathBuf::from("/home/alice")
    );
    // cwd itself and descendants of cwd are allowed.
    assert_eq!(
        PathJail::validate_project_dir(cwd, home, cwd).unwrap(),
        PathBuf::from("/home/alice/work")
    );
    assert_eq!(
        PathJail::validate_project_dir(
            Path::new("/home/alice/work/app"),
            home,
            Path::new("/somewhere/else")
        )
        .unwrap(),
        PathBuf::from("/home/alice/work/app")
    );
}

#[test]
fn mcp_project_dir_rejects_outside_paths_with_exact_message() {
    let home = Path::new("/home/alice");
    let cwd = Path::new("/home/alice/work");
    let error = PathJail::validate_project_dir(Path::new("/etc/passwd"), home, cwd).unwrap_err();
    match error {
        CrspError::Validation(message) => assert_eq!(
            message,
            "Security Error: projectDir must be within the user home directory or current working directory. Resolved path \"/etc/passwd\" is not permitted."
        ),
        other => panic!("expected Validation error, got {other:?}"),
    }
    // Sibling of home (not a descendant) is rejected.
    let error =
        PathJail::validate_project_dir(Path::new("/home/alice_fake/x"), home, cwd).unwrap_err();
    assert!(matches!(error, CrspError::Validation(_)), "{error:?}");
}

#[test]
fn mcp_project_dir_resolves_relative_candidates_against_cwd() {
    let home = Path::new("/home/alice");
    let cwd = Path::new("/home/alice/work");
    assert_eq!(
        PathJail::validate_project_dir(Path::new("app"), home, cwd).unwrap(),
        PathBuf::from("/home/alice/work/app")
    );
    assert_eq!(
        PathJail::validate_project_dir(Path::new("./app/../app2"), home, cwd).unwrap(),
        PathBuf::from("/home/alice/work/app2")
    );
}

// ---------------------------------------------------------------------------
// remote-name validation (pull side jail, files.ts:706-713)
// ---------------------------------------------------------------------------

#[test]
fn remote_target_path_joins_and_validates_names() {
    let content_dir = Path::new("/app/src");
    assert_eq!(
        PathJail::remote_target_path(content_dir, "Code.gs").unwrap(),
        content_dir.join("Code.gs")
    );
    assert_eq!(
        PathJail::remote_target_path(content_dir, "sub/dir/x.js").unwrap(),
        content_dir.join("sub/dir/x.js")
    );
    assert_eq!(
        PathJail::remote_target_path(content_dir, "./x.js").unwrap(),
        content_dir.join("x.js")
    );
}

#[test]
fn remote_target_path_rejects_escapes_and_windows_separators() {
    let content_dir = Path::new("/app/src");
    // Traversal escape -> rejected (outside_content_dir on the pull side).
    assert_eq!(
        PathJail::remote_target_path(content_dir, "../evil.js"),
        None
    );
    assert_eq!(
        PathJail::remote_target_path(content_dir, "..\\evil.js"),
        None
    );
    assert_eq!(PathJail::remote_target_path(content_dir, "/etc/x.js"), None);
    // A remote name that normalizes to the content dir itself is not inside.
    assert_eq!(PathJail::remote_target_path(content_dir, "."), None);
    // Windows-style separators inside valid names are normalized.
    assert_eq!(
        PathJail::remote_target_path(content_dir, "sub\\dir\\x.js").unwrap(),
        content_dir.join("sub/dir/x.js")
    );
}
