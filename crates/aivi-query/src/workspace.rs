use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{RootDatabase, SourceFile};

/// Deterministic workspace discovery rooted at the closest `aivi.toml` ancestor,
/// or the entry file's parent directory when no manifest exists yet.
#[derive(Clone, Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub(crate) fn discover(db: &RootDatabase, file: SourceFile) -> Self {
        let path = file.path(db);
        Self {
            root: discover_workspace_root(&path),
        }
    }

    pub(crate) fn module_name_for_file(
        &self,
        db: &RootDatabase,
        file: SourceFile,
    ) -> Option<String> {
        module_name_for_path(&self.root, &file.path(db))
    }

    pub(crate) fn resolve_module_file(
        &self,
        db: &RootDatabase,
        module: &[&str],
    ) -> Option<SourceFile> {
        if module.is_empty() {
            return None;
        }

        let mut path = self.root.clone();
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
