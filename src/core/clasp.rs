use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::api::{ApiClient, ApiClientConfig, RefreshFn};
use crate::auth::oauth_client::OAuthClient;
use crate::auth::{CredentialStore, StoredCredentials, load_credentials, refresh_and_save};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;

pub struct Clasp {
    pub config: ProjectConfig,
    pub client: ApiClient,
    pub store: CredentialStore,
    pub credentials: Option<StoredCredentials>,
    pub user: String,
}

impl Clasp {
    pub async fn init(
        project: Option<&Path>,
        ignore: Option<&Path>,
        auth: Option<&Path>,
        user: &str,
        adc: bool,
        allow_symlinks: bool,
    ) -> Result<Arc<Self>, CrspError> {
        Self::init_context(project, ignore, auth, user, adc, allow_symlinks).await
    }

    pub async fn init_context(
        project: Option<&Path>,
        ignore: Option<&Path>,
        auth: Option<&Path>,
        user: &str,
        adc: bool,
        allow_symlinks: bool,
    ) -> Result<Arc<Self>, CrspError> {
        let cwd = std::env::current_dir()?;
        let project = project.filter(|path| path.exists());
        let mut config = ProjectConfig::discover(project, &cwd).await?;
        if let Some(ignore) = ignore {
            config.ignore_file_path = Some(resolve_file_or_dir(
                ignore,
                crate::constants::PROJECT_IGNORE_FILENAME,
            ));
        }
        config.allow_symlinks = allow_symlinks || config.allow_symlinks;
        let store = CredentialStore::new(auth_path(auth)?, allow_symlinks);
        let mut credentials = load_credentials(&store, user, adc).await?;
        // google-auth-library parity (clasp `getAuthorizedOAuth2Client`): a
        // saved entry with a missing/empty access token is kept when a
        // refresh token exists — the client refreshes lazily at the FIRST API
        // request (`getRequestMetadataAsync`), never at init, so `logout` and
        // `login` stay network-free even when the refresh token is broken.
        // Only an entry with nothing to refresh with is discarded.
        if credentials.as_ref().is_some_and(|current| {
            current.access_token.as_deref().is_none_or(str::is_empty)
                && current.refresh_token.is_none()
        }) {
            credentials = None;
        }
        if let Some(current) = credentials.as_ref()
            && current
                .expiry_date
                .is_some_and(|expiry| expiry <= now_millis())
            && current.refresh_token.is_some()
        {
            let client = env_oauth_client();
            credentials = Some(
                refresh_and_save(&client, current, &store, user, &reqwest::Client::new()).await?,
            );
        }
        let token = credentials
            .as_ref()
            .and_then(|value| value.access_token.clone())
            .unwrap_or_default();
        let refresh_store = store.clone();
        let refresh_user = user.to_string();
        let refresh_credentials = Arc::new(Mutex::new(credentials.clone()));
        let refresh_credentials_for_closure = Arc::clone(&refresh_credentials);
        let refresh: RefreshFn = std::sync::Arc::new(move || {
            let store = refresh_store.clone();
            let user = refresh_user.clone();
            let credentials_state = Arc::clone(&refresh_credentials_for_closure);
            let credentials = credentials_state.lock().unwrap().clone();
            Box::pin(async move {
                let credentials = credentials.ok_or_else(|| {
                    // google-auth-library `getRequestMetadataAsync` throws
                    // this exact error when there is no access token AND no
                    // refresh mechanism (empty store entry / not logged in).
                    CrspError::Auth(crate::i18n::NO_CREDENTIALS.to_string())
                })?;
                let client = env_oauth_client();
                let refreshed = refresh_and_save(
                    &client,
                    &credentials,
                    &store,
                    &user,
                    &reqwest::Client::new(),
                )
                .await?;
                let access_token = refreshed
                    .access_token
                    .clone()
                    .ok_or_else(|| CrspError::Auth("Authentication is required.".to_string()))?;
                *credentials_state.lock().unwrap() = Some(refreshed);
                Ok(access_token)
            })
        });
        let client = ApiClient::new(ApiClientConfig::new(token, refresh))?;
        Ok(Arc::new(Self {
            config,
            client,
            store,
            credentials,
            user: user.to_string(),
        }))
    }
}

/// The default OAuth client with the runtime/golden base-URL overrides
/// (T4 contract): the token endpoint follows `CRSP_OAUTH2_BASE_URL` /
/// `CRSP_API_BASE_URL`; production URLs remain the defaults.
fn env_oauth_client() -> OAuthClient {
    OAuthClient::default_client().with_token_url(crate::api::client::service_url(
        &crate::api::BaseUrls::from_env().oauth2,
        "/token",
    ))
}

fn auth_path(auth: Option<&Path>) -> Result<PathBuf, CrspError> {
    let path = match auth {
        Some(path) => path.to_path_buf(),
        None => home::home_dir()
            .ok_or_else(|| {
                CrspError::Auth("Unable to determine the user home directory.".to_string())
            })?
            .join(".clasprc.json"),
    };
    if path.is_dir() {
        Ok(path.join(crate::constants::CREDENTIALS_FILENAME))
    } else {
        Ok(path)
    }
}

fn resolve_file_or_dir(path: &Path, filename: &str) -> PathBuf {
    if path.is_dir() {
        path.join(filename)
    } else {
        path.to_path_buf()
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}
