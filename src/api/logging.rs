//! Cloud Logging endpoint (spec §2.2; clasp `core/logs.ts` `getLogEntries`).
//!
//! The request is a JSON POST with the page size and token inside the body;
//! entries keep the full server shape (known envelope fields typed, the rest
//! preserved through `extra`) because tail-logs `--json` re-serializes whole
//! entries.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::client::{ApiClient, service_url};
use crate::api::error::parse_error;
use crate::api::{ApiBody, ApiRequest, PagedResults};
use crate::core::pagination::{Page, PageOptions};
use crate::error::CrspError;

/// POST `…/v2/entries:list` body (clasp logs.ts:62-70). The page token is a
/// body field here (unlike the list endpoints that use a query parameter).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListEntriesRequest {
    pub resource_names: Vec<String>,
    pub filter: String,
    pub order_by: String,
    pub page_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
}

/// A Cloud Logging entry. Known clasp-consumed fields are typed; unknown
/// fields are preserved for `--json` re-serialization (ordered maps).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default, rename = "insertId")]
    pub insert_id: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default, rename = "textPayload")]
    pub text_payload: Option<String>,
    #[serde(default, rename = "jsonPayload")]
    pub json_payload: Option<Value>,
    #[serde(default)]
    pub resource: Option<LogResource>,
    /// Every other field of the entry, order-preserved.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogResource {
    #[serde(default)]
    pub labels: Option<Map<String, Value>>,
}

impl LogEntry {
    /// Function name label (clasp `resource.labels?.['function_name'] ?? 'N/A'`).
    pub fn function_name(&self) -> Option<&str> {
        self.resource
            .as_ref()
            .and_then(|resource| resource.labels.as_ref())
            .and_then(|labels| labels.get("function_name"))
            .and_then(Value::as_str)
    }
}

/// POST `…/entries:list` page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntriesListPage {
    #[serde(default)]
    pub entries: Option<Vec<LogEntry>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// Typed Cloud Logging API methods.
pub struct LoggingApi<'a>(pub(crate) &'a ApiClient);

impl LoggingApi<'_> {
    /// POST `{logging}/v2/entries:list` with clasp's default pagination
    /// (page size 100, max 10 pages). `filter` is the already-built filter
    /// string (empty string when absent, matching clasp).
    pub async fn list_entries(
        &self,
        project_id: &str,
        filter: &str,
    ) -> Result<PagedResults<LogEntry>, CrspError> {
        let client = self.0;
        let resource_names = vec![format!("projects/{project_id}")];
        let order_by = "timestamp desc".to_string();
        crate::core::pagination::fetch_pages(
            |page_size: usize, page_token: Option<String>| {
                let body = ListEntriesRequest {
                    resource_names: resource_names.clone(),
                    filter: filter.to_string(),
                    order_by: order_by.clone(),
                    page_size,
                    page_token: page_token.clone(),
                };
                let url = service_url(&client.base_urls().logging, "/v2/entries:list");
                async move {
                    let body_value = serde_json::to_value(&body).map_err(parse_error)?;
                    let page: EntriesListPage = client
                        .request(ApiRequest {
                            method: reqwest::Method::POST,
                            url,
                            body: Some(ApiBody::Json(body_value)),
                        })
                        .await?
                        .json()
                        .await?;
                    Ok(Page {
                        results: page.entries.unwrap_or_default(),
                        page_token: page.next_page_token,
                    })
                }
            },
            PageOptions::default(),
        )
        .await
    }
}
