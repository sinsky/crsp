//! Prompt and spinner boundary over `demand` (spec §3.1, brief Step 5).
//!
//! Command modules never touch `demand` directly: every interaction goes
//! through [`Ui`], which consults the adapter's interactivity flag (stdout TTY,
//! matching clasp's `process.stdout.isTTY`) and applies deterministic
//! noninteractive fallbacks so prompt logic never leaks into commands.

use std::io::{self, IsTerminal};
use std::ops::{Deref, DerefMut};

use crate::error::CrspError;

/// Data for a free-form text prompt.
#[derive(Debug, Clone)]
pub struct PromptInput {
    pub prompt: String,
    pub placeholder: Option<String>,
    pub default: Option<String>,
}

/// Data for a single-choice prompt; options are `(value, label)` pairs.
#[derive(Debug, Clone)]
pub struct PromptSelect {
    pub prompt: String,
    pub options: Vec<(String, String)>,
    pub default: Option<String>,
}

/// Data for a multi-choice prompt.
#[derive(Debug, Clone)]
pub struct PromptMultiSelect {
    pub prompt: String,
    pub options: Vec<(String, String)>,
    pub defaults: Vec<String>,
}

/// Data for a yes/no prompt.
#[derive(Debug, Clone)]
pub struct PromptConfirm {
    pub prompt: String,
    pub default: bool,
}

/// Data for a button dialog; resolves to the selected button's label.
#[derive(Debug, Clone)]
pub struct PromptDialog {
    pub title: String,
    pub description: String,
    pub buttons: Vec<String>,
    pub default: Option<usize>,
}

/// Data for a spinner wrapping a blocking operation.
#[derive(Debug, Clone)]
pub struct PromptSpinner {
    pub message: String,
}

/// Injectable adapter behind [`Ui`] so tests can drive prompts deterministically.
pub trait PromptAdapter {
    /// Whether the environment is interactive (stdout is a TTY).
    fn is_interactive(&self) -> bool;

    fn input(&self, spec: &PromptInput) -> io::Result<String>;

    fn select(&self, spec: &PromptSelect) -> io::Result<String>;

    fn multi_select(&self, spec: &PromptMultiSelect) -> io::Result<Vec<String>>;

    fn confirm(&self, spec: &PromptConfirm) -> io::Result<bool>;

    fn dialog(&self, spec: &PromptDialog) -> io::Result<String>;

    /// Runs `f` under a spinner; the spinner is only rendered interactively.
    fn spinner<T, F>(&self, spec: PromptSpinner, f: F) -> io::Result<T>
    where
        F: FnOnce() -> T + Send,
        T: Send;
}

/// The production adapter wrapping `demand`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DemandAdapter;

impl PromptAdapter for DemandAdapter {
    fn is_interactive(&self) -> bool {
        io::stdout().is_terminal()
    }

    fn input(&self, spec: &PromptInput) -> io::Result<String> {
        let mut input = demand::Input::new(spec.prompt.clone());
        if let Some(placeholder) = &spec.placeholder {
            input = input.placeholder(placeholder);
        }
        if let Some(default) = &spec.default {
            input = input.default_value(default.clone());
        }
        input.run()
    }

    fn select(&self, spec: &PromptSelect) -> io::Result<String> {
        let mut select = demand::Select::new(spec.prompt.clone());
        for (value, label) in &spec.options {
            select = select.option(demand::DemandOption::new(value.clone()).label(label));
        }
        select.run()
    }

    fn multi_select(&self, spec: &PromptMultiSelect) -> io::Result<Vec<String>> {
        let mut select = demand::MultiSelect::new(spec.prompt.clone());
        for (value, label) in &spec.options {
            select = select.option(demand::DemandOption::new(value.clone()).label(label));
        }
        select.run()
    }

    fn confirm(&self, spec: &PromptConfirm) -> io::Result<bool> {
        demand::Confirm::new(spec.prompt.clone())
            .selected(spec.default)
            .run()
    }

    fn dialog(&self, spec: &PromptDialog) -> io::Result<String> {
        let mut dialog = demand::Dialog::new(spec.title.clone());
        if !spec.description.is_empty() {
            dialog = dialog.description(&spec.description);
        }
        dialog = dialog.buttons(
            spec.buttons
                .iter()
                .map(|label| demand::DialogButton::new(label))
                .collect(),
        );
        if let Some(index) = spec.default {
            dialog = dialog.selected_button(index);
        }
        dialog.run()
    }

    fn spinner<T, F>(&self, spec: PromptSpinner, f: F) -> io::Result<T>
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        demand::Spinner::new(spec.message)
            .style(&demand::SpinnerStyle::line())
            .run(|_| f())
    }
}

/// The only prompt/spinner boundary command modules may use.
pub struct Ui<A: PromptAdapter = DemandAdapter> {
    adapter: A,
}

impl<A: PromptAdapter> Ui<A> {
    /// Wraps an adapter; pass [`DemandAdapter`] in production.
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }

    /// Whether the environment is interactive (stdout is a TTY).
    pub fn is_interactive(&self) -> bool {
        self.adapter.is_interactive()
    }

    /// Returns the wrapped adapter (used by tests to assert call recording).
    pub fn into_adapter(self) -> A {
        self.adapter
    }

    /// Prompts for free-form text. Noninteractive fallback: the default value,
    /// or [`CrspError::Aborted`] when no default exists.
    pub fn input(&self, spec: PromptInput) -> Result<String, CrspError> {
        if self.adapter.is_interactive() {
            return self.adapter.input(&spec).map_err(map_prompt_error);
        }
        spec.default.ok_or(CrspError::Aborted)
    }

    /// Prompts for a single choice. Noninteractive fallback: the default
    /// option value, or [`CrspError::Aborted`].
    pub fn select(&self, spec: PromptSelect) -> Result<String, CrspError> {
        if self.adapter.is_interactive() {
            return self.adapter.select(&spec).map_err(map_prompt_error);
        }
        spec.default.ok_or(CrspError::Aborted)
    }

    /// Prompts for multiple choices. Noninteractive fallback: the defaults.
    pub fn multi_select(&self, spec: PromptMultiSelect) -> Result<Vec<String>, CrspError> {
        if self.adapter.is_interactive() {
            return self.adapter.multi_select(&spec).map_err(map_prompt_error);
        }
        Ok(spec.defaults)
    }

    /// Prompts for confirmation. Noninteractive fallback: the default answer
    /// (clasp: manifest confirm refused, deletions skipped).
    pub fn confirm(&self, spec: PromptConfirm) -> Result<bool, CrspError> {
        if self.adapter.is_interactive() {
            return self.adapter.confirm(&spec).map_err(map_prompt_error);
        }
        Ok(spec.default)
    }

    /// Shows a button dialog. Noninteractive fallback: the default button's
    /// label, or [`CrspError::Aborted`].
    pub fn dialog(&self, spec: PromptDialog) -> Result<String, CrspError> {
        if self.adapter.is_interactive() {
            return self.adapter.dialog(&spec).map_err(map_prompt_error);
        }
        match spec.default.and_then(|index| spec.buttons.get(index)) {
            Some(label) => Ok(label.clone()),
            None => Err(CrspError::Aborted),
        }
    }

    /// Runs `f` under a spinner, shown only when interactive (clasp's
    /// `withSpinner`). The spinner starts and stops around `f` even on failure.
    pub fn with_spinner<T, F>(&self, message: &str, f: F) -> Result<T, CrspError>
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        if self.adapter.is_interactive() {
            let spec = PromptSpinner {
                message: message.to_string(),
            };
            return self.adapter.spinner(spec, f).map_err(map_prompt_error);
        }
        Ok(f())
    }

    /// Runs the async `body` under a spinner when interactive, and as a plain
    /// await on the calling (ambient) runtime when not interactive.
    ///
    /// Interactive runs hand the body to the spinner's worker thread, where
    /// the ambient runtime context is unavailable, so the body is driven on
    /// the dedicated isolated runtime ([`drive_isolated`]). The non-TTY path
    /// must not switch runtimes or block a thread: it awaits `body` directly,
    /// keeping the pre-spinner async call graph and thread behavior intact.
    ///
    /// `body` resolves to the operation's own value (a `Vec`, a tuple, or a
    /// `Result` the caller still unwraps); `.await?` this method's future to
    /// obtain that value (a spinner-transport error surfaces as
    /// [`CrspError`] on the outer result).
    pub async fn with_async_spinner<Fut>(
        &self,
        message: &str,
        body: Fut,
    ) -> Result<Fut::Output, CrspError>
    where
        Fut: std::future::Future + Send,
        Fut::Output: Send,
    {
        if self.adapter.is_interactive() {
            let spec = PromptSpinner {
                message: message.to_string(),
            };
            let outcome = self
                .adapter
                .spinner(spec, move || drive_isolated(body))
                .map_err(map_prompt_error)?;
            Ok::<Fut::Output, CrspError>(outcome)
        } else {
            Ok::<Fut::Output, CrspError>(body.await)
        }
    }
}

fn map_prompt_error(error: io::Error) -> CrspError {
    if error.kind() == io::ErrorKind::Interrupted {
        CrspError::Aborted
    } else {
        CrspError::Io(error)
    }
}

/// Drives the async `body` to completion on a dedicated scoped worker thread
/// backed by the shared isolated runtime, blocking the calling thread and
/// returning the body's output.
///
/// Used only by [`Ui::with_async_spinner`]'s interactive branch: the demand
/// spinner runs its closure on a raw scoped thread where the ambient runtime
/// context is unavailable, so the body is driven on the isolated runtime
/// instead. The non-TTY path never reaches this function — it awaits the body
/// directly on the calling runtime (see [`Ui::with_async_spinner`]).
///
/// A process-wide Tokio runtime provides the isolated driver
/// (`ISOLATED_RUNTIME`): one runtime instead of one per call, because
/// per-call runtimes are measurable (each builds a thread pool) and their
/// churn starves busy test environments.
static ISOLATED_RUNTIME: std::sync::Mutex<Option<tokio::runtime::Runtime>> =
    std::sync::Mutex::new(None);

/// A `Handle` to the shared isolated runtime, building it on first use.
/// The ambient runtime cannot be reused from the spinner thread (the thread
/// local context is not shared across threads), so service calls are driven
/// on this runtime instead.
fn isolated_handle() -> tokio::runtime::Handle {
    let mut guard = ISOLATED_RUNTIME.lock().unwrap();
    if guard.is_none() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("isolated spinner runtime");
        *guard.deref_mut() = Some(runtime);
    }
    guard.deref().as_ref().unwrap().handle().clone()
}

pub fn drive_isolated<'a, F: std::future::Future + Send + 'a>(body: F) -> F::Output
where
    F::Output: Send + 'a,
{
    let handle = isolated_handle();
    std::thread::scope(|scope| {
        let join = scope.spawn(move || handle.block_on(body));
        match join.join() {
            // The scoped thread's failure to return means it panicked; the
            // scope re-raises before returning, so reaching this fallback is a
            // compiler-only shim.
            Ok(output) => output,
            Err(_) => unreachable!(),
        }
    })
}
