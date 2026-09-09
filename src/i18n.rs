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
