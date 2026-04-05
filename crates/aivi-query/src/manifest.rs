use std::{fs, path::Path};

use serde::Deserialize;

/// Parsed representation of an `aivi.toml` workspace manifest.
///
/// All fields are optional — an empty or comment-only `aivi.toml` is still
/// valid and produces `AiviManifest::default()`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AiviManifest {
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub run: RunConfig,
    /// Entries from `[[app]]` arrays, each declaring a named application.
    #[serde(rename = "app", default)]
    pub apps: Vec<AppConfig>,
}

/// Metadata from the `[workspace]` table.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkspaceConfig {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
}

/// Configuration defaults for `aivi run` and `aivi build`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RunConfig {
    /// Default entry file, relative to the workspace root.
    /// Overridden by `--path` or a positional path argument on the CLI.
    pub entry: Option<String>,

    /// Default view name for `aivi run` / `aivi build`.
    /// Overridden by `--view` on the CLI.
    pub view: Option<String>,
}

/// One entry from a `[[app]]` array, declaring a named application target.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AppConfig {
    /// Unique app identifier used with `--app <name>`.
    pub name: String,
    /// Entry file path relative to the workspace root.
    pub entry: String,
    /// Human-readable description shown in disambiguation messages.
    pub description: Option<String>,
    /// Default view name for this app, equivalent to `[run] view`.
    pub view: Option<String>,
}

/// Parse an `aivi.toml` manifest from the given workspace root.
///
/// Returns `AiviManifest::default()` when the file is empty, comment-only, or
/// absent (the caller is expected to verify existence before calling).
pub fn parse_manifest(workspace_root: &Path) -> Result<AiviManifest, String> {
    let manifest_path = workspace_root.join("aivi.toml");
    if !manifest_path.is_file() {
        return Ok(AiviManifest::default());
    }
    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "failed to read `{}`: {error}",
            manifest_path.display()
        )
    })?;
    if content.trim().is_empty() || content.trim().starts_with('#') && !content.contains('[') {
        return Ok(AiviManifest::default());
    }
    toml::from_str(&content).map_err(|error| {
        format!(
            "failed to parse `{}`: {error}",
            manifest_path.display()
        )
    })
}
