use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::api::{ApiClient, ApiClientConfig, RefreshFn};
use crate::auth::oauth_client::OAuthClient;
use crate::auth::{CredentialStore, StoredCredentials, load_credentials, refresh_and_save};
use crate::core::config::ProjectConfig;
use crate::error::CrspError;

pub struct Clasp {
    pub config: ProjectConfig,
    client: OnceLock<Result<ApiClient, String>>,
    client_config: ApiClientConfig,
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
        let mut config = ProjectConfig::discover(project, &cwd).await?;
        if let Some(ignore) = ignore {
            if !ignore.exists() {
                return Err(CrspError::Config(crate::i18n::invalid_ignore_path(
                    &ignore.to_string_lossy(),
                )));
            }
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
        // refresh token exists, and NO refresh happens at init — the client
        // refreshes lazily at the FIRST API request (`getRequestMetadataAsync`
        // + `isTokenExpiring`), so `logout` and `login` stay network-free even
        // when the stored token is expired or the refresh token is broken.
        // Only an entry with nothing to refresh with is discarded.
        if credentials.as_ref().is_some_and(|current| {
            current.access_token.as_deref().is_none_or(str::is_empty)
                && current.refresh_token.is_none()
        }) {
            credentials = None;
        }
        let token = credentials
            .as_ref()
            .and_then(|value| value.access_token.clone())
            .unwrap_or_default();
        let refresh_store = store.clone();
        let refresh_user = user.to_string();
        let refresh_credentials = Arc::new(Mutex::new(credentials.clone()));
        let refresh_credentials_for_closure = Arc::clone(&refresh_credentials);
        let expiry_cell: crate::api::client::ExpiryCell = Arc::new(Mutex::new(
            credentials.as_ref().and_then(|value| value.expiry_date),
        ));
        let expiry_cell_for_closure = Arc::clone(&expiry_cell);
        let refresh: RefreshFn = std::sync::Arc::new(move || {
            let store = refresh_store.clone();
            let user = refresh_user.clone();
            let credentials_state = Arc::clone(&refresh_credentials_for_closure);
            let expiry_cell = Arc::clone(&expiry_cell_for_closure);
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
                *expiry_cell.lock().unwrap() = refreshed.expiry_date;
                *credentials_state.lock().unwrap() = Some(refreshed);
                Ok(access_token)
            })
        });
        let client_config = ApiClientConfig {
            token_expiry: Some(expiry_cell),
            ..ApiClientConfig::new(token, refresh)
        };
        Ok(Arc::new(Self {
            config,
            client: OnceLock::new(),
            client_config,
            store,
            credentials,
            user: user.to_string(),
        }))
    }

    pub fn client(&self) -> Result<&ApiClient, CrspError> {
        self.client
            .get_or_init(|| {
                ApiClient::new(self.client_config.clone()).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|message| CrspError::Config(message.clone()))
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
