use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{RootDatabase, SourceFile};

/// Deterministic workspace discovery rooted at the closest `aivi.toml` ancestor,
/// or the entry file's parent directory when no manifest exists yet.
#[derive(Clone, Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
    bundled_stdlib_root: Option<PathBuf>,
}

impl Workspace {
    pub(crate) fn discover(db: &RootDatabase, file: SourceFile) -> Self {
        let path = file.path(db);
        let root = discover_workspace_root(&path);
        Self {
            bundled_stdlib_root: discover_bundled_stdlib_root()
                .filter(|bundled_root| bundled_root != &root),
            root,
        }
    }

    pub(crate) fn module_name_for_file(
        &self,
        db: &RootDatabase,
        file: SourceFile,
    ) -> Option<String> {
        let path = file.path(db);
        module_name_for_path(&self.root, &path).or_else(|| {
            self.bundled_stdlib_root
                .as_deref()
                .and_then(|root| module_name_for_path(root, &path))
        })
    }

    pub(crate) fn resolve_module_file(
        &self,
        db: &RootDatabase,
        module: &[&str],
    ) -> Option<SourceFile> {
        if module.is_empty() {
            return None;
        }

        // Workspace resolution always takes precedence over the bundled stdlib.
        // If the module path starts with "aivi" and a file exists in both the
        // workspace and the bundled stdlib, the workspace file wins (intentional
        // user override). We emit a warning so the override is never silent.
        //
        // TODO: On case-insensitive filesystems (macOS HFS+, Windows NTFS) a
        // user directory `Aivi/` could collide with `aivi/` in ways this check
        // doesn't catch.  Normalise the first segment to lowercase before
        // comparing if case-insensitive FS support is needed in the future.
        let workspace_file = self.resolve_module_file_in_root(db, &self.root, module);

        if let Some(ref file) = workspace_file {
            if is_bundled_stdlib_module(module) {
                tracing::warn!(
                    "user workspace file {:?} shadows bundled stdlib module {:?}",
                    file.path(db),
                    module,
                );
            }
            return workspace_file;
        }

        if !is_bundled_stdlib_module(module) {
            return None;
        }

        let root = self.bundled_stdlib_root.as_deref()?;
        self.resolve_module_file_in_root(db, root, module)
    }

    fn resolve_module_file_in_root(
        &self,
        db: &RootDatabase,
        root: &Path,
        module: &[&str],
    ) -> Option<SourceFile> {
        let mut path = root.to_path_buf();
        for segment in module {
            path.push(segment);
        }
        path.set_extension("aivi");

        if let Some(file) = db.file_at_path(&path) {
            return Some(file);
        }

        let text = fs::read_to_string(&path).ok()?;
        Some(SourceFile::new(db, path, text))
    }
}

fn discover_workspace_root(path: &Path) -> PathBuf {
    let start = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for ancestor in start.ancestors() {
        if ancestor.join("aivi.toml").is_file() {
            return ancestor.to_path_buf();
        }
    }

    start.to_path_buf()
}

fn module_name_for_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.extension()?.to_str()? != "aivi" {
        return None;
    }

    let mut segments = relative
        .iter()
        .map(|segment| segment.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;

    let file_name = segments.pop()?;
    let stem = Path::new(&file_name).file_stem()?.to_str()?.to_owned();
    segments.push(stem);
    Some(segments.join("."))
}

fn is_bundled_stdlib_module(module: &[&str]) -> bool {
    matches!(module.first(), Some(segment) if *segment == "aivi")
}

fn discover_bundled_stdlib_root() -> Option<PathBuf> {
    static BUNDLED_STDLIB_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

    BUNDLED_STDLIB_ROOT
        .get_or_init(find_bundled_stdlib_root)
        .clone()
}

fn find_bundled_stdlib_root() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")];

    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("stdlib"));
            candidates.push(parent.join("../stdlib"));
        }
    }

    candidates
        .into_iter()
        .find_map(|candidate| canonical_existing_workspace_root(&candidate))
}

fn canonical_existing_workspace_root(path: &Path) -> Option<PathBuf> {
    let manifest = path.join("aivi.toml");
    if !manifest.is_file() {
        return None;
    }

    fs::canonicalize(path)
        .ok()
        .or_else(|| Some(path.to_path_buf()))
}
