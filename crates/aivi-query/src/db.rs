use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::RwLock;

use crate::{
    SourceFile,
    queries::{HirModuleResult, ParsedFileResult},
};

#[derive(Clone)]
pub(crate) struct SourceInput {
    pub(crate) revision: u64,
    pub(crate) source: Arc<aivi_base::SourceFile>,
}

impl SourceInput {
    fn new(file: SourceFile, path: PathBuf, text: String, revision: u64) -> Self {
        Self {
            revision,
            source: Arc::new(aivi_base::SourceFile::new(
                aivi_base::FileId::new(file.id),
                path,
                text,
            )),
        }
    }
}

#[derive(Clone)]
struct Cached<T> {
    revision: u64,
    value: Arc<T>,
}

#[derive(Default)]
struct DbState {
    next_id: u32,
    files: HashMap<u32, SourceInput>,
    paths: HashMap<PathBuf, SourceFile>,
    parsed: HashMap<u32, Cached<ParsedFileResult>>,
    hir: HashMap<u32, Cached<HirModuleResult>>,
}

/// Shared query database for tooling features.
pub struct RootDatabase {
    state: RwLock<DbState>,
}

impl Default for RootDatabase {
    fn default() -> Self {
        Self {
            state: RwLock::new(DbState::default()),
        }
    }
}

impl RootDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a file input, reusing the existing handle when the same path is already known.
    pub fn open_file(&self, path: impl Into<PathBuf>, text: impl Into<String>) -> SourceFile {
        let path = path.into();
        let text = text.into();
        let mut state = self.state.write();

        if let Some(file) = state.paths.get(&path).copied() {
            let changed = {
                let input = state
                    .files
                    .get_mut(&file.id)
                    .expect("path index must reference a stored source file");
                if input.source.text() == text {
                    false
                } else {
                    let revision = input.revision + 1;
                    *input = SourceInput::new(file, path.clone(), text, revision);
                    true
                }
            };
            if changed {
                state.parsed.remove(&file.id);
                state.hir.remove(&file.id);
            }
            return file;
        }

        let id = state.next_id;
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("source file table exceeded u32::MAX entries");

        let file = SourceFile { id };
        state
            .files
            .insert(id, SourceInput::new(file, path.clone(), text, 0));
        state.paths.insert(path, file);
        file
    }

    /// Look up a file handle by its path.
    pub fn file_at_path(&self, path: &Path) -> Option<SourceFile> {
        self.state.read().paths.get(path).copied()
    }

    /// Return every file currently known to the database.
    pub fn files(&self) -> Vec<SourceFile> {
        self.state
            .read()
            .files
            .keys()
            .copied()
            .map(|id| SourceFile { id })
            .collect()
    }

    pub(crate) fn set_text(&self, file: SourceFile, text: String) -> bool {
        let mut state = self.state.write();
        let changed = {
            let input = state
                .files
                .get_mut(&file.id)
                .expect("source file handle must refer to a stored input");
            if input.source.text() == text {
                false
            } else {
                let revision = input.revision + 1;
                let path = input.source.path().to_path_buf();
                *input = SourceInput::new(file, path, text, revision);
                true
            }
        };
        if changed {
            state.parsed.remove(&file.id);
            state.hir.remove(&file.id);
        }
        changed
    }

    pub(crate) fn source_input(&self, file: SourceFile) -> SourceInput {
        self.state
            .read()
            .files
            .get(&file.id)
            .cloned()
            .expect("source file handle must refer to a stored input")
    }

    pub(crate) fn cached_parsed(
        &self,
        file: SourceFile,
        revision: u64,
    ) -> Option<Arc<ParsedFileResult>> {
        let state = self.state.read();
        let cached = state.parsed.get(&file.id)?;
        (cached.revision == revision).then(|| Arc::clone(&cached.value))
    }

    pub(crate) fn store_parsed(
        &self,
        file: SourceFile,
        revision: u64,
        computed: Arc<ParsedFileResult>,
    ) -> Option<Arc<ParsedFileResult>> {
        let mut state = self.state.write();
        let current = state
            .files
            .get(&file.id)
            .expect("source file handle must refer to a stored input");
        if current.revision != revision {
            return None;
        }
        if let Some(cached) = state.parsed.get(&file.id) {
            if cached.revision == revision {
                return Some(Arc::clone(&cached.value));
            }
        }
        state.parsed.insert(
            file.id,
            Cached {
                revision,
                value: Arc::clone(&computed),
            },
        );
        Some(computed)
    }

    pub(crate) fn cached_hir(
        &self,
        file: SourceFile,
        revision: u64,
    ) -> Option<Arc<HirModuleResult>> {
        let state = self.state.read();
        let cached = state.hir.get(&file.id)?;
        (cached.revision == revision).then(|| Arc::clone(&cached.value))
    }

    pub(crate) fn store_hir(
        &self,
        file: SourceFile,
        revision: u64,
        computed: Arc<HirModuleResult>,
    ) -> Option<Arc<HirModuleResult>> {
        let mut state = self.state.write();
        let current = state
            .files
            .get(&file.id)
            .expect("source file handle must refer to a stored input");
        if current.revision != revision {
            return None;
        }
        if let Some(cached) = state.hir.get(&file.id) {
            if cached.revision == revision {
                return Some(Arc::clone(&cached.value));
            }
        }
        state.hir.insert(
            file.id,
            Cached {
                revision,
                value: Arc::clone(&computed),
            },
        );
        Some(computed)
    }
}
