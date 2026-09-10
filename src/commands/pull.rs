use std::fs;
use std::path::Path;

use crate::core::config::ProjectConfig;
use crate::core::files::{
    LocalExtensions, PullResult, SkipReason, collect_local_files, pull_files,
};
use crate::core::project::RemoteFile;
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

pub async fn pull<A: PromptAdapter>(
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
    // clasp pull.ts:45-66: locals are collected before the pull (the list
    // also feeds the --deleteUnusedFiles comparison). clasp assigns a
    // `Checking local files...` message here but never passes it to
    // `withSpinner` — the local collect is a plain await; only the remote
    // pull carries the spinner.
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
    let pull_inputs_ref: &[crate::core::files::PullFile] = &pull_inputs;
    let extensions_ref: &crate::core::files::LocalExtensions = &extensions;
    let mut result = ui
        .with_async_spinner(crate::i18n::PULLING_FILES, async move {
            pull_files(
                pull_inputs_ref,
                &config.content_dir,
                config.allow_symlinks,
                32,
                extensions_ref,
            )
            .await
        })
        .await??;
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
    if !files_to_delete.is_empty() {
        let confirmed = force
            || ui.confirm(PromptConfirm {
                prompt: "Delete unused files?".to_string(),
                default: false,
            })?;
        if confirmed {
            for file in files_to_delete {
                let target = config.content_dir.join(&file.local_path);
                if delete_bound(&config.content_dir, &target, config.allow_symlinks)? {
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

#[cfg(unix)]
fn delete_bound(
    content_dir: &Path,
    target: &Path,
    allow_symlinks: bool,
) -> Result<bool, CrspError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    if !Path::new(target).starts_with(content_dir)
        || (!allow_symlinks && fs::symlink_metadata(target)?.file_type().is_symlink())
    {
        return Ok(false);
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
    match unlink_error {
        None => Ok(true),
        Some(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(false),
        Some(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
fn delete_bound(
    content_dir: &Path,
    target: &Path,
    allow_symlinks: bool,
) -> Result<bool, CrspError> {
    if !target.starts_with(content_dir)
        || (!allow_symlinks && fs::symlink_metadata(target)?.file_type().is_symlink())
    {
        return Ok(false);
    }
    fs::remove_file(target)?;
    Ok(true)
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}
