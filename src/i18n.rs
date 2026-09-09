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
