//! English message constants, kept identical to clasp except for the
//! intentional differences listed in spec §5.

use crate::constants::PROJECT_NAME;

/// clasp's `Unknown command "clasp {command}"` with the crsp name substituted
/// (spec §5 #1).
pub fn unknown_command(command: &str) -> String {
    format!("Unknown command \"{PROJECT_NAME} {command}\"")
}

/// `'{}' is not a valid integer.`
pub fn not_a_valid_integer(value: &str) -> String {
    format!("'{value}' is not a valid integer.")
}

/// `'{}' should be >= {start} and <= {end}.`
pub fn integer_out_of_range(value: &str, start: i64, end: i64) -> String {
    format!("'{value}' should be >= {start} and <= {end}.")
}

/// `--extra-scopes` value rejection message from clasp's `parseExtraScopes`.
pub const EXTRA_SCOPES_INVALID: &str = "must be a comma-separated list of non-empty scopes.";

/// `Project settings not found.` (clasp `assertScriptConfigured`).
pub const PROJECT_SETTINGS_NOT_FOUND: &str = "Project settings not found.";

/// `Invalid --project path: {path}. File or directory does not exist.`
pub fn invalid_project_path(path: &str) -> String {
    format!("Invalid --project path: {path}. File or directory does not exist.")
}

/// Multi-line error thrown when a `.clasp.json` content directory escapes the
/// project root (clasp `clasp.ts:215-220`).
pub fn src_dir_escapes_project_root(raw: &str, resolved: &str, root: &str) -> String {
    format!(
        "Security Error: srcDir \"{raw}\" escapes project root.\n  \
         Resolved: {resolved}\n  Project root: {root}\n\
         This may indicate a malicious .clasp.json file attempting path traversal."
    )
}

/// `Security Error: Content directory "{resolved}" resolves outside the project root "{root}". This may indicate a path traversal attempt.` (clasp `withContentDir`, `clasp.ts:135-138`).
pub fn content_dir_resolves_outside_project_root(resolved: &str, root: &str) -> String {
    format!(
        "Security Error: Content directory \"{resolved}\" resolves outside the project root \"{root}\". \
         This may indicate a path traversal attempt."
    )
}

/// MCP `projectDir` jail message (clasp `server.ts:45-48`).
pub fn project_dir_not_permitted(resolved: &str) -> String {
    format!(
        "Security Error: projectDir must be within the user home directory or current working directory. \
         Resolved path \"{resolved}\" is not permitted."
    )
}

// ---------------------------------------------------------------------------
// Auth (spec §2.3, §2.4, §7.4; clasp auth/* and commands/login|logout|…)
// ---------------------------------------------------------------------------

/// `.clasprc.json` symlink rejection (clasp `file_credential_store.ts:194-198`).
pub fn credential_file_is_symlink(path: &str) -> String {
    format!(
        "Security Error: Credential file is a symlink.\n  \
         Path: \"{path}\"\n\
         Remove the symlink and run login again."
    )
}

/// `O_NOFOLLOW` rejection (clasp `file_credential_store.ts:223`).
pub fn credential_path_symlink_detected(path: &str) -> String {
    format!("Security Error: Symlink detected in credential path.\n  Path: \"{path}\"")
}

/// Local redirect server port conflict (clasp
/// `localhost_auth_code_flow.ts:57-64`).
pub fn port_already_in_use(port: u16) -> String {
    format!(
        "Error: Port {port} is already in use. Please specify a different port with --redirect-port"
    )
}

/// Local redirect server startup failure (clasp
/// `localhost_auth_code_flow.ts:68-76`).
pub fn unable_to_start_server_on_port(port: u16) -> String {
    format!("Error: Unable to start the server on port {port}")
}

/// Browser-facing success body (clasp `localhost_auth_code_flow.ts:140-142`).
pub const LOGGED_IN_YOU_MAY_CLOSE: &str = "Logged in! You may close this page.";

/// Browser-facing OAuth error body (clasp
/// `localhost_auth_code_flow.ts:114-116`).
pub const AUTHORIZATION_FAILED_RETRY: &str = "Authorization failed. Please try again.";

/// CSRF rejection (clasp `localhost_auth_code_flow.ts:125-127` and
/// `serverless_auth_code_flow.ts:89-91`).
pub const STATE_MISMATCH_CSRF: &str =
    "Authorization rejected: state parameter mismatch. This may indicate a CSRF attack.";

/// Serverless paste missing the code (clasp
/// `serverless_auth_code_flow.ts:95-98`).
pub const MISSING_CODE_IN_RESPONSE_URL: &str = "Missing code in response URL";

/// Localhost callback missing the code (clasp
/// `localhost_auth_code_flow.ts:137`).
pub const MISSING_AUTHORIZATION_CODE: &str = "Missing authorization code";

/// Serverless authorization URL banner (clasp
/// `serverless_auth_code_flow.ts:63-70`).
pub fn authorize_url_serverless(url: &str) -> String {
    format!("🔑 Authorize clasp by visiting this url:\n{url}\n")
}

/// Localhost authorization URL banner, including clasp's stray backtick
/// (clasp `localhost_auth_code_flow.ts:149-156`).
pub fn authorize_url_localhost(url: &str) -> String {
    format!("`🔑 Authorize clasp by visiting this url:\n{url}\n")
}

/// Serverless paste prompt (clasp `serverless_auth_code_flow.ts:73-75`).
pub const PASTE_AUTH_URL_PROMPT: &str =
    "After authorizing, copy the URL from your browser and paste it here:";

/// Already-logged-in warning (clasp `commands/login.ts:124-128`).
pub const ALREADY_LOGGED_IN_WARNING: &str = "Warning: You seem to already be logged in.";

/// Scope banner (clasp `commands/login.ts:158-167`).
pub const AUTHORIZING_WITH_SCOPES: &str = "Authorizing with the following scopes:";

/// `--include-clasp-scopes` precondition (clasp `commands/login.ts:137-142`).
pub const INCLUDE_CLASP_SCOPES_REQUIRES_PROJECT: &str =
    "--include-clasp-scopes can only be used with --use-project-scopes.";

/// Successful logout (clasp `commands/logout.ts:51-53`).
pub const DELETED_CREDENTIALS: &str = "Deleted credentials.";

/// Not logged in (clasp `commands/show-authorized-user.ts:51-55`).
pub const NOT_LOGGED_IN: &str = "Not logged in.";

/// Logged-in message (clasp `commands/show-authorized-user.ts:59-68`).
pub fn logged_in_as(email: &str) -> String {
    format!("You are logged in as {email}.")
}

/// Logged-in with unknown email (clasp `commands/show-authorized-user.ts`).
pub const LOGGED_IN_UNKNOWN_USER: &str = "You are logged in as an unknown user.";

/// OAuth client summary (clasp `commands/show-authorized-user.ts:70-78`).
pub fn oauth_client_summary(client_id: &str, client_type: &str) -> String {
    format!("OAuth client ID: {client_id} ({client_type}).")
}

/// Refresh failure (spec §7.4: keep the old token, prompt `crsp login`;
/// clasp surfaces the raw Google error without custom wording).
pub fn refresh_failed(detail: &str) -> String {
    format!(
        "Failed to refresh access token: {detail}. Run `{PROJECT_NAME} login` to authorize again."
    )
}

/// Timeout while waiting for the authorization callback (spec §2.4's required
/// timeout integration; clasp waits indefinitely, crsp times out).
pub const AUTH_TIMED_OUT: &str = "Timed out waiting for authorization.";

/// ADC without a usable credential file (mirrors google-auth-library's
/// message that clasp surfaces unchanged).
pub const ADC_NOT_DETERMINED: &str = "Could not automatically determine credentials. Please use https://cloud.google.com/docs/authentication to set up your credentials.";

/// ADC service-account JSON files require JWT signing, which crsp does not
/// support (no RSA dependency; documented divergence).
pub const ADC_SERVICE_ACCOUNT_UNSUPPORTED: &str = "Service account credentials are not supported. Use an authorized_user Application Default Credentials file or run `crsp login`.";

/// userinfo fetch failure inside an access-token request wrapper.
pub fn access_token_request_failed(detail: &str) -> String {
    format!("Failed to fetch access token: {detail}")
}
