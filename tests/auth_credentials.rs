//! Credential store tests (.clasprc.json V3/V1 compatibility, secure direct
//! writes, token refresh persistence; spec §2.3, §7.4, clasp
//! file_credential_store.ts).
//!
//! SECURITY: every token value used here is a placeholder; tests assert
//! masking behavior and never log credential values.

use std::path::Path;

use crsp::auth::credential_store::{CredentialStore, StoredCredentials};
use crsp::auth::flow;
use crsp::error::CrspError;

const ACCESS_V1: &str = "ACCESS-PLACEHOLDER-V1";
const ACCESS_V3: &str = "ACCESS-PLACEHOLDER-V3";
const REFRESH_V1: &str = "REFRESH-PLACEHOLDER-V1";
const REFRESH_V3: &str = "REFRESH-PLACEHOLDER-V3";
const CLIENT_ID_USER: &str = "123-userprovided-client-id.apps.googleusercontent.com";

fn v3_credentials() -> StoredCredentials {
    StoredCredentials {
        client_id: Some(CLIENT_ID_USER.to_string()),
        client_secret: Some("USER-PROVIDED-SECRET-PLACEHOLDER".to_string()),
        credential_type: Some("authorized_user".to_string()),
        refresh_token: Some(REFRESH_V3.to_string()),
        access_token: Some(ACCESS_V3.to_string()),
        expiry_date: Some(1_700_000_000_000),
        ..StoredCredentials::default()
    }
}

fn v1_local_file() -> String {
    format!(
        r#"{{
  "token": {{
    "access_token": "{ACCESS_V1}",
    "refresh_token": "{REFRESH_V1}",
    "scope": "https://www.googleapis.com/auth/script.projects",
    "token_type": "Bearer",
    "expiry_date": 1600000000000
  }},
  "oauth2ClientSettings": {{
    "clientId": "{CLIENT_ID_USER}",
    "clientSecret": "USER-PROVIDED-SECRET-PLACEHOLDER",
    "redirectUri": "http://localhost"
  }},
  "isLocalCreds": true
}}"#
    )
}

fn v1_global_file() -> String {
    format!(
        r#"{{
  "access_token": "{ACCESS_V1}",
  "refresh_token": "{REFRESH_V1}",
  "scope": "https://www.googleapis.com/auth/script.projects",
  "token_type": "Bearer",
  "exprity_date": 1600000000000
}}"#
    )
}

fn temp_store() -> (tempfile::TempDir, CredentialStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = CredentialStore::new(dir.path().join(".clasprc.json"), false);
    (dir, store)
}

fn read_file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn assert_auth_error_contains(error: CrspError, needle: &str) {
    let message = error.to_string();
    assert!(
        message.contains(needle),
        "error message missing {needle:?}: {message}"
    );
}

#[tokio::test]
async fn save_then_load_round_trips_v3_credentials() {
    let (_guard, store) = temp_store();
    let credentials = v3_credentials();
    store.save("default", Some(&credentials)).await.unwrap();
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(loaded, credentials);
    // The V3 shape wraps entries under `tokens`.
    let raw = read_file(store.path());
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value.get("tokens").and_then(|t| t.get("default")).is_some());
}

#[tokio::test]
async fn save_keeps_other_users_and_unknown_top_level_keys() {
    let (_guard, store) = temp_store();
    write_file(
        store.path(),
        r#"{
  "isLocalCreds": true,
  "token": {"access_token": "keep-me"},
  "tokens": {"work": {"client_id": "id-work", "type": "authorized_user"}}
}"#,
    );
    store.save("other", Some(&v3_credentials())).await.unwrap();
    let raw = read_file(store.path());
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    // V1 top-level keys survive a save for non-default users (clasp parity).
    assert!(value.get("isLocalCreds").is_some());
    assert!(value.get("token").is_some());
    assert!(
        value
            .get("tokens")
            .and_then(|tokens| tokens.get("work"))
            .is_some()
    );
    assert!(
        value
            .get("tokens")
            .and_then(|tokens| tokens.get("other"))
            .is_some()
    );
}

#[tokio::test]
async fn load_returns_none_for_missing_file() {
    let (_guard, store) = temp_store();
    assert!(store.load("default").await.unwrap().is_none());
    assert!(store.load("work").await.unwrap().is_none());
}

#[tokio::test]
async fn load_v3_entry_wins_over_v1_legacy_fallback() {
    let (_guard, store) = temp_store();
    let mut combined = v1_global_file();
    combined.truncate(combined.len() - 1);
    combined.push_str(
        r#",
  "tokens": {"default": {"access_token": "ACCESS-PLACEHOLDER-V3", "type": "authorized_user"}}
}"#,
    );
    write_file(store.path(), &combined);
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(loaded.access_token.as_deref(), Some(ACCESS_V3));
    assert!(loaded.refresh_token.is_none());
}

#[tokio::test]
async fn load_v1_local_fallback_maps_settings_for_default_user() {
    let (_guard, store) = temp_store();
    write_file(store.path(), &v1_local_file());
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(loaded.access_token.as_deref(), Some(ACCESS_V1));
    assert_eq!(loaded.refresh_token.as_deref(), Some(REFRESH_V1));
    assert_eq!(loaded.expiry_date, Some(1_600_000_000_000));
    assert_eq!(loaded.token_type.as_deref(), Some("Bearer"));
    assert_eq!(
        loaded.scope.as_deref(),
        Some("https://www.googleapis.com/auth/script.projects")
    );
    assert_eq!(loaded.client_id.as_deref(), Some(CLIENT_ID_USER));
    assert_eq!(
        loaded.client_secret.as_deref(),
        Some("USER-PROVIDED-SECRET-PLACEHOLDER")
    );
    assert_eq!(loaded.credential_type.as_deref(), Some("authorized_user"));
}

#[tokio::test]
async fn load_v1_local_fallback_is_default_user_only() {
    let (_guard, store) = temp_store();
    write_file(store.path(), &v1_local_file());
    assert!(store.load("work").await.unwrap().is_none());
}

#[tokio::test]
async fn load_v1_global_fallback_injects_default_client_and_reads_typo_field() {
    let (_guard, store) = temp_store();
    write_file(store.path(), &v1_global_file());
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(loaded.access_token.as_deref(), Some(ACCESS_V1));
    assert_eq!(loaded.refresh_token.as_deref(), Some(REFRESH_V1));
    // The historical `exprity_date` misspelling is the real field name.
    assert_eq!(loaded.expiry_date, Some(1_600_000_000_000));
    assert_eq!(loaded.token_type.as_deref(), Some("Bearer"));
    // V1 global files never stored a client; the default clasp client is used.
    assert_eq!(
        loaded.client_id.as_deref(),
        Some(crsp::constants::DEFAULT_OAUTH_CLIENT_ID)
    );
    assert_eq!(
        loaded.client_secret.as_deref(),
        Some(crsp::constants::DEFAULT_OAUTH_CLIENT_SECRET)
    );
    assert_eq!(loaded.credential_type.as_deref(), Some("authorized_user"));
}

#[tokio::test]
async fn load_v1_global_fallback_is_default_user_only() {
    let (_guard, store) = temp_store();
    write_file(store.path(), &v1_global_file());
    assert!(store.load("work").await.unwrap().is_none());
}

#[tokio::test]
async fn load_v1_local_format_wins_over_v1_global_format() {
    let (_guard, store) = temp_store();
    let mut combined = v1_global_file();
    combined.truncate(combined.len() - 1);
    combined.push_str(
        r#",
  "token": {
    "access_token": "ACCESS-PLACEHOLDER-LOCAL",
    "refresh_token": "REFRESH-PLACEHOLDER-LOCAL"
  },
  "oauth2ClientSettings": {"clientId": "id-local", "clientSecret": "s-local"}
}"#,
    );
    write_file(store.path(), &combined);
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(
        loaded.access_token.as_deref(),
        Some("ACCESS-PLACEHOLDER-LOCAL")
    );
    assert_eq!(loaded.client_id.as_deref(), Some("id-local"));
}

#[tokio::test]
async fn delete_default_user_cleans_v1_keys_but_keeps_other_tokens() {
    let (_guard, store) = temp_store();
    let mut combined = v1_global_file();
    combined.truncate(combined.len() - 1);
    combined.push_str(
        r#",
  "tokens": {"default": {"access_token": "a"}, "work": {"client_id": "id-work"}}
}"#,
    );
    write_file(store.path(), &combined);
    store.delete("default").await.unwrap();
    let raw = read_file(store.path());
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    // V1 legacy top-level keys are gone...
    assert!(value.get("access_token").is_none());
    assert!(value.get("exprity_date").is_none());
    assert!(value.get("token_type").is_none());
    // ...the default entry is gone, but named tokens survive.
    let tokens = value.get("tokens").unwrap();
    assert!(tokens.get("default").is_none());
    assert!(tokens.get("work").is_some());
    // `delete` must be a no-op for tokens keys that were absent (clasp sets
    // `undefined`, which serializes as a missing key).
    assert!(tokens.as_object().unwrap().is_empty() || tokens.get("work").is_some());
}

#[tokio::test]
async fn delete_non_default_user_keeps_v1_keys() {
    let (_guard, store) = temp_store();
    let mut combined = v1_global_file();
    combined.truncate(combined.len() - 1);
    combined.push_str(
        r#",
  "tokens": {"work": {"client_id": "id-work"}}
}"#,
    );
    write_file(store.path(), &combined);
    store.delete("work").await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&read_file(store.path())).unwrap();
    // Only the named token entry is removed; V1 keys stay.
    assert!(value.get("access_token").is_some());
    let tokens = value.get("tokens").unwrap();
    assert!(tokens.get("work").is_none());
}

#[tokio::test]
async fn delete_on_missing_file_writes_empty_tokens_object() {
    let (_guard, store) = temp_store();
    store.delete("default").await.unwrap();
    let raw = read_file(store.path());
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        value
            .get("tokens")
            .and_then(|t| t.as_object())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn save_omits_unset_fields_like_json_stringify() {
    let (_guard, store) = temp_store();
    let credentials = StoredCredentials {
        client_id: Some("id".to_string()),
        client_secret: Some("secret".to_string()),
        credential_type: Some("authorized_user".to_string()),
        refresh_token: Some(REFRESH_V3.to_string()),
        access_token: Some(ACCESS_V3.to_string()),
        ..StoredCredentials::default()
    };
    store.save("default", Some(&credentials)).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&read_file(store.path())).unwrap();
    let entry = &value["tokens"]["default"];
    for key in ["expiry_date", "id_token", "scope", "token_type"] {
        assert!(entry.get(key).is_none(), "{key} must be omitted when unset");
    }
}

#[tokio::test]
async fn save_preserves_unknown_fields_inside_a_token_entry() {
    let (_guard, store) = temp_store();
    write_file(
        store.path(),
        r#"{"tokens": {"default": {"access_token": "a", "futureField": {"x": 1}}}}"#,
    );
    store
        .save("default", Some(&v3_credentials()))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&read_file(store.path())).unwrap();
    assert!(value["tokens"]["default"].get("futureField").is_none());
    // A loaded entry keeps unknown fields through a save round trip.
    write_file(
        store.path(),
        r#"{"tokens": {"default": {"access_token": "a", "futureField": 7}}}"#,
    );
    let loaded = store.load("default").await.unwrap().unwrap();
    store.save("default", Some(&loaded)).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&read_file(store.path())).unwrap();
    assert_eq!(value["tokens"]["default"]["futureField"], 7);
}

#[tokio::test]
async fn malformed_json_is_a_config_error() {
    let (_guard, store) = temp_store();
    write_file(store.path(), "not json at all {");
    let error = store.load("default").await.unwrap_err();
    assert!(matches!(error, CrspError::Config(_)), "got: {error:?}");
}

// ---------------------------------------------------------------------------
// Secure direct writes: 0600 + O_NOFOLLOW + symlink rejection (POSIX)
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[tokio::test]
async fn save_creates_the_file_with_mode_0600() {
    let (_guard, store) = temp_store();
    store
        .save("default", Some(&v3_credentials()))
        .await
        .unwrap();
    assert_eq!(file_mode(store.path()), 0o600);
}

#[cfg(unix)]
#[tokio::test]
async fn save_resets_preexisting_files_to_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, store) = temp_store();
    write_file(store.path(), r#"{"tokens": {}}"#);
    std::fs::set_permissions(store.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    store
        .save("default", Some(&v3_credentials()))
        .await
        .unwrap();
    assert_eq!(file_mode(store.path()), 0o600);
}

#[cfg(unix)]
#[tokio::test]
async fn save_rejects_symlinks_and_leaves_the_target_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real-clasprc.json");
    write_file(&target, r#"{"tokens": {}}"#);
    let link = dir.path().join("link.clasprc.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let store = CredentialStore::new(&link, false);
    let error = store
        .save("default", Some(&v3_credentials()))
        .await
        .unwrap_err();
    assert_auth_error_contains(error, "Security Error");
    // The symlink target is untouched.
    assert_eq!(read_file(&target), r#"{"tokens": {}}"#);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn delete_rejects_symlinks_too() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("real-clasprc.json");
    write_file(&target, r#"{"tokens": {}}"#);
    let link = dir.path().join("link.clasprc.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let store = CredentialStore::new(&link, false);
    let error = store.delete("default").await.unwrap_err();
    assert_auth_error_contains(error, "Security Error");
}

#[cfg(windows)]
#[tokio::test]
async fn save_and_load_round_trip_without_posix_modes_on_windows() {
    let (_guard, store) = temp_store();
    let credentials = v3_credentials();
    store.save("default", Some(&credentials)).await.unwrap();
    let loaded = store.load("default").await.unwrap().unwrap();
    assert_eq!(loaded, credentials);
    // Best-effort: the write still succeeds even though chmod/O_NOFOLLOW are
    // unavailable; a pre-existing file is not aborted on.
    store.save("default", Some(&credentials)).await.unwrap();
}

// ---------------------------------------------------------------------------
// Token refresh persistence (spec §7.4) over a wiremock token endpoint
// ---------------------------------------------------------------------------

async fn token_mock(server: &wiremock::MockServer, status: u16, body: serde_json::Value) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/token"))
        .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn refresh_success_saves_new_access_token_and_keeps_refresh_token() {
    let (guard, store) = temp_store();
    let mut credentials = v3_credentials();
    credentials.client_id = Some(crsp::constants::DEFAULT_OAUTH_CLIENT_ID.to_string());
    credentials.client_secret = Some(crsp::constants::DEFAULT_OAUTH_CLIENT_SECRET.to_string());
    store.save("default", Some(&credentials)).await.unwrap();

    let server = wiremock::MockServer::start().await;
    let _mock = token_mock(
        &server,
        200,
        serde_json::json!({
            "access_token": "ACCESS-PLACEHOLDER-REFRESHED",
            "expires_in": 3600,
            "token_type": "Bearer"
        }),
    )
    .await;

    let client = crsp::auth::oauth_client::OAuthClient::default_client()
        .with_token_url(format!("{}/token", server.uri()));
    let http = reqwest::Client::new();
    let updated = flow::refresh_and_save(&client, &credentials, &store, "default", &http)
        .await
        .unwrap();

    assert_eq!(
        updated.access_token.as_deref(),
        Some("ACCESS-PLACEHOLDER-REFRESHED")
    );
    // Clasp parity: refresh keeps the stored refresh token and updates only
    // access_token, id_token and expiry_date.
    assert_eq!(updated.refresh_token.as_deref(), Some(REFRESH_V3));
    assert!(updated.id_token.is_none());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let expiry = updated.expiry_date.unwrap();
    assert!(
        expiry > now + 3_000_000 && expiry <= now + 3_700_000,
        "expiry_date should be now + expires_in*1000"
    );
    // The refreshed credentials were persisted with 0600.
    let raw = read_file(store.path());
    assert!(raw.contains("ACCESS-PLACEHOLDER-REFRESHED"));
    #[cfg(unix)]
    assert_eq!(file_mode(store.path()), 0o600);
    drop(guard);
}

#[tokio::test]
async fn refresh_failure_keeps_the_old_token_and_prompts_relogin() {
    let (_guard, store) = temp_store();
    let credentials = v3_credentials();
    store.save("default", Some(&credentials)).await.unwrap();
    let before = read_file(store.path());

    let server = wiremock::MockServer::start().await;
    let _mock = token_mock(
        &server,
        400,
        serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        }),
    )
    .await;

    let client = crsp::auth::oauth_client::OAuthClient::default_client()
        .with_token_url(format!("{}/token", server.uri()));
    let http = reqwest::Client::new();
    let error = flow::refresh_and_save(&client, &credentials, &store, "default", &http)
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)), "got: {error:?}");
    assert_auth_error_contains(error, "crsp login");
    // The stored credentials are untouched on refresh failure.
    assert_eq!(read_file(store.path()), before);
}

#[tokio::test]
async fn refresh_without_a_stored_refresh_token_prompts_relogin() {
    let (_guard, store) = temp_store();
    let credentials = StoredCredentials {
        client_id: Some("id".to_string()),
        access_token: Some(ACCESS_V3.to_string()),
        credential_type: Some("authorized_user".to_string()),
        ..StoredCredentials::default()
    };
    store.save("default", Some(&credentials)).await.unwrap();
    let before = read_file(store.path());
    let client = crsp::auth::oauth_client::OAuthClient::default_client();
    let http = reqwest::Client::new();
    let error = flow::refresh_and_save(&client, &credentials, &store, "default", &http)
        .await
        .unwrap_err();
    assert!(matches!(error, CrspError::Auth(_)));
    assert_eq!(read_file(store.path()), before);
}
