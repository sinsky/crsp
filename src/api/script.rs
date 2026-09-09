//! Apps Script API endpoints (spec §2.2; clasp `core/project.ts`,
//! `core/files.ts`, `core/functions.ts`).
//!
//! Every path, query name, body field, and pagination parameter mirrors
//! clasp's googleapis calls exactly (pageSize 100 / maxPages 10 via
//! [`crate::core::pagination::fetch_pages`]).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::client::{ApiClient, ApiResponse, append_query, append_query_if_some, service_url};
use crate::api::error::parse_error;
use crate::api::{ApiBody, ApiRequest};
use crate::core::pagination::{Page, PageOptions, PagedResults};
use crate::error::CrspError;

/// Deployment manifest file name (clasp `manifestFileName: 'appsscript'`).
pub const MANIFEST_FILE_NAME: &str = "appsscript";

/// POST `…/v1/projects` request body (clasp `createScript`, project.ts:92-97;
/// JSON.stringify drops an absent `parentId`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
}

/// POST `…/v1/projects` response (clasp reads `res.data.scriptId`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptProject {
    #[serde(default)]
    pub script_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// A remote script file (clasp `ScriptFile`; pull/push/run).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptFile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub file_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub function_set: Option<FunctionSet>,
}

impl ScriptFile {
    /// clasp `getFunctionNames`: `file.functionSet?.values ?? []`.
    pub fn function_names(&self) -> Vec<String> {
        self.function_set
            .as_ref()
            .and_then(|set| set.values.clone())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.name)
            .collect()
    }
}

/// A file's runnable functions (clasp `file.functionSet?.values ?? []`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct FunctionSet {
    #[serde(default)]
    pub values: Option<Vec<FunctionValue>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FunctionValue {
    #[serde(default)]
    pub name: Option<String>,
}

/// GET/PUT `…/v1/projects/{scriptId}/content` payload.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptContent {
    #[serde(default)]
    pub files: Option<Vec<ScriptFile>>,
}

impl ScriptContent {
    /// clasp `response.data.files ?? []`.
    pub fn files(&self) -> &[ScriptFile] {
        self.files.as_deref().unwrap_or(&[])
    }
}

/// PUT `…/content` file entry (clasp push, files.ts:599-604).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushFile {
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateContentRequest {
    pub files: Vec<PushFile>,
}

/// POST `…/versions` body (clasp `version`, project.ts:235-240 —
/// `description ?? ''` is always present).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CreateVersionRequest {
    pub description: String,
}

/// A script version (clasp `script_v1.Schema$Version` fields in use).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    #[serde(default)]
    pub version_number: Option<i32>,
    #[serde(default)]
    pub description: Option<String>,
}

/// GET `…/versions` page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsPage {
    #[serde(default)]
    pub versions: Option<Vec<Version>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// POST `…/deployments` body (clasp `deploy`, project.ts:349-356).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeploymentRequest {
    pub description: String,
    pub version_number: i32,
    pub manifest_file_name: String,
}

/// PUT `…/deployments/{id}` body (clasp redeploy, project.ts:362-372).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDeploymentRequest {
    pub deployment_config: DeploymentConfigInput,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfigInput {
    pub description: String,
    pub version_number: i32,
    pub script_id: String,
    pub manifest_file_name: String,
}

/// A deployment (clasp `script_v1.Schema$Deployment` fields in use).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    #[serde(default)]
    pub deployment_id: Option<String>,
    #[serde(default)]
    pub deployment_config: Option<DeploymentConfig>,
    #[serde(default)]
    pub update_time: Option<String>,
    #[serde(default)]
    pub entry_points: Option<Vec<EntryPoint>>,
}

impl Deployment {
    /// Versioned check for delete-deployment (clasp
    /// `deploymentConfig?.versionNumber !== undefined`).
    pub fn version_number(&self) -> Option<i32> {
        self.deployment_config
            .as_ref()
            .and_then(|config| config.version_number)
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfig {
    #[serde(default)]
    pub script_id: Option<String>,
    #[serde(default)]
    pub version_number: Option<i32>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub manifest_file_name: Option<String>,
}

/// An entry point (clasp open-web-app reads `entryPointType`/`webApp.url`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    #[serde(default)]
    pub entry_point_type: Option<String>,
    #[serde(default)]
    pub web_app: Option<WebApp>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebApp {
    #[serde(default)]
    pub url: Option<String>,
}

/// GET `…/deployments` page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentsPage {
    #[serde(default)]
    pub deployments: Option<Vec<Deployment>>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// POST `…/v1/scripts/{scriptId}/run` body (clasp `runFunction`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub function: String,
    pub parameters: Vec<Value>,
    pub dev_mode: bool,
}

/// Deserializes an API response into its typed model (degrade semantics).
pub(crate) async fn parse_response<T: DeserializeOwned>(
    response: ApiResponse,
) -> Result<T, CrspError> {
    response.json().await
}

/// Serializes a typed request body into the JSON wire body.
pub(crate) fn json_body<B: Serialize>(body: &B) -> Result<Option<ApiBody>, CrspError> {
    let value = serde_json::to_value(body).map_err(parse_error)?;
    Ok((!value.is_null()).then(|| ApiBody::Json(value)))
}

/// Typed Apps Script API methods.
pub struct ScriptApi<'a>(pub(crate) &'a ApiClient);

impl ScriptApi<'_> {
    /// POST `{script}/v1/projects` (clasp `createScript`).
    pub async fn create_project(
        &self,
        title: &str,
        parent_id: Option<&str>,
    ) -> Result<ScriptProject, CrspError> {
        let url = service_url(&self.0.base_urls().script, "/v1/projects");
        let body = CreateProjectRequest {
            parent_id: parent_id.map(str::to_string),
            title: title.to_string(),
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

    /// GET `{script}/v1/projects/{scriptId}/content[?versionNumber=N]`
    /// (clasp push diff, pull, clone, run function list).
    pub async fn get_content(
        &self,
        script_id: &str,
        version_number: Option<i32>,
    ) -> Result<ScriptContent, CrspError> {
        let url = append_query_if_some(
            &service_url(
                &self.0.base_urls().script,
                &format!("/v1/projects/{script_id}/content"),
            ),
            "versionNumber",
            version_number.map(|n| n.to_string()).as_deref(),
        );
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

    /// PUT `{script}/v1/projects/{scriptId}/content` (full-replacement push;
    /// clasp awaits `updateContent` and ignores the response data).
    pub async fn update_content(
        &self,
        script_id: &str,
        files: &[PushFile],
    ) -> Result<(), CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/content"),
        );
        let body = UpdateContentRequest {
            files: files.to_vec(),
        };
        self.0
            .request(ApiRequest {
                method: reqwest::Method::PUT,
                url,
                body: json_body(&body)?,
            })
            .await?;
        Ok(())
    }

    /// POST `{script}/v1/projects/{scriptId}/versions` (clasp `version`).
    pub async fn create_version(
        &self,
        script_id: &str,
        description: &str,
    ) -> Result<Version, CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/versions"),
        );
        let body = CreateVersionRequest {
            description: description.to_string(),
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

    /// GET `{script}/v1/projects/{scriptId}/versions` with clasp's default
    /// pagination (page size 100, max 10 pages).
    pub async fn list_versions(&self, script_id: &str) -> Result<PagedResults<Version>, CrspError> {
        let client = self.0;
        let path = format!("/v1/projects/{script_id}/versions");
        crate::core::pagination::fetch_pages(
            |page_size: usize, page_token: Option<String>| {
                let url = append_query_if_some(
                    &append_query(
                        &service_url(&client.base_urls().script, &path),
                        "pageSize",
                        &page_size.to_string(),
                    ),
                    "pageToken",
                    page_token.as_deref(),
                );
                async move {
                    let page: VersionsPage = client
                        .request(ApiRequest {
                            method: reqwest::Method::GET,
                            url,
                            body: None,
                        })
                        .await?
                        .json()
                        .await?;
                    Ok(Page {
                        results: page.versions.unwrap_or_default(),
                        page_token: page.next_page_token,
                    })
                }
            },
            PageOptions::default(),
        )
        .await
    }

    /// POST `{script}/v1/projects/{scriptId}/deployments` (clasp `deploy`).
    pub async fn create_deployment(
        &self,
        script_id: &str,
        description: &str,
        version_number: i32,
    ) -> Result<Deployment, CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/deployments"),
        );
        let body = CreateDeploymentRequest {
            description: description.to_string(),
            version_number,
            manifest_file_name: MANIFEST_FILE_NAME.to_string(),
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

    /// PUT `{script}/v1/projects/{scriptId}/deployments/{deploymentId}`
    /// (clasp redeploy).
    pub async fn update_deployment(
        &self,
        script_id: &str,
        deployment_id: &str,
        description: &str,
        version_number: i32,
    ) -> Result<Deployment, CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/deployments/{deployment_id}"),
        );
        let body = UpdateDeploymentRequest {
            deployment_config: DeploymentConfigInput {
                description: description.to_string(),
                version_number,
                script_id: script_id.to_string(),
                manifest_file_name: MANIFEST_FILE_NAME.to_string(),
            },
        };
        parse_response(
            self.0
                .request(ApiRequest {
                    method: reqwest::Method::PUT,
                    url,
                    body: json_body(&body)?,
                })
                .await?,
        )
        .await
    }

    /// GET `{script}/v1/projects/{scriptId}/deployments/{deploymentId}`
    /// (clasp open-web-app entry points).
    pub async fn get_deployment(
        &self,
        script_id: &str,
        deployment_id: &str,
    ) -> Result<Deployment, CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/deployments/{deployment_id}"),
        );
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

    /// DELETE `{script}/v1/projects/{scriptId}/deployments/{deploymentId}`.
    pub async fn delete_deployment(
        &self,
        script_id: &str,
        deployment_id: &str,
    ) -> Result<(), CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/projects/{script_id}/deployments/{deployment_id}"),
        );
        self.0
            .request(ApiRequest {
                method: reqwest::Method::DELETE,
                url,
                body: None,
            })
            .await?;
        Ok(())
    }

    /// GET `{script}/v1/projects/{scriptId}/deployments` (default pagination).
    pub async fn list_deployments(
        &self,
        script_id: &str,
    ) -> Result<PagedResults<Deployment>, CrspError> {
        let client = self.0;
        let path = format!("/v1/projects/{script_id}/deployments");
        crate::core::pagination::fetch_pages(
            |page_size: usize, page_token: Option<String>| {
                let url = append_query_if_some(
                    &append_query(
                        &service_url(&client.base_urls().script, &path),
                        "pageSize",
                        &page_size.to_string(),
                    ),
                    "pageToken",
                    page_token.as_deref(),
                );
                async move {
                    let page: DeploymentsPage = client
                        .request(ApiRequest {
                            method: reqwest::Method::GET,
                            url,
                            body: None,
                        })
                        .await?
                        .json()
                        .await?;
                    Ok(Page {
                        results: page.deployments.unwrap_or_default(),
                        page_token: page.next_page_token,
                    })
                }
            },
            PageOptions::default(),
        )
        .await
    }

    /// POST `{script}/v1/scripts/{scriptId}/run` (clasp `runFunction`); the
    /// response is genuinely dynamic, so it is returned as raw JSON.
    pub async fn run(
        &self,
        script_id: &str,
        function: &str,
        parameters: Vec<Value>,
        dev_mode: bool,
    ) -> Result<Value, CrspError> {
        let url = service_url(
            &self.0.base_urls().script,
            &format!("/v1/scripts/{script_id}/run"),
        );
        let body = RunRequest {
            function: function.to_string(),
            parameters,
            dev_mode,
        };
        self.0
            .request(ApiRequest {
                method: reqwest::Method::POST,
                url,
                body: json_body(&body)?,
            })
            .await?
            .json_value()
            .await
    }
}
