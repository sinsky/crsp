//! File-based credential store for `.clasprc.json` (spec §2.3, §7.4; clasp
//! `file_credential_store.ts`).
//!
//! Reads the V3 named-token format and, for the `default` user only, the two
//! legacy V1 formats (local `token` + `oauth2ClientSettings`, and global
//! top-level keys including the historical `exprity_date` misspelling).
//! Writes are direct (`O_WRONLY|O_CREAT|O_TRUNC`, never temp→rename) with
//! `O_NOFOLLOW` + a `symlink_metadata` pre-check, and the file is always
//! reset to mode `0600` (POSIX). Windows treats permissions as best-effort
//! (no POSIX chmod/O_NOFOLLOW).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::constants::{DEFAULT_OAUTH_CLIENT_ID, DEFAULT_OAUTH_CLIENT_SECRET};
use crate::error::CrspError;
use crate::i18n;

/// The user key whose entries fall back to the legacy V1 formats
/// (clasp `file_credential_store.ts:144-147`).
pub const DEFAULT_USER: &str = "default";

/// Credentials as stored in `.clasprc.json` (clasp `StoredCredential`):
/// a merge of google-auth-library's JWT input and OAuth credentials.
/// Unknown JSON fields are preserved through load→save round trips, and
/// unset fields are omitted on write (`JSON.stringify` undefined behavior).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredentials {
    #[serde(rename = "client_id", default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(
        rename = "client_secret",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
    #[serde(
        rename = "refresh_token",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub refresh_token: Option<String>,
    #[serde(
        rename = "access_token",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub access_token: Option<String>,
    /// Milliseconds since the Unix epoch (google-auth-library `expiry_date`).
    #[serde(
        rename = "expiry_date",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub expiry_date: Option<i64>,
    #[serde(rename = "id_token", default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(
        rename = "token_type",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub token_type: Option<String>,
    /// Unknown fields from other clasp/credential versions, preserved
    /// verbatim (clasp stores are opaque JSON objects).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, Value>,
}

/// File-based store bound to one `.clasprc.json` path (clasp
/// `FileCredentialStore`).
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
    allow_symlinks: bool,
}

impl CredentialStore {
    /// `allow_symlinks` mirrors clasp's `--allow-symlinks` escape hatch:
    /// when false (the default), writes refuse symlinked credential files.
    pub fn new(path: impl Into<PathBuf>, allow_symlinks: bool) -> Self {
        Self {
            path: path.into(),
            allow_symlinks,
        }
    }

    /// The `.clasprc.json` path this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the credentials for `user` (V3 entry, or a legacy V1 fallback
    /// for the `default` user; clasp `load`, file_credential_store.ts:136-174).
    pub async fn load(&self, user: &str) -> Result<Option<StoredCredentials>, CrspError> {
        let store = self.read_file()?;
        if let Some(entry) = store
            .get("tokens")
            .and_then(Value::as_object)
            .and_then(|tokens| tokens.get(user))
            && is_truthy(entry)
        {
            // clasp returns any truthy entry, including empty objects.
            let credentials: StoredCredentials = deserialize_credentials(entry, &self.path)?;
            return Ok(Some(credentials));
        }

        // Legacy V1 formats are only consulted for the `default` user.
        if user != DEFAULT_USER {
            return Ok(None);
        }
        // V1 local: project-root `.clasprc.json` with `token` +
        // `oauth2ClientSettings` (both required to be truthy).
        let token = store.get("token").filter(|value| !value.is_null());
        let settings = store
            .get("oauth2ClientSettings")
            .filter(|value| !value.is_null());
        if let (Some(token), Some(settings)) = (token, settings)
            && let (Some(token), Some(settings)) = (token.as_object(), settings.as_object())
        {
            let mut credentials = StoredCredentials {
                credential_type: Some("authorized_user".to_string()),
                access_token: optional_string(token.get("access_token")),
                refresh_token: optional_string(token.get("refresh_token")),
                scope: optional_string(token.get("scope")),
                token_type: optional_string(token.get("token_type")),
                expiry_date: optional_i64(token.get("expiry_date")),
                client_id: optional_string(settings.get("clientId")),
                client_secret: optional_string(settings.get("clientSecret")),
                ..StoredCredentials::default()
            };
            // Keep unknown fields of the legacy token entry.
            credentials.extra = token
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "access_token" | "refresh_token" | "scope" | "token_type" | "expiry_date"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            return Ok(Some(credentials));
        }

        // V1 global: old `~/.clasprc.json` with top-level keys; the
        // `exprity_date` misspelling is the real historical field name.
        // clasp checks `!!store.access_token` (non-empty string is truthy).
        let legacy_access_token = store
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty());
        if let Some(access_token) = legacy_access_token {
            return Ok(Some(StoredCredentials {
                credential_type: Some("authorized_user".to_string()),
                access_token: Some(access_token.to_string()),
                refresh_token: optional_string(store.get("refresh_token")),
                expiry_date: optional_i64(store.get("exprity_date")),
                token_type: optional_string(store.get("token_type")),
                client_id: Some(DEFAULT_OAUTH_CLIENT_ID.to_string()),
                client_secret: Some(DEFAULT_OAUTH_CLIENT_SECRET.to_string()),
                ..StoredCredentials::default()
            }));
        }

        Ok(None)
    }

    /// Saves the credentials for `user`, or removes the entry when
    /// `credentials` is `None` (clasp `save`, file_credential_store.ts:85-92).
    /// Legacy top-level keys of the file are preserved.
    pub async fn save(
        &self,
        user: &str,
        credentials: Option<&StoredCredentials>,
    ) -> Result<(), CrspError> {
        let mut store = self.read_file()?;
        let tokens = ensure_tokens(&mut store)?;
        match credentials {
            Some(credentials) => {
                let value = serde_json::to_value(credentials)
                    .map_err(|error| CrspError::Config(error.to_string()))?;
                tokens.insert(user.to_string(), value);
            }
            None => {
                tokens.remove(user);
            }
        }
        self.write_file(&store)
    }

    /// Deletes the credentials for `user` (clasp `delete`,
    /// file_credential_store.ts:100-116). For `default`, the whole document
    /// is reduced to `{tokens: …}` so no legacy V1 keys survive.
    pub async fn delete(&self, user: &str) -> Result<(), CrspError> {
        let mut store = self.read_file()?;
        let tokens = ensure_tokens(&mut store)?;
        tokens.remove(user);
        if user == DEFAULT_USER {
            store = serde_json::json!({ "tokens": tokens });
        }
        self.write_file(&store)
    }

    fn read_file(&self) -> Result<Value, CrspError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if is_missing(&error) => return Ok(Value::Object(Default::default())),
            Err(error) => return Err(error.into()),
        };
        serde_json::from_str(&content).map_err(|error| CrspError::Config(error.to_string()))
    }

    fn write_file(&self, store: &Value) -> Result<(), CrspError> {
        let content = serde_json::to_string_pretty(store)
            .map_err(|error| CrspError::Config(error.to_string()))?;
        write_securely(&self.path, content.as_bytes(), self.allow_symlinks)
    }
}

fn deserialize_credentials(value: &Value, path: &Path) -> Result<StoredCredentials, CrspError> {
    if !value.is_object() {
        return Err(CrspError::Config(format!(
            "Invalid credentials entry in {}: expected an object",
            path.display()
        )));
    }
    serde_json::from_value(value.clone()).map_err(|error| CrspError::Config(error.to_string()))
}

/// JS truthiness for stored token entries (clasp `if (credentials)`).
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::String(string) => !string.is_empty(),
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        _ => true,
    }
}

fn ensure_tokens(store: &mut Value) -> Result<&mut serde_json::Map<String, Value>, CrspError> {
    let needs_object = !store.is_object() || {
        let tokens = store.get("tokens");
        matches!(tokens, None | Some(Value::Null))
    };
    if needs_object {
        // clasp replaces missing/falsy `tokens` with a fresh map; a corrupt
        // non-object `tokens` value is a config error.
        if let Some(tokens) = store.get("tokens")
            && !tokens.is_null()
            && !tokens.is_object()
        {
            return Err(CrspError::Config(
                "Invalid .clasprc.json: `tokens` must be an object".to_string(),
            ));
        }
        if !store.is_object() {
            *store = Value::Object(Default::default());
        }
        store
            .as_object_mut()
            .unwrap()
            .insert("tokens".to_string(), Value::Object(Default::default()));
    }
    Ok(store
        .get_mut("tokens")
        .and_then(Value::as_object_mut)
        .expect("tokens object ensured"))
}

fn write_securely(path: &Path, content: &[u8], allow_symlinks: bool) -> Result<(), CrspError> {
    // SECURITY: pre-open symlink check (clasp lstatSync, ts:190-206). Errors
    // other than the security rejection are tolerated like clasp's catch
    // (the file may have been removed concurrently).
    if !allow_symlinks
        && let Ok(metadata) = std::fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(CrspError::Auth(i18n::credential_file_is_symlink(
            &path.to_string_lossy(),
        )));
    }

    use std::fs::OpenOptions;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        if !allow_symlinks {
            options.custom_flags(libc::O_NOFOLLOW);
        }
    }

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) => {
            #[cfg(unix)]
            if !allow_symlinks && error.raw_os_error() == Some(libc::ELOOP) {
                // O_NOFOLLOW caught a symlink swapped in after the pre-check.
                return Err(CrspError::Auth(i18n::credential_path_symlink_detected(
                    &path.to_string_lossy(),
                )));
            }
            return Err(error.into());
        }
    };
    file.write_all(content)?;
    file.flush()?;

    // Ensure restrictive permissions even for pre-existing files (clasp
    // chmodSync, ts:230). POSIX failures propagate; Windows has no POSIX
    // chmod (best-effort per spec §2.3).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    let _ = path;

    Ok(())
}

fn is_missing(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn optional_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64)
}
