//! Project configuration (`.clasp.json`, spec §2.3): JSON5 discovery/read,
//! contentDir resolution with jail checks, and clasp-compatible
//! `updateSettings` writes (2-space JSON, ordered keys, omitted undefined
//! values, recomputed `rootDir`, reset `filePushOrder`).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::constants::{PROJECT_CONFIG_FILENAME, PROJECT_IGNORE_FILENAME};
use crate::core::ignore::IgnoreMatcher;
use crate::core::path::{PathJail, normalize_lexical, normalize_slashes, relative_path};
use crate::error::CrspError;
use crate::i18n;

/// Defaults from clasp `readFileExtensions` (fixed-up form: lowercase with a
/// leading dot).
const DEFAULT_SCRIPT_EXTENSIONS: [&str; 2] = [".js", ".gs"];
const DEFAULT_HTML_EXTENSIONS: [&str; 1] = [".html"];
const DEFAULT_JSON_EXTENSIONS: [&str; 1] = [".json"];

/// The loaded `.clasp.json` project configuration plus the directories clasp
/// derives from it (spec §8): the project root is the directory containing
/// the config file, and the content dir is `srcDir|rootDir` resolved inside
/// it (empty string = unset → project root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Path of the `.clasp.json` file (real or, for default instances, the
    /// path it would be created at).
    pub config_file_path: PathBuf,
    /// The directory containing `.clasp.json` (clasp `projectRootDir`).
    pub project_root_dir: PathBuf,
    /// The content directory (clasp `contentDir`), resolved against the
    /// project root and verified to stay inside it.
    pub content_dir: PathBuf,
    pub script_id: Option<String>,
    pub project_id: Option<String>,
    /// `parentId` array values use the first element (clasp `firstValue`).
    pub parent_id: Option<String>,
    pub file_push_order: Vec<String>,
    /// Fixed-up extensions (lowercase, leading dot).
    pub script_extensions: Vec<String>,
    pub html_extensions: Vec<String>,
    pub json_extensions: Vec<String>,
    pub skip_subdirectories: bool,
    pub allow_symlinks: bool,
    /// `.claspignore` found at the project root (filled by [`discover`];
    /// `None` means the default patterns apply).
    pub ignore_file_path: Option<PathBuf>,
}

impl ProjectConfig {
    /// Finds `.clasp.json` by explicit `-P` path (file or directory) or via
    /// find-up from `cwd` (clasp `initClaspInstance` + `findProjectRootDir`).
    /// A missing explicit path is a hard error; when no config is found the
    /// default instance is rooted at `cwd` (clasp behavior).
    pub async fn discover(project_arg: Option<&Path>, cwd: &Path) -> Result<Self, CrspError> {
        let config_path = match project_arg {
            Some(arg) => match tokio::fs::metadata(arg).await {
                Ok(metadata) if metadata.is_dir() => arg.join(PROJECT_CONFIG_FILENAME),
                Ok(_) => arg.to_path_buf(),
                Err(error) if is_path_not_found(&error) => {
                    return Err(CrspError::Config(i18n::invalid_project_path(
                        &arg.to_string_lossy(),
                    )));
                }
                Err(error) => return Err(error.into()),
            },
            None => match find_up(cwd, PROJECT_CONFIG_FILENAME).await {
                Some(found) => found,
                None => return Ok(Self::default_instance(cwd).await),
            },
        };

        if !is_readable(&config_path).await {
            // clasp: no readable .clasp.json -> default instance rooted at cwd.
            return Ok(Self::default_instance(cwd).await);
        }

        let mut config = Self::load(&config_path).await?;
        config.ignore_file_path = find_ignore_file(&config.project_root_dir).await;
        Ok(config)
    }

    /// Loads and parses a `.clasp.json` file, applying clasp's defaults and
    /// the contentDir jail (spec §2.3, §4 rule 1; clasp.ts:191-241).
    pub async fn load(path: &Path) -> Result<Self, CrspError> {
        let content = tokio::fs::read_to_string(path).await?;
        let value: Value =
            json5::from_str(&content).map_err(|error| CrspError::Config(error.to_string()))?;

        // clasp uses `path.dirname(configFilePath)`; a relative path with an
        // empty parent resolves against the process cwd (Node semantics).
        let raw_parent = path.parent().unwrap_or_else(|| Path::new("."));
        let project_root_dir =
            match raw_parent.as_os_str().is_empty() || raw_parent == Path::new(".") {
                true => normalize_lexical(&std::env::current_dir().map_err(CrspError::Io)?),
                false => normalize_lexical(raw_parent),
            };
        let config_file_path = normalize_lexical(path);

        let script_id = optional_string(&value, "scriptId");
        let project_id = optional_string(&value, "projectId");
        let parent_id = parent_id_of(&value);
        let src_dir = optional_string(&value, "srcDir");
        let root_dir = optional_string(&value, "rootDir");

        // Content directory: `config.srcDir || config.rootDir || '.'`
        // (clasp.ts:200) — empty strings are falsy and fall through.
        let raw_src_dir = non_empty(&src_dir)
            .or_else(|| non_empty(&root_dir))
            .unwrap_or(".");
        let content_dir = PathJail::resolve_lexical(&project_root_dir, raw_src_dir);

        let allow_symlinks = truthy(value.get("allowSymlinks"));
        let skip_subdirectories = truthy(value.get("skipSubdirectories"));

        // SECURITY: strict validation — the resolved content dir must be the
        // root or inside it, both lexically and physically via realpath
        // (skipped when symlinks are allowed; clasp.ts:206-221).
        // The physical check runs only when BOTH paths canonicalize. On
        // Windows `canonicalize` returns a `\\?\` verbatim path and fails for a
        // content dir that does not exist yet (e.g. a clone target); mixing a
        // canonical root with a lexical content dir would false-positive an
        // escape, so the lexical check governs that case.
        let lexical_ok =
            content_dir == project_root_dir || PathJail::is_inside(&project_root_dir, &content_dir);
        let real_ok = allow_symlinks
            || match (
                tokio::fs::canonicalize(&project_root_dir).await,
                tokio::fs::canonicalize(&content_dir).await,
            ) {
                (Ok(root_real), Ok(content_real)) => {
                    content_real == root_real || PathJail::is_inside(&root_real, &content_real)
                }
                _ => true,
            };
        if !(lexical_ok && real_ok) {
            // Display follows clasp's `config.srcDir ?? config.rootDir`:
            // an empty-string srcDir still wins over rootDir.
            let raw_display = src_dir
                .as_ref()
                .or(root_dir.as_ref())
                .map(String::as_str)
                .unwrap_or("undefined");
            return Err(CrspError::Config(i18n::src_dir_escapes_project_root(
                raw_display,
                &normalize_slashes(&content_dir.to_string_lossy()),
                &normalize_slashes(&project_root_dir.to_string_lossy()),
            )));
        }

        let file_push_order = value
            .get("filePushOrder")
            .filter(|value| truthy(Some(value)))
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let (script_extensions, html_extensions, json_extensions) = file_extensions(&value);

        Ok(Self {
            config_file_path,
            project_root_dir,
            content_dir,
            script_id,
            project_id,
            parent_id,
            file_push_order,
            script_extensions,
            html_extensions,
            json_extensions,
            skip_subdirectories,
            allow_symlinks,
            ignore_file_path: None,
        })
    }

    /// Writes the project settings (clasp `updateSettings`, project.ts:443-460):
    /// 2-space JSON with clasp's key order, `rootDir` recomputed as
    /// `path.relative(projectRootDir, contentDir)`, `filePushOrder` reset to
    /// `[]`, and keys with undefined values (`parentId`/`projectId`) omitted.
    pub async fn update_settings(&self) -> Result<(), CrspError> {
        let script_id = self
            .script_id
            .as_deref()
            .ok_or_else(|| CrspError::Config(i18n::PROJECT_SETTINGS_NOT_FOUND.to_string()))?;

        let settings = SettingsWrite {
            script_id,
            root_dir: relative_path(&self.project_root_dir, &self.content_dir),
            parent_id: self.parent_id.as_deref(),
            project_id: self.project_id.as_deref(),
            script_extensions: &self.script_extensions,
            html_extensions: &self.html_extensions,
            json_extensions: &self.json_extensions,
            file_push_order: &[],
            skip_subdirectories: self.skip_subdirectories,
        };
        let json = serde_json::to_string_pretty(&settings)
            .map_err(|error| CrspError::Config(error.to_string()))?;
        tokio::fs::write(&self.config_file_path, json).await?;
        Ok(())
    }

    /// Builds the ignore matcher from the discovered ignore file or the
    /// default patterns (clasp `loadIgnoreFileOrDefaults`).
    pub async fn ignore_matcher(&self) -> Result<IgnoreMatcher, CrspError> {
        match &self.ignore_file_path {
            Some(path) => IgnoreMatcher::from_file(path).await,
            None => IgnoreMatcher::from_patterns(IgnoreMatcher::default_patterns()),
        }
    }

    /// The default instance used when no `.clasp.json` is found (clasp
    /// `initClaspInstance`'s no-project branch): rooted at `cwd`, content
    /// dir = root, no script configuration, and a `.claspignore` lookup at
    /// the default location.
    async fn default_instance(cwd: &Path) -> Self {
        let mut config = Self::default_at(cwd);
        config.ignore_file_path = find_ignore_file(&config.project_root_dir).await;
        config
    }

    /// The default instance fields without the ignore-file lookup.
    fn default_at(cwd: &Path) -> Self {
        let root = normalize_lexical(cwd);
        Self {
            config_file_path: root.join(PROJECT_CONFIG_FILENAME),
            project_root_dir: root.clone(),
            content_dir: root.clone(),
            script_id: None,
            project_id: None,
            parent_id: None,
            file_push_order: Vec::new(),
            script_extensions: DEFAULT_SCRIPT_EXTENSIONS
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
            html_extensions: DEFAULT_HTML_EXTENSIONS
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
            json_extensions: DEFAULT_JSON_EXTENSIONS
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
            skip_subdirectories: false,
            allow_symlinks: false,
            ignore_file_path: None,
        }
    }
}

/// The update-settings document. Field declaration order is the serialized
/// key order (matching clasp's object literal); `Option` keys are omitted
/// when unset (`JSON.stringify` undefined-key behavior).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsWrite<'a> {
    script_id: &'a str,
    root_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<&'a str>,
    script_extensions: &'a [String],
    html_extensions: &'a [String],
    json_extensions: &'a [String],
    file_push_order: &'a [String],
    skip_subdirectories: bool,
}

/// Reads an optional string field (non-string JSON values are treated as
/// absent; `.clasp.json` values for these keys are strings in practice).
fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Returns the string when it is present and non-empty (JS truthiness for
/// the `||` fallback chain).
fn non_empty(value: &Option<String>) -> Option<&str> {
    match value {
        Some(string) if !string.is_empty() => Some(string),
        _ => None,
    }
}

/// `parentId` handling (clasp `firstValue`): arrays use the first string
/// element; strings pass through.
fn parent_id_of(value: &Value) -> Option<String> {
    match value.get("parentId") {
        Some(Value::Array(entries)) => entries.first().and_then(Value::as_str).map(str::to_string),
        Some(other) => other.as_str().map(str::to_string),
        None => None,
    }
}

/// JS truthiness for the JSON values clasp reads (`undefined`/`null`/`false`
/// /`""`/`0` are falsy).
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(string)) => !string.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// Extension settings (clasp `readFileExtensions`, clasp.ts:244-277): the
/// legacy singular `fileExtension` seeds the script list, then the plural
/// settings override (when truthy), then all lists are fixed up to lowercase
/// dotted form. An empty-string value is falsy and keeps the current list.
fn file_extensions(value: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut script = DEFAULT_SCRIPT_EXTENSIONS
        .iter()
        .map(|extension| extension.to_string())
        .collect::<Vec<_>>();
    if let Some(legacy) = optional_string(value, "fileExtension").filter(|s| !s.is_empty()) {
        script = vec![legacy];
    }
    if let Some(setting) = value.get("scriptExtensions")
        && let Some(list) = extension_list(setting)
    {
        script = list;
    }
    let html = value
        .get("htmlExtensions")
        .and_then(extension_list)
        .unwrap_or_else(|| {
            DEFAULT_HTML_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let json = value
        .get("jsonExtensions")
        .and_then(extension_list)
        .unwrap_or_else(|| {
            DEFAULT_JSON_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    (
        script.iter().map(|s| fixup_extension(s)).collect(),
        html.iter().map(|s| fixup_extension(s)).collect(),
        json.iter().map(|s| fixup_extension(s)).collect(),
    )
}

/// Interprets one extension setting (clasp `ensureStringArray` + JS
/// truthiness): strings become single-element lists, arrays keep their
/// string elements (possibly empty), and anything else (absent, `null`,
/// empty string) keeps the current default.
fn extension_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Null => None,
        Value::String(string) if string.is_empty() => None,
        Value::String(string) => Some(vec![string.clone()]),
        Value::Array(entries) => Some(
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
        ),
        // Truthy non-string, non-array values yield an empty list.
        _ => Some(Vec::new()),
    }
}

/// clasp `fixupExtension`: lowercase, trim, prepend `.` when missing.
fn fixup_extension(extension: &str) -> String {
    let extension = extension.trim().to_lowercase();
    if let Some(stripped) = extension.strip_prefix('.') {
        format!(".{stripped}")
    } else {
        format!(".{extension}")
    }
}

/// Find-up search for a readable file, starting at `start` and walking to
/// the filesystem root (clasp `findUpSync`).
async fn find_up(start: &Path, file_name: &str) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(file_name);
        if is_readable(&candidate).await {
            return Some(candidate);
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return None,
        }
    }
}

/// The `.claspignore` at a project root, when readable (clasp
/// `findIgnoreFile`'s default location).
async fn find_ignore_file(project_root: &Path) -> Option<PathBuf> {
    let candidate = project_root.join(PROJECT_IGNORE_FILENAME);
    is_readable(&candidate).await.then_some(candidate)
}

/// Read-access probe (clasp `hasReadAccess`).
async fn is_readable(path: &Path) -> bool {
    tokio::fs::File::open(path).await.is_ok()
}

/// ENOENT/ENOTDIR detection (clasp `isPathNotFoundError`).
fn is_path_not_found(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}
