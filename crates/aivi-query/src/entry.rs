use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    discover_workspace_root, discover_workspace_root_from_directory, manifest::parse_manifest,
    manifest::AppConfig,
};

/// How the entrypoint path was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntrypointOrigin {
    ExplicitPath,
    ManifestEntry,
    ImplicitWorkspaceMain,
}

/// A v1 entrypoint selection paired with the workspace root it should compile
/// against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedEntrypoint {
    entry_path: PathBuf,
    workspace_root: PathBuf,
    origin: EntrypointOrigin,
    manifest_view: Option<String>,
}

impl ResolvedEntrypoint {
    fn new(
        entry_path: PathBuf,
        workspace_root: PathBuf,
        origin: EntrypointOrigin,
        manifest_view: Option<String>,
    ) -> Self {
        Self {
            entry_path,
            workspace_root,
            origin,
            manifest_view,
        }
    }

    pub fn entry_path(&self) -> &Path {
        &self.entry_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn origin(&self) -> EntrypointOrigin {
        self.origin
    }

    /// Default view name from `aivi.toml` `[run] view`, if any.
    pub fn manifest_view(&self) -> Option<&str> {
        self.manifest_view.as_deref()
    }
}

/// v1 entry discovery can only fail when the implicit `<workspace-root>/main.aivi`
/// target is absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntrypointResolutionError {
    MissingImplicitEntrypoint {
        workspace_root: PathBuf,
        expected_path: PathBuf,
    },
    ManifestEntryNotFound {
        workspace_root: PathBuf,
        manifest_entry: String,
        resolved_path: PathBuf,
    },
    ManifestParseError {
        message: String,
    },
    /// `[[app]]` array has more than one entry and no `--app` was given.
    AmbiguousApp {
        workspace_root: PathBuf,
        apps: Vec<AppConfig>,
    },
    /// `--app <name>` was given but no matching `[[app]]` entry exists.
    UnknownApp {
        workspace_root: PathBuf,
        requested: String,
        available: Vec<String>,
    },
}

impl EntrypointResolutionError {
    pub fn workspace_root(&self) -> &Path {
        match self {
            Self::MissingImplicitEntrypoint { workspace_root, .. }
            | Self::ManifestEntryNotFound { workspace_root, .. }
            | Self::AmbiguousApp { workspace_root, .. }
            | Self::UnknownApp { workspace_root, .. } => workspace_root,
            Self::ManifestParseError { .. } => Path::new("."),
        }
    }

    pub fn expected_path(&self) -> &Path {
        match self {
            Self::MissingImplicitEntrypoint { expected_path, .. } => expected_path,
            Self::ManifestEntryNotFound { resolved_path, .. } => resolved_path,
            Self::AmbiguousApp { .. }
            | Self::UnknownApp { .. }
            | Self::ManifestParseError { .. } => Path::new("."),
        }
    }
}

impl fmt::Display for EntrypointResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingImplicitEntrypoint { expected_path, .. } => write!(
                f,
                "expected implicit entry file at {}; \
                 pass `--path <entry-file>` or set `[run] entry` in aivi.toml",
                expected_path.display()
            ),
            Self::ManifestEntryNotFound {
                manifest_entry,
                resolved_path,
                ..
            } => write!(
                f,
                "`[run] entry = \"{}\"` in aivi.toml resolves to {}, which does not exist",
                manifest_entry,
                resolved_path.display()
            ),
            Self::ManifestParseError { message } => write!(f, "{message}"),
            Self::AmbiguousApp { apps, .. } => {
                writeln!(
                    f,
                    "multiple apps are defined in aivi.toml; use --app <name> to select one:"
                )?;
                let name_width = apps.iter().map(|a| a.name.len()).max().unwrap_or(0);
                for app in apps {
                    match &app.description {
                        Some(desc) => writeln!(
                            f,
                            "  {:<width$}  {}",
                            app.name,
                            desc,
                            width = name_width
                        )?,
                        None => writeln!(f, "  {}", app.name)?,
                    }
                }
                Ok(())
            }
            Self::UnknownApp {
                requested,
                available,
                ..
            } => write!(
                f,
                "no app named `{requested}` in aivi.toml; available: {}",
                available.join(", ")
            ),
        }
    }
}

impl Error for EntrypointResolutionError {}

/// Resolve the v1 entrypoint contract for tooling that starts from a current
/// working directory and an optional explicit `--path` override.
///
/// Resolution order:
/// 1. Explicit CLI path (`--path` or positional argument)
/// 2. `--app <name>` matched against `[[app]]` entries in `aivi.toml`
/// 3. Single `[[app]]` entry (auto-selected when only one app is defined)
/// 4. `[run] entry` from `aivi.toml` in the workspace root
/// 5. Implicit `<workspace-root>/main.aivi`
pub fn resolve_v1_entrypoint(
    current_dir: &Path,
    explicit_path: Option<&Path>,
    app_name: Option<&str>,
) -> Result<ResolvedEntrypoint, EntrypointResolutionError> {
    if let Some(explicit_path) = explicit_path {
        let workspace_root = discover_workspace_root(explicit_path);
        let manifest_view = parse_manifest(&workspace_root)
            .ok()
            .and_then(|m| m.run.view);
        return Ok(ResolvedEntrypoint::new(
            explicit_path.to_path_buf(),
            workspace_root,
            EntrypointOrigin::ExplicitPath,
            manifest_view,
        ));
    }

    let workspace_root = discover_workspace_root_from_directory(current_dir);

    let manifest = parse_manifest(&workspace_root)
        .map_err(|message| EntrypointResolutionError::ManifestParseError { message })?;

    // [[app]] resolution: named or auto-select when exactly one entry exists.
    if !manifest.apps.is_empty() {
        let app = if let Some(name) = app_name {
            manifest
                .apps
                .iter()
                .find(|a| a.name == name)
                .ok_or_else(|| EntrypointResolutionError::UnknownApp {
                    workspace_root: workspace_root.clone(),
                    requested: name.to_owned(),
                    available: manifest.apps.iter().map(|a| a.name.clone()).collect(),
                })?
        } else if manifest.apps.len() == 1 {
            &manifest.apps[0]
        } else {
            return Err(EntrypointResolutionError::AmbiguousApp {
                workspace_root,
                apps: manifest.apps,
            });
        };

        let entry_path = workspace_root.join(&app.entry);
        if !entry_path.is_file() {
            return Err(EntrypointResolutionError::ManifestEntryNotFound {
                workspace_root,
                manifest_entry: app.entry.clone(),
                resolved_path: entry_path,
            });
        }
        let manifest_view = app.view.clone().or(manifest.run.view);
        return Ok(ResolvedEntrypoint::new(
            entry_path,
            workspace_root,
            EntrypointOrigin::ManifestEntry,
            manifest_view,
        ));
    }

    if let Some(manifest_entry) = &manifest.run.entry {
        let entry_path = workspace_root.join(manifest_entry);
        if !entry_path.is_file() {
            return Err(EntrypointResolutionError::ManifestEntryNotFound {
                workspace_root,
                manifest_entry: manifest_entry.clone(),
                resolved_path: entry_path,
            });
        }
        return Ok(ResolvedEntrypoint::new(
            entry_path,
            workspace_root,
            EntrypointOrigin::ManifestEntry,
            manifest.run.view,
        ));
    }

    let entry_path = workspace_root.join("main.aivi");
    if !entry_path.is_file() {
        return Err(EntrypointResolutionError::MissingImplicitEntrypoint {
            workspace_root,
            expected_path: entry_path,
        });
    }

    Ok(ResolvedEntrypoint::new(
        entry_path,
        workspace_root,
        EntrypointOrigin::ImplicitWorkspaceMain,
        manifest.run.view,
    ))
}
