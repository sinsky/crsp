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

// ---------------------------------------------------------------------------
// Project lifecycle and version/deployment commands (spec §2.5 rows 4-15;
// clasp commands/clone-script.ts, create-script.ts, create-version.ts,
// list-versions.ts, create-deployment.ts, update-deployment.ts,
// delete-deployment.ts, list-deployments.ts, list-scripts.ts,
// delete-script.ts, core/project.ts)
// ---------------------------------------------------------------------------

/// `Project file already exists.` (clasp clone-script.ts:39,
/// create-script.ts:63).
pub const PROJECT_FILE_ALREADY_EXISTS: &str = "Project file already exists.";

/// `No script ID.` (clasp clone-script.ts:80).
pub const NO_SCRIPT_ID: &str = "No script ID.";

/// `Invalid script ID.` (clasp clone-script.ts:146).
pub const INVALID_SCRIPT_ID: &str = "Invalid script ID.";

/// `Clone which script?` (clasp clone-script.ts:65).
pub const CLONE_WHICH_SCRIPT: &str = "Clone which script?";

/// `Security Warning: Skipping write of {file} ({reason}).` (clasp
/// clone-script.ts:107).
pub fn security_warning_skipping_write(file: &str, reason: &str) -> String {
    format!("Security Warning: Skipping write of {file} ({reason}).")
}

/// `Unexpected error, script ID missing from response.` (clasp
/// project.ts:101).
pub const UNEXPECTED_SCRIPT_ID_MISSING: &str = "Unexpected error, script ID missing from response.";

/// `Unexpected error, container ID missing from response.` (clasp
/// project.ts:171).
pub const UNEXPECTED_CONTAINER_ID_MISSING: &str =
    "Unexpected error, container ID missing from response.";

/// `Security Error: Remote file name "{name}" attempts to write outside the
/// project directory.` (clasp files.ts fetchRemote jail check).
pub fn remote_file_attempts_outside_write(name: &str) -> String {
    format!(
        "Security Error: Remote file name \"{name}\" attempts to write outside the project directory."
    )
}

/// `Invalid script type "{type}". Valid types are: {validTypes}.` (clasp
/// create-script.ts:87).
pub fn invalid_script_type(script_type: &str, valid_types: &str) -> String {
    format!("Invalid script type \"{script_type}\". Valid types are: {valid_types}.")
}

/// `Tip: to deploy this script as a {kind}, configure "{field}" in
/// appsscript.json and run `clasp create-deployment`.` (clasp
/// create-script.ts:152-154; the `clasp` literal is kept verbatim because the
/// spec §5 differences do not amend this message).
pub fn deployment_tip(deployment_kind: &str, manifest_field: &str) -> String {
    format!(
        "Tip: to deploy this script as a {deployment_kind}, configure \"{manifest_field}\" \
         in appsscript.json and run `clasp create-deployment`."
    )
}

/// `Give a description:` (clasp create-version.ts:37).
pub const GIVE_A_DESCRIPTION: &str = "Give a description:";

/// `Delete which deployment?` (clasp delete-deployment.ts:100).
pub const DELETE_WHICH_DEPLOYMENT: &str = "Delete which deployment?";

/// `Are you sure you want to delete the script?` (clasp delete-script.ts:37).
pub const ARE_YOU_SURE_YOU_WANT_TO_DELETE_SCRIPT: &str =
    "Are you sure you want to delete the script?";

/// `Script ID not set, unable to delete the script.` (clasp
/// delete-script.ts:28).
pub const SCRIPT_ID_NOT_SET_UNABLE_TO_DELETE: &str =
    "Script ID not set, unable to delete the script.";

/// `No deployments found.` (clasp delete-deployment.ts:126).
pub const NO_DEPLOYMENTS_FOUND: &str = "No deployments found.";

/// `No deployments.` (clasp list-deployments.ts:50).
pub const NO_DEPLOYMENTS: &str = "No deployments.";

/// `No deployed versions of script.` (clasp list-versions.ts:52).
pub const NO_DEPLOYED_VERSIONS: &str = "No deployed versions of script.";

/// `No script files found.` (clasp list-scripts.ts:53).
pub const NO_SCRIPT_FILES_FOUND: &str = "No script files found.";

/// ICU en-US default number formatting for integer arguments (grouping every
/// three digits), used by the `{count, plural}` `#` substitutions and
/// `{version, number}` placeholders in clasp's messages.
fn icu_number(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// `Found {count, plural, one {# version} other {# versions}}.` and siblings
/// (clasp list-versions.ts:60, list-deployments.ts:57, list-scripts.ts:61).
/// ICU plural: `one` applies to exactly 1 in the en locale.
pub fn found_items(count: usize, singular: &str, plural: &str) -> String {
    let word = if count == 1 { singular } else { plural };
    format!("Found {} {word}.", icu_number(count as i64))
}

/// `Cloned {count, plural, =0 {no files.} one {one file.} other {# files}}.`
/// (clasp clone-script.ts:132, create-script.ts:183).
pub fn cloned_files(count: usize) -> String {
    match count {
        0 => "Cloned no files.".to_string(),
        1 => "Cloned one file.".to_string(),
        other => format!("Cloned {} files.", icu_number(other as i64)),
    }
}

/// `Created version {version, number}` (clasp create-version.ts:64).
pub fn created_version(version_number: i32) -> String {
    format!("Created version {}", icu_number(version_number as i64))
}

/// `{version, number} - {description, select, undefined {No description}
/// other {{description}}}` (clasp list-versions.ts:72).
pub fn version_line(version_number: i32, description: Option<&str>) -> String {
    let description = description.unwrap_or("No description");
    format!("{} - {description}", icu_number(version_number as i64))
}

/// `Deployed {deploymentId} {version, select, undefined {@HEAD}
/// other {@{version}}}` (clasp create-deployment.ts:62).
pub fn deployed(deployment_id: &str, version_number: Option<i32>) -> String {
    let version = version_number.map_or_else(|| "@HEAD".to_string(), |n| format!("@{n}"));
    format!("Deployed {deployment_id} {version}")
}

/// `Redeployed {deploymentId} {version, select, undefined {@HEAD}
/// other {@{version}}}` (clasp update-deployment.ts:69).
pub fn redeployed(deployment_id: &str, version_number: Option<i32>) -> String {
    let version = version_number.map_or_else(|| "@HEAD".to_string(), |n| format!("@{n}"));
    format!("Redeployed {deployment_id} {version}")
}

/// `Deleted deployment {id}` (clasp delete-deployment.ts:51).
pub fn deleted_deployment(deployment_id: &str) -> String {
    format!("Deleted deployment {deployment_id}")
}

/// `Deleted all deployments.` (clasp delete-deployment.ts:84).
pub const DELETED_ALL_DEPLOYMENTS: &str = "Deleted all deployments.";

/// `Deleted script {scriptId}` (clasp delete-script.ts:61).
pub fn deleted_script(script_id: &str) -> String {
    format!("Deleted script {script_id}")
}

/// `Created new document: {parentUrl}{br}Created new script: {scriptUrl}`
/// (clasp create-script.ts:107, container branch).
pub fn created_container_script(parent_url: &str, script_url: &str) -> String {
    format!("Created new document: {parent_url}\nCreated new script: {script_url}")
}

/// `Created new script: {scriptUrl}{parentId, select, undefined {}
/// other {{br}Bound to document: {parentUrl}}}` (clasp
/// create-script.ts:130-133, standalone branch).
pub fn created_standalone_script(script_url: &str, parent_id: Option<&str>) -> String {
    match parent_id {
        Some(parent_id) => format!(
            "Created new script: {script_url}\nBound to document: \
             https://drive.google.com/open?id={parent_id}"
        ),
        None => format!("Created new script: {script_url}"),
    }
}
