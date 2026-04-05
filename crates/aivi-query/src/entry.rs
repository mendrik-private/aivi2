use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    discover_workspace_root, discover_workspace_root_from_directory, manifest::parse_manifest,
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
}

impl EntrypointResolutionError {
    pub fn workspace_root(&self) -> &Path {
        match self {
            Self::MissingImplicitEntrypoint { workspace_root, .. }
            | Self::ManifestEntryNotFound { workspace_root, .. } => workspace_root,
            Self::ManifestParseError { .. } => Path::new("."),
        }
    }

    pub fn expected_path(&self) -> &Path {
        match self {
            Self::MissingImplicitEntrypoint { expected_path, .. } => expected_path,
            Self::ManifestEntryNotFound { resolved_path, .. } => resolved_path,
            Self::ManifestParseError { .. } => Path::new("."),
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
        }
    }
}

impl Error for EntrypointResolutionError {}

/// Resolve the v1 entrypoint contract for tooling that starts from a current
/// working directory and an optional explicit `--path` override.
///
/// Resolution order:
/// 1. Explicit CLI path (`--path` or positional argument)
/// 2. `[run] entry` from `aivi.toml` in the workspace root
/// 3. Implicit `<workspace-root>/main.aivi`
pub fn resolve_v1_entrypoint(
    current_dir: &Path,
    explicit_path: Option<&Path>,
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
