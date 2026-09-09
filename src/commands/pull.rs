use std::fs;
use std::path::Path;

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{LocalExtensions, PullFile, PullResult, pull_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

pub async fn pull<A: PromptAdapter>(
    _client: &ApiClient,
    config: &ProjectConfig,
    remote: &[PullFile],
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
    let mut result = pull_files(
        remote,
        &config.content_dir,
        config.allow_symlinks,
        32,
        &extensions,
    )
    .await?;
    if delete_unused && (force || ui.is_interactive()) {
        let confirmed = force
            || ui.confirm(PromptConfirm {
                prompt: "Delete unused files?".to_string(),
                default: false,
            })?;
        if confirmed {
            let remote_names: std::collections::HashSet<_> = remote
                .iter()
                .map(|file| extensions.local_name(file))
                .collect();
            let tracked = crate::core::files::collect_local_files(config).await?;
            for file in tracked.files {
                if !remote_names.contains(&file.local_path) {
                    let target = config.content_dir.join(&file.local_path);
                    if delete_bound(&config.content_dir, &target, config.allow_symlinks)? {
                        result.deleted.push(file.local_path);
                    }
                }
            }
        }
    }
    if output.is_json() {
        #[derive(serde::Serialize)]
        struct PullOutput<'a> {
            #[serde(rename = "pulledFiles")]
            pulled_files: &'a [String],
            #[serde(rename = "deletedFiles")]
            deleted_files: &'a [String],
        }
        output.print_json(&PullOutput {
            pulled_files: &result.written,
            deleted_files: &result.deleted,
        })?;
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
