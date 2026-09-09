use std::path::Path;

use crate::core::config::ProjectConfig;
use crate::core::files::{CollectLocalFilesResult, LocalFile, collect_local_files};
use crate::error::CrspError;

pub async fn assert_script_configured(config: &ProjectConfig) -> Result<&str, CrspError> {
    config
        .script_id
        .as_deref()
        .ok_or_else(|| CrspError::Config(crate::i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))
}

pub async fn collect_for_status(
    config: &ProjectConfig,
) -> Result<CollectLocalFilesResult, CrspError> {
    collect_local_files(config).await
}

pub fn tracked_names(files: &[LocalFile]) -> Vec<String> {
    files.iter().map(|file| file.local_path.clone()).collect()
}

pub fn project_path(config: &ProjectConfig, relative: &str) -> std::path::PathBuf {
    config.content_dir.join(Path::new(relative))
}
