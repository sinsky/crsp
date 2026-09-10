//! Tests for the demand-backed UI boundary and title humanization.

use std::cell::{Cell, RefCell};
use std::io::{self, IsTerminal};
use std::path::Path;

use crsp::api::{ApiClient, ApiClientConfig};
use crsp::core::config::ProjectConfig;
use crsp::error::CrspError;
use crsp::text::humanize_title;
use crsp::ui::{
    DemandAdapter, PromptAdapter, PromptConfirm, PromptDialog, PromptInput, PromptMultiSelect,
    PromptSelect, PromptSpinner, Ui,
};
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct FakeAdapter {
    interactive: bool,
    input_responses: RefCell<Vec<String>>,
    select_responses: RefCell<Vec<String>>,
    multi_select_responses: RefCell<Vec<Vec<String>>>,
    dialog_responses: RefCell<Vec<String>>,
    confirm_response: Cell<bool>,
    interrupt_input: Cell<bool>,
    io_fail_input: Cell<bool>,
    input_calls: Cell<usize>,
    select_calls: Cell<usize>,
    multi_select_calls: Cell<usize>,
    confirm_calls: Cell<usize>,
    dialog_calls: Cell<usize>,
    spinner_starts: RefCell<Vec<String>>,
    spinner_stops: Cell<usize>,
}

impl FakeAdapter {
    fn interactive() -> Self {
        Self {
            interactive: true,
            ..Self::default()
        }
    }

    fn non_interactive() -> Self {
        Self::default()
    }
}

impl PromptAdapter for FakeAdapter {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn input(&self, _spec: &PromptInput) -> io::Result<String> {
        self.input_calls.set(self.input_calls.get() + 1);
        if self.interrupt_input.get() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if self.io_fail_input.get() {
            return Err(io::Error::other("broken pipe"));
        }
        Ok(self.input_responses.borrow_mut().pop().unwrap_or_default())
    }

    fn select(&self, _spec: &PromptSelect) -> io::Result<String> {
        self.select_calls.set(self.select_calls.get() + 1);
        Ok(self.select_responses.borrow_mut().pop().unwrap_or_default())
    }

    fn multi_select(&self, _spec: &PromptMultiSelect) -> io::Result<Vec<String>> {
        self.multi_select_calls
            .set(self.multi_select_calls.get() + 1);
        Ok(self
            .multi_select_responses
            .borrow_mut()
            .pop()
            .unwrap_or_default())
    }

    fn confirm(&self, _spec: &PromptConfirm) -> io::Result<bool> {
        self.confirm_calls.set(self.confirm_calls.get() + 1);
        Ok(self.confirm_response.get())
    }

    fn dialog(&self, _spec: &PromptDialog) -> io::Result<String> {
        self.dialog_calls.set(self.dialog_calls.get() + 1);
        Ok(self.dialog_responses.borrow_mut().pop().unwrap_or_default())
    }

    fn spinner<T, F: FnOnce() -> T>(&self, spec: PromptSpinner, f: F) -> io::Result<T> {
        self.spinner_starts.borrow_mut().push(spec.message);
        let result = f();
        self.spinner_stops.set(self.spinner_stops.get() + 1);
        Ok(result)
    }
}

fn select_spec() -> PromptSelect {
    PromptSelect {
        prompt: "Choose a project:".to_string(),
        options: vec![
            ("id-1".to_string(), "Project One".to_string()),
            ("id-2".to_string(), "Project Two".to_string()),
        ],
        default: Some("id-2".to_string()),
    }
}

fn input_spec() -> PromptInput {
    PromptInput {
        prompt: "Give a description:".to_string(),
        placeholder: None,
        default: Some("fallback".to_string()),
    }
}

fn dialog_spec() -> PromptDialog {
    PromptDialog {
        title: "Delete this project?".to_string(),
        description: "This cannot be undone.".to_string(),
        buttons: vec!["Cancel".to_string(), "Delete".to_string()],
        default: Some(0),
    }
}

#[test]
fn humanize_title_matches_clasp_inflection() {
    assert_eq!(humanize_title("my-project-folder"), "My-project-folder");
    assert_eq!(humanize_title("My_App"), "My app");
    assert_eq!(humanize_title("blog_id"), "Blog");
    assert_eq!(humanize_title("blog_ids"), "Blog");
    assert_eq!(humanize_title("hello_world_id"), "Hello world");
    assert_eq!(humanize_title("ABC_def"), "Abc def");
    assert_eq!(humanize_title("plain"), "Plain");
    assert_eq!(humanize_title("_id"), "");
    assert_eq!(humanize_title(""), "");
}

#[test]
fn non_interactive_confirm_falls_back_to_the_default_without_prompting() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    assert!(
        !ui.confirm(PromptConfirm {
            prompt: "Manifest will be overwritten. Continue?".to_string(),
            default: false,
        })
        .unwrap()
    );
    assert!(
        Ui::new(FakeAdapter::non_interactive())
            .confirm(PromptConfirm {
                prompt: "Continue?".to_string(),
                default: true,
            })
            .unwrap()
    );
}

#[test]
fn non_interactive_input_uses_the_default_value() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    assert_eq!(ui.input(input_spec()).unwrap(), "fallback");
}

#[test]
fn non_interactive_input_without_default_is_aborted() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    let error = ui
        .input(PromptInput {
            prompt: "Give a description:".to_string(),
            placeholder: None,
            default: None,
        })
        .unwrap_err();
    assert!(matches!(error, CrspError::Aborted), "{error:?}");
}

#[test]
fn non_interactive_select_uses_the_default_option() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    assert_eq!(ui.select(select_spec()).unwrap(), "id-2");
}

#[test]
fn non_interactive_select_without_default_is_aborted() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    let spec = PromptSelect {
        default: None,
        ..select_spec()
    };
    assert!(matches!(ui.select(spec), Err(CrspError::Aborted)));
}

#[test]
fn non_interactive_multi_select_returns_the_defaults() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    let spec = PromptMultiSelect {
        prompt: "Delete files:".to_string(),
        options: vec![("a".to_string(), "A".to_string())],
        defaults: vec!["a".to_string()],
    };
    assert_eq!(ui.multi_select(spec).unwrap(), vec!["a"]);
    let empty = PromptMultiSelect {
        prompt: "Delete files:".to_string(),
        options: vec![("a".to_string(), "A".to_string())],
        defaults: Vec::new(),
    };
    assert!(ui.multi_select(empty).unwrap().is_empty());
}

#[test]
fn non_interactive_dialog_uses_the_default_button() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    assert_eq!(ui.dialog(dialog_spec()).unwrap(), "Cancel");
    let no_default = PromptDialog {
        default: None,
        ..dialog_spec()
    };
    let ui = Ui::new(FakeAdapter::non_interactive());
    assert!(matches!(ui.dialog(no_default), Err(CrspError::Aborted)));
}

#[test]
fn non_interactive_spinner_runs_the_closure_without_rendering() {
    let ui = Ui::new(FakeAdapter::non_interactive());
    let value = ui.with_spinner("Pushing files...", || 41 + 1).unwrap();
    assert_eq!(value, 42);
}

#[test]
fn interactive_prompts_delegate_to_the_adapter() {
    let mut adapter = FakeAdapter::interactive();
    adapter.input_responses = RefCell::new(vec!["typed answer".to_string()]);
    adapter.select_responses = RefCell::new(vec!["id-1".to_string()]);
    adapter.multi_select_responses = RefCell::new(vec![vec!["a".to_string(), "b".to_string()]]);
    adapter.dialog_responses = RefCell::new(vec!["Delete".to_string()]);
    adapter.confirm_response = Cell::new(true);
    let ui = Ui::new(adapter);
    assert_eq!(ui.input(input_spec()).unwrap(), "typed answer");
    assert_eq!(ui.select(select_spec()).unwrap(), "id-1");
    assert_eq!(
        ui.multi_select(PromptMultiSelect {
            prompt: "Delete files:".to_string(),
            options: vec![],
            defaults: vec![],
        })
        .unwrap(),
        vec!["a", "b"]
    );
    assert!(
        ui.confirm(PromptConfirm {
            prompt: "Continue?".to_string(),
            default: false,
        })
        .unwrap()
    );
    assert_eq!(ui.dialog(dialog_spec()).unwrap(), "Delete");
}

#[test]
fn interactive_spinner_wraps_the_closure_with_lifecycle_events() {
    let ui = Ui::new(FakeAdapter::interactive());
    let value = ui.with_spinner("Fetching files...", || "done").unwrap();
    assert_eq!(value, "done");
    let adapter = ui.into_adapter();
    assert_eq!(
        adapter.spinner_starts.borrow().clone(),
        vec!["Fetching files...".to_string()]
    );
    assert_eq!(adapter.spinner_stops.get(), 1);
    assert_eq!(adapter.input_calls.get(), 0);
}

#[test]
fn spinner_still_stops_when_the_closure_fails() {
    let ui = Ui::new(FakeAdapter::interactive());
    let result = ui
        .with_spinner("Pushing files...", || -> Result<i32, CrspError> {
            Err(CrspError::Config("boom".to_string()))
        })
        .unwrap();
    assert!(matches!(result, Err(CrspError::Config(_))));
    let adapter = ui.into_adapter();
    assert_eq!(adapter.spinner_stops.get(), 1);
}

#[test]
fn prompt_cancellation_maps_to_aborted() {
    let adapter = FakeAdapter {
        interrupt_input: Cell::new(true),
        ..FakeAdapter::interactive()
    };
    let ui = Ui::new(adapter);
    let error = ui.input(input_spec()).unwrap_err();
    assert!(matches!(error, CrspError::Aborted), "{error:?}");
}

#[test]
fn prompt_io_errors_map_to_the_io_variant() {
    let adapter = FakeAdapter {
        io_fail_input: Cell::new(true),
        ..FakeAdapter::interactive()
    };
    let ui = Ui::new(adapter);
    let error = ui.input(input_spec()).unwrap_err();
    assert!(matches!(error, CrspError::Io(_)), "{error:?}");
}

#[test]
fn is_interactive_is_delegated() {
    assert!(Ui::new(FakeAdapter::interactive()).is_interactive());
    assert!(!Ui::new(FakeAdapter::non_interactive()).is_interactive());
}

#[test]
fn demand_adapter_reports_non_interactive_in_captured_tests() {
    if io::stdout().is_terminal() {
        return;
    }
    assert!(!DemandAdapter.is_interactive());
}

// ---------------------------------------------------------------------------
// §2.1 spinner message wiring: a TTY-interactive adapter must receive the
// command's spinner message around each wrapped service call.
// ---------------------------------------------------------------------------

fn configured_config(root: &Path) -> ProjectConfig {
    ProjectConfig {
        config_file_path: root.join(".clasp.json"),
        project_root_dir: root.to_path_buf(),
        content_dir: root.to_path_buf(),
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

fn api_client(base: &str) -> ApiClient {
    let refresh: crsp::api::RefreshFn =
        std::sync::Arc::new(|| Box::pin(async { Ok("token".to_string()) }));
    ApiClient::with_base_urls(
        ApiClientConfig::new("token", refresh),
        crsp::api::BaseUrls {
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

#[tokio::test]
async fn list_versions_shows_the_spinner_message_on_an_interactive_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": [
                {"versionNumber": 1, "description": "first"},
                {"versionNumber": 2},
                {"versionNumber": 3, "description": "third"},
            ]
        })))
        .mount(&server)
        .await;
    let client = api_client(&server.uri());
    let temp = TempDir::new().unwrap();
    let config = configured_config(temp.path());
    let mut out = Vec::new();
    let mut output = crsp::output::Output::new(false, &mut out, Vec::new());
    let ui = Ui::new(FakeAdapter::interactive());
    crsp::commands::list_versions::list_versions(&client, &config, None, &ui, &mut output)
        .await
        .unwrap();
    let adapter = ui.into_adapter();
    assert_eq!(
        adapter.spinner_starts.borrow().clone(),
        vec!["Fetching versions...".to_string()]
    );
    assert_eq!(adapter.spinner_stops.get(), 1);
}
