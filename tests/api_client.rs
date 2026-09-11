//! Google API client tests (spec §2.2 endpoints, §6.1 error normalization,
//! §7.1-§7.4 retry/refresh/paging contracts; clasp `core/*` call sites and
//! googleapis-common retry defaults).
//!
//! SECURITY: access/refresh token values are masked placeholders; no real
//! credentials are used or printed anywhere in this suite.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::json;

use google_clasp_rs::api::{
    ApiClient, ApiClientConfig, ApiErrorKind, ApiRequest, BaseUrls, PagedResults,
};
use google_clasp_rs::auth::oauth_client::OAuthClient;
use google_clasp_rs::constants::{
    DISCOVERY_API_BASE_URL, DRIVE_API_BASE_URL, ENV_API_BASE_URL, ENV_DISCOVERY_BASE_URL,
    ENV_DRIVE_BASE_URL, ENV_LOGGING_BASE_URL, ENV_OAUTH2_BASE_URL, ENV_SCRIPT_BASE_URL,
    ENV_SERVICE_USAGE_BASE_URL, ENV_USERINFO_BASE_URL, LOGGING_API_BASE_URL, OAUTH2_API_BASE_URL,
    SCRIPT_API_BASE_URL, SERVICE_USAGE_API_BASE_URL, USERINFO_API_BASE_URL,
};
use google_clasp_rs::error::CrspError;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const ACCESS_TOKEN: &str = "ACCESS-PLACEHOLDER-V1";
const REFRESHED_TOKEN: &str = "ACCESS-PLACEHOLDER-REFRESHED";
const USER_EMAIL: &str = "user@example.com";

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

type Delays = Arc<Mutex<Vec<u64>>>;

struct Recorder {
    delays: Delays,
    refreshes: Arc<AtomicUsize>,
}

impl Recorder {
    fn delays(&self) -> Vec<u64> {
        self.delays.lock().unwrap().clone()
    }

    fn refreshes(&self) -> usize {
        self.refreshes.load(Ordering::SeqCst)
    }
}

fn base_urls_all(base: &str) -> BaseUrls {
    BaseUrls {
        script: base.to_string(),
        drive: base.to_string(),
        service_usage: base.to_string(),
        discovery: base.to_string(),
        logging: base.to_string(),
        oauth2: base.to_string(),
        userinfo: base.to_string(),
    }
}

fn ok_refresh() -> BoxFuture<'static, Result<String, CrspError>> {
    Box::pin(async { Ok(REFRESHED_TOKEN.to_string()) })
}

fn failing_refresh() -> BoxFuture<'static, Result<String, CrspError>> {
    Box::pin(async {
        Err(CrspError::Auth(
            "token refresh failed (masked placeholder)".to_string(),
        ))
    })
}

fn recorder_client(
    base: &str,
    refresh: impl Fn() -> BoxFuture<'static, Result<String, CrspError>> + Send + Sync + 'static,
) -> (ApiClient, Recorder) {
    recorder_client_with(base, refresh, |_| {})
}

fn recorder_client_with(
    base: &str,
    refresh: impl Fn() -> BoxFuture<'static, Result<String, CrspError>> + Send + Sync + 'static,
    tune: impl FnOnce(&mut ApiClientConfig),
) -> (ApiClient, Recorder) {
    let delays: Delays = Arc::default();
    let refreshes = Arc::new(AtomicUsize::new(0));
    let client = {
        let delays_for_sleeper = Arc::clone(&delays);
        let refreshes_for_fn = Arc::clone(&refreshes);
        let refresh = Arc::new(refresh);
        let refresh_fn = Arc::new(move || {
            refreshes_for_fn.fetch_add(1, Ordering::SeqCst);
            refresh()
        });
        let mut config = ApiClientConfig::new(ACCESS_TOKEN, refresh_fn);
        tune(&mut config);
        ApiClient::with_base_urls(config, base_urls_all(base))
            .expect("client construction")
            .with_sleeper(Arc::new(move |duration: Duration| {
                let delays = Arc::clone(&delays_for_sleeper);
                Box::pin(async move { delays.lock().unwrap().push(duration.as_millis() as u64) })
            }))
    };
    (client, Recorder { delays, refreshes })
}

struct Harness {
    server: MockServer,
    client: ApiClient,
    recorder: Recorder,
}

async fn harness_with(
    refresh: impl Fn() -> BoxFuture<'static, Result<String, CrspError>> + Send + Sync + 'static,
) -> Harness {
    let server = MockServer::start().await;
    let (client, recorder) = recorder_client(server.uri().as_str(), refresh);
    Harness {
        server,
        client,
        recorder,
    }
}

async fn harness() -> Harness {
    harness_with(|| ok_refresh()).await
}

async fn harness_failing_refresh() -> Harness {
    harness_with(|| failing_refresh()).await
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

async fn received_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("wiremock records requests")
}

fn query_map(request: &Request) -> BTreeMap<String, String> {
    request.url.query_pairs().into_owned().collect()
}

/// Asserts the request carries the expected masked bearer placeholder without
/// printing the header value.
fn assert_bearer(request: &Request, placeholder: &str) {
    let value = request
        .headers
        .get("authorization")
        .unwrap_or_else(|| panic!("request is missing the authorization header"))
        .to_str()
        .expect("authorization header is ASCII");
    assert!(
        value == format!("Bearer {placeholder}"),
        "authorization header does not carry the expected masked bearer placeholder"
    );
}

fn assert_api_error(error: &CrspError, kind: ApiErrorKind, message: &str) {
    match error {
        CrspError::Api {
            kind: actual_kind,
            message: actual_message,
        } => {
            assert_eq!(actual_kind, &kind, "unexpected ApiErrorKind");
            assert_eq!(actual_message, message, "unexpected API error message");
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

fn default_oauth_client(token_base: &str) -> OAuthClient {
    OAuthClient {
        client_id: "CLIENT-ID-PLACEHOLDER".to_string(),
        client_secret: "CLIENT-SECRET-PLACEHOLDER".to_string(),
        redirect_uri: "http://localhost".to_string(),
        endpoints: google_clasp_rs::auth::oauth_client::AuthEndpoints {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: format!("{token_base}/token"),
            userinfo_url: format!("{token_base}/v2/userinfo"),
        },
    }
}
// ---------------------------------------------------------------------------
// Script API (spec §2.2; clasp core/project.ts, core/files.ts, core/functions.ts)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_project_posts_title_and_parent_id() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .and(body_json(
            json!({"title": "My project", "parentId": "parent-1"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"scriptId": "script-1"})))
        .mount(&h.server)
        .await;

    let project = h
        .client
        .script()
        .create_project("My project", Some("parent-1"))
        .await
        .expect("create project succeeds");
    assert_eq!(project.script_id.as_deref(), Some("script-1"));
    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 1);
    assert_bearer(&requests[0], ACCESS_TOKEN);
}

#[tokio::test]
async fn create_project_omits_absent_parent_id() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .and(body_json(json!({"title": "Solo"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"scriptId": "s1"})))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .script()
        .create_project("Solo", None)
        .await
        .expect("create project succeeds");
}

#[tokio::test]
async fn get_content_parses_files_and_function_set() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                {"name": "appsscript", "type": "JSON", "source": "{}"},
                {
                    "name": "Code",
                    "type": "SERVER_JS",
                    "source": "function f(){}",
                    "functionSet": {"values": [{"name": "f"}]}
                }
            ]
        })))
        .mount(&h.server)
        .await;

    let content = h
        .client
        .script()
        .get_content("s1", None)
        .await
        .expect("content fetch succeeds");
    assert_eq!(content.files().len(), 2);
    assert_eq!(content.files()[1].name.as_deref(), Some("Code"));
    assert_eq!(content.files()[1].file_type.as_deref(), Some("SERVER_JS"));
    let values = content.files()[1]
        .function_set
        .as_ref()
        .and_then(|set| set.values.clone())
        .unwrap_or_default();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].name.as_deref(), Some("f"));
}

#[tokio::test]
async fn get_content_sends_version_number_query() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .and(query_param("versionNumber", "3"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"files": [{"name": "a", "type": "SERVER_JS", "source": "x"}]}),
            ),
        )
        .mount(&h.server)
        .await;

    let content = h
        .client
        .script()
        .get_content("s1", Some(3))
        .await
        .expect("content fetch succeeds");
    assert_eq!(content.files().len(), 1);
    let requests = received_requests(&h.server).await;
    let query = query_map(&requests[0]);
    assert_eq!(query.get("versionNumber").map(String::as_str), Some("3"));
    assert_eq!(query.len(), 1);
}

#[tokio::test]
async fn update_content_puts_file_list() {
    let h = harness().await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/s1/content"))
        .and(body_json(json!({
            "files": [
                {"name": "Code", "type": "SERVER_JS", "source": "function f(){}"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .update_content(
            "s1",
            &[google_clasp_rs::api::PushFile {
                name: "Code".to_string(),
                file_type: "SERVER_JS".to_string(),
                source: "function f(){}".to_string(),
            }],
        )
        .await
        .expect("update content succeeds");
}

#[tokio::test]
async fn create_version_posts_description_and_parses_number() {
    let h = harness().await;
    // Empty description is still serialized (clasp sends `description ?? ''`).
    Mock::given(method("POST"))
        .and(path("/v1/projects/s1/versions"))
        .and(body_json(json!({"description": ""})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 4})))
        .mount(&h.server)
        .await;

    let version = h
        .client
        .script()
        .create_version("s1", "")
        .await
        .expect("create version succeeds");
    assert_eq!(version.version_number, Some(4));
}

#[tokio::test]
async fn list_versions_paginates_with_page_token() {
    let h = harness().await;
    // wiremock checks mocks in mount order: mount the token-specific page-2
    // mock first, then the loose page-1 mock for the tokenless first request.
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .and(query_param("pageToken", "t1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"versions": [{"versionNumber": 3}]})),
        )
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"versions": [{"versionNumber": 1}, {"versionNumber": 2}], "nextPageToken": "t1"}),
            ),
        )
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .script()
        .list_versions("s1")
        .await
        .expect("list versions succeeds");
    assert!(!partial_results);
    let numbers: Vec<i32> = results.iter().filter_map(|v| v.version_number).collect();
    assert_eq!(numbers, vec![1, 2, 3]);

    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 2);
    let first_query = query_map(&requests[0]);
    assert_eq!(first_query.get("pageSize").map(String::as_str), Some("100"));
    assert!(
        !first_query.contains_key("pageToken"),
        "first page must not send pageToken"
    );
    let second_query = query_map(&requests[1]);
    assert_eq!(
        second_query.get("pageToken").map(String::as_str),
        Some("t1")
    );
    assert_eq!(
        second_query.get("pageSize").map(String::as_str),
        Some("100")
    );
}

#[tokio::test]
async fn list_deployments_parses_page() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "deployments": [
                    {"deploymentId": "d1", "deploymentConfig": {"versionNumber": 1, "description": "first"}}
                ]
            })),
        )
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .script()
        .list_deployments("s1")
        .await
        .expect("list deployments succeeds");
    assert!(!partial_results);
    assert_eq!(results[0].deployment_id.as_deref(), Some("d1"));
    let config = results[0].deployment_config.as_ref().expect("config");
    assert_eq!(config.version_number, Some(1));
    assert_eq!(config.description.as_deref(), Some("first"));
}

#[tokio::test]
async fn create_deployment_posts_full_body_and_parses() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/s1/deployments"))
        .and(body_json(json!({
            "description": "d",
            "versionNumber": 2,
            "manifestFileName": "appsscript"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deploymentId": "dep-1",
            "deploymentConfig": {
                "description": "d",
                "versionNumber": 2,
                "scriptId": "s1",
                "manifestFileName": "appsscript"
            }
        })))
        .mount(&h.server)
        .await;

    let deployment = h
        .client
        .script()
        .create_deployment("s1", "d", 2)
        .await
        .expect("create deployment succeeds");
    assert_eq!(deployment.deployment_id.as_deref(), Some("dep-1"));
}

#[tokio::test]
async fn update_deployment_puts_deployment_config() {
    let h = harness().await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .and(body_json(json!({
            "deploymentConfig": {
                "description": "d",
                "versionNumber": 2,
                "scriptId": "s1",
                "manifestFileName": "appsscript"
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"deploymentId": "dep-1", "deploymentConfig": {"versionNumber": 2}}),
        ))
        .mount(&h.server)
        .await;

    let deployment = h
        .client
        .script()
        .update_deployment("s1", "dep-1", "d", 2)
        .await
        .expect("update deployment succeeds");
    assert_eq!(deployment.deployment_id.as_deref(), Some("dep-1"));
}

#[tokio::test]
async fn get_deployment_parses_entry_points() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "deploymentId": "dep-1",
                "entryPoints": [
                    {"entryPointType": "WEB_APP", "webApp": {"url": "https://script.google.com/macros/s/dep-1/exec"}}
                ]
            })),
        )
        .mount(&h.server)
        .await;

    let deployment = h
        .client
        .script()
        .get_deployment("s1", "dep-1")
        .await
        .expect("get deployment succeeds");
    let entry_points = deployment.entry_points.unwrap_or_default();
    assert_eq!(entry_points.len(), 1);
    assert_eq!(entry_points[0].entry_point_type.as_deref(), Some("WEB_APP"));
    assert_eq!(
        entry_points[0]
            .web_app
            .as_ref()
            .and_then(|app| app.url.clone())
            .as_deref(),
        Some("https://script.google.com/macros/s/dep-1/exec")
    );
}

#[tokio::test]
async fn delete_deployment_sends_delete_method() {
    let h = harness().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .script()
        .delete_deployment("s1", "dep-1")
        .await
        .expect("delete deployment succeeds");
}

#[tokio::test]
async fn run_posts_function_parameters_and_devmode() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .and(body_json(json!({
            "function": "myFunc",
            "parameters": [1, "a"],
            "devMode": true
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"response": {"result": 42}, "done": true})),
        )
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .and(body_json(json!({
            "function": "myFunc",
            "parameters": [],
            "devMode": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"done": true})))
        .mount(&h.server)
        .await;

    let dev = h
        .client
        .script()
        .run("s1", Some("myFunc"), json!([1, "a"]), true)
        .await
        .expect("run succeeds");
    assert_eq!(dev, json!({"response": {"result": 42}, "done": true}));

    let nondev = h
        .client
        .script()
        .run("s1", Some("myFunc"), json!([]), false)
        .await
        .expect("non-dev run succeeds");
    assert_eq!(nondev, json!({"done": true}));
}

// ---------------------------------------------------------------------------
// Drive API (spec §2.2; clasp core/project.ts createWithContainer,
// listScripts, trashScript)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_files_sends_query_and_parses() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v3/files"))
        .and(query_param(
            "q",
            "mimeType=\"application/vnd.google-apps.script\"",
        ))
        .and(query_param("pageSize", "100"))
        .and(query_param("fields", "nextPageToken, files(id, name)"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"files": [{"id": "a", "name": "A"}, {"id": "b", "name": "B"}]}),
            ),
        )
        .mount(&h.server)
        .await;

    let PagedResults { results, .. } = h
        .client
        .drive()
        .list_files()
        .await
        .expect("list files succeeds");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id.as_deref(), Some("a"));
    assert_eq!(results[1].name.as_deref(), Some("B"));
}

#[tokio::test]
async fn create_container_posts_mime_type_and_name() {
    let h = harness().await;
    // clasp parity (project.ts:167): drive.files.create with only a
    // requestBody sends a plain JSON POST to /v3/files — no multipart.
    Mock::given(method("POST"))
        .and(path("/v3/files"))
        .and(body_json(json!({
            "mimeType": "application/vnd.google-apps.spreadsheet",
            "name": "Bound"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "container-1"})))
        .mount(&h.server)
        .await;

    let file = h
        .client
        .drive()
        .create_file("application/vnd.google-apps.spreadsheet", "Bound")
        .await
        .expect("container creation succeeds");
    assert_eq!(file.id.as_deref(), Some("container-1"));
}

#[tokio::test]
async fn trash_file_patches_trashed_true() {
    let h = harness().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/files/file-1"))
        .and(body_json(json!({"trashed": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "file-1"})))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .drive()
        .trash_file("file-1")
        .await
        .expect("trash succeeds");
}

// ---------------------------------------------------------------------------
// Service Usage API (spec §2.2, §2.1; clasp core/services.ts)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_enabled_services_paginates_with_page_size_200() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/p1/services"))
        .and(query_param("pageToken", "n1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "services": [
                {"name": "projects/p1/services/docs.googleapis.com",
                 "config": {"name": "docs", "documentation": {"summary": "Docs"}}}
            ]
        })))
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/p1/services"))
        .and(query_param("filter", "state:ENABLED"))
        .and(query_param("pageSize", "200"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "services": [
                {"name": "projects/p1/services/sheets.googleapis.com",
                 "config": {"name": "sheets", "documentation": {"summary": "Sheets"}}}
            ],
            "nextPageToken": "n1"
        })))
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .service_usage()
        .list_enabled("p1")
        .await
        .expect("list enabled services succeeds");
    assert!(!partial_results);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0]
            .config
            .as_ref()
            .and_then(|config| config.name.clone())
            .as_deref(),
        Some("sheets")
    );

    let requests = received_requests(&h.server).await;
    let first_query = query_map(&requests[0]);
    assert_eq!(first_query.get("pageSize").map(String::as_str), Some("200"));
    assert_eq!(
        first_query.get("filter").map(String::as_str),
        Some("state:ENABLED")
    );
    assert!(!first_query.contains_key("pageToken"));
    let second_query = query_map(&requests[1]);
    assert_eq!(
        second_query.get("pageToken").map(String::as_str),
        Some("n1")
    );
}

#[tokio::test]
async fn list_enabled_services_stops_at_max_pages() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/p1/services"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "services": [{"name": "projects/p1/services/sheets.googleapis.com"}],
            "nextPageToken": "endless"
        })))
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .service_usage()
        .list_enabled("p1")
        .await
        .expect("list enabled services succeeds");
    // maxPages 10 with an endless token: partial results (spec §7.3).
    assert!(partial_results);
    assert_eq!(results.len(), 10);
}

#[tokio::test]
async fn enable_service_posts_empty_body() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1/projects/p1/services/sheets.googleapis.com:enable",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .service_usage()
        .enable("p1", "sheets")
        .await
        .expect("enable succeeds");

    let requests = received_requests(&h.server).await;
    assert!(
        requests[0].body.is_empty(),
        "enable must send no request body"
    );
    assert!(
        requests[0].headers.get("content-type").is_none(),
        "enable must not set a content-type header"
    );
}

#[tokio::test]
async fn disable_service_posts_empty_body() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1/projects/p1/services/sheets.googleapis.com:disable",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .service_usage()
        .disable("p1", "sheets")
        .await
        .expect("disable succeeds");
}

// ---------------------------------------------------------------------------
// Discovery API (spec §2.2; clasp core/services.ts getAvailableServices)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_list_apis_gets_preferred() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/discovery/v1/apis"))
        .and(query_param("preferred", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [
                {"id": "sheets:v4", "name": "sheets", "description": "Google Sheets"},
                {"id": "other:v1", "name": "other", "description": "Other"}
            ]
        })))
        .mount(&h.server)
        .await;

    let apis = h
        .client
        .discovery()
        .list_apis()
        .await
        .expect("discovery list succeeds");
    assert_eq!(apis.len(), 2);
    assert_eq!(apis[0].id.as_deref(), Some("sheets:v4"));
    assert_eq!(apis[0].name.as_deref(), Some("sheets"));
}

// ---------------------------------------------------------------------------
// Logging API (spec §2.2; clasp core/logs.ts getLogEntries)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_entries_posts_body_and_paginates_page_token_in_body() {
    let h = harness().await;
    let first_body = json!({
        "resourceNames": ["projects/p1"],
        "filter": "",
        "orderBy": "timestamp desc",
        "pageSize": 100
    });
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(first_body))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"insertId": "i1", "severity": "INFO", "timestamp": "2026-01-01T00:00:00Z", "textPayload": "hi"}],
                "nextPageToken": "t1"
            })),
        )
        .mount(&h.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/entries:list"))
        .and(body_json(json!({
            "resourceNames": ["projects/p1"],
            "filter": "",
            "orderBy": "timestamp desc",
            "pageSize": 100,
            "pageToken": "t1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"insertId": "i2", "severity": "ERROR", "timestamp": "2026-01-01T01:00:00Z",
                         "resource": {"labels": {"function_name": "myFunc"}}}]
        })))
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .logging()
        .list_entries("p1", "")
        .await
        .expect("list entries succeeds");
    assert!(!partial_results);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].insert_id(), Some("i1"));
    assert_eq!(results[0].text_payload(), Some("hi"));
    assert_eq!(results[1].function_name_label(), Some("myFunc"));
    // The raw entry keeps the server key order for --json re-serialization.
    let raw = results[0].raw().as_object().unwrap();
    let keys: Vec<&String> = raw.keys().collect();
    assert_eq!(keys, ["insertId", "severity", "timestamp", "textPayload"]);
}

// ---------------------------------------------------------------------------
// OAuth2 endpoints (spec §2.2; clasp auth/auth.ts getUserInfo, token exchange)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn userinfo_gets_email_with_masked_bearer() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v2/userinfo"))
        .and(header("authorization", format!("Bearer {ACCESS_TOKEN}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "42", "email": USER_EMAIL, "name": "Test User"})),
        )
        .mount(&h.server)
        .await;

    let user = h
        .client
        .oauth2()
        .userinfo()
        .await
        .expect("userinfo succeeds");
    assert_eq!(user.email.as_deref(), Some(USER_EMAIL));
}

#[tokio::test]
async fn exchange_code_posts_form_to_token_endpoint() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "ACCESS-PLACEHOLDER-EXCHANGED",
            "expires_in": 3600,
            "token_type": "Bearer"
        })))
        .mount(&h.server)
        .await;

    let client = default_oauth_client(h.server.uri().as_str());
    let tokens = h
        .client
        .oauth2()
        .exchange_code(
            &client,
            "CODE-PLACEHOLDER",
            "http://localhost:1",
            "VERIFIER-PLACEHOLDER",
        )
        .await
        .expect("exchange succeeds");
    assert_eq!(
        tokens.access_token.as_deref(),
        Some("ACCESS-PLACEHOLDER-EXCHANGED")
    );
    assert_eq!(tokens.expires_in, Some(3600));

    let requests = received_requests(&h.server).await;
    let body = String::from_utf8(requests[0].body.clone()).expect("form body is UTF-8");
    assert!(
        body.contains("grant_type=authorization_code"),
        "body: {body}"
    );
    assert!(body.contains("code=CODE-PLACEHOLDER"), "body: {body}");
    assert!(
        body.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1"),
        "body: {body}"
    );
    assert!(
        body.contains("code_verifier=VERIFIER-PLACEHOLDER"),
        "body: {body}"
    );
    assert!(
        body.contains("client_id=CLIENT-ID-PLACEHOLDER"),
        "body: {body}"
    );
    assert!(
        body.contains("client_secret=CLIENT-SECRET-PLACEHOLDER"),
        "body: {body}"
    );
}

#[tokio::test]
async fn refresh_posts_form_to_token_endpoint() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": REFRESHED_TOKEN,
            "expires_in": 3600
        })))
        .mount(&h.server)
        .await;

    let client = default_oauth_client(h.server.uri().as_str());
    let tokens = h
        .client
        .oauth2()
        .refresh_token(&client, "REFRESH-TOKEN-PLACEHOLDER")
        .await
        .expect("refresh succeeds");
    assert_eq!(tokens.access_token.as_deref(), Some(REFRESHED_TOKEN));

    let requests = received_requests(&h.server).await;
    let body = String::from_utf8(requests[0].body.clone()).expect("form body is UTF-8");
    assert!(body.contains("grant_type=refresh_token"), "body: {body}");
    assert!(
        body.contains("refresh_token=REFRESH-TOKEN-PLACEHOLDER"),
        "body: {body}"
    );
}

// ---------------------------------------------------------------------------
// Status normalization (spec §2.1, §6.1; clasp handleApiError)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_400_maps_to_invalid_argument() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "code": 400,
                "message": "Invalid function",
                "errors": [
                    {"message": "m1", "domain": "global", "reason": "badRequest"},
                    {"message": "m2", "domain": "global", "reason": "badRequest"}
                ]
            }
        })))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .run("s1", Some("f"), json!([]), true)
        .await
        .expect_err("400 fails");
    assert_api_error(&error, ApiErrorKind::InvalidArgument, "m1\nm2");
    assert_eq!(h.recorder.refreshes(), 0);
    assert!(h.recorder.delays().is_empty());
}

#[tokio::test]
async fn status_403_maps_to_not_authorized_without_refresh() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": {"code": 403, "message": "The caller does not have permission"}
        })))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .get_content("s1", None)
        .await
        .expect_err("403 fails");
    assert_api_error(
        &error,
        ApiErrorKind::NotAuthorized,
        "The caller does not have permission",
    );
    assert_eq!(
        h.recorder.refreshes(),
        0,
        "403 must not trigger a token refresh"
    );
    assert_eq!(received_requests(&h.server).await.len(), 1);
}

#[tokio::test]
async fn status_404_maps_to_not_found() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/missing/content"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {"code": 404, "message": "Requested entity was not found."}
        })))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .get_content("missing", None)
        .await
        .expect_err("404 fails");
    assert_api_error(
        &error,
        ApiErrorKind::NotFound,
        "Requested entity was not found.",
    );
    assert_eq!(received_requests(&h.server).await.len(), 1);
}

#[tokio::test]
async fn status_500_maps_to_unexpected_api_error() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({"error": {"message": "Backend error"}})),
        )
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .run("s1", Some("f"), json!([]), true)
        .await
        .expect_err("500 fails");
    assert_api_error(&error, ApiErrorKind::UnexpectedApiError, "Backend error");
    assert_eq!(received_requests(&h.server).await.len(), 1);
}

#[tokio::test]
async fn error_message_uses_string_error() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .respond_with(ResponseTemplate::new(502).set_body_json(json!({"error": "generic"})))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .run("s1", Some("f"), json!([]), true)
        .await
        .expect_err("502 fails");
    assert_api_error(&error, ApiErrorKind::UnexpectedApiError, "generic");
}

#[tokio::test]
async fn error_message_uses_raw_body_when_not_json() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .respond_with(
            ResponseTemplate::new(500)
                .insert_header("content-type", "text/plain")
                .set_body_string("backend exploded"),
        )
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .run("s1", Some("f"), json!([]), true)
        .await
        .expect_err("500 fails");
    assert_api_error(&error, ApiErrorKind::UnexpectedApiError, "backend exploded");
}

#[tokio::test]
async fn error_message_defaults_when_body_empty() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/scripts/s1/run"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .run("s1", Some("f"), json!([]), true)
        .await
        .expect_err("503 fails");
    assert_api_error(
        &error,
        ApiErrorKind::UnexpectedApiError,
        "Request failed with status code 503",
    );
}

// ---------------------------------------------------------------------------
// Retry contract (spec §7.1, §7.2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_retries_transient_statuses_with_backoff_delays() {
    let h = harness().await;
    // wiremock checks mocks in mount order, and an exhausted mock falls
    // through to the next: mount the failures in request order, then the
    // always-success mock.
    for status in [500u16, 408, 429] {
        Mock::given(method("GET"))
            .and(path("/v1/projects/s1/versions"))
            .respond_with(ResponseTemplate::new(status))
            .up_to_n_times(1)
            .mount(&h.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"versions": [{"versionNumber": 7}]})),
        )
        .mount(&h.server)
        .await;

    let versions = h
        .client
        .script()
        .list_versions("s1")
        .await
        .expect("eventually succeeds");
    assert_eq!(versions.results.len(), 1);
    assert_eq!(versions.results[0].version_number, Some(7));

    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 4, "initial + 3 status retries");
    assert_eq!(h.recorder.delays(), vec![100, 500, 1500]);
    for request in &requests {
        assert_bearer(request, ACCESS_TOKEN);
    }
}

#[tokio::test]
async fn get_stops_after_three_status_retries() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .list_versions("s1")
        .await
        .expect_err("exhausted retries fail");
    assert_api_error(
        &error,
        ApiErrorKind::UnexpectedApiError,
        "Request failed with status code 500",
    );
    assert_eq!(received_requests(&h.server).await.len(), 4);
    assert_eq!(h.recorder.delays(), vec![100, 500, 1500]);
}

#[tokio::test]
async fn get_does_not_retry_400() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "bad"}})),
        )
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .list_versions("s1")
        .await
        .expect_err("400 is not retryable");
    assert_api_error(&error, ApiErrorKind::InvalidArgument, "bad");
    assert!(h.recorder.delays().is_empty());
}

#[tokio::test]
async fn post_is_never_transient_retried() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .create_version("s1", "d")
        .await
        .expect_err("POST must not be retried");
    assert_api_error(
        &error,
        ApiErrorKind::UnexpectedApiError,
        "Request failed with status code 503",
    );
    assert!(h.recorder.delays().is_empty());
}

#[tokio::test]
async fn put_retries_idempotently() {
    let h = harness().await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .update_content(
            "s1",
            &[google_clasp_rs::api::PushFile {
                name: "Code".to_string(),
                file_type: "SERVER_JS".to_string(),
                source: "x".to_string(),
            }],
        )
        .await
        .expect("put succeeds after one retry");
    assert_eq!(received_requests(&h.server).await.len(), 2);
    assert_eq!(h.recorder.delays(), vec![100]);
}

#[tokio::test]
async fn delete_retries_idempotently() {
    let h = harness().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .delete_deployment("s1", "dep-1")
        .await
        .expect("delete succeeds after one retry");
    assert_eq!(received_requests(&h.server).await.len(), 2);
    assert_eq!(h.recorder.delays(), vec![100]);
}

#[tokio::test]
async fn head_and_options_are_idempotent_methods() {
    let h = harness().await;
    Mock::given(method("HEAD"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&h.server)
        .await;
    Mock::given(method("OPTIONS"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("OPTIONS"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&h.server)
        .await;

    let url = format!("{}/v1/projects/s1/content", h.server.uri());
    let head = h
        .client
        .request(ApiRequest {
            method: reqwest::Method::HEAD,
            url: url.clone(),
            body: None,
        })
        .await
        .expect("HEAD succeeds after retry");
    assert_eq!(head.status().as_u16(), 200);
    let options = h
        .client
        .request(ApiRequest {
            method: reqwest::Method::OPTIONS,
            url,
            body: None,
        })
        .await
        .expect("OPTIONS succeeds after retry");
    assert_eq!(options.status().as_u16(), 200);

    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 4, "each idempotent method retries once");
    assert_eq!(h.recorder.delays(), vec![100, 100]);
}

async fn closed_port_base() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn network_errors_retry_twice_max() {
    let base = closed_port_base().await;
    let (client, recorder) = recorder_client(&base, || ok_refresh());
    let url = format!("{base}/v1/projects/s1/versions");

    let error = client
        .request(ApiRequest {
            method: reqwest::Method::GET,
            url,
            body: None,
        })
        .await
        .expect_err("closed port fails");
    assert!(
        matches!(
            error,
            CrspError::Api {
                kind: ApiErrorKind::UnexpectedApiError,
                ..
            }
        ),
        "network errors normalize to UNEXPECTED_API_ERROR, got {error:?}"
    );
    assert_eq!(
        recorder.delays(),
        vec![100, 500],
        "initial + max 2 network retries"
    );
}

#[tokio::test]
async fn post_never_retries_network_errors() {
    let base = closed_port_base().await;
    let (client, recorder) = recorder_client(&base, || ok_refresh());
    let url = format!("{base}/v1/projects/s1/versions");

    let error = client
        .request(ApiRequest {
            method: reqwest::Method::POST,
            url,
            body: Some(google_clasp_rs::api::ApiBody::Json(
                json!({"description": "d"}),
            )),
        })
        .await
        .expect_err("closed port fails");
    assert!(
        matches!(
            error,
            CrspError::Api {
                kind: ApiErrorKind::UnexpectedApiError,
                ..
            }
        ),
        "got {error:?}"
    );
    assert!(
        recorder.delays().is_empty(),
        "POST never retries network errors"
    );
}

#[tokio::test]
async fn network_and_status_retry_counters_are_independent() {
    // First attempt: a real 500 response. Later attempts: the listener accepts
    // and immediately closes the connection (no response), exercising the
    // separate network retry counter.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let mut served = false;
        loop {
            // A transient `accept` error (Windows can surface one when an
            // earlier connection resets) must not tear down the listener
            // before it serves its single 500; keep accepting.
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    continue;
                }
            };
            if !served {
                served = true;
                // Read the request before replying. Closing a socket that
                // still holds unread request bytes sends a TCP RST, which on
                // Windows can discard the buffered 500; consuming the request
                // and shutting down gracefully delivers it reliably.
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
                let _ = socket.shutdown().await;
            }
            // Subsequent connections are dropped without a response: the
            // request bytes stay unread, so the close is a reset the client
            // observes as a network error.
        }
    });
    let base = format!("http://127.0.0.1:{port}");
    let (client, recorder) = recorder_client(&base, || ok_refresh());

    let error = client
        .request(ApiRequest {
            method: reqwest::Method::GET,
            url: format!("{base}/v1/projects/s1/versions"),
            body: None,
        })
        .await
        .expect_err("exhausted mixed retries fail");
    assert!(
        matches!(
            error,
            CrspError::Api {
                kind: ApiErrorKind::UnexpectedApiError,
                ..
            }
        ),
        "got {error:?}"
    );
    // 1 status retry (500) + 2 network retries, each delaying per the formula.
    assert_eq!(recorder.delays(), vec![100, 500, 1500]);
}

#[tokio::test]
async fn retry_after_header_is_ignored() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versions": []})))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .list_versions("s1")
        .await
        .expect("succeeds after retry");
    // Retry-After: 120 must be ignored in favor of the 100ms first-retry delay.
    assert_eq!(h.recorder.delays(), vec![100]);
    assert_eq!(received_requests(&h.server).await.len(), 2);
}

/// gaxios `getNextRetryDelay` clamps: `min(calculated, maxRetryDelay)`. With
/// `maxRetryDelay` = 200ms the series becomes [100, 200] (calculated 500ms is
/// clamped; the defaults leave it unlimited).
#[tokio::test]
async fn max_retry_delay_clamps_the_backoff_series() {
    let base = closed_port_base().await;
    let (client, recorder) = recorder_client_with(
        &base,
        || ok_refresh(),
        |config| {
            config.max_retry_delay = Some(Duration::from_millis(200));
        },
    );

    let error = client
        .request(ApiRequest {
            method: reqwest::Method::GET,
            url: format!("{base}/v1/projects/s1/versions"),
            body: None,
        })
        .await
        .expect_err("closed port fails");
    assert!(
        matches!(
            error,
            CrspError::Api {
                kind: ApiErrorKind::UnexpectedApiError,
                ..
            }
        ),
        "got {error:?}"
    );
    assert_eq!(recorder.delays(), vec![100, 200]);
}

/// gaxios `getNextRetryDelay` clamp: `min(calculated, totalTimeout − elapsed)`.
/// A scripted clock drives the elapsed time; once the total budget is
/// exhausted the remaining budget goes negative and the retry fires
/// immediately (gaxios passes a negative timeout to `setTimeout`).
#[tokio::test]
async fn total_timeout_clamps_the_backoff_series() {
    let base = closed_port_base().await;
    let clock_readings: Arc<Mutex<Vec<u128>>> = Arc::new(Mutex::new(vec![0, 2600, 5200]));
    let (client, recorder) = recorder_client_with(
        &base,
        || ok_refresh(),
        |config| {
            config.total_timeout = Some(Duration::from_millis(3000));
            let readings = Arc::clone(&clock_readings);
            config.clock = Some(Arc::new(move || readings.lock().unwrap().remove(0)));
        },
    );

    let error = client
        .request(ApiRequest {
            method: reqwest::Method::GET,
            url: format!("{base}/v1/projects/s1/versions"),
            body: None,
        })
        .await
        .expect_err("closed port fails");
    assert!(
        matches!(
            error,
            CrspError::Api {
                kind: ApiErrorKind::UnexpectedApiError,
                ..
            }
        ),
        "got {error:?}"
    );
    // Retry 1: remaining = 3000 − 2600 = 400 → min(100, 400) = 100.
    // Retry 2: remaining = 3000 − 5200 < 0 → immediate (clamped to 0).
    assert_eq!(recorder.delays(), vec![100, 0]);
}

// ---------------------------------------------------------------------------
// 401 refresh layer (spec §7.4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_401_refreshes_once_and_retries_with_new_token() {
    let h = harness().await;
    // Mount order = request order: the 401 (consumed once) is checked first,
    // then the post-refresh success mock.
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .and(header("authorization", format!("Bearer {REFRESHED_TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"files": []})))
        .mount(&h.server)
        .await;

    let content = h
        .client
        .script()
        .get_content("s1", None)
        .await
        .expect("retry after refresh succeeds");
    assert!(content.files().is_empty());

    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 2);
    assert_bearer(&requests[0], ACCESS_TOKEN);
    assert_bearer(&requests[1], REFRESHED_TOKEN);
    assert_eq!(h.recorder.refreshes(), 1);
    assert!(
        h.recorder.delays().is_empty(),
        "the refresh retry does not back off"
    );
}

#[tokio::test]
async fn post_401_refreshes_once_and_retries_once() {
    let h = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/projects/s1/versions"))
        .and(header("authorization", format!("Bearer {REFRESHED_TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versionNumber": 9})))
        .mount(&h.server)
        .await;

    let version = h
        .client
        .script()
        .create_version("s1", "d")
        .await
        .expect("POST retry after refresh succeeds");
    assert_eq!(version.version_number, Some(9));
    assert_eq!(received_requests(&h.server).await.len(), 2);
    assert_eq!(h.recorder.refreshes(), 1);
}

#[tokio::test]
async fn second_401_after_refresh_is_error_without_second_refresh() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"code": 401, "message": "Invalid Credentials"}
        })))
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .get_content("s1", None)
        .await
        .expect_err("401 after refresh fails");
    assert_api_error(
        &error,
        ApiErrorKind::NotAuthenticated,
        "Invalid Credentials",
    );
    assert_eq!(received_requests(&h.server).await.len(), 2);
    assert_eq!(h.recorder.refreshes(), 1, "no second refresh");
}

#[tokio::test]
async fn refresh_failure_returns_auth_error_without_retry() {
    let h = harness_failing_refresh().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&h.server)
        .await;

    let error = h
        .client
        .script()
        .get_content("s1", None)
        .await
        .expect_err("refresh failure surfaces as Auth error");
    assert!(
        matches!(error, CrspError::Auth(_)),
        "expected Auth error, got {error:?}"
    );
    assert_eq!(
        received_requests(&h.server).await.len(),
        1,
        "no retry after failed refresh"
    );
    assert_eq!(h.recorder.refreshes(), 1);
    assert!(h.recorder.delays().is_empty());
}

#[tokio::test]
async fn get_401_refresh_then_500_continues_transient_retries() {
    let h = harness().await;
    // Request order: 401, then 500 (one status retry), then success.
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versions": []})))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .list_versions("s1")
        .await
        .expect("recovers after refresh and one transient retry");
    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 3);
    assert_eq!(h.recorder.refreshes(), 1);
    assert_eq!(
        h.recorder.delays(),
        vec![100],
        "only the 500 retry backs off"
    );
}

// ---------------------------------------------------------------------------
// Base URL overrides (spec §2.2, §9.1)
// ---------------------------------------------------------------------------

#[test]
fn from_lookup_defaults_to_google_urls() {
    let base = BaseUrls::from_lookup(|_| None);
    assert_eq!(base.script, SCRIPT_API_BASE_URL);
    assert_eq!(base.drive, DRIVE_API_BASE_URL);
    assert_eq!(base.service_usage, SERVICE_USAGE_API_BASE_URL);
    assert_eq!(base.discovery, DISCOVERY_API_BASE_URL);
    assert_eq!(base.logging, LOGGING_API_BASE_URL);
    assert_eq!(base.oauth2, OAUTH2_API_BASE_URL);
    assert_eq!(base.userinfo, USERINFO_API_BASE_URL);
}

#[test]
fn from_lookup_applies_shared_override_to_all_services() {
    let base = BaseUrls::from_lookup(|name| {
        (name == ENV_API_BASE_URL).then(|| "https://shared.example".to_string())
    });
    assert_eq!(base.script, "https://shared.example");
    assert_eq!(base.drive, "https://shared.example");
    assert_eq!(base.service_usage, "https://shared.example");
    assert_eq!(base.discovery, "https://shared.example");
    assert_eq!(base.logging, "https://shared.example");
    assert_eq!(base.oauth2, "https://shared.example");
    assert_eq!(base.userinfo, "https://shared.example");
}

#[test]
fn from_lookup_prefers_service_specific_over_shared() {
    let base = BaseUrls::from_lookup(|name| match name {
        ENV_API_BASE_URL => Some("https://shared.example".to_string()),
        ENV_SCRIPT_BASE_URL => Some("https://script.example".to_string()),
        ENV_DRIVE_BASE_URL => Some("https://drive.example".to_string()),
        ENV_SERVICE_USAGE_BASE_URL => Some("https://serviceusage.example".to_string()),
        ENV_DISCOVERY_BASE_URL => Some("https://discovery.example".to_string()),
        ENV_LOGGING_BASE_URL => Some("https://logging.example".to_string()),
        ENV_OAUTH2_BASE_URL => Some("https://oauth2.example".to_string()),
        ENV_USERINFO_BASE_URL => Some("https://userinfo.example".to_string()),
        _ => None,
    });
    assert_eq!(base.script, "https://script.example");
    assert_eq!(base.drive, "https://drive.example");
    assert_eq!(base.service_usage, "https://serviceusage.example");
    assert_eq!(base.discovery, "https://discovery.example");
    assert_eq!(base.logging, "https://logging.example");
    assert_eq!(base.oauth2, "https://oauth2.example");
    assert_eq!(base.userinfo, "https://userinfo.example");
}

#[tokio::test]
async fn trailing_slash_base_url_is_normalized() {
    let server = MockServer::start().await;
    let (client, _recorder) = recorder_client(&format!("{}/", server.uri()), || ok_refresh());
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"scriptId": "s1"})))
        .expect(1)
        .mount(&server)
        .await;

    client
        .script()
        .create_project("T", None)
        .await
        .expect("trailing slash must not double-slash the path");
}

// ---------------------------------------------------------------------------
// Masking (task contract: no token values in wiremock transcript assertions)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_flow_requests_carry_only_masked_bearer_placeholders() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&h.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/content"))
        .and(header("authorization", format!("Bearer {REFRESHED_TOKEN}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"files": []})))
        .mount(&h.server)
        .await;

    h.client
        .script()
        .get_content("s1", None)
        .await
        .expect("succeeds");
    let requests = received_requests(&h.server).await;
    assert_eq!(requests.len(), 2);
    assert_bearer(&requests[0], ACCESS_TOKEN);
    assert_bearer(&requests[1], REFRESHED_TOKEN);
}

// ---------------------------------------------------------------------------
// Degrade-parse parity (clasp never fails on non-JSON bodies; typed accessors
// treat every field as absent)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_with_empty_body_succeeds() {
    let h = harness().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/projects/s1/deployments/dep-1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&h.server)
        .await;

    h.client
        .script()
        .delete_deployment("s1", "dep-1")
        .await
        .expect("empty response body must not fail the call");
}

#[tokio::test]
async fn non_json_success_body_degrades_to_absent_fields() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/s1/versions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("not json"),
        )
        .expect(1)
        .mount(&h.server)
        .await;

    let PagedResults {
        results,
        partial_results,
    } = h
        .client
        .script()
        .list_versions("s1")
        .await
        .expect("non-JSON bodies degrade like clasp instead of failing");
    assert!(results.is_empty());
    assert!(!partial_results);
}
