//! Advanced-service management (spec §2.5 rows 21-22; clasp
//! `core/services.ts` `enableService`/`disableService`): update the manifest
//! `dependencies.enabledAdvancedServices` (key order preserved), then call
//! Service Usage `:enable`/`:disable` for `{api}.googleapis.com`.

use std::path::Path;

use crate::api::ApiClient;
use crate::core::apis::AdvancedService;
use crate::core::config::ProjectConfig;
use crate::core::manifest::{EnabledAdvancedService, Manifest};
use crate::error::CrspError;
use crate::i18n;

/// The manifest file name used by enable/disable-api.
const MANIFEST_FILE: &str = "appsscript.json";

/// clasp core `assertGcpProjectConfigured` (utils.ts:149-160): the script
/// configuration assert runs first, then the project ID (`Project ID not
/// found.`).
pub fn assert_gcp_project_configured(config: &ProjectConfig) -> Result<(), CrspError> {
    if config.script_id.is_none() {
        return Err(CrspError::Validation(
            i18n::PROJECT_SETTINGS_NOT_FOUND.to_string(),
        ));
    }
    if config
        .project_id
        .as_deref()
        .filter(|project_id| !project_id.is_empty())
        .is_none()
    {
        return Err(CrspError::Validation(
            i18n::PROJECT_ID_NOT_FOUND.to_string(),
        ));
    }
    Ok(())
}

/// clasp `hasReadWriteAccess` (services.ts:333-341).
async fn has_read_write_access(path: &Path) -> bool {
    tokio::fs::File::options()
        .read(true)
        .write(true)
        .open(path)
        .await
        .is_ok()
}

/// Shared prologue of clasp `enableService`/`disableService`: asserts, the
/// service-name check, the manifest access check, and the advanced-service
/// lookup. Returns the resolved service and the manifest document.
async fn prepare(
    config: &ProjectConfig,
    service_name: &str,
) -> Result<(&'static AdvancedService, Manifest), CrspError> {
    assert_gcp_project_configured(config)?;
    if service_name.is_empty() {
        return Err(CrspError::Validation(
            "No service name provided.".to_string(),
        ));
    }
    let manifest_path = config.content_dir.join(MANIFEST_FILE);
    if !has_read_write_access(&manifest_path).await {
        return Err(CrspError::Validation(
            i18n::MANIFEST_FILE_DOES_NOT_EXIST.to_string(),
        ));
    }
    let advanced_service = AdvancedService::by_service_id(service_name).ok_or(
        CrspError::Validation(i18n::SERVICE_NOT_A_VALID_ADVANCED_SERVICE.to_string()),
    )?;
    let manifest = Manifest::read(&manifest_path).await?;
    Ok((advanced_service, manifest))
}

/// clasp `enableService` (services.ts:190-235): add the service to the
/// manifest (deduplicated by `userSymbol`) and write it before the Service
/// Usage enable call — an API failure leaves the manifest updated (clasp
/// documents this inconsistency).
pub async fn enable_service(
    client: &ApiClient,
    config: &ProjectConfig,
    service_name: &str,
) -> Result<(), CrspError> {
    let (advanced_service, mut manifest) = prepare(config, service_name).await?;
    let manifest_path = config.content_dir.join(MANIFEST_FILE);
    manifest.enable_advanced_service(&EnabledAdvancedService {
        user_symbol: advanced_service.user_symbol.to_string(),
        version: advanced_service.version.to_string(),
        service_id: advanced_service.service_id.to_string(),
    })?;
    manifest.write(&manifest_path).await?;

    let project_id = config.project_id.as_deref().unwrap_or_default();
    client
        .service_usage()
        .enable(project_id, service_name)
        .await
}

/// clasp `disableService` (services.ts:243-300): remove the entry by
/// `serviceId` and write the manifest only when the
/// `enabledAdvancedServices` structure exists (clasp skips the manifest
/// write otherwise), then call Service Usage disable.
pub async fn disable_service(
    client: &ApiClient,
    config: &ProjectConfig,
    service_name: &str,
) -> Result<(), CrspError> {
    let (_advanced_service, mut manifest) = prepare(config, service_name).await?;
    let manifest_path = config.content_dir.join(MANIFEST_FILE);
    let has_structure = manifest
        .as_value()
        .get("dependencies")
        .and_then(|dependencies| dependencies.get("enabledAdvancedServices"))
        .is_some();
    if has_structure {
        manifest.disable_advanced_service(service_name)?;
        manifest.write(&manifest_path).await?;
    }

    let project_id = config.project_id.as_deref().unwrap_or_default();
    client
        .service_usage()
        .disable(project_id, service_name)
        .await
}
