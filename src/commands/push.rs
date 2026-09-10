use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};

use crate::api::ApiClient;
use crate::core::config::ProjectConfig;
use crate::core::files::{PushResult, prepare_push, put_push_files};
use crate::error::CrspError;
use crate::output::Output;
use crate::ui::{PromptAdapter, PromptConfirm, Ui};

/// `push --watch` debounce (clasp `watchLocalFiles`, spec §2.6).
pub const WATCH_DEBOUNCE_MS: u64 = 500;
/// Banner printed at the start of the `push --watch` loop (clasp push.ts:141).
pub const MESSAGE_WAITING_FOR_CHANGES: &str = "Waiting for changes...";

pub async fn watch_files<'a, F>(
    root: &Path,
    debounce: Duration,
    callback: F,
) -> Result<(), CrspError>
where
    F: FnMut(Vec<PathBuf>) -> Pin<Box<dyn Future<Output = Result<bool, CrspError>> + 'a>> + 'a,
{
    watch_files_filtered(root, debounce, |_| true, callback).await
}

pub async fn watch_files_filtered<'a, F, P>(
    root: &Path,
    debounce: Duration,
    mut accepted: P,
    mut callback: F,
) -> Result<(), CrspError>
where
    F: FnMut(Vec<PathBuf>) -> Pin<Box<dyn Future<Output = Result<bool, CrspError>> + 'a>> + 'a,
    P: FnMut(&Path) -> bool,
{
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = sender.send(result);
    })
    .map_err(|error| CrspError::Io(std::io::Error::other(error.to_string())))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| CrspError::Io(std::io::Error::other(error.to_string())))?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            event = receiver.recv() => {
                let Some(event) = event else { return Ok(()); };
                let mut paths = event_paths(event)?;
                let deadline = tokio::time::sleep(debounce);
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        _ = &mut deadline => break,
                        next = receiver.recv() => {
                            match next {
                                Some(next) => paths.extend(event_paths(next)?),
                                None => return Ok(()),
                            }
                        }
                    }
                }
                paths.retain(|path| accepted(path));
                paths.sort();
                paths.dedup();
                if !paths.is_empty() && !callback(paths).await? { return Ok(()); }
            }
        }
    }
}

fn event_paths(event: notify::Result<notify::Event>) -> Result<Vec<PathBuf>, CrspError> {
    let event = event.map_err(|error| CrspError::Io(std::io::Error::other(error.to_string())))?;
    if matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        Ok(event.paths)
    } else {
        Ok(Vec::new())
    }
}

pub async fn push<A: PromptAdapter>(
    client: &ApiClient,
    config: &ProjectConfig,
    cwd: &Path,
    mut force: bool,
    watch: bool,
    ui: &Ui<A>,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<PushResult, CrspError> {
    let result = prepare_push(client, config).await?;
    if !force
        && result
            .changed
            .iter()
            .any(|file| file.local_path == "appsscript.json")
    {
        if !ui.confirm(PromptConfirm {
            prompt: "Manifest file has been updated. Do you want to push and overwrite?"
                .to_string(),
            default: false,
        })? {
            output.message("Skipping push.");
            return Ok(result);
        }
        force = true;
    }
    put_push_files(client, config, &result).await?;
    print_result(cwd, config, &result, output)?;
    if !watch {
        return Ok(result);
    }
    output.message(MESSAGE_WAITING_FOR_CHANGES);
    let client_ref = client;
    let config_ref = config;
    let ui_ref = ui;
    let output_ref = std::rc::Rc::new(std::cell::RefCell::new(output));
    let force_state = std::sync::Arc::new(std::sync::Mutex::new(force));
    let ignore = config.ignore_matcher().await?;
    let project_root = config.project_root_dir.clone();
    watch_files_filtered(
        &config.content_dir,
        Duration::from_millis(WATCH_DEBOUNCE_MS),
        move |path| is_tracked_event_path(&project_root, &ignore, path),
        move |paths| {
            let relevant = paths.iter().any(|path| {
                path.file_name().and_then(|name| name.to_str()) == Some("appsscript.json")
                    || path.extension().is_some_and(|extension| {
                        extension == "js"
                            || extension == "gs"
                            || extension == "html"
                            || extension == "json"
                    })
            });
            let force_state = std::sync::Arc::clone(&force_state);
            let output_ref = std::rc::Rc::clone(&output_ref);
            Box::pin(async move {
                if !relevant {
                    return Ok(true);
                }
                let next = prepare_push(client_ref, config_ref).await?;
                let needs_confirmation = !*force_state.lock().unwrap()
                    && next
                        .changed
                        .iter()
                        .any(|file| file.local_path == "appsscript.json");
                if needs_confirmation {
                    if !ui_ref.confirm(PromptConfirm {
                        prompt:
                            "Manifest file has been updated. Do you want to push and overwrite?"
                                .to_string(),
                        default: false,
                    })? {
                        return Ok(false);
                    }
                    *force_state.lock().unwrap() = true;
                }
                put_push_files(client_ref, config_ref, &next).await?;
                print_result(cwd, config_ref, &next, *output_ref.borrow_mut())?;
                Ok(true)
            })
        },
    )
    .await?;
    Ok(result)
}

/// clasp push.ts `new Date().toLocaleTimeString()` (en-US default ICU
/// rendering: 12-hour `h:mm:ss AM/PM` in the process-local timezone).
pub fn format_push_time(offset: time::UtcOffset) -> String {
    let now = time::OffsetDateTime::now_utc().to_offset(offset);
    let (hour, suffix) = match now.hour() {
        0 => (12, "AM"),
        12 => (12, "PM"),
        hour if hour < 12 => (hour, "AM"),
        hour => (hour - 12, "PM"),
    };
    format!("{hour}:{:02}:{:02} {suffix}", now.minute(), now.second())
}

fn print_result(
    cwd: &Path,
    config: &ProjectConfig,
    result: &PushResult,
    output: &mut Output<impl std::io::Write, impl std::io::Write>,
) -> Result<(), CrspError> {
    if result.up_to_date {
        // clasp push.ts:100-106: JSON mode prints an empty array for no
        // pending changes; human mode prints the up-to-date line.
        if output.is_json() {
            output.print_json(&Vec::<String>::new())?;
        } else {
            output.message("Script is already up to date.");
        }
        return Ok(());
    }
    // clasp push.ts: JSON mode prints the pushed paths; human mode prints
    // `Pushed {count} files at {toLocaleTimeString()}.` plus one
    // `└─ {cwd-relative path}` line per file (files.ts cwd-relative
    // `localPath`).
    let display_paths: Vec<String> = result
        .files
        .iter()
        .map(|file| {
            crate::commands::show_file_status::cwd_relative(
                cwd,
                &config.content_dir,
                &file.local_path,
            )
        })
        .collect();
    if output.is_json() {
        output.print_json(&display_paths)?;
    } else {
        output.message(&crate::i18n::pushed_files(
            result.files.len(),
            &format_push_time(crate::commands::tail_logs::local_utc_offset()),
        ));
        for path in &display_paths {
            output.message(&format!("└─ {path}"));
        }
    }
    Ok(())
}

pub fn is_tracked_event_path(
    project_root: &Path,
    ignore: &crate::core::ignore::IgnoreMatcher,
    path: &Path,
) -> bool {
    path.strip_prefix(project_root)
        .ok()
        .map(|relative| ignore.is_tracked(&relative.to_string_lossy().replace('\\', "/")))
        .unwrap_or(false)
}

pub fn content_dir(config: &ProjectConfig) -> &Path {
    &config.content_dir
}
