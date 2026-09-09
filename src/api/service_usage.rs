//! Service Usage API endpoints (spec §2.1, §2.2; clasp `core/services.ts`).
//!
//! `services.list` uses the clasp list-apis exception: page size 200 and a
//! 10000-result cap instead of the defaults.

use serde::Deserialize;

use crate::api::client::{ApiClient, append_query, append_query_if_some, service_url};
use crate::api::{ApiRequest, PagedResults};
use crate::core::pagination::{
    DEFAULT_MAX_PAGES, Page, PageOptions, SERVICE_USAGE_MAX_RESULTS, SERVICE_USAGE_PAGE_SIZE,
};
use crate::error::CrspError;

/// Service filter (clasp services.ts:75).
pub const ENABLED_FILTER: &str = "state:ENABLED";

/// A Service Usage service (clasp reads `service.name` and
/// `service.config.name` / `service.config.documentation.summary`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub config: Option<ServiceConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub documentation: Option<ServiceDocumentation>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDocumentation {
    #[serde(default)]
    pub summary: Option<String>,
}

impl Service {
    /// Short service name (clasp `truncateName`: text before the first dot).
    pub fn short_name(&self) -> Option<&str> {
        self.config
            .as_ref()
            .and_then(|config| config.name.as_deref())
            .and_then(|name| name.split('.').next())
    }

    /// Documentation summary (clasp `service.config?.documentation?.summary ?? ''`).
    pub fn summary(&self) -> &str {
        self.config
            .as_ref()
            .and_then(|config| config.documentation.as_ref())
            .and_then(|documentation| documentation.summary.as_deref())
            .unwrap_or("")
    }
}

/// GET `…/services` page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesListPage {
    #[serde(default)]
    pub services: Option<Vec<Service>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// Typed Service Usage API methods.
pub struct ServiceUsageApi<'a>(pub(crate) &'a ApiClient);

impl ServiceUsageApi<'_> {
    /// GET `{service_usage}/v1/projects/{projectId}/services` with the clasp
    /// list-apis pagination exception (page size 200, max 10000 results).
    pub async fn list_enabled(&self, project_id: &str) -> Result<PagedResults<Service>, CrspError> {
        let client = self.0;
        let path = format!("/v1/projects/{project_id}/services");
        crate::core::pagination::fetch_pages(
            |page_size: usize, page_token: Option<String>| {
                let url = append_query_if_some(
                    &append_query(
                        &append_query(
                            &service_url(&client.base_urls().service_usage, &path),
                            "filter",
                            ENABLED_FILTER,
                        ),
                        "pageSize",
                        &page_size.to_string(),
                    ),
                    "pageToken",
                    page_token.as_deref(),
                );
                async move {
                    let page: ServicesListPage = client
                        .request(ApiRequest {
                            method: reqwest::Method::GET,
                            url,
                            body: None,
                        })
                        .await?
                        .json()
                        .await?;
                    Ok(Page {
                        results: page.services.unwrap_or_default(),
                        page_token: page.next_page_token,
                    })
                }
            },
            PageOptions {
                page_size: SERVICE_USAGE_PAGE_SIZE,
                max_pages: DEFAULT_MAX_PAGES,
                max_results: SERVICE_USAGE_MAX_RESULTS,
            },
        )
        .await
    }

    /// POST `{service_usage}/v1/projects/{projectId}/services/{api}.googleapis.com:enable`
    /// (clasp services.ts:221-230; no request body).
    pub async fn enable(&self, project_id: &str, api: &str) -> Result<(), CrspError> {
        self.toggle(project_id, api, ":enable").await
    }

    /// POST `{service_usage}/v1/projects/{projectId}/services/{api}.googleapis.com:disable`.
    pub async fn disable(&self, project_id: &str, api: &str) -> Result<(), CrspError> {
        self.toggle(project_id, api, ":disable").await
    }

    async fn toggle(&self, project_id: &str, api: &str, action: &str) -> Result<(), CrspError> {
        let url = service_url(
            &self.0.base_urls().service_usage,
            &format!("/v1/projects/{project_id}/services/{api}.googleapis.com{action}"),
        );
        self.0
            .request(ApiRequest {
                method: reqwest::Method::POST,
                url,
                body: None,
            })
            .await?;
        Ok(())
    }
}
