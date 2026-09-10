use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::{ApiClient, ApiClientConfig, RefreshFn};
use crate::auth::oauth_client::{AuthEndpoints, OAuthClient};
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
        let cwd = std::env::current_dir()?;
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
        if let Some(current) = credentials.as_ref()
            && current.access_token.is_none()
        {
            credentials = None;
        }
        if let Some(current) = credentials.as_ref()
            && current
                .expiry_date
                .is_some_and(|expiry| expiry <= now_millis())
            && current.refresh_token.is_some()
        {
            let client = OAuthClient {
                endpoints: AuthEndpoints::default(),
                ..OAuthClient::default_client()
            };
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
        let refresh_credentials = credentials.clone();
        let refresh: RefreshFn = std::sync::Arc::new(move || {
            let store = refresh_store.clone();
            let user = refresh_user.clone();
            let credentials = refresh_credentials.clone();
            Box::pin(async move {
                let credentials = credentials
                    .ok_or_else(|| CrspError::Auth("Authentication is required.".to_string()))?;
                let client = OAuthClient::default_client();
                let refreshed = refresh_and_save(
                    &client,
                    &credentials,
                    &store,
                    &user,
                    &reqwest::Client::new(),
                )
                .await?;
                refreshed
                    .access_token
                    .ok_or_else(|| CrspError::Auth("Authentication is required.".to_string()))
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
