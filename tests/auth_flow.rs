//! OAuth flow tests (spec §2.4, §7.4; clasp auth_code_flow.ts,
//! localhost_auth_code_flow.ts, serverless_auth_code_flow.ts, login.ts,
//! logout.ts, show-authorized-user.ts).
//!
//! SECURITY: token/code values are placeholders; nothing real is printed.

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use sha2::Digest as _;

use google_clasp_rs::auth::credential_store::{CredentialStore, StoredCredentials};
use google_clasp_rs::auth::flow;
use google_clasp_rs::auth::localhost_flow::LocalhostListener;
use google_clasp_rs::auth::oauth_client::{
    AuthEndpoints, OAuthClient, OAuthClientType, client_type,
};
use google_clasp_rs::auth::serverless_flow;
use google_clasp_rs::error::CrspError;
use google_clasp_rs::output::Output;
use google_clasp_rs::ui::{
    PromptAdapter, PromptDialog, PromptInput, PromptMultiSelect, PromptSelect, Ui,
};

const USER_EMAIL: &str = "user@example.com";
const ACCESS_LOGIN: &str = "ACCESS-PLACEHOLDER-LOGIN";
const REFRESH_LOGIN: &str = "REFRESH-PLACEHOLDER-LOGIN";
const AUTH_CODE: &str = "AUTH-CODE-PLACEHOLDER";
const CLIENT_ID_USER: &str = "999-other-client.apps.googleusercontent.com";

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct FakeAdapter {
    inputs: Mutex<VecDeque<String>>,
}

impl FakeAdapter {
    fn with_inputs(inputs: &[&str]) -> Self {
        Self {
            inputs: Mutex::new(inputs.iter().map(|input| input.to_string()).collect()),
        }
    }
}

impl PromptAdapter for FakeAdapter {
    fn is_interactive(&self) -> bool {
        true
    }

    fn input(&self, _spec: &PromptInput) -> std::io::Result<String> {
        self.inputs
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| std::io::Error::other("no queued input"))
    }

    fn select(&self, _spec: &PromptSelect) -> std::io::Result<String> {
        Err(std::io::Error::other("unexpected select"))
    }

    fn multi_select(&self, _spec: &PromptMultiSelect) -> std::io::Result<Vec<String>> {
        Err(std::io::Error::other("unexpected multi_select"))
    }

    fn confirm(&self, _spec: &google_clasp_rs::ui::PromptConfirm) -> std::io::Result<bool> {
        Err(std::io::Error::other("unexpected confirm"))
    }

    fn dialog(&self, _spec: &PromptDialog) -> std::io::Result<String> {
        Err(std::io::Error::other("unexpected dialog"))
    }

    fn spinner<T, F>(&self, _spec: google_clasp_rs::ui::PromptSpinner, _f: F) -> std::io::Result<T>
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        Err(std::io::Error::other("unexpected spinner"))
    }
}

fn store_in_temp() -> (tempfile::TempDir, CredentialStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = CredentialStore::new(dir.path().join(".clasprc.json"), false);
    (dir, store)
}

fn default_options() -> flow::AuthOptions {
    flow::AuthOptions {
        no_localhost: false,
        creds_file: None,
        use_project_scopes: false,
        include_clasp_scopes: false,
        extra_scopes: Vec::new(),
        redirect_port: None,
        adc: false,
    }
}

fn assert_error_contains(error: &CrspError, needle: &str) {
    let message = error.to_string();
    assert!(message.contains(needle), "missing {needle:?} in: {message}");
}

// ---------------------------------------------------------------------------
// Scopes (clasp login.ts DEFAULT_SCOPES, mergeScopes, buildScopes)
// ---------------------------------------------------------------------------

#[test]
fn default_scopes_match_clasp_exactly() {
    assert_eq!(
        flow::DEFAULT_SCOPES,
        [
            "https://www.googleapis.com/auth/script.deployments",
            "https://www.googleapis.com/auth/script.projects",
            "https://www.googleapis.com/auth/script.webapp.deploy",
            "https://www.googleapis.com/auth/drive.metadata.readonly",
            "https://www.googleapis.com/auth/drive.file",
            "https://www.googleapis.com/auth/service.management",
            "https://www.googleapis.com/auth/logging.read",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
            "https://www.googleapis.com/auth/cloud-platform",
        ]
    );
    assert_eq!(flow::DEFAULT_SCOPES.len(), 10);
}

#[test]
fn build_scopes_uses_defaults_without_project_scopes() {
    let scopes = flow::build_scopes(&flow::DEFAULT_SCOPES, None, false, false, None);
    assert_eq!(
        scopes,
        flow::DEFAULT_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn build_scopes_uses_manifest_scopes_when_requested() {
    let manifest = vec!["https://www.googleapis.com/auth/drive.file".to_string()];
    let scopes = flow::build_scopes(&flow::DEFAULT_SCOPES, Some(&manifest), true, false, None);
    assert_eq!(scopes, manifest);
}

#[test]
fn build_scopes_falls_back_to_defaults_when_manifest_has_none() {
    let scopes = flow::build_scopes(&flow::DEFAULT_SCOPES, None, true, false, None);
    assert_eq!(
        scopes,
        flow::DEFAULT_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn build_scopes_include_clasp_scopes_merges_defaults_with_manifest() {
    let manifest = vec![
        "https://www.googleapis.com/auth/script.projects".to_string(),
        "https://example.com/custom".to_string(),
    ];
    let scopes = flow::build_scopes(&flow::DEFAULT_SCOPES, Some(&manifest), true, true, None);
    // Defaults first, then project-only additions, deduplicated in order.
    assert_eq!(scopes.len(), 11);
    assert_eq!(scopes[0], flow::DEFAULT_SCOPES[0]);
    assert_eq!(scopes[1], flow::DEFAULT_SCOPES[1]);
    assert_eq!(scopes[10], "https://example.com/custom");
    assert_eq!(
        scopes
            .iter()
            .filter(|s| **s == flow::DEFAULT_SCOPES[1])
            .count(),
        1
    );
}

#[test]
fn build_scopes_merges_extra_scopes_and_dedupes() {
    let extra = vec![
        "https://example.com/custom".to_string(),
        "https://www.googleapis.com/auth/drive.file".to_string(),
    ];
    let scopes = flow::build_scopes(&flow::DEFAULT_SCOPES, None, false, false, Some(&extra));
    assert_eq!(scopes.len(), 11);
    assert_eq!(scopes[10], "https://example.com/custom");
}

#[test]
fn validate_scope_options_rejects_include_clasp_scopes_alone() {
    assert!(flow::validate_scope_options(true, true).is_ok());
    assert!(flow::validate_scope_options(false, false).is_ok());
    let error = flow::validate_scope_options(false, true).unwrap_err();
    assert!(matches!(error, CrspError::Validation(_)));
    assert_error_contains(
        &error,
        "--include-clasp-scopes can only be used with --use-project-scopes.",
    );
}

// ---------------------------------------------------------------------------
// PKCE + state (clasp auth_code_flow.ts)
// ---------------------------------------------------------------------------

#[test]
fn generate_state_has_256_bits_of_entropy() {
    let state = flow::generate_state();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&state)
        .unwrap();
    assert_eq!(decoded.len(), 32);
    assert_eq!(state.len(), 43);
}

#[test]
fn generate_state_is_unique_and_unpadded() {
    let a = flow::generate_state();
    let b = flow::generate_state();
    assert_ne!(a, b);
    assert!(!a.contains(['+', '/', '=']));
}

#[test]
fn pkce_challenge_matches_the_rfc7636_vector() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        flow::pkce_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn generate_code_verifier_is_url_safe_and_43_chars() {
    let verifier = flow::generate_code_verifier();
    assert_eq!(verifier.len(), 43);
    assert!(
        verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    );
    assert_eq!(
        flow::pkce_challenge(&verifier),
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(verifier.as_bytes()))
    );
}

// ---------------------------------------------------------------------------
// OAuth client (clasp oauth_client.ts, auth.ts createOauthClient)
// ---------------------------------------------------------------------------

#[test]
fn default_client_uses_the_clasp_client_and_is_google_provided() {
    let client = OAuthClient::default_client();
    assert_eq!(
        client.client_id,
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID
    );
    assert_eq!(
        client.client_secret,
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET
    );
    assert_eq!(client.redirect_uri, "http://localhost");
    assert_eq!(client.client_type(), OAuthClientType::GoogleProvided);
    assert_eq!(
        client_type(Some(google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID)),
        Some(OAuthClientType::GoogleProvided)
    );
    assert_eq!(client_type(None), None);
}

#[test]
fn custom_clients_are_user_provided() {
    let client = OAuthClient {
        client_id: CLIENT_ID_USER.to_string(),
        client_secret: "s".to_string(),
        redirect_uri: "http://localhost".to_string(),
        endpoints: AuthEndpoints::default(),
    };
    assert_eq!(client.client_type(), OAuthClientType::UserProvided);
    assert_eq!(
        client_type(Some(CLIENT_ID_USER)),
        Some(OAuthClientType::UserProvided)
    );
}

#[test]
fn client_secret_file_parses_installed_and_web_entries() {
    let installed = format!(
        r#"{{"installed": {{"client_id": "{CLIENT_ID_USER}", "client_secret": "s", "redirect_uris": ["http://localhost:1", "https://example.com"]}}}}"#
    );
    let client = OAuthClient::from_client_secret_json(&installed).unwrap();
    assert_eq!(client.client_id, CLIENT_ID_USER);
    // The registered redirect used is the localhost entry.
    assert_eq!(client.redirect_uri, "http://localhost:1");

    let web = format!(
        r#"{{"web": {{"client_id": "{CLIENT_ID_USER}", "client_secret": "s", "redirect_uris": ["http://localhost:2"]}}}}"#
    );
    let client = OAuthClient::from_client_secret_json(&web).unwrap();
    assert_eq!(client.redirect_uri, "http://localhost:2");
}

#[test]
fn client_secret_file_errors_match_clasp() {
    let error = OAuthClient::from_client_secret_json("{}").unwrap_err();
    assert_error_contains(&error, "Invalid credentials");

    let error = OAuthClient::from_client_secret_json(
        r#"{"installed": {"client_id": "i", "client_secret": "s"}}"#,
    )
    .unwrap_err();
    assert_error_contains(&error, "Invalid redirect URL");

    let error = OAuthClient::from_client_secret_json(
        r#"{"installed": {"client_id": "i", "client_secret": "s", "redirect_uris": ["https://example.com"]}}"#,
    )
    .unwrap_err();
    assert_error_contains(&error, "No localhost redirect URL found");
}

// ---------------------------------------------------------------------------
// Token endpoint calls (wiremock)
// ---------------------------------------------------------------------------

async fn token_mock(server: &wiremock::MockServer, status: u16, body: serde_json::Value) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/token"))
        .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn form_pairs(request: &wiremock::Request) -> Vec<(String, String)> {
    url::form_urlencoded::parse(&request.body)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn assert_form_contains(pairs: &[(String, String)], key: &str, value: &str) {
    assert!(
        pairs.iter().any(|(k, v)| k == key && v == value),
        "form missing {key}={value}: {pairs:?}"
    );
}

#[tokio::test]
async fn exchange_code_posts_the_authorization_code_grant() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({
            "access_token": ACCESS_LOGIN,
            "refresh_token": REFRESH_LOGIN,
            "expires_in": 3600,
            "scope": "https://www.googleapis.com/auth/script.projects",
            "token_type": "Bearer"
        }),
    )
    .await;

    let client = OAuthClient {
        endpoints: AuthEndpoints {
            token_url: format!("{}/token", server.uri()),
            ..AuthEndpoints::default()
        },
        ..OAuthClient::default_client()
    };
    let http = reqwest::Client::new();
    let tokens = client
        .exchange_code(
            &http,
            AUTH_CODE,
            "http://localhost:1234",
            "CODE-VERIFIER-PLACEHOLDER",
        )
        .await
        .unwrap();
    assert_eq!(tokens.access_token.as_deref(), Some(ACCESS_LOGIN));
    assert_eq!(tokens.refresh_token.as_deref(), Some(REFRESH_LOGIN));
    assert_eq!(tokens.expires_in, Some(3600));

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|request| request.url.path() == "/token")
        .unwrap();
    let pairs = form_pairs(token_request);
    assert_form_contains(&pairs, "grant_type", "authorization_code");
    assert_form_contains(&pairs, "code", AUTH_CODE);
    assert_form_contains(&pairs, "redirect_uri", "http://localhost:1234");
    assert_form_contains(&pairs, "code_verifier", "CODE-VERIFIER-PLACEHOLDER");
    assert_form_contains(
        &pairs,
        "client_id",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID,
    );
    assert_form_contains(
        &pairs,
        "client_secret",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET,
    );
}

#[tokio::test]
async fn refresh_posts_the_refresh_token_grant() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({"access_token": ACCESS_LOGIN, "expires_in": 3600}),
    )
    .await;

    let client = OAuthClient {
        endpoints: AuthEndpoints {
            token_url: format!("{}/token", server.uri()),
            ..AuthEndpoints::default()
        },
        ..OAuthClient::default_client()
    };
    let http = reqwest::Client::new();
    let tokens = client.refresh(&http, REFRESH_LOGIN).await.unwrap();
    assert_eq!(tokens.access_token.as_deref(), Some(ACCESS_LOGIN));

    let requests = server.received_requests().await.unwrap();
    let refresh_request = requests
        .iter()
        .find(|request| request.url.path() == "/token")
        .unwrap();
    let pairs = form_pairs(refresh_request);
    assert_form_contains(&pairs, "grant_type", "refresh_token");
    assert_form_contains(&pairs, "refresh_token", REFRESH_LOGIN);
    assert_form_contains(
        &pairs,
        "client_id",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID,
    );
    assert_form_contains(
        &pairs,
        "client_secret",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET,
    );
}

#[tokio::test]
async fn token_endpoint_errors_surface_the_google_error() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        400,
        serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        }),
    )
    .await;

    let client = OAuthClient {
        endpoints: AuthEndpoints {
            token_url: format!("{}/token", server.uri()),
            ..AuthEndpoints::default()
        },
        ..OAuthClient::default_client()
    };
    let http = reqwest::Client::new();
    let error = client.refresh(&http, REFRESH_LOGIN).await.unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)), "got: {error:?}");
    assert_error_contains(&error, "invalid_grant");
}

// ---------------------------------------------------------------------------
// Serverless flow (clasp serverless_auth_code_flow.ts)
// ---------------------------------------------------------------------------

#[test]
fn serverless_redirect_uri_defaults_to_port_8888() {
    assert_eq!(serverless_flow::redirect_uri(None), "http://localhost:8888");
    assert_eq!(
        serverless_flow::redirect_uri(Some(9090)),
        "http://localhost:9090"
    );
}

#[test]
fn parse_pasted_response_accepts_valid_urls() {
    let code = serverless_flow::parse_pasted_response(
        "http://localhost:8888/?code=CODE-1&state=STATE-1",
        "STATE-1",
    )
    .unwrap();
    assert_eq!(code, "CODE-1");
}

#[test]
fn parse_pasted_response_rejects_state_mismatches() {
    let error = serverless_flow::parse_pasted_response(
        "http://localhost:8888/?code=CODE-1&state=OTHER",
        "STATE-1",
    )
    .unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)));
    assert_error_contains(&error, "CSRF");
}

#[test]
fn parse_pasted_response_surfaces_oauth_errors() {
    let error = serverless_flow::parse_pasted_response(
        "http://localhost:8888/?error=access_denied",
        "STATE-1",
    )
    .unwrap_err();
    assert_error_contains(&error, "access_denied");
}

#[test]
fn parse_pasted_response_requires_a_code() {
    let error =
        serverless_flow::parse_pasted_response("http://localhost:8888/?state=STATE-1", "STATE-1")
            .unwrap_err();
    assert_error_contains(&error, "Missing code in response URL");

    // A valid state but no code is the only way to reach the missing-code
    // error; non-URL pastes (no state at all) hit the CSRF check first,
    // exactly like clasp's parseAuthResponseUrl + validation order.
    let error = serverless_flow::parse_pasted_response("not-a-url", "STATE-1").unwrap_err();
    assert_error_contains(&error, "CSRF");
}

#[tokio::test]
async fn serverless_flow_prompts_and_returns_the_code() {
    let ui = Ui::new(FakeAdapter::with_inputs(&[
        "http://localhost:8888/?code=CODE-PASTED&state=STATE-1",
    ]));
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let code = serverless_flow::obtain_code(
        "https://accounts.google.com/o/oauth2/v2/auth?x=1",
        "STATE-1",
        &ui,
        &mut output,
    )
    .await
    .unwrap();
    assert_eq!(code, "CODE-PASTED");
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "🔑 Authorize clasp by visiting this url:\n\
         https://accounts.google.com/o/oauth2/v2/auth?x=1\n\n"
    );
    assert!(String::from_utf8(err).unwrap().is_empty());
}

#[tokio::test]
async fn serverless_flow_rejects_bad_pastes() {
    let ui = Ui::new(FakeAdapter::with_inputs(&[
        "http://localhost:8888/?code=CODE-PASTED&state=WRONG",
    ]));
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let error = serverless_flow::obtain_code(
        "https://accounts.google.com/o/oauth2/v2/auth?x=1",
        "STATE-1",
        &ui,
        &mut output,
    )
    .await
    .unwrap_err();
    assert_error_contains(&error, "CSRF");
}

// ---------------------------------------------------------------------------
// Localhost flow (spec §2.4, tokio TcpListener + select!)
// ---------------------------------------------------------------------------

async fn send_request(port: u16, target: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    use tokio::io::AsyncWriteExt;
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response).await;
    String::from_utf8_lossy(&response).into_owned()
}

async fn wait_for_code(
    listener: &LocalhostListener,
    state: &str,
    timeout: Option<Duration>,
) -> Result<String, CrspError> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let result = listener.wait_for_code(state, rx, timeout).await;
    drop(tx);
    result
}

#[tokio::test]
async fn localhost_bind_reports_the_redirect_uri() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    assert_ne!(listener.port(), 0);
    assert_eq!(
        listener.redirect_uri(),
        format!("http://localhost:{}", listener.port())
    );
}

#[tokio::test]
async fn localhost_bind_rejects_an_occupied_port() {
    let occupant = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupant.local_addr().unwrap().port();
    let error = LocalhostListener::bind(port).await.unwrap_err();
    assert_error_contains(&error, "already in use");
    assert_error_contains(&error, "--redirect-port");
}

#[tokio::test]
async fn localhost_accepts_a_valid_callback() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let port = listener.port();
    let waiter = tokio::spawn(async move {
        let listener = listener;
        wait_for_code(&listener, "STATE-1", None).await
    });
    // Let the accept loop start before the callback arrives.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = send_request(port, "/?code=CODE-1&state=STATE-1").await;
    assert!(response.contains("HTTP/1.1 200 OK"), "got: {response}");
    assert!(response.contains("text/html"));
    assert!(response.contains("Logged in! You may close this page."));
    assert_eq!(waiter.await.unwrap().unwrap(), "CODE-1");
}

#[tokio::test]
async fn localhost_ignores_probes_and_favicon_then_accepts() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let port = listener.port();
    let waiter = tokio::spawn(async move {
        let listener = listener;
        wait_for_code(&listener, "STATE-1", None).await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let probe = send_request(port, "/").await;
    assert!(probe.contains("404"), "got: {probe}");
    let favicon = send_request(port, "/favicon.ico").await;
    assert!(
        favicon.contains("404") || favicon.contains("204"),
        "got: {favicon}"
    );
    let other = send_request(port, "/some/other/path").await;
    assert!(other.contains("404"), "got: {other}");
    let response = send_request(port, "/?code=CODE-2&state=STATE-1").await;
    assert!(response.contains("HTTP/1.1 200 OK"), "got: {response}");
    assert_eq!(waiter.await.unwrap().unwrap(), "CODE-2");
}

#[tokio::test]
async fn localhost_rejects_oauth_error_params() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let port = listener.port();
    let waiter = tokio::spawn(async move {
        let listener = listener;
        wait_for_code(&listener, "STATE-1", None).await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = send_request(port, "/?error=access_denied&state=STATE-1").await;
    assert!(response.contains("HTTP/1.1 400"), "got: {response}");
    assert!(response.contains("Authorization failed. Please try again."));
    let error = waiter.await.unwrap().unwrap_err();
    assert_error_contains(&error, "access_denied");
}

#[tokio::test]
async fn localhost_rejects_state_mismatches() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let port = listener.port();
    let waiter = tokio::spawn(async move {
        let listener = listener;
        wait_for_code(&listener, "STATE-1", None).await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = send_request(port, "/?code=CODE-3&state=EVIL").await;
    assert!(response.contains("HTTP/1.1 400"), "got: {response}");
    assert!(response.contains(
        "Authorization rejected: state parameter mismatch. This may indicate a CSRF attack."
    ));
    let error = waiter.await.unwrap().unwrap_err();
    assert_error_contains(&error, "CSRF");
}

#[tokio::test]
async fn localhost_rejects_missing_code() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let port = listener.port();
    let waiter = tokio::spawn(async move {
        let listener = listener;
        wait_for_code(&listener, "STATE-1", None).await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = send_request(port, "/?state=STATE-1").await;
    assert!(response.contains("HTTP/1.1 400"), "got: {response}");
    let error = waiter.await.unwrap().unwrap_err();
    assert_error_contains(&error, "Missing authorization code");
}

#[tokio::test]
async fn localhost_cancellation_aborts_promptly() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let waiter = tokio::spawn(async move { listener.wait_for_code("STATE-1", rx, None).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    tx.send(()).unwrap();
    let error = waiter.await.unwrap().unwrap_err();
    assert!(matches!(error, CrspError::Aborted), "got: {error:?}");
}

#[tokio::test]
async fn localhost_timeout_aborts() {
    let listener = LocalhostListener::bind(0).await.unwrap();
    let error = wait_for_code(&listener, "STATE-1", Some(Duration::from_millis(50)))
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)), "got: {error:?}");
    assert_error_contains(&error, "Timed out");
}

// ---------------------------------------------------------------------------
// Userinfo (clasp auth.ts getUserInfo: errors degrade to no email)
// ---------------------------------------------------------------------------

async fn login_credentials() -> StoredCredentials {
    let client = OAuthClient::default_client();
    StoredCredentials {
        client_id: Some(client.client_id),
        client_secret: Some(client.client_secret),
        credential_type: Some("authorized_user".to_string()),
        refresh_token: Some(REFRESH_LOGIN.to_string()),
        access_token: Some(ACCESS_LOGIN.to_string()),
        ..StoredCredentials::default()
    }
}

#[tokio::test]
async fn fetch_user_email_returns_the_email() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": USER_EMAIL, "id": "account-id"})),
        )
        .mount(&server)
        .await;
    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        ..AuthEndpoints::default()
    };
    let http = reqwest::Client::new();
    let credentials = login_credentials().await;
    let email = flow::fetch_user_email(&credentials, None, "", &http, &endpoints).await;
    assert_eq!(email.as_deref(), Some(USER_EMAIL));
}

#[tokio::test]
async fn fetch_user_email_degrades_to_none_on_api_errors() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(wiremock::ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        ..AuthEndpoints::default()
    };
    let http = reqwest::Client::new();
    let credentials = login_credentials().await;
    let email = flow::fetch_user_email(&credentials, None, "", &http, &endpoints).await;
    assert_eq!(email, None);
    // A 500 must not trigger a token refresh (google-auth-library refreshes
    // only on 401): exactly one userinfo request was made.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[tokio::test]
async fn fetch_user_email_refreshes_once_on_401_and_saves() {
    let (guard, store) = store_in_temp();
    let credentials = login_credentials().await;
    store.save("default", Some(&credentials)).await.unwrap();

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .and(wiremock::matchers::header(
            "authorization",
            format!("Bearer {ACCESS_LOGIN}"),
        ))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer ACCESS-PLACEHOLDER-REFRESHED",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": USER_EMAIL})),
        )
        .mount(&server)
        .await;
    token_mock(
        &server,
        200,
        serde_json::json!({"access_token": "ACCESS-PLACEHOLDER-REFRESHED", "expires_in": 3600}),
    )
    .await;

    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        token_url: format!("{}/token", server.uri()),
        ..AuthEndpoints::default()
    };
    let http = reqwest::Client::new();
    let email =
        flow::fetch_user_email(&credentials, Some(&store), "default", &http, &endpoints).await;
    assert_eq!(email.as_deref(), Some(USER_EMAIL));
    // The refreshed token was persisted (spec §7.4 save-on-success).
    let saved = store.load("default").await.unwrap().unwrap();
    assert_eq!(
        saved.access_token.as_deref(),
        Some("ACCESS-PLACEHOLDER-REFRESHED")
    );
    drop(guard);
}

// ---------------------------------------------------------------------------
// Service functions: login / logout / show_authorized_user
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_completes_the_localhost_flow_end_to_end() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({
            "access_token": ACCESS_LOGIN,
            "refresh_token": REFRESH_LOGIN,
            "expires_in": 3600,
            "token_type": "Bearer"
        }),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": USER_EMAIL})),
        )
        .mount(&server)
        .await;
    let endpoints = AuthEndpoints {
        token_url: format!("{}/token", server.uri()),
        userinfo_url: format!("{}/userinfo", server.uri()),
        ..AuthEndpoints::default()
    };

    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join(".clasprc.json");
    let out = SharedBuf::default();
    let err = SharedBuf::default();
    let options = default_options();
    let http = reqwest::Client::new();
    let out_for_task = out.clone();

    let handle = tokio::spawn(async move {
        let store = CredentialStore::new(&store_path, false);
        let ui = Ui::new(FakeAdapter::default());
        let mut output = Output::new(false, out_for_task, err);
        flow::login(
            &options,
            None,
            &store,
            "default",
            &ui,
            &mut output,
            &http,
            &endpoints,
            false,
        )
        .await
    });

    // Simulate the browser: wait for the printed authorization URL, extract
    // the state and the redirect port, then hit the local listener.
    let url = wait_for_output_line(&out, "/o/oauth2/v2/auth").await;
    let url = url::Url::parse(url.trim()).unwrap();
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let redirect = url
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    let response = send_request(port, &format!("/?code={AUTH_CODE}&state={state}")).await;
    assert!(response.contains("HTTP/1.1 200 OK"), "got: {response}");

    let payload = handle.await.unwrap().unwrap();
    assert_eq!(payload.email.as_deref(), Some(USER_EMAIL));

    // Credentials were saved in the clasp post-login shape.
    let store = CredentialStore::new(dir.path().join(".clasprc.json"), false);
    let saved = store.load("default").await.unwrap().unwrap();
    assert_eq!(saved.access_token.as_deref(), Some(ACCESS_LOGIN));
    assert_eq!(saved.refresh_token.as_deref(), Some(REFRESH_LOGIN));
    assert_eq!(saved.credential_type.as_deref(), Some("authorized_user"));
    assert_eq!(
        saved.client_id.as_deref(),
        Some(google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID)
    );
    assert_eq!(
        saved.client_secret.as_deref(),
        Some(google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET)
    );
    // clasp saves no expiry_date on the initial login.
    assert_eq!(saved.expiry_date, None);

    // The localhost authorize message carries clasp's exact wording.
    let text = out.text();
    assert!(
        text.contains("`🔑 Authorize clasp by visiting this url:"),
        "got: {text}"
    );
    // No scope banner for a default login.
    assert!(!text.contains("Authorizing with the following scopes:"));
}

async fn wait_for_output_line(buf: &SharedBuf, needle: &str) -> String {
    for _ in 0..500 {
        for line in buf.text().lines() {
            if line.contains(needle) {
                return line.to_string();
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("output never contained {needle:?}: {}", buf.text());
}

#[tokio::test]
async fn login_warns_when_already_logged_in() {
    let (guard, store) = store_in_temp();
    store
        .save("default", Some(&login_credentials().await))
        .await
        .unwrap();
    let out = SharedBuf::default();
    let err = SharedBuf::default();
    let ui = Ui::new(FakeAdapter::default());
    let mut output = Output::new(false, out.clone(), err.clone());
    let http = reqwest::Client::new();
    let endpoints = AuthEndpoints::default();
    // Serverless flow: the fake adapter has no input, so the flow aborts after
    // the warning was emitted (clasp prints the warning before authorizing).
    let options = flow::AuthOptions {
        no_localhost: true,
        ..default_options()
    };
    let result = flow::login(
        &options,
        None,
        &store,
        "default",
        &ui,
        &mut output,
        &http,
        &endpoints,
        false,
    )
    .await;
    assert!(result.is_err());
    assert!(
        err.text()
            .contains("Warning: You seem to already be logged in."),
        "got: {}",
        err.text()
    );
    drop(guard);
}

#[tokio::test]
async fn login_suppresses_the_warning_in_json_mode() {
    let (guard, store) = store_in_temp();
    store
        .save("default", Some(&login_credentials().await))
        .await
        .unwrap();
    let out = SharedBuf::default();
    let err = SharedBuf::default();
    let ui = Ui::new(FakeAdapter::default());
    let mut output = Output::new(true, out.clone(), err.clone());
    let http = reqwest::Client::new();
    let endpoints = AuthEndpoints::default();
    let options = flow::AuthOptions {
        no_localhost: true,
        ..default_options()
    };
    let _ = flow::login(
        &options,
        None,
        &store,
        "default",
        &ui,
        &mut output,
        &http,
        &endpoints,
        false,
    )
    .await;
    assert!(
        !err.text()
            .contains("Warning: You seem to already be logged in.")
    );
    drop(guard);
}

#[tokio::test]
async fn login_rejects_include_clasp_scopes_standalone() {
    let (_guard, store) = store_in_temp();
    let options = flow::AuthOptions {
        include_clasp_scopes: true,
        ..default_options()
    };
    let ui = Ui::new(FakeAdapter::default());
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let http = reqwest::Client::new();
    let error = flow::login(
        &options,
        None,
        &store,
        "default",
        &ui,
        &mut output,
        &http,
        &AuthEndpoints::default(),
        false,
    )
    .await
    .unwrap_err();
    assert_error_contains(
        &error,
        "--include-clasp-scopes can only be used with --use-project-scopes.",
    );
}

#[tokio::test]
async fn login_prints_scopes_when_project_or_extra_scopes_are_used() {
    let (_guard, store) = store_in_temp();
    let options = flow::AuthOptions {
        no_localhost: true,
        use_project_scopes: true,
        extra_scopes: vec!["https://example.com/extra".to_string()],
        ..default_options()
    };
    let manifest = vec!["https://www.googleapis.com/auth/drive.file".to_string()];
    let ui = Ui::new(FakeAdapter::default());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut output = Output::new(false, &mut out, &mut err);
    let http = reqwest::Client::new();
    // The flow fails at the prompt (no queued input), after the scope banner.
    let _ = flow::login(
        &options,
        Some(&manifest),
        &store,
        "default",
        &ui,
        &mut output,
        &http,
        &AuthEndpoints::default(),
        false,
    )
    .await;
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\nAuthorizing with the following scopes:\n"));
    assert!(text.contains("https://www.googleapis.com/auth/drive.file"));
    assert!(text.contains("https://example.com/extra"));
}

#[tokio::test]
async fn login_from_a_user_provided_client_secret_file() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({
            "access_token": ACCESS_LOGIN,
            "refresh_token": REFRESH_LOGIN,
            "expires_in": 3600
        }),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": USER_EMAIL})),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let creds_file = dir.path().join("client-secret.json");
    std::fs::write(
        &creds_file,
        format!(
            r#"{{"installed": {{"client_id": "{CLIENT_ID_USER}", "client_secret": "USER-SECRET-PLACEHOLDER", "redirect_uris": ["http://localhost"]}}}}"#
        ),
    )
    .unwrap();

    let out = SharedBuf::default();
    let err = SharedBuf::default();
    let options = flow::AuthOptions {
        creds_file: Some(creds_file),
        ..default_options()
    };
    let endpoints = AuthEndpoints {
        token_url: format!("{}/token", server.uri()),
        userinfo_url: format!("{}/userinfo", server.uri()),
        ..AuthEndpoints::default()
    };
    let http = reqwest::Client::new();
    let store_path = dir.path().join(".clasprc.json");
    let out_for_task = out.clone();

    let handle = tokio::spawn(async move {
        let store = CredentialStore::new(&store_path, false);
        let ui = Ui::new(FakeAdapter::default());
        let mut output = Output::new(false, out_for_task, err);
        flow::login(
            &options,
            None,
            &store,
            "default",
            &ui,
            &mut output,
            &http,
            &endpoints,
            false,
        )
        .await
    });

    let url = wait_for_output_line(&out, "/o/oauth2/v2/auth").await;
    let url = url::Url::parse(url.trim()).unwrap();
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let redirect = url
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    assert!(redirect.starts_with("http://localhost:"));
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    send_request(port, &format!("/?code={AUTH_CODE}&state={state}")).await;

    let payload = handle.await.unwrap().unwrap();
    assert_eq!(payload.email.as_deref(), Some(USER_EMAIL));
    let store = CredentialStore::new(dir.path().join(".clasprc.json"), false);
    let saved = store.load("default").await.unwrap().unwrap();
    assert_eq!(saved.client_id.as_deref(), Some(CLIENT_ID_USER));
    assert_eq!(
        saved.client_secret.as_deref(),
        Some("USER-SECRET-PLACEHOLDER")
    );
}

#[tokio::test]
async fn login_reports_invalid_client_secret_files() {
    let dir = tempfile::tempdir().unwrap();
    let creds_file = dir.path().join("client-secret.json");
    std::fs::write(&creds_file, "{}").unwrap();
    let (_guard, store) = store_in_temp();
    let options = flow::AuthOptions {
        creds_file: Some(creds_file),
        ..default_options()
    };
    let ui = Ui::new(FakeAdapter::default());
    let mut output = Output::new(false, Vec::new(), Vec::new());
    let http = reqwest::Client::new();
    let error = flow::login(
        &options,
        None,
        &store,
        "default",
        &ui,
        &mut output,
        &http,
        &AuthEndpoints::default(),
        false,
    )
    .await
    .unwrap_err();
    assert_error_contains(&error, "Invalid credentials");
}

#[tokio::test]
async fn logout_deletes_stored_credentials() {
    let (guard, store) = store_in_temp();
    store
        .save("default", Some(&login_credentials().await))
        .await
        .unwrap();
    store
        .save("work", Some(&login_credentials().await))
        .await
        .unwrap();
    let result = flow::logout(&store, "default").await.unwrap();
    assert!(result.deleted);
    let raw = std::fs::read_to_string(store.path()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value["tokens"].get("default").is_none());
    assert!(value["tokens"].get("work").is_some());
    drop(guard);
}

#[tokio::test]
async fn logout_succeeds_without_credentials_and_writes_nothing() {
    let (guard, store) = store_in_temp();
    let result = flow::logout(&store, "default").await.unwrap();
    assert!(!result.deleted);
    assert!(!store.path().exists(), "logout must not create the file");
    drop(guard);
}

#[tokio::test]
async fn logout_cleans_legacy_files_for_the_default_user() {
    let (guard, store) = store_in_temp();
    std::fs::write(
        store.path(),
        r#"{"access_token": "ACCESS-PLACEHOLDER-V1", "refresh_token": "REFRESH-PLACEHOLDER-V1"}"#,
    )
    .unwrap();
    let result = flow::logout(&store, "default").await.unwrap();
    assert!(result.deleted);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert!(value.get("access_token").is_none());
    assert!(value.get("tokens").is_some());
    drop(guard);
}

#[tokio::test]
async fn show_authorized_user_reports_not_logged_in_payload() {
    let payload = flow::show_authorized_user(
        None,
        None,
        "default",
        &reqwest::Client::new(),
        &AuthEndpoints::default(),
    )
    .await
    .unwrap();
    assert!(!payload.logged_in);
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(json, r#"{"loggedIn":false}"#);
}

#[tokio::test]
async fn show_authorized_user_reports_client_classification() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": USER_EMAIL})),
        )
        .mount(&server)
        .await;
    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        ..AuthEndpoints::default()
    };
    let http = reqwest::Client::new();

    let google = flow::show_authorized_user(
        Some(login_credentials().await),
        None,
        "default",
        &http,
        &endpoints,
    )
    .await
    .unwrap();
    assert!(google.logged_in);
    assert_eq!(google.email.as_deref(), Some(USER_EMAIL));
    assert_eq!(google.client_type, Some(OAuthClientType::GoogleProvided));
    assert_eq!(
        google.client_id.as_deref(),
        Some(google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID)
    );
    let json = serde_json::to_value(&google).unwrap();
    assert_eq!(json["clientType"], "google-provided");

    let mut user_provided = login_credentials().await;
    user_provided.client_id = Some(CLIENT_ID_USER.to_string());
    let payload =
        flow::show_authorized_user(Some(user_provided), None, "default", &http, &endpoints)
            .await
            .unwrap();
    assert_eq!(payload.client_type, Some(OAuthClientType::UserProvided));
    assert_eq!(
        serde_json::to_value(&payload).unwrap()["clientType"],
        "user-provided"
    );
}

// ---------------------------------------------------------------------------
// ADC modeling (spec §2.4 --adc; clasp auth.ts createApplicationDefaultCredentials)
// ---------------------------------------------------------------------------

#[test]
fn adc_candidates_prefer_the_env_file() {
    let home = Path::new("/home/tester");
    assert_eq!(
        flow::adc_file_candidates(Some("/custom/adc.json"), home),
        vec![Path::new("/custom/adc.json")]
    );
    #[cfg(unix)]
    assert_eq!(
        flow::adc_file_candidates(None, home),
        vec![home.join(".config/gcloud/application_default_credentials.json")]
    );
}

#[test]
fn adc_credentials_parse_authorized_user_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adc.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"type": "authorized_user", "client_id": "{CLIENT_ID_USER}", "client_secret": "USER-SECRET-PLACEHOLDER", "refresh_token": "{REFRESH_LOGIN}"}}"#
        ),
    )
    .unwrap();
    let credentials = flow::adc_credentials_from_file(&path).unwrap().unwrap();
    assert_eq!(credentials.client_id.as_deref(), Some(CLIENT_ID_USER));
    assert_eq!(credentials.refresh_token.as_deref(), Some(REFRESH_LOGIN));

    let service_account = dir.path().join("sa.json");
    std::fs::write(
        &service_account,
        r#"{"type": "service_account", "client_email": "bot@project.iam.gserviceaccount.com"}"#,
    )
    .unwrap();
    let error = flow::adc_credentials_from_file(&service_account).unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)), "got: {error:?}");
}

#[tokio::test]
async fn load_credentials_reads_the_store_without_adc() {
    let (guard, store) = store_in_temp();
    store
        .save("default", Some(&login_credentials().await))
        .await
        .unwrap();
    let loaded = flow::load_credentials(&store, "default", false)
        .await
        .unwrap();
    assert!(loaded.is_some());
    let missing = flow::load_credentials(&store, "other", false)
        .await
        .unwrap();
    assert!(missing.is_none());
    drop(guard);
}

// ---------------------------------------------------------------------------
// Refresh-token-only .clasprc.json (parked audit item 15, fix round 1):
// google-auth-library keeps the entry and refreshes lazily at the FIRST API
// request — never at init — so `logout` and `login` stay network-free.
// ---------------------------------------------------------------------------

static API_BASE_ENV_LOCK: Mutex<()> = Mutex::new(());
const API_ENV_VARS: [&str; 2] = ["CRSP_API_BASE_URL", "CRSP_OAUTH2_BASE_URL"];

/// Holds the `CRSP_API_BASE_URL` / `CRSP_OAUTH2_BASE_URL` lock for the
/// guard's lifetime, unsetting the variables on drop (the API and refresh
/// paths read them per process).
#[allow(dead_code)]
struct ApiBaseEnvGuard(std::sync::MutexGuard<'static, ()>);

impl Drop for ApiBaseEnvGuard {
    fn drop(&mut self) {
        for name in API_ENV_VARS {
            unsafe { std::env::remove_var(name) };
        }
    }
}

fn api_base_env_guard(url: &str) -> ApiBaseEnvGuard {
    let guard = API_BASE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for name in API_ENV_VARS {
        unsafe { std::env::set_var(name, url) };
    }
    ApiBaseEnvGuard(guard)
}

fn refresh_only_store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join(".clasprc.json");
    std::fs::write(
        &store_path,
        r#"{"tokens": {"default": {"type": "authorized_user", "refresh_token": "REFRESH-LOGIN"}}}"#,
    )
    .unwrap();
    (dir, store_path)
}

#[tokio::test]
async fn refresh_token_only_entry_refreshes_at_first_request_like_clasp() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({"access_token": "ACCESS-REFRESHED-AT-FIRST-USE", "expires_in": 3600}),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/projects/script/content"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"files": [{"name": "Code", "type": "SERVER_JS", "source": "// ok\n"}]})),
        )
        .mount(&server)
        .await;
    let (_dir, store_path) = refresh_only_store();
    let _env = api_base_env_guard(&server.uri());
    let context = google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        None,
        Some(&store_path),
        "default",
        false,
        false,
    )
    .await
    .unwrap();
    // NO refresh at init: the entry is kept with its empty access token and
    // the store is untouched (logout/login never reach the network).
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0,
        "init must not refresh"
    );
    let credentials = context.credentials.as_ref().expect("entry kept");
    assert_eq!(credentials.access_token, None);
    assert_eq!(credentials.refresh_token.as_deref(), Some("REFRESH-LOGIN"));
    // First API request: exactly one refresh POST, then the call with the
    // fresh bearer (google-auth-library `getRequestMetadataAsync`).
    context
        .client()
        .expect("client init")
        .script()
        .get_content("script", None)
        .await
        .expect("first-use refresh makes the API call succeed");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "one refresh POST + one API call");
    assert_eq!(requests[0].url.path(), "/token");
    let pairs = form_pairs(&requests[0]);
    assert_form_contains(&pairs, "grant_type", "refresh_token");
    assert_form_contains(&pairs, "refresh_token", "REFRESH-LOGIN");
    assert_form_contains(
        &pairs,
        "client_id",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_ID,
    );
    assert_form_contains(
        &pairs,
        "client_secret",
        google_clasp_rs::constants::DEFAULT_OAUTH_CLIENT_SECRET,
    );
    assert_eq!(requests[1].url.path(), "/v1/projects/script/content");
    assert_eq!(
        requests[1]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer ACCESS-REFRESHED-AT-FIRST-USE")
    );
    // The refreshed entry is persisted with the refresh token kept.
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert_eq!(
        saved["tokens"]["default"]["access_token"],
        "ACCESS-REFRESHED-AT-FIRST-USE"
    );
    assert_eq!(saved["tokens"]["default"]["refresh_token"], "REFRESH-LOGIN");
    assert!(
        saved["tokens"]["default"]["expiry_date"].as_i64().unwrap()
            > std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
    );
}

#[tokio::test]
async fn logout_with_refresh_token_only_entry_succeeds_offline() {
    // Fix round 1 regression guard: logout is a pure store delete with no
    // network (clasp logout.ts:37-44). A broken refresh token must never
    // block the recovery path — the wiremock has NO mocks mounted, so any
    // refresh attempt would fail the command.
    let server = wiremock::MockServer::start().await;
    let (_dir, store_path) = refresh_only_store();
    let _env = api_base_env_guard(&server.uri());
    let context = google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        None,
        Some(&store_path),
        "default",
        false,
        false,
    )
    .await
    .unwrap();
    let result = google_clasp_rs::auth::logout(&context.store, "default")
        .await
        .unwrap();
    assert!(result.deleted, "the refresh-only entry must be deleted");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert!(
        saved["tokens"].get("default").is_none(),
        "entry removed from the store: {saved}"
    );
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0,
        "logout must not touch the network"
    );
}

#[tokio::test]
async fn entry_without_any_token_stays_logged_out_at_init() {
    // The discard path is preserved: an entry with neither token is still
    // treated as logged out (the client then fails at request time with the
    // no-credentials error).
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join(".clasprc.json");
    std::fs::write(
        &store_path,
        r#"{"tokens": {"default": {"type": "authorized_user"}}}"#,
    )
    .unwrap();
    let context = google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        None,
        Some(&store_path),
        "default",
        false,
        false,
    )
    .await
    .unwrap();
    assert!(
        context.credentials.is_none(),
        "a tokenless entry must stay discarded"
    );
}

// ---------------------------------------------------------------------------
// Fix round 2: lazy refresh on expiry (no network at init) and userinfo
// parity for show-authorized-user (clasp `getUserInfo` →
// `getRequestMetadataAsync`/`getAccessToken` semantics).
// ---------------------------------------------------------------------------

fn expired_token_store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join(".clasprc.json");
    let expired = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 60_000;
    let entry = format!(
        r#"{{"tokens": {{"default": {{"type": "authorized_user", "access_token": "OLD", "refresh_token": "REFRESH-LOGIN", "expiry_date": {expired}}}}}}}"#
    );
    std::fs::write(&store_path, entry).unwrap();
    (dir, store_path)
}

#[tokio::test]
async fn expired_token_entry_refreshes_at_first_request_like_clasp() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({"access_token": "ACCESS-REFRESHED-AT-FIRST-USE", "expires_in": 3600}),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1/projects/script/content"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"files": [{"name": "Code", "type": "SERVER_JS", "source": "// ok\\n"}]}),
            ),
        )
        .mount(&server)
        .await;
    let (_dir, store_path) = expired_token_store();
    let _env = api_base_env_guard(&server.uri());
    let context = google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        None,
        Some(&store_path),
        "default",
        false,
        false,
    )
    .await
    .unwrap();
    // NO refresh at init despite the expired token: the entry is kept as-is.
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0,
        "init must not refresh expired tokens"
    );
    let credentials = context.credentials.as_ref().expect("entry kept");
    assert_eq!(credentials.access_token.as_deref(), Some("OLD"));
    // First API request: exactly one refresh POST, then the call with the
    // fresh bearer (google-auth-library `isTokenExpiring` at request time).
    context
        .client()
        .expect("client init")
        .script()
        .get_content("script", None)
        .await
        .expect("the expired-token refresh makes the API call succeed");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "one refresh POST + one API call");
    assert_eq!(requests[0].url.path(), "/token");
    assert_eq!(requests[1].url.path(), "/v1/projects/script/content");
    assert_eq!(
        requests[1]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer ACCESS-REFRESHED-AT-FIRST-USE")
    );
    // The refreshed entry is persisted with the refresh token kept.
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert_eq!(
        saved["tokens"]["default"]["access_token"],
        "ACCESS-REFRESHED-AT-FIRST-USE"
    );
    assert_eq!(saved["tokens"]["default"]["refresh_token"], "REFRESH-LOGIN");
}

#[tokio::test]
async fn logout_with_expired_token_entry_succeeds_offline() {
    // Same lockout guard as round 1, now for expired tokens: logout is a pure
    // store delete with no network, even when the stored token is expired and
    // the refresh would fail (the wiremock has NO mocks mounted).
    let server = wiremock::MockServer::start().await;
    let (_dir, store_path) = expired_token_store();
    let _env = api_base_env_guard(&server.uri());
    let context = google_clasp_rs::core::clasp::Clasp::init_context(
        None,
        None,
        Some(&store_path),
        "default",
        false,
        false,
    )
    .await
    .unwrap();
    let result = google_clasp_rs::auth::logout(&context.store, "default")
        .await
        .unwrap();
    assert!(result.deleted, "the expired-token entry must be deleted");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert!(
        saved["tokens"].get("default").is_none(),
        "entry removed from the store: {saved}"
    );
    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0,
        "logout must not touch the network"
    );
}

#[tokio::test]
async fn show_authorized_user_refreshes_lazily_and_shows_real_email() {
    let server = wiremock::MockServer::start().await;
    token_mock(
        &server,
        200,
        serde_json::json!({"access_token": "ACCESS-REFRESHED-AT-FIRST-USE", "expires_in": 3600}),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer ACCESS-REFRESHED-AT-FIRST-USE",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "1234567890", "email": USER_EMAIL})),
        )
        .mount(&server)
        .await;
    let (guard, store) = store_in_temp();
    let credentials = StoredCredentials {
        credential_type: Some("authorized_user".to_string()),
        refresh_token: Some("REFRESH-LOGIN".to_string()),
        ..StoredCredentials::default()
    };
    store.save("default", Some(&credentials)).await.unwrap();
    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        token_url: format!("{}/token", server.uri()),
        ..AuthEndpoints::default()
    };
    let payload = flow::show_authorized_user(
        Some(credentials),
        Some(&store),
        "default",
        &reqwest::Client::new(),
        &endpoints,
    )
    .await
    .unwrap();
    // clasp refreshes inside `getUserInfo` (via the client's
    // `getRequestMetadataAsync`) and shows the real email.
    assert!(payload.logged_in);
    assert_eq!(payload.email.as_deref(), Some(USER_EMAIL));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "one refresh POST + one userinfo GET");
    assert_eq!(requests[0].url.path(), "/token");
    let pairs = form_pairs(&requests[0]);
    assert_form_contains(&pairs, "grant_type", "refresh_token");
    assert_eq!(requests[1].url.path(), "/userinfo");
    assert_eq!(
        requests[1]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer ACCESS-REFRESHED-AT-FIRST-USE")
    );
    // The refreshed entry is persisted (spec §7.4 save-on-success).
    let saved = store.load("default").await.unwrap().unwrap();
    assert_eq!(
        saved.access_token.as_deref(),
        Some("ACCESS-REFRESHED-AT-FIRST-USE")
    );
    drop(guard);
}

#[tokio::test]
async fn show_authorized_user_refresh_failure_shows_unknown_user_like_clasp() {
    // clasp `getUserInfo` catches refresh failures → undefined → "You are
    // logged in as an unknown user." with exit 0 (the output rendering is
    // pinned by the auth-show-authorized-user-unknown golden); the command
    // must NOT fail.
    let server = wiremock::MockServer::start().await;
    token_mock(&server, 400, serde_json::json!({"error": "invalid_grant"})).await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/userinfo"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let (guard, store) = store_in_temp();
    let credentials = StoredCredentials {
        credential_type: Some("authorized_user".to_string()),
        refresh_token: Some("REFRESH-LOGIN".to_string()),
        ..StoredCredentials::default()
    };
    store.save("default", Some(&credentials)).await.unwrap();
    let endpoints = AuthEndpoints {
        userinfo_url: format!("{}/userinfo", server.uri()),
        token_url: format!("{}/token", server.uri()),
        ..AuthEndpoints::default()
    };
    let payload = flow::show_authorized_user(
        Some(credentials),
        Some(&store),
        "default",
        &reqwest::Client::new(),
        &endpoints,
    )
    .await
    .unwrap();
    assert!(payload.logged_in, "the entry itself is still valid");
    assert_eq!(
        payload.email, None,
        "unknown user (clasp getUserInfo catch)"
    );
    // Only the (failed) refresh POST happened; userinfo was never called.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/token");
    drop(guard);
}
