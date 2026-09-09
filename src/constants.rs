//! Project-wide constants shared across crsp (mirrors clasp's `constants.ts`).

/// Name of the binary and of the MCP server (spec §1.1).
pub const PROJECT_NAME: &str = "crsp";

/// Basename of the Apps Script manifest as used by the remote API.
pub const PROJECT_MANIFEST_BASENAME: &str = "appsscript";

/// Local manifest file name.
pub const PROJECT_MANIFEST_FILENAME: &str = "appsscript.json";

/// Local project settings file discovered via find-up (JSON5 read, JSON write).
pub const PROJECT_CONFIG_FILENAME: &str = ".clasp.json";

/// Ignore rules file located at the project root.
pub const PROJECT_IGNORE_FILENAME: &str = ".claspignore";

/// Global credential store file name (read/written with 0600, `O_NOFOLLOW`).
pub const CREDENTIALS_FILENAME: &str = ".clasprc.json";

/// Default OAuth client ID (identical to clasp's so existing tokens keep working).
pub const DEFAULT_OAUTH_CLIENT_ID: &str =
    "1072944905499-vm2v2i5dvn0a0d2o4ca36i1vge8cvbn0.apps.googleusercontent.com";

/// Default OAuth client secret (Google-provided desktop client, public by design).
pub const DEFAULT_OAUTH_CLIENT_SECRET: &str = "v6V3fKV_zWU7iw1DrpO1rknX";
