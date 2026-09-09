//! Google API clients (spec §2.2, §6.1, §7.1-§7.4): the authenticated
//! [`ApiClient`] with the clasp-compatible retry and 401-refresh contracts,
//! typed per-service modules, and the base URL override chain used by golden
//! subprocesses.

pub mod client;
pub mod discovery;
pub mod drive;
pub mod error;
pub mod logging;
pub mod oauth2;
pub mod script;
pub mod service_usage;

use crate::constants::{
    DISCOVERY_API_BASE_URL, DRIVE_API_BASE_URL, ENV_API_BASE_URL, ENV_DISCOVERY_BASE_URL,
    ENV_DRIVE_BASE_URL, ENV_LOGGING_BASE_URL, ENV_OAUTH2_BASE_URL, ENV_SCRIPT_BASE_URL,
    ENV_SERVICE_USAGE_BASE_URL, ENV_USERINFO_BASE_URL, LOGGING_API_BASE_URL, OAUTH2_API_BASE_URL,
    SCRIPT_API_BASE_URL, SERVICE_USAGE_API_BASE_URL, USERINFO_API_BASE_URL,
};

pub use crate::api::client::{
    ApiBody, ApiClient, ApiClientConfig, ApiRequest, ApiResponse, RefreshFn, SleepFn,
};
pub use crate::api::discovery::{DiscoveryApi, DiscoveryApis};
pub use crate::api::drive::DriveFile;
pub use crate::api::error::ApiErrorKind;
pub use crate::api::logging::LogEntry;
pub use crate::api::oauth2::UserInfo;
pub use crate::api::script::{
    Deployment, DeploymentConfig, DeploymentConfigInput, EntryPoint, PushFile, ScriptContent,
    ScriptFile, ScriptProject, Version, WebApp,
};
pub use crate::api::service_usage::Service;
pub use crate::core::pagination::PagedResults;

/// Base URLs for the Google API services (spec §2.2). Each entry is the
/// service root; endpoint paths are appended verbatim (Drive and userinfo
/// bases carry their API path prefix, e.g. `https://www.googleapis.com/drive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrls {
    /// Apps Script API root.
    pub script: String,
    /// Drive v3 root.
    pub drive: String,
    /// Service Usage root.
    pub service_usage: String,
    /// Discovery root.
    pub discovery: String,
    /// Cloud Logging root.
    pub logging: String,
    /// OAuth token endpoint root.
    pub oauth2: String,
    /// Userinfo endpoint root.
    pub userinfo: String,
}

impl Default for BaseUrls {
    fn default() -> Self {
        Self {
            script: SCRIPT_API_BASE_URL.to_string(),
            drive: DRIVE_API_BASE_URL.to_string(),
            service_usage: SERVICE_USAGE_API_BASE_URL.to_string(),
            discovery: DISCOVERY_API_BASE_URL.to_string(),
            logging: LOGGING_API_BASE_URL.to_string(),
            oauth2: OAUTH2_API_BASE_URL.to_string(),
            userinfo: USERINFO_API_BASE_URL.to_string(),
        }
    }
}

impl BaseUrls {
    /// Reads the runtime/golden overrides (spec §9.1): the shared
    /// `CRSP_API_BASE_URL` applies to every service, service-specific values
    /// take precedence, and the production Google URLs remain the defaults.
    /// Overrides replace only the base origin; paths and query semantics are
    /// unaffected.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
    }

    /// [`BaseUrls::from_env`] with an injectable lookup so tests can exercise
    /// the precedence chain without mutating process environment variables.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let shared = lookup(ENV_API_BASE_URL);
        let specific = |name: &str| lookup(name).or_else(|| shared.clone());
        Self {
            script: specific(ENV_SCRIPT_BASE_URL)
                .unwrap_or_else(|| SCRIPT_API_BASE_URL.to_string()),
            drive: specific(ENV_DRIVE_BASE_URL).unwrap_or_else(|| DRIVE_API_BASE_URL.to_string()),
            service_usage: specific(ENV_SERVICE_USAGE_BASE_URL)
                .unwrap_or_else(|| SERVICE_USAGE_API_BASE_URL.to_string()),
            discovery: specific(ENV_DISCOVERY_BASE_URL)
                .unwrap_or_else(|| DISCOVERY_API_BASE_URL.to_string()),
            logging: specific(ENV_LOGGING_BASE_URL)
                .unwrap_or_else(|| LOGGING_API_BASE_URL.to_string()),
            oauth2: specific(ENV_OAUTH2_BASE_URL)
                .unwrap_or_else(|| OAUTH2_API_BASE_URL.to_string()),
            userinfo: specific(ENV_USERINFO_BASE_URL)
                .unwrap_or_else(|| USERINFO_API_BASE_URL.to_string()),
        }
    }
}
