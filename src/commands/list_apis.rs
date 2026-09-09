//! `list-apis` (alias `apis`) — clasp commands/list-apis.ts.
//!
//! Enabled services come from Service Usage (`state:ENABLED`, page size
//! 200 / cap 10000) and available services from Discovery
//! (`?preferred=true`); both are filtered to the known advanced-service ids
//! and printed as padded rows or the `{enabledApis, availableApis}` JSON.

use std::io::Write;

use serde::Serialize;

use crate::api::ApiClient;
use crate::commands::shared::{maybe_prompt_for_project_id, pad_end};
use crate::core::apis::PUBLIC_ADVANCED_SERVICES;
use crate::core::config::ProjectConfig;
use crate::core::services::assert_gcp_project_configured;
use crate::error::CrspError;
use crate::i18n;
use crate::output::Output;
use crate::ui::{PromptAdapter, Ui};

/// A list-apis service entry (clasp `Service`: id, name, description).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceEntry {
    pub name: String,
    pub description: String,
}

#[derive(Serialize)]
struct ServiceJson<'a> {
    name: &'a str,
    description: &'a str,
}

#[derive(Serialize)]
struct ListApisJson<'a> {
    #[serde(rename = "enabledApis")]
    enabled_apis: Vec<ServiceJson<'a>>,
    #[serde(rename = "availableApis")]
    available_apis: Vec<ServiceJson<'a>>,
}

fn to_service_json(services: &[ServiceEntry]) -> Vec<ServiceJson<'_>> {
    services
        .iter()
        .map(|service| ServiceJson {
            name: &service.name,
            description: &service.description,
        })
        .collect()
}

/// clasp `truncateName` (services.ts:118-126): the text before the first dot.
fn truncate_name(name: &str) -> &str {
    match name.find('.') {
        Some(index) => &name[..index],
        None => name,
    }
}

/// clasp `getEnabledServices` (services.ts:60-135): maps the Service Usage
/// list to `{id, name, description}` and keeps only known advanced services.
/// The core `getEnabledServices` asserts script configuration first (clasp
/// `assertGcpProjectConfigured` runs `assertScriptConfigured`).
pub(crate) async fn get_enabled_services(
    client: &ApiClient,
    config: &ProjectConfig,
) -> Result<Vec<ServiceEntry>, CrspError> {
    assert_gcp_project_configured(config)?;
    let project_id = config.project_id.as_deref().unwrap_or_default();
    let services = client
        .service_usage()
        .list_enabled(project_id)
        .await?
        .results;
    let allowed: Vec<&str> = PUBLIC_ADVANCED_SERVICES
        .iter()
        .map(|service| service.service_id)
        .collect();
    Ok(services
        .iter()
        .map(|service| ServiceEntry {
            name: truncate_name(
                service
                    .config
                    .as_ref()
                    .and_then(|config| config.name.as_deref())
                    .unwrap_or("Unknown name"),
            )
            .to_string(),
            description: service.summary().to_string(),
        })
        .filter(|service| allowed.contains(&service.name.as_str()))
        .collect())
}

/// clasp `getAvailableServices` (services.ts:144-186): Discovery items with
/// id, name, and description, filtered to known advanced services and sorted
/// by id (`localeCompare`).
pub(crate) async fn get_available_services(
    client: &ApiClient,
) -> Result<Vec<ServiceEntry>, CrspError> {
    let items = client.discovery().list_apis().await?;
    let allowed: Vec<&str> = PUBLIC_ADVANCED_SERVICES
        .iter()
        .map(|service| service.service_id)
        .collect();
    let mut services: Vec<(String, ServiceEntry)> = items
        .iter()
        .filter_map(|item| {
            let id = item.id.as_deref()?;
            let name = item.name.as_deref()?;
            let description = item.description.as_deref()?;
            if !allowed.contains(&name) {
                return None;
            }
            Some((
                id.to_string(),
                ServiceEntry {
                    name: name.to_string(),
                    description: description.to_string(),
                },
            ))
        })
        .collect();
    services.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(services.into_iter().map(|(_, service)| service).collect())
}

pub async fn list_apis<A: PromptAdapter, O: crate::commands::shared::UrlOpener>(
    client: &ApiClient,
    config: &mut ProjectConfig,
    ui: &Ui<A>,
    opener: &O,
    output: &mut Output<impl Write, impl Write>,
) -> Result<(), CrspError> {
    maybe_prompt_for_project_id(config, ui, opener, output).await?;
    crate::commands::shared::assert_gcp_project_configured(config)?;

    let (enabled_apis, available_apis) = tokio::try_join!(
        get_enabled_services(client, config),
        get_available_services(client)
    )?;

    if output.is_json() {
        output.print_json(&ListApisJson {
            enabled_apis: to_service_json(&enabled_apis),
            available_apis: to_service_json(&available_apis),
        })?;
        return Ok(());
    }

    output.message(&format!("\n{}", i18n::ENABLED_APIS_LABEL));
    for service in &enabled_apis {
        output.message(&format!(
            "{} - {}",
            pad_end(&service.name, 25),
            pad_end(&service.description, 60)
        ));
    }
    output.message(&format!("\n{}", i18n::AVAILABLE_APIS_LABEL));
    for service in &available_apis {
        output.message(&format!(
            "{} - {}",
            pad_end(&service.name, 25),
            pad_end(&service.description, 60)
        ));
    }
    Ok(())
}
