//! Cloud Logging endpoint (spec §2.2; clasp `core/logs.ts` `getLogEntries`).
//!
//! The request is a JSON POST with the page size and token inside the body;
//! entries keep the full server shape as an ordered document because
//! tail-logs `--json` re-serializes whole entries (`JSON.stringify(entry,
//! null, 2)`) with the original key order (spec §6.2 tail-logs row).

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// A Cloud Logging entry: the raw ordered document (field order preserved
/// for `--json` re-serialization) plus typed accessors for the fields clasp
/// consumes.
#[derive(Debug, Clone, Default)]
pub struct LogEntry {
    raw: Value,
}

impl<'de> Deserialize<'de> for LogEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(|raw| Self { raw })
    }
}

impl LogEntry {
    /// The raw entry document, in server key order.
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// clasp `entry.timestamp`.
    pub fn timestamp(&self) -> Option<&str> {
        self.raw.get("timestamp").and_then(Value::as_str)
    }

    /// clasp `entry.insertId`.
    pub fn insert_id(&self) -> Option<&str> {
        self.raw.get("insertId").and_then(Value::as_str)
    }

    /// clasp `entry.severity`.
    pub fn severity(&self) -> Option<&str> {
        self.raw.get("severity").and_then(Value::as_str)
    }

    /// clasp `entry.textPayload`.
    pub fn text_payload(&self) -> Option<&str> {
        self.raw.get("textPayload").and_then(Value::as_str)
    }

    /// clasp `entry.jsonPayload`.
    pub fn json_payload(&self) -> Option<&Value> {
        self.raw.get("jsonPayload")
    }

    /// clasp `resource.labels?.['function_name']`.
    pub fn function_name_label(&self) -> Option<&str> {
        self.raw
            .get("resource")
            .and_then(|resource| resource.get("labels"))
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
