//! OAuth client primitives (spec §2.3, §2.4, §2.2 token/userinfo endpoints;
//! clasp `oauth_client.ts`, `auth.ts`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::constants::{DEFAULT_OAUTH_CLIENT_ID, DEFAULT_OAUTH_CLIENT_SECRET};
use crate::error::CrspError;
use crate::i18n;

/// Google's OAuth 2.0 authorization endpoint.
pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google's OAuth 2.0 token endpoint.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google's userinfo endpoint used by login/show-authorized-user (spec §2.2).
pub const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

/// Redirect URI registered for the default desktop client (spec §2.3:
/// Google's desktop-app convention; runtime redirects use any localhost port).
pub const DEFAULT_REDIRECT_URI: &str = "http://localhost";

/// Serverless flow redirect port (clasp `ServerlessAuthorizationCodeFlow`).
pub const SERVERLESS_REDIRECT_PORT: u16 = 8888;

/// The ten default scopes (clasp `commands/login.ts` DEFAULT_SCOPES; spec
/// §2.4).
pub const DEFAULT_SCOPES: [&str; 10] = [
    "https://www.googleapis.com/auth/script.deployments",
    "https://www.googleapis.com/auth/script.projects",
    "https://www.googleapis.com/auth/script.webapp.deploy",
    "https://www.googleapis.com/auth/drive.metadata.readonly",
    "https://www.googleapis.com/auth/drive.file",
    "https://www.googleapis.com/auth/service.management",
    "https://www.googleapis.com/auth/logging.read",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cloud-platform",
];

/// Endpoint set for OAuth HTTP calls; overridable in tests (spec §9.1
/// base-URL override strategy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEndpoints {
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
}

impl Default for AuthEndpoints {
    fn default() -> Self {
        Self {
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: GOOGLE_TOKEN_URL.to_string(),
            userinfo_url: GOOGLE_USERINFO_URL.to_string(),
        }
    }
}

impl AuthEndpoints {
    /// Environment-aware endpoints (T4 golden-subprocess contract): the
    /// token/userinfo URLs follow the `CRSP_*_BASE_URL` overrides while the
    /// production Google URLs remain the defaults. Paths stay unchanged.
    pub fn from_base_urls(base: &crate::api::BaseUrls) -> Self {
        Self {
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: crate::api::client::service_url(&base.oauth2, "/token"),
            userinfo_url: crate::api::client::service_url(&base.userinfo, "/v2/userinfo"),
        }
    }
}

/// Client classification shown by show-authorized-user (clasp
/// `oauth_client.ts:28-33`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OAuthClientType {
    #[serde(rename = "google-provided")]
    GoogleProvided,
    #[serde(rename = "user-provided")]
    UserProvided,
}

impl OAuthClientType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GoogleProvided => "google-provided",
            Self::UserProvided => "user-provided",
        }
    }
}

/// Classifies a client ID (clasp `getOAuthClientType`); `None` for unknown
/// clients (no credentials / metadata-server ADC).
pub fn client_type(client_id: Option<&str>) -> Option<OAuthClientType> {
    client_id.map(|id| {
        if id == DEFAULT_OAUTH_CLIENT_ID {
            OAuthClientType::GoogleProvided
        } else {
            OAuthClientType::UserProvided
        }
    })
}

/// Tokens returned by the Google token endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenSet {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    /// Seconds until expiry (converted to `expiry_date` ms when saving).
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
    pub token_type: Option<String>,
}

/// An OAuth client: default Google-provided or user-provided from a client
/// secret file (clasp `auth.ts:116-263`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub endpoints: AuthEndpoints,
}

impl OAuthClient {
    /// The default clasp desktop client (existing tokens stay compatible).
    pub fn default_client() -> Self {
        Self {
            client_id: DEFAULT_OAUTH_CLIENT_ID.to_string(),
            client_secret: DEFAULT_OAUTH_CLIENT_SECRET.to_string(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            endpoints: AuthEndpoints::default(),
        }
    }

    /// Builds a client from a Google client-secret JSON file (clasp
    /// `createOauthClient`, auth.ts:237-263): `installed` or `web` entries
    /// only, with a registered `localhost` redirect URI.
    pub fn from_client_secret_json(json: &str) -> Result<Self, CrspError> {
        let file: Value = serde_json::from_str(json)
            .map_err(|error| CrspError::Config(format!("Invalid credentials: {error}")))?;
        let keys = file
            .get("installed")
            .or_else(|| file.get("web"))
            .filter(|keys| !keys.is_null());
        let keys = keys.ok_or_else(|| CrspError::Auth("Invalid credentials".to_string()))?;

        let client_id = keys
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or_else(|| CrspError::Auth("Invalid credentials".to_string()))?;
        let client_secret = keys
            .get("client_secret")
            .and_then(Value::as_str)
            .ok_or_else(|| CrspError::Auth("Invalid credentials".to_string()))?;

        let redirect_uris = keys.get("redirect_uris").and_then(Value::as_array);
        let redirect_uris = match redirect_uris {
            Some(uris) if !uris.is_empty() => uris,
            _ => {
                return Err(CrspError::Auth("Invalid redirect URL".to_string()));
            }
        };
        let redirect_uri = redirect_uris
            .iter()
            .find_map(|uri| {
                let raw = uri.as_str()?;
                let parsed = url::Url::parse(raw).ok()?;
                (parsed.host_str() == Some("localhost")).then(|| raw.to_string())
            })
            .ok_or_else(|| CrspError::Auth("No localhost redirect URL found".to_string()))?;

        Ok(Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            redirect_uri: redirect_uri.to_string(),
            endpoints: AuthEndpoints::default(),
        })
    }

    pub fn with_token_url(self, token_url: String) -> Self {
        Self {
            endpoints: AuthEndpoints {
                token_url,
                ..self.endpoints
            },
            ..self
        }
    }

    pub fn client_type(&self) -> OAuthClientType {
        client_type(Some(&self.client_id)).expect("client_type is Some for any non-empty id")
    }

    /// Exchanges an authorization code for tokens (clasp
    /// `oauth2Client.getToken({code, redirect_uri, codeVerifier})`).
    pub async fn exchange_code(
        &self,
        http: &reqwest::Client,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenSet, CrspError> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        let response = http
            .post(&self.endpoints.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|error| CrspError::Auth(error.to_string()))?;
        let value = read_token_response(response).await?;
        Ok(TokenSet {
            access_token: optional_string(&value, "access_token"),
            refresh_token: optional_string(&value, "refresh_token"),
            id_token: optional_string(&value, "id_token"),
            expires_in: value.get("expires_in").and_then(Value::as_i64),
            scope: optional_string(&value, "scope"),
            token_type: optional_string(&value, "token_type"),
        })
    }

    /// Refreshes an access token (grant_type=refresh_token).
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        refresh_token: &str,
    ) -> Result<TokenSet, CrspError> {
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        let response = http
            .post(&self.endpoints.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|error| CrspError::Auth(error.to_string()))?;
        let value = read_token_response(response).await?;
        Ok(TokenSet {
            access_token: optional_string(&value, "access_token"),
            refresh_token: optional_string(&value, "refresh_token"),
            id_token: optional_string(&value, "id_token"),
            expires_in: value.get("expires_in").and_then(Value::as_i64),
            scope: optional_string(&value, "scope"),
            token_type: optional_string(&value, "token_type"),
        })
    }
}

/// Parses a token endpoint response; non-2xx responses become `Auth` errors
/// carrying Google's `error`/`error_description` (clasp surfaces the raw
/// google-auth-library error).
async fn read_token_response(response: reqwest::Response) -> Result<Value, CrspError> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !status.is_success() {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let description = value
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let detail = match (error.is_empty(), description.is_empty()) {
            (false, false) => format!("{error} - {description}"),
            (false, true) => error.to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(CrspError::Auth(i18n::access_token_request_failed(&detail)));
    }
    if !value.is_object() {
        return Err(CrspError::Auth(i18n::access_token_request_failed(
            "invalid token response",
        )));
    }
    Ok(value)
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// ADC settings parsed from an authorized_user Application Default
/// Credentials file (subset of google-auth-library's JWTInput).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdcAuthorizedUser {
    #[serde(rename = "type")]
    pub credential_type: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}
