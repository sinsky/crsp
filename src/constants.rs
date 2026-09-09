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

// Production Google API base URLs (spec §2.2). Request paths are appended to
// these; Drive and userinfo bases carry their API path prefix.

/// Apps Script API base (`/v1/...` resource paths).
pub const SCRIPT_API_BASE_URL: &str = "https://script.googleapis.com";

/// Drive v3 base (`/v3/files...` resource paths).
pub const DRIVE_API_BASE_URL: &str = "https://www.googleapis.com/drive";

/// Service Usage base (`/v1/projects/...` resource paths).
pub const SERVICE_USAGE_API_BASE_URL: &str = "https://serviceusage.googleapis.com";

/// Discovery base (`/discovery/v1/apis`).
pub const DISCOVERY_API_BASE_URL: &str = "https://discovery.googleapis.com";

/// Cloud Logging base (`/v2/entries:list`).
pub const LOGGING_API_BASE_URL: &str = "https://logging.googleapis.com";

/// OAuth token endpoint base (`/token`).
pub const OAUTH2_API_BASE_URL: &str = "https://oauth2.googleapis.com";

/// Userinfo endpoint base (`/v2/userinfo`).
pub const USERINFO_API_BASE_URL: &str = "https://www.googleapis.com/oauth2";

// Runtime/golden base URL overrides (spec §9.1): the shared override applies
// to every service; service-specific values take precedence. Overrides replace
// only the base origin — paths and query semantics stay unchanged.

/// Shared base URL override for every Google service.
pub const ENV_API_BASE_URL: &str = "CRSP_API_BASE_URL";

/// Apps Script base URL override.
pub const ENV_SCRIPT_BASE_URL: &str = "CRSP_SCRIPT_BASE_URL";

/// Drive base URL override.
pub const ENV_DRIVE_BASE_URL: &str = "CRSP_DRIVE_BASE_URL";

/// Service Usage base URL override.
pub const ENV_SERVICE_USAGE_BASE_URL: &str = "CRSP_SERVICE_USAGE_BASE_URL";

/// Discovery base URL override.
pub const ENV_DISCOVERY_BASE_URL: &str = "CRSP_DISCOVERY_BASE_URL";

/// Cloud Logging base URL override.
pub const ENV_LOGGING_BASE_URL: &str = "CRSP_LOGGING_BASE_URL";

/// OAuth token endpoint base URL override.
pub const ENV_OAUTH2_BASE_URL: &str = "CRSP_OAUTH2_BASE_URL";

/// Userinfo endpoint base URL override.
pub const ENV_USERINFO_BASE_URL: &str = "CRSP_USERINFO_BASE_URL";
