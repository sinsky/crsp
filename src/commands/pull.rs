use std::fs;
use std::path::Path;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{
    LocalExtensions, PullResult, SkipReason, collect_local_files, pull_files_with_progress,
};
use crate::core::project::{RemoteFile, assert_script_configured, fetch_remote_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

pub struct PullArgs<'a> {
    pub version_number: Option<&'a str>,
    pub delete_unused: bool,
    pub force: bool,
}

pub async fn pull<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    cwd: &Path,
    args: PullArgs<'_>,
    ui: &Ui<A>,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PullResult, CrspError> {
    let version_number = args
        .version_number
        .map(|value| {
            value
                .parse()
                .map_err(|_| CrspError::Validation(format!("'{value}' is not a valid integer.")))
        })
        .transpose()?;
    let script_id = assert_script_configured(config).await?.to_string();
    let remote = ui
        .with_async_spinner(crate::i18n::FETCHING_SCRIPT_CONTENT, async move {
            fetch_remote_files(client, &script_id, config, cwd, version_number).await
        })
        .await??;
    pull_with_remote_files(
        config,
        cwd,
        &remote,
        args.delete_unused,
        args.force,
        ui,
        output,
    )
    .await
}

pub async fn pull_with_remote_files<A: PromptAdapter>(
    config: &ProjectConfig,
    cwd: &Path,
    remote: &[RemoteFile],
    delete_unused: bool,
    force: bool,
    ui: &Ui<A>,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PullResult, CrspError> {
    if delete_unused && !force && !ui.is_interactive() {
        output.warn(
            "You are not in an interactive terminal and --force not used. Skipping file deletion.",
        );
    }
    let extensions = LocalExtensions::from_config(config);
    let pull_inputs: Vec<crate::core::files::PullFile> =
        remote.iter().map(|entry| entry.file.clone()).collect();
    // Local collection is a plain await; only the remote fetch carries the
    // spinner, while the write phase reports `Pulling files... N/T` progress.
    let collected = collect_local_files(config).await?;
    if !output.is_json() {
        for item in &collected.skipped {
            if item.reason == SkipReason::Symlink {
                output.warn(&crate::i18n::security_warning_skipping_symbolic_link(
                    &crate::commands::show_file_status::cwd_relative(
                        cwd,
                        &config.content_dir,
                        &item.local_path,
                    ),
                ));
            }
        }
    }
    let show_progress = !output.is_json() && ui.is_interactive();
    let progress = show_progress.then_some(crate::output::progress::report_pull_progress);
    let progress_ref = progress
        .as_ref()
        .map(|callback| callback as &(dyn Fn(usize, usize) + Send + Sync));
    let mut result = pull_files_with_progress(
        &pull_inputs,
        &config.content_dir,
        config.allow_symlinks,
        32,
        &extensions,
        progress_ref,
    )
    .await?;
    // clasp pull.ts:75-91: every write skip warns with the clasp reason text
    // (human mode only).
    if !output.is_json() {
        for item in &result.skipped {
            let display = remote
                .iter()
                .find(|entry| entry.file.remote_path == item.local_path)
                .map(|entry| entry.local_path.clone())
                .unwrap_or_else(|| item.local_path.clone());
            output.warn(&crate::i18n::security_warning_skipping_write(
                &display,
                crate::commands::shared::write_skip_reason_text(item.reason),
            ));
        }
    }
    // clasp pull.ts:101-106 + deleteLocalFiles:129-132: deletion candidates
    // are computed before any prompt; an empty set short-circuits with no
    // prompt while still printing the normal pull output below.
    let files_to_delete: Vec<_> = if delete_unused && (force || ui.is_interactive()) {
        let remote_names: std::collections::HashSet<_> = pull_inputs
            .iter()
            .map(|file| extensions.local_name(file))
            .collect();
        collected
            .files
            .iter()
            .filter(|file| !remote_names.contains(&file.local_path))
            .collect()
    } else {
        Vec::new()
    };
    // clasp deleteLocalFiles:148-153: contentDir itself must not be a
    // symlink (unless allowSymlinks); fail before any prompt, even when
    // there is nothing to delete.
    // clasp `path.resolve(contentDir)`: relative content dirs resolve
    // against the process cwd; test/absolute configs pass through.
    let absolute_content_dir = if config.content_dir.is_absolute() {
        config.content_dir.clone()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&config.content_dir))
            .unwrap_or_else(|_| config.content_dir.clone())
    };
    if delete_unused && (force || ui.is_interactive()) && !config.allow_symlinks {
        let link = fs::symlink_metadata(&absolute_content_dir)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        if link {
            // clasp pull.ts:148-153 compares `realpath(contentDir)` with the
            // resolved path verbatim; on macOS `/tmp` itself resolves under
            // `/private`, so only a directly-symlinked content dir fails.
            // A stricter ancestor check would false-positive on TempDir and
            // real projects mounted through symlinked parents.
            return Err(CrspError::Validation(
                "Security Error: Content directory is a symlink. Possible race attack.".to_string(),
            ));
        }
    }
    if !files_to_delete.is_empty() {
        let real_content_dir = if config.allow_symlinks {
            absolute_content_dir.clone()
        } else {
            fs::canonicalize(&absolute_content_dir).unwrap_or_else(|_| absolute_content_dir.clone())
        };
        if force {
            for file in files_to_delete {
                // clasp `deleteLocalFiles` reports the collected `localPath`
                // verbatim; in crsp that already is the cwd-relative display
                // path (`collect_local_files` returns contentDir-relative
                // names here because cwd == contentDir's parent in tests, and
                // production callers pass cwd-relative remotes through the
                // same jail).
                let display = file.local_path.clone();
                if !is_safe_to_delete(
                    &real_content_dir,
                    &absolute_content_dir,
                    &file.local_path,
                    config.allow_symlinks,
                ) {
                    return Err(CrspError::Validation(format!(
                        "Security Error: Attempted to delete unsafe file: {display}"
                    )));
                }
                let target = absolute_content_dir.join(&file.local_path);
                delete_bound(
                    &absolute_content_dir,
                    &target,
                    &display,
                    config.allow_symlinks,
                )?;
                result.deleted.push(file.local_path.clone());
            }
        } else {
            for file in files_to_delete {
                let display = file.local_path.clone();
                if !is_safe_to_delete(
                    &real_content_dir,
                    &absolute_content_dir,
                    &file.local_path,
                    config.allow_symlinks,
                ) {
                    return Err(CrspError::Validation(format!(
                        "Security Error: Attempted to delete unsafe file: {display}"
                    )));
                }
                let confirmed = ui.confirm(PromptConfirm {
                    prompt: format!("Delete {display}?"),
                    default: false,
                })?;
                if confirmed {
                    let target = absolute_content_dir.join(&file.local_path);
                    delete_bound(
                        &absolute_content_dir,
                        &target,
                        &display,
                        config.allow_symlinks,
                    )?;
                    result.deleted.push(file.local_path.clone());
                }
            }
        }
    }
    // clasp pull.ts display: `files.map(f => f.localPath)` — the fetched
    // remote files' cwd-relative paths (files.ts `fetchRemote`), not the
    // write result's contentDir-relative paths.
    let display_pulled: Vec<String> = remote
        .iter()
        .map(|entry| entry.local_path.clone())
        .collect();
    let display_deleted: Vec<String> = result
        .deleted
        .iter()
        .map(|path| crate::commands::show_file_status::cwd_relative(cwd, &config.content_dir, path))
        .collect();
    if output.is_json() {
        #[derive(serde::Serialize)]
        struct PullOutput<'a> {
            #[serde(rename = "pulledFiles")]
            pulled_files: &'a [String],
            #[serde(rename = "deletedFiles")]
            deleted_files: &'a [String],
        }
        output.print_json(&PullOutput {
            pulled_files: &display_pulled,
            deleted_files: &display_deleted,
        })?;
    } else {
        // clasp pull.ts order: the unused-file deletions are reported first
        // (deleteLocalFiles runs before the pulled-file listing), then the
        // `└─ {path}` lines, then the count.
        for path in &display_deleted {
            output.message(&format!("Deleted {path}"));
        }
        for path in &display_pulled {
            output.message(&format!("└─ {path}"));
        }
        output.message(&crate::i18n::pulled_files(display_pulled.len()));
    }
    Ok(result)
}

/// clasp `isSafeToDelete` (pull.ts:190-223): resolved target must be inside
/// the real content dir, no symlinked parent, and the target itself must
/// exist as a non-symlink. `Err` is only I/O that cannot be classified here;
/// every `false` becomes clasp's Security Error at the call site.
fn is_safe_to_delete(
    real_content_dir: &Path,
    content_dir: &Path,
    relative: &str,
    allow_symlinks: bool,
) -> bool {
    let target = content_dir.join(relative.replace('\\', "/"));
    // `real_content_dir` is canonical while `target` may carry a symlinked
    // ancestor above the content dir (macOS `/var` -> `/private/var`), so
    // canonicalize before the containment check (clasp `isInside(realDir,
    // path.resolve(localPath))` sees resolved paths through Node).
    let canonical_target = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if canonical_target != *real_content_dir
        && !crate::core::path::PathJail::is_inside(real_content_dir, &canonical_target)
    {
        return false;
    }
    if !allow_symlinks {
        let mut current = target.parent().unwrap_or(content_dir).to_path_buf();
        while current != *real_content_dir
            && crate::core::path::PathJail::is_inside(real_content_dir, &current)
        {
            // clasp `realpath(current) !== current`: the walk itself is done
            // on canonical paths, so compare against the canonical form to
            // avoid false positives from symlinked ancestors above the
            // content dir (e.g. macOS `/var` -> `/private/var`).
            let canonical_current = fs::canonicalize(&current).unwrap_or_else(|_| current.clone());
            if canonical_current != current {
                return false;
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => break,
            }
        }
    }
    match fs::symlink_metadata(&target) {
        Ok(metadata) => allow_symlinks || !metadata.file_type().is_symlink(),
        Err(_) => false,
    }
}

#[cfg(unix)]
fn delete_bound(
    content_dir: &Path,
    target: &Path,
    local_path: &str,
    allow_symlinks: bool,
) -> Result<bool, CrspError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    // The `is_safe_to_delete` gate already classified every clasp failure as
    // a Security Error; only a `realpath` mismatch between the joined target
    // and the resolved path can still slip through lexical checks here.
    if !Path::new(target).starts_with(content_dir) {
        return Err(CrspError::Validation(format!(
            "Security Error: Attempted to delete unsafe file: {local_path}"
        )));
    }
    let parent = target.parent().unwrap_or(content_dir);
    let name = target.file_name().unwrap();
    let parent_c = CString::new(parent.as_os_str().as_bytes())
        .map_err(|_| CrspError::Validation("invalid deletion path".to_string()))?;
    let fd = unsafe {
        libc::open(
            parent_c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | if allow_symlinks { 0 } else { libc::O_NOFOLLOW },
        )
    };
    if fd < 0 {
        return Ok(false);
    }
    let verified = fs::canonicalize(parent)
        .map(|real| {
            let root = fs::canonicalize(content_dir).unwrap_or_else(|_| content_dir.to_path_buf());
            allow_symlinks || real == root || crate::core::path::PathJail::is_inside(&root, &real)
        })
        .unwrap_or(false);
    if !verified {
        unsafe {
            libc::close(fd);
        }
        return Ok(false);
    }
    let name_c = CString::new(name.as_bytes())
        .map_err(|_| CrspError::Validation("invalid deletion name".to_string()))?;
    let result = unsafe { libc::unlinkat(fd, name_c.as_ptr(), 0) };
    let unlink_error = if result == 0 {
        None
    } else {
        Some(std::io::Error::last_os_error())
    };
    unsafe {
        libc::close(fd);
    }
    // clasp pull.ts:155-160: every `isSafeToDelete` false is a command-level
    // failure, never a silent skip. `O_NOFOLLOW`/`unlinkat` remains the
    // race-safe deletion mechanism; post-`isSafeToDelete` I/O races after the
    // gate still propagate as I/O errors (ENOENT included).
    match unlink_error {
        None => Ok(true),
        Some(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
fn delete_bound(
    content_dir: &Path,
    target: &Path,
    local_path: &str,
    _allow_symlinks: bool,
) -> Result<bool, CrspError> {
    // The `is_safe_to_delete` gate already classified every clasp failure as
    // a Security Error; keep the lexical guard as defense in depth.
    if !target.starts_with(content_dir) {
        return Err(CrspError::Validation(format!(
            "Security Error: Attempted to delete unsafe file: {local_path}"
        )));
    }
    fs::remove_file(target)?;
    Ok(true)
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}

#[cfg(test)]
mod tests {
    use super::is_safe_to_delete;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn deletion_safety_rejects_outside_and_missing_targets() {
        let temp = TempDir::new().unwrap();
        let content = temp.path().join("content");
        fs::create_dir_all(&content).unwrap();

        assert!(!is_safe_to_delete(
            &content,
            &content,
            "../outside.js",
            false
        ));
        assert!(!is_safe_to_delete(&content, &content, "missing.js", false));
    }

    #[cfg(unix)]
    #[test]
    fn deletion_safety_rejects_symlink_parent_and_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let content = temp.path().join("content");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&content).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("kept.js"), "kept").unwrap();
        symlink(&outside, content.join("linked")).unwrap();
        fs::write(content.join("plain.js"), "plain").unwrap();
        symlink(outside.join("kept.js"), content.join("file-link.js")).unwrap();

        let real = fs::canonicalize(&content).unwrap_or_else(|_| content.clone());
        assert!(is_safe_to_delete(&real, &content, "plain.js", false));
        assert!(!is_safe_to_delete(&real, &content, "linked/kept.js", false));
        assert!(!is_safe_to_delete(&real, &content, "file-link.js", false));
        assert!(outside.join("kept.js").exists());
    }
}
