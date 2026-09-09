//! Userinfo and token endpoints (spec §2.2; clasp `auth/auth.ts`
//! `getUserInfo`, google-auth-library token exchange/refresh).
//!
//! `userinfo` goes through the standard [`ApiClient`] pipeline (transient
//! GET retries + one-shot 401 refresh, matching google-auth-library). The
//! token endpoint calls delegate to [`crate::auth::oauth_client::OAuthClient`]
//! (no retries: they are POSTs) with the token URL routed through the
//! configured `oauth2` base URL override.

use serde::Deserialize;

use crate::api::ApiRequest;
use crate::api::client::{ApiClient, service_url};
use crate::api::script::parse_response;
use crate::auth::oauth_client::{AuthEndpoints, DEFAULT_REDIRECT_URI, OAuthClient, TokenSet};
use crate::error::CrspError;

/// GET `…/oauth2/v2/userinfo` response (clasp reads `data.email`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

/// Userinfo and token endpoint methods.
pub struct OAuth2Api<'a>(pub(crate) &'a ApiClient);

impl OAuth2Api<'_> {
    /// GET `{userinfo}/v2/userinfo` with the current bearer token.
    pub async fn userinfo(&self) -> Result<UserInfo, CrspError> {
        let url = service_url(&self.0.base_urls().userinfo, "/v2/userinfo");
        parse_response(
            self.0
                .request(ApiRequest {
                    method: reqwest::Method::GET,
                    url,
                    body: None,
                })
                .await?,
        )
        .await
    }

    /// POST `{oauth2}/token` — authorization-code exchange with PKCE
    /// (clasp `oauth2Client.getToken`).
    pub async fn exchange_code(
        &self,
        client: &OAuthClient,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenSet, CrspError> {
        let client = Self::with_token_base(client, &self.0.base_urls().oauth2);
        client
            .exchange_code(&self.0.http, code, redirect_uri, code_verifier)
            .await
    }

    /// POST `{oauth2}/token` — refresh-token grant (spec §7.4's underlying
    /// call, reused from the auth layer).
    pub async fn refresh_token(
        &self,
        client: &OAuthClient,
        refresh_token: &str,
    ) -> Result<TokenSet, CrspError> {
        let client = Self::with_token_base(client, &self.0.base_urls().oauth2);
        client.refresh(&self.0.http, refresh_token).await
    }

    /// Rebuilds the client with the token URL from the `oauth2` base override.
    fn with_token_base(client: &OAuthClient, token_base: &str) -> OAuthClient {
        OAuthClient {
            client_id: client.client_id.clone(),
            client_secret: client.client_secret.clone(),
            redirect_uri: if client.redirect_uri.is_empty() {
                DEFAULT_REDIRECT_URI.to_string()
            } else {
                client.redirect_uri.clone()
            },
            endpoints: AuthEndpoints {
                auth_url: client.endpoints.auth_url.clone(),
                token_url: service_url(token_base, "/token"),
                userinfo_url: client.endpoints.userinfo_url.clone(),
            },
        }
    }
}
