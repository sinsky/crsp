//! Drive v3 endpoints used by crsp (spec §2.2; clasp `core/project.ts`
//! `listScripts`, `createWithContainer`, `trashScript`).
//!
//! clasp parity note: container creation calls `drive.files.create` with only
//! a `requestBody` (project.ts:160-167), and googleapis-common sends a plain
//! JSON POST to `/v3/files` when no `media.body` is given — so crsp sends the
//! same JSON POST, not a multipart upload.

use serde::{Deserialize, Serialize};

use crate::api::client::{ApiClient, append_query, append_query_if_some, service_url};
use crate::api::script::{json_body, parse_response};
use crate::api::{ApiRequest, PagedResults};
use crate::core::pagination::{Page, PageOptions};
use crate::error::CrspError;

/// Drive filter for Apps Script projects (clasp listScripts, project.ts:204).
pub const SCRIPT_MIME_TYPE_QUERY: &str = "mimeType=\"application/vnd.google-apps.script\"";

/// Drive field mask (clasp listScripts, project.ts:203).
pub const LIST_FIELDS: &str = "nextPageToken, files(id, name)";

/// POST `…/v3/files` body (container creation, clasp project.ts:160-165).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileRequest {
    pub mime_type: String,
    pub name: String,
}

/// A Drive file (clasp `Script = {name, id}`; container creation).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// GET `…/v3/files` page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesListPage {
    #[serde(default)]
    pub files: Option<Vec<DriveFile>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// PATCH `…/v3/files/{fileId}` body (trash, clasp project.ts:125-130).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrashFileRequest {
    pub trashed: bool,
}

/// Typed Drive v3 API methods.
pub struct DriveApi<'a>(pub(crate) &'a ApiClient);

impl DriveApi<'_> {
    /// GET `{drive}/v3/files` with clasp's query and default pagination
    /// (page size 100, max 10 pages).
    pub async fn list_files(&self) -> Result<PagedResults<DriveFile>, CrspError> {
        let client = self.0;
        crate::core::pagination::fetch_pages_send(
            |page_size: usize, page_token: Option<String>| {
                let url = append_query_if_some(
                    &append_query(
                        &append_query(
                            &append_query(
                                &service_url(&client.base_urls().drive, "/v3/files"),
                                "pageSize",
                                &page_size.to_string(),
                            ),
                            "fields",
                            LIST_FIELDS,
                        ),
                        "q",
                        SCRIPT_MIME_TYPE_QUERY,
                    ),
                    "pageToken",
                    page_token.as_deref(),
                );
                async move {
                    let page: FilesListPage = client
                        .request(ApiRequest {
                            method: reqwest::Method::GET,
                            url,
                            body: None,
                        })
                        .await?
                        .json()
                        .await?;
                    Ok(Page {
                        results: page.files.unwrap_or_default(),
                        page_token: page.next_page_token,
                    })
                }
            },
            PageOptions::default(),
        )
        .await
    }

    /// POST `{drive}/v3/files` — plain JSON POST (clasp parity; see module
    /// docs). Returns the created container file.
    pub async fn create_file(&self, mime_type: &str, name: &str) -> Result<DriveFile, CrspError> {
        let url = service_url(&self.0.base_urls().drive, "/v3/files");
        let body = CreateFileRequest {
            mime_type: mime_type.to_string(),
            name: name.to_string(),
        };
        parse_response(
            self.0
                .request(ApiRequest {
                    method: reqwest::Method::POST,
                    url,
                    body: json_body(&body)?,
                })
                .await?,
        )
        .await
    }

    /// PATCH `{drive}/v3/files/{fileId}` with `{trashed: true}` (clasp
    /// `trashScript`).
    pub async fn trash_file(&self, file_id: &str) -> Result<(), CrspError> {
        let url = service_url(&self.0.base_urls().drive, &format!("/v3/files/{file_id}"));
        let body = TrashFileRequest { trashed: true };
        self.0
            .request(ApiRequest {
                method: reqwest::Method::PATCH,
                url,
                body: json_body(&body)?,
            })
            .await?;
        Ok(())
    }
}
