//! Discovery API endpoint (spec §2.2; clasp `core/services.ts`
//! `getAvailableServices`). A single unpaginated call.

use serde::Deserialize;

use crate::api::ApiRequest;
use crate::api::client::{ApiClient, append_query, service_url};
use crate::api::script::parse_response;
use crate::error::CrspError;

/// A discoverable API (clasp filters on `id`, `name`, `description`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryApi {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// GET `…/discovery/v1/apis?preferred=true` response.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApisListPage {
    #[serde(default)]
    pub items: Option<Vec<DiscoveryApi>>,
}

impl ApisListPage {
    /// clasp `data.items ?? []`.
    pub fn items(&self) -> &[DiscoveryApi] {
        self.items.as_deref().unwrap_or(&[])
    }
}

/// Typed Discovery API methods (the wrapper name is plural to stay distinct
/// from the [`DiscoveryApi`] item model).
pub struct DiscoveryApis<'a>(pub(crate) &'a ApiClient);

impl DiscoveryApis<'_> {
    /// GET `{discovery}/discovery/v1/apis?preferred=true` (unpaginated; clasp
    /// reads `data.items ?? []`).
    pub async fn list_apis(&self) -> Result<Vec<DiscoveryApi>, CrspError> {
        let url = append_query(
            &service_url(&self.0.base_urls().discovery, "/discovery/v1/apis"),
            "preferred",
            "true",
        );
        let page: ApisListPage = parse_response(
            self.0
                .request(ApiRequest {
                    method: reqwest::Method::GET,
                    url,
                    body: None,
                })
                .await?,
        )
        .await?;
        Ok(page.items.unwrap_or_default())
    }
}
