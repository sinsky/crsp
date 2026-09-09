//! `appsscript.json` manifest handling (spec §2.3): ordered JSON read/write
//! (2-space output, key order preserved — clasp `JSON.parse` +
//! `JSON.stringify(manifest, null, 2)`) plus the
//! `dependencies.enabledAdvancedServices` mutations used by enable/disable-api.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::error::CrspError;

/// An enabled Google Advanced Service entry (clasp `AdvancedService`). The
/// serialized key order mirrors clasp's `PUBLIC_ADVANCED_SERVICES` data:
/// `userSymbol`, `version`, `serviceId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledAdvancedService {
    pub user_symbol: String,
    pub version: String,
    pub service_id: String,
}

/// An ordered `appsscript.json` document. Key order is preserved on
/// round-trip (`serde_json` with `preserve_order`, spec §3.1: ordered maps
/// are local to the manifest and `.clasp.json`).
#[derive(Debug, Clone)]
pub struct Manifest {
    root: Value,
}

impl Manifest {
    /// Parses a manifest document (strict JSON, clasp `JSON.parse`).
    pub fn parse(input: &str) -> Result<Self, CrspError> {
        serde_json::from_str(input)
            .map(|root| Self { root })
            .map_err(|error| CrspError::Config(error.to_string()))
    }

    /// Reads `appsscript.json` from a path.
    pub async fn read(path: &Path) -> Result<Self, CrspError> {
        let content = tokio::fs::read_to_string(path).await?;
        Self::parse(&content)
    }

    /// Serializes as clasp's `JSON.stringify(manifest, null, 2)` (2-space
    /// indent, no trailing newline).
    pub fn to_json_pretty(&self) -> Result<String, CrspError> {
        serde_json::to_string_pretty(&self.root)
            .map_err(|error| CrspError::Config(error.to_string()))
    }

    /// Writes the manifest document to a path.
    pub async fn write(&self, path: &Path) -> Result<(), CrspError> {
        tokio::fs::write(path, self.to_json_pretty()?).await?;
        Ok(())
    }

    /// The underlying ordered document (for typed readers).
    pub fn as_value(&self) -> &Value {
        &self.root
    }

    /// The currently enabled advanced services, skipping malformed entries.
    pub fn enabled_advanced_services(&self) -> Vec<EnabledAdvancedService> {
        self.root
            .get("dependencies")
            .and_then(|dependencies| dependencies.get("enabledAdvancedServices"))
            .and_then(Value::as_array)
            .map(|services| {
                services
                    .iter()
                    .filter_map(|service| {
                        Some(EnabledAdvancedService {
                            user_symbol: string_field(service, "userSymbol")?,
                            version: string_field(service, "version").unwrap_or_default(),
                            service_id: string_field(service, "serviceId")?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Adds a service to `dependencies.enabledAdvancedServices` unless one
    /// with the same `userSymbol` is already present (clasp
    /// `enableService`, `services.ts:200-214`). Returns whether the
    /// document changed; callers decide when to persist. Non-object
    /// documents fail like clasp's assignment `TypeError` would.
    pub fn enable_advanced_service(
        &mut self,
        service: &EnabledAdvancedService,
    ) -> Result<bool, CrspError> {
        let root = self
            .root
            .as_object_mut()
            .ok_or_else(|| CrspError::Config("Manifest is not an object.".to_string()))?;

        let dependencies = match root.get_mut("dependencies") {
            // Existing object (possibly empty): append the new key after the
            // existing ones, mirroring JS property insertion order.
            Some(dependencies @ Value::Object(_)) => dependencies.as_object_mut().expect("object"),
            // Missing, null, or a non-object value: clasp replaces `null`
            // `dependencies` with the full structure; other non-objects would
            // throw in clasp, crsp replaces them as well.
            _ => {
                let mut dependencies = serde_json::Map::new();
                dependencies.insert(
                    "enabledAdvancedServices".to_string(),
                    Value::Array(vec![service_entry(service)]),
                );
                root.insert("dependencies".to_string(), Value::Object(dependencies));
                return Ok(true);
            }
        };

        let existing = dependencies
            .entry("enabledAdvancedServices".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = existing.as_array_mut() else {
            return Err(CrspError::Config(
                "Manifest dependencies.enabledAdvancedServices is not an array.".to_string(),
            ));
        };
        if list.iter().any(|entry| {
            entry.get("userSymbol").and_then(Value::as_str) == Some(service.user_symbol.as_str())
        }) {
            return Ok(false);
        }
        list.push(service_entry(service));
        Ok(true)
    }

    /// Removes a service with the given `serviceId` from
    /// `dependencies.enabledAdvancedServices` (clasp `disableService`,
    /// `services.ts:278-285`). Returns whether the document changed. When
    /// the structure is missing, nothing changes (clasp skips the manifest
    /// write in that case); a non-array `enabledAdvancedServices` fails
    /// like clasp's `filter` `TypeError` would.
    pub fn disable_advanced_service(&mut self, service_id: &str) -> Result<bool, CrspError> {
        let Some(list) = self
            .root
            .get_mut("dependencies")
            .and_then(|dependencies| dependencies.get_mut("enabledAdvancedServices"))
            .and_then(Value::as_array_mut)
        else {
            let corrupted = self
                .root
                .get("dependencies")
                .and_then(|dependencies| dependencies.get("enabledAdvancedServices"))
                .is_some();
            if corrupted {
                return Err(CrspError::Config(
                    "Manifest dependencies.enabledAdvancedServices is not an array.".to_string(),
                ));
            }
            return Ok(false);
        };
        let original = list.len();
        list.retain(|entry| entry.get("serviceId").and_then(Value::as_str) != Some(service_id));
        Ok(list.len() != original)
    }
}

/// Reads a string field from a manifest entry.
fn string_field(service: &Value, key: &str) -> Option<String> {
    service.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Serializes a service entry preserving clasp's key order
/// (`userSymbol`, `version`, `serviceId`).
fn service_entry(service: &EnabledAdvancedService) -> Value {
    serde_json::to_value(service).expect("service entry is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_entry_serializes_in_clasp_key_order() {
        let service = EnabledAdvancedService {
            user_symbol: "Drive".to_string(),
            version: "v3".to_string(),
            service_id: "drive".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&service_entry(&service)).unwrap(),
            r#"{"userSymbol":"Drive","version":"v3","serviceId":"drive"}"#
        );
    }
}
