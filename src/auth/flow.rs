//! Authorization flows and auth services (spec §2.4, §7.4; clasp
//! `auth_code_flow.ts`, `auth.ts`, `commands/login.ts`, `commands/logout.ts`,
//! `commands/show-authorized-user.ts`).
//!
//! `login` orchestrates PKCE + state generation, scope selection, the
//! localhost or serverless redirect, the token exchange, and the credential
//! save. `logout` and `show_authorized_user` expose typed results and JSON
//! payloads for the command layer. No spinner is used inside async code
//! (plain output lines instead; spec §3.1 note).

use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::auth::credential_store::{CredentialStore, StoredCredentials};
use crate::auth::localhost_flow::LocalhostListener;
use crate::auth::oauth_client::{
    AuthEndpoints, DEFAULT_REDIRECT_URI, OAuthClient, OAuthClientType, client_type,
};
use crate::auth::serverless_flow;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

pub use crate::auth::oauth_client::DEFAULT_SCOPES;

/// Login/logout/show-authorized-user options (spec §2.4; clasp
/// `CommandOptions` + global `--adc`).
#[derive(Debug, Clone, Default)]
pub struct AuthOptions {
    /// `--no-localhost`: serverless flow.
    pub no_localhost: bool,
    /// `--creds <file>`: user-provided OAuth client secret file.
    pub creds_file: Option<std::path::PathBuf>,
    /// `--use-project-scopes`: manifest `oauthScopes` (falls back to the
    /// defaults when the manifest sets none).
    pub use_project_scopes: bool,
    /// `--include-clasp-scopes`: requires `--use-project-scopes`.
    pub include_clasp_scopes: bool,
    /// `--extra-scopes` (already comma-split and trimmed by the CLI parser).
    pub extra_scopes: Vec<String>,
    /// `--redirect-port` (0–65535, validated by the CLI parser).
    pub redirect_port: Option<u16>,
    /// `--adc`: Application Default Credentials.
    pub adc: bool,
}

/// `--json` payload for login (spec §6.2: `{"email": "..."}`).
#[derive(Debug, Clone, Serialize)]
pub struct LoginPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Typed logout result: `deleted` distinguishes the clasp branches (the
/// `Deleted credentials.` message is only printed when credentials existed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoutResult {
    pub deleted: bool,
}

/// `--json` payload for logout (spec §6.2: `{"success": true}` always).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct LogoutPayload {
    pub success: bool,
}

/// `--json` payload for show-authorized-user (spec §6.2).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShowUserPayload {
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_type: Option<OAuthClientType>,
}

/// Validates the scope-option preconditions (clasp `commands/login.ts:137-142`).
/// Command wiring also calls this before reading the manifest so the
/// precedence matches clasp.
pub fn validate_scope_options(
    use_project_scopes: bool,
    include_clasp_scopes: bool,
) -> Result<(), CrspError> {
    if include_clasp_scopes && !use_project_scopes {
        return Err(CrspError::Validation(
            i18n::INCLUDE_CLASP_SCOPES_REQUIRES_PROJECT.to_string(),
        ));
    }
    Ok(())
}

/// Port of clasp's `mergeScopes` (login.ts:42-48): concatenate, then
/// deduplicate preserving first-seen order.
fn merge_scopes(base: Vec<String>, extra: &[String]) -> Vec<String> {
    let mut scopes = base;
    for scope in extra {
        if !scopes.contains(scope) {
            scopes.push(scope.clone());
        }
    }
    scopes
}

/// Port of clasp's `buildScopes` (login.ts:58-82): project scopes replace the
/// defaults (falling back when the manifest has none), `--include-clasp-scopes`
/// merges the defaults back in, and extra scopes are appended and deduped.
pub fn build_scopes(
    default_scopes: &[&str],
    manifest_scopes: Option<&[String]>,
    use_project_scopes: bool,
    include_clasp_scopes: bool,
    extra_scopes: Option<&[String]>,
) -> Vec<String> {
    let mut scopes = default_scopes
        .iter()
        .map(|scope| scope.to_string())
        .collect();
    if use_project_scopes {
        scopes = match manifest_scopes {
            Some(manifest) => manifest.to_vec(),
            None => scopes,
        };
        if include_clasp_scopes {
            scopes = merge_scopes(
                default_scopes.iter().map(|s| s.to_string()).collect(),
                &scopes,
            );
        }
    }
    if let Some(extra_scopes) = extra_scopes {
        scopes = merge_scopes(scopes, extra_scopes);
    }
    scopes
}

/// 256-bit random state as unpadded base64url (clasp `generateState`,
/// auth_code_flow.ts:28-30).
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE code verifier: 32 random bytes as unpadded base64url (43 characters,
/// google-auth-library's default).
pub fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 challenge: base64url(SHA-256(verifier)) without padding.
pub fn pkce_challenge(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Builds the authorization URL (clasp `generateAuthUrl`: response_type=code,
/// access_type=offline, PKCE S256).
pub fn build_authorization_url(
    auth_base: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    code_challenge: &str,
) -> String {
    let mut url = url::Url::parse(auth_base).expect("authorization base URL must parse");
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("redirect_uri", redirect_uri);
        pairs.append_pair("scope", &scopes.join(" "));
        pairs.append_pair("state", state);
        pairs.append_pair("code_challenge", code_challenge);
        pairs.append_pair("code_challenge_method", "S256");
        pairs.append_pair("access_type", "offline");
    }
    url.to_string()
}

/// Runs the authorization flow and stores the resulting credentials (clasp
/// `authorize` + `login.ts`). Progress messages (warning, scopes, URL banner)
/// are printed here; the result message and `--json` payload belong to the
/// command layer.
#[allow(clippy::too_many_arguments)]
pub async fn login<W: Write, E: Write, A: PromptAdapter>(
    options: &AuthOptions,
    manifest_scopes: Option<&[String]>,
    store: &CredentialStore,
    user: &str,
    ui: &Ui<A>,
    output: &mut Output<W, E>,
    http: &reqwest::Client,
    endpoints: &AuthEndpoints,
    open_browser: bool,
) -> Result<LoginPayload, CrspError> {
    // clasp's initAuth resolves ADC credentials up front; a resolution
    // failure fails the command before anything else.
    if options.adc {
        resolve_adc_credentials().await?;
    }

    // clasp login.ts:122-129 — warn when already authenticated (loaded
    // credentials, or an ADC session).
    if !output.is_json() {
        let already_logged_in = options.adc || store.load(user).await?.is_some();
        if already_logged_in {
            output.warn(i18n::ALREADY_LOGGED_IN_WARNING);
        }
    }

    // clasp login.ts:135 — the client (and its creds-file errors) come
    // before the scope validation.
    let client = match &options.creds_file {
        Some(path) => {
            let content = std::fs::read_to_string(path)?;
            OAuthClient::from_client_secret_json(&content)?
        }
        None => OAuthClient::default_client(),
    };
    let client = OAuthClient {
        endpoints: endpoints.clone(),
        ..client
    };

    validate_scope_options(options.use_project_scopes, options.include_clasp_scopes)?;

    let scopes = build_scopes(
        &DEFAULT_SCOPES,
        manifest_scopes,
        options.use_project_scopes,
        options.include_clasp_scopes,
        if options.extra_scopes.is_empty() {
            None
        } else {
            Some(&options.extra_scopes)
        },
    );
    if (options.use_project_scopes || !options.extra_scopes.is_empty()) && !output.is_json() {
        output.message("");
        output.message(i18n::AUTHORIZING_WITH_SCOPES);
        for scope in &scopes {
            output.message(scope);
        }
    }

    let expected_state = generate_state();
    let code_verifier = generate_code_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    let (redirect_uri, code) = if options.no_localhost {
        let redirect_uri = serverless_flow::redirect_uri(options.redirect_port);
        let authorization_url = build_authorization_url(
            &endpoints.auth_url,
            &client.client_id,
            &redirect_uri,
            &scopes,
            &expected_state,
            &code_challenge,
        );
        let code =
            serverless_flow::obtain_code(&authorization_url, &expected_state, ui, output).await?;
        (redirect_uri, code)
    } else {
        let listener = LocalhostListener::bind(options.redirect_port.unwrap_or(0)).await?;
        let redirect_uri = listener.redirect_uri();
        let authorization_url = build_authorization_url(
            &endpoints.auth_url,
            &client.client_id,
            &redirect_uri,
            &scopes,
            &expected_state,
            &code_challenge,
        );
        // Browser launch is fire-and-forget like clasp's `void open(url)`.
        if open_browser {
            let _ = open::that(&authorization_url);
        }
        output.message(&i18n::authorize_url_localhost(&authorization_url));

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let signal_task = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = cancel_tx.send(());
            }
        });
        let result = listener
            .wait_for_code(&expected_state, cancel_rx, None)
            .await;
        signal_task.abort();
        (redirect_uri, result?)
    };

    let tokens = client
        .exchange_code(http, &code, &redirect_uri, &code_verifier)
        .await?;

    // clasp saveOauthClientCredentials (auth.ts:210-230): exactly the five
    // fields, no expiry_date on the initial save.
    let credentials = StoredCredentials {
        client_id: Some(client.client_id),
        client_secret: Some(client.client_secret),
        credential_type: Some("authorized_user".to_string()),
        refresh_token: tokens.refresh_token,
        access_token: tokens.access_token,
        ..StoredCredentials::default()
    };
    store.save(user, Some(&credentials)).await?;

    let email = fetch_user_email(&credentials, Some(store), user, http, endpoints).await;
    Ok(LoginPayload { email })
}

/// Logs out the current user (clasp `commands/logout.ts`): a no-op that
/// still succeeds when not logged in; deletes the token entry otherwise
/// (legacy cleanup is handled by the store for `default`).
pub async fn logout(store: &CredentialStore, user: &str) -> Result<LogoutResult, CrspError> {
    if store.load(user).await?.is_none() {
        return Ok(LogoutResult { deleted: false });
    }
    store.delete(user).await?;
    Ok(LogoutResult { deleted: true })
}

/// The constant logout `--json` payload (spec §6.2).
pub const fn logout_payload() -> LogoutPayload {
    LogoutPayload { success: true }
}

/// Reports the current authorization state (clasp
/// `commands/show-authorized-user.ts`): fetches the email via userinfo
/// (degrading to none on errors, clasp `getUserInfo`) and classifies the
/// client.
pub async fn show_authorized_user(
    credentials: Option<StoredCredentials>,
    store: Option<&CredentialStore>,
    user: &str,
    http: &reqwest::Client,
    endpoints: &AuthEndpoints,
) -> Result<ShowUserPayload, CrspError> {
    let Some(credentials) = credentials else {
        return Ok(ShowUserPayload {
            logged_in: false,
            ..ShowUserPayload::default()
        });
    };
    let email = fetch_user_email(&credentials, store, user, http, endpoints).await;
    Ok(ShowUserPayload {
        logged_in: true,
        email,
        client_id: credentials.client_id.clone(),
        client_type: client_type(credentials.client_id.as_deref()),
    })
}

/// initAuth equivalent: ADC credentials when `--adc` is set, stored
/// credentials otherwise (clasp `auth.ts:61-82`).
pub async fn load_credentials(
    store: &CredentialStore,
    user: &str,
    adc: bool,
) -> Result<Option<StoredCredentials>, CrspError> {
    if adc {
        return resolve_adc_credentials().await.map(Some);
    }
    store.load(user).await
}

/// Resolves Application Default Credentials: the
/// `GOOGLE_APPLICATION_CREDENTIALS` file or the gcloud well-known file
/// (authorized_user JSON only; service accounts need RSA JWT signing, which
/// crsp does not support — documented divergence).
pub async fn resolve_adc_credentials() -> Result<StoredCredentials, CrspError> {
    let env_value = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").ok();
    let home = home::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let candidates = adc_file_candidates(env_value.as_deref(), &home);
    for candidate in candidates {
        match adc_credentials_from_file(&candidate) {
            Ok(Some(credentials)) => return Ok(credentials),
            Ok(None) => continue,
            Err(CrspError::Auth(message)) => return Err(CrspError::Auth(message)),
            Err(_) => continue,
        }
    }
    Err(CrspError::Auth(i18n::ADC_NOT_DETERMINED.to_string()))
}

/// Candidate ADC files, env override first, then the gcloud well-known
/// location (`~/.config/gcloud` on unix, `%APPDATA%\gcloud` on Windows).
pub fn adc_file_candidates(env_value: Option<&str>, home: &Path) -> Vec<std::path::PathBuf> {
    if let Some(env_value) = env_value {
        return vec![std::path::PathBuf::from(env_value)];
    }
    #[cfg(unix)]
    {
        vec![home.join(".config/gcloud/application_default_credentials.json")]
    }
    #[cfg(windows)]
    {
        let appdata = std::env::var("APPDATA").ok();
        match appdata {
            Some(appdata) => vec![
                std::path::PathBuf::from(appdata)
                    .join("gcloud")
                    .join("application_default_credentials.json"),
            ],
            None => vec![
                home.join("AppData")
                    .join("Roaming")
                    .join("gcloud")
                    .join("application_default_credentials.json"),
            ],
        }
    }
}

/// Parses one ADC file: `Ok(None)` when unreadable, an `Auth` error for
/// unsupported service-account files.
pub fn adc_credentials_from_file(path: &Path) -> Result<Option<StoredCredentials>, CrspError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| CrspError::Auth(format!("Invalid credentials: {error}")))?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some("service_account") {
        return Err(CrspError::Auth(
            i18n::ADC_SERVICE_ACCOUNT_UNSUPPORTED.to_string(),
        ));
    }
    let adc: crate::auth::oauth_client::AdcAuthorizedUser = serde_json::from_value(value)
        .map_err(|error| CrspError::Auth(format!("Invalid credentials: {error}")))?;
    if adc.credential_type.as_deref() != Some("authorized_user") {
        return Ok(None);
    }
    Ok(Some(StoredCredentials {
        client_id: adc.client_id,
        client_secret: adc.client_secret,
        credential_type: Some("authorized_user".to_string()),
        refresh_token: adc.refresh_token,
        ..StoredCredentials::default()
    }))
}

/// Refreshes an access token and persists the result (spec §7.4; clasp
/// `auth.ts:149-158` tokens-event handler): only `access_token`, `id_token`,
/// and `expiry_date` change; the stored refresh token is kept (last-write-
/// wins, no locks). On failure the old token stays untouched and the error
/// prompts `crsp login`.
pub async fn refresh_and_save(
    client: &OAuthClient,
    credentials: &StoredCredentials,
    store: &CredentialStore,
    user: &str,
    http: &reqwest::Client,
) -> Result<StoredCredentials, CrspError> {
    let refresh_token = credentials
        .refresh_token
        .as_deref()
        .ok_or_else(|| CrspError::Auth(i18n::refresh_failed("no refresh token stored")))?;
    let tokens = match client.refresh(http, refresh_token).await {
        Ok(tokens) => tokens,
        Err(CrspError::Auth(detail)) => return Err(CrspError::Auth(i18n::refresh_failed(&detail))),
        Err(error) => return Err(error),
    };
    let mut updated = credentials.clone();
    updated.access_token = tokens.access_token;
    // clasp spread: `id_token: tokens.id_token` — an absent id_token drops
    // the stored one (JSON.stringify undefined-key behavior).
    updated.id_token = tokens.id_token;
    updated.expiry_date = tokens
        .expires_in
        .map(|seconds| unix_millis() + seconds * 1000);
    store.save(user, Some(&updated)).await?;
    Ok(updated)
}

/// Fetches the account email via userinfo (clasp `getUserInfo`, which calls
/// the google-auth client's `request` → `getRequestMetadataAsync`): every
/// failure degrades to `None` (unknown user, exit 0). The token is refreshed
/// lazily BEFORE the userinfo call when it is missing or expiring (google
/// auth-library `getAccessToken` semantics), and a 401 triggers exactly one
/// token refresh with save-on-success (spec §7.4); a refresh failure still
/// degrades to `None`.
pub async fn fetch_user_email(
    credentials: &StoredCredentials,
    store: Option<&CredentialStore>,
    user: &str,
    http: &reqwest::Client,
    endpoints: &AuthEndpoints,
) -> Option<String> {
    // google-auth-library `getAccessToken`: refresh when the access token is
    // falsy or expiring (eager-refresh threshold), before the request. clasp's
    // `getUserInfo` catches any refresh failure → unknown user (`None`).
    let mut token = credentials.access_token.clone().unwrap_or_default();
    let expiring = credentials.expiry_date.is_some_and(|expiry| {
        expiry <= unix_millis() + crate::api::client::EAGER_REFRESH_THRESHOLD_MS
    });
    if (token.is_empty() || expiring) && credentials.refresh_token.is_some() {
        let store = store?;
        let client = lazy_refresh_client(credentials, endpoints);
        let refreshed = refresh_and_save(&client, credentials, store, user, http)
            .await
            .ok()?;
        token = refreshed.access_token.unwrap_or_default();
    }
    if token.is_empty() {
        return None;
    }
    match userinfo_request(http, &endpoints.userinfo_url, &token).await {
        UserInfoOutcome::Email(email) => return Some(email),
        UserInfoOutcome::Failed => return None,
        UserInfoOutcome::Unauthorized => {}
    }

    // Refresh once on 401 and retry with the new token (google-auth-library
    // refreshes only on 401).
    let store = store?;
    let client = OAuthClient {
        client_id: credentials.client_id.clone()?,
        client_secret: credentials.client_secret.clone()?,
        endpoints: endpoints.clone(),
        redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
    };
    let refreshed = refresh_and_save(&client, credentials, store, user, http)
        .await
        .ok()?;
    match userinfo_request(
        http,
        &endpoints.userinfo_url,
        refreshed.access_token.as_deref()?,
    )
    .await
    {
        UserInfoOutcome::Email(email) => Some(email),
        _ => None,
    }
}

/// The lazy pre-send refresh client (clasp `getAuthorizedOAuth2Client`): the
/// stored client id/secret when the entry has them, the default clasp client
/// otherwise (the established fallback for refresh-token-only entries).
fn lazy_refresh_client(credentials: &StoredCredentials, endpoints: &AuthEndpoints) -> OAuthClient {
    OAuthClient {
        client_id: credentials
            .client_id
            .clone()
            .unwrap_or_else(|| crate::constants::DEFAULT_OAUTH_CLIENT_ID.to_string()),
        client_secret: credentials
            .client_secret
            .clone()
            .unwrap_or_else(|| crate::constants::DEFAULT_OAUTH_CLIENT_SECRET.to_string()),
        endpoints: endpoints.clone(),
        redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
    }
}

enum UserInfoOutcome {
    Email(String),
    Unauthorized,
    Failed,
}

async fn userinfo_request(
    http: &reqwest::Client,
    userinfo_url: &str,
    access_token: &str,
) -> UserInfoOutcome {
    let Ok(response) = http
        .get(userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
    else {
        return UserInfoOutcome::Failed;
    };
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return UserInfoOutcome::Unauthorized;
    }
    if !response.status().is_success() {
        return UserInfoOutcome::Failed;
    }
    match response.json::<serde_json::Value>().await {
        Ok(value) => value
            .get("email")
            .and_then(serde_json::Value::as_str)
            .map(|email| UserInfoOutcome::Email(email.to_string()))
            .unwrap_or(UserInfoOutcome::Failed),
        Err(_) => UserInfoOutcome::Failed,
    }
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
