use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

include!(concat!(env!("OUT_DIR"), "/stdlib_embedded.rs"));

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

    /// Return every `.aivi` file found under the project workspace root.
    ///
    /// Unlike `db.files()`, which only returns files that have already been
    /// imported during the current compilation, this walks the workspace root
    /// directory on disk and loads every `.aivi` file it finds.  This lets the
    /// workspace-wide hoist scanner discover `hoist` declarations in files that
    /// have not yet been explicitly imported by the module being compiled.
    ///
    /// Directories starting with `.` or named `target` are skipped.
    pub(crate) fn all_project_files(&self, db: &RootDatabase) -> Vec<SourceFile> {
        let mut result = Vec::new();
        walk_aivi_files(&self.root, db, &mut result);
        result
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

        // For the bundled stdlib, check the embedded map first to avoid disk I/O.
        if let Some(bundled_root) = &self.bundled_stdlib_root {
            if root == bundled_root.as_path() {
                if let Ok(relative) = path.strip_prefix(bundled_root) {
                    let key = relative.to_str()?.replace('\\', "/");
                    if let Some(text) =
                        STDLIB_EMBEDDED.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
                    {
                        return Some(SourceFile::new(db, path, text.to_owned()));
                    }
                }
            }
        }

        let text = fs::read_to_string(&path).ok()?;
        Some(SourceFile::new(db, path, text))
    }
}

/// Deterministic workspace discovery rooted at the closest `aivi.toml` ancestor,
/// or the provided directory when no manifest exists yet.
pub fn discover_workspace_root_from_directory(path: &Path) -> PathBuf {
    let start = if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path.to_path_buf()
    };

    for ancestor in start.ancestors() {
        if ancestor.join("aivi.toml").is_file() {
            return ancestor.to_path_buf();
        }
    }

    start
}

/// Deterministic workspace discovery rooted at the closest `aivi.toml` ancestor,
/// or the entry file's parent directory when no manifest exists yet.
pub fn discover_workspace_root(path: &Path) -> PathBuf {
    let start = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    discover_workspace_root_from_directory(start)
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

/// Recursively walk `dir` and push every `.aivi` file found into `result`.
/// Skips hidden directories (`.*`) and the `target` directory.
fn walk_aivi_files(dir: &Path, db: &RootDatabase, result: &mut Vec<SourceFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.') || n == "target")
                .unwrap_or(false);
            if !skip {
                walk_aivi_files(&path, db, result);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("aivi") {
            if let Some(file) = db.file_at_path(&path) {
                result.push(file);
            } else if let Ok(text) = fs::read_to_string(&path) {
                result.push(SourceFile::new(db, path, text));
            }
        }
    }
}
