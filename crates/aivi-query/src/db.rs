use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rustc_hash::{FxHashMap, FxHashSet};

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

/// Dependency graph kept alongside the query cache.
///
/// `deps[A]` is the set of files that file `A` *imports from* (direct
/// dependencies).  `rdeps[B]` is the set of files that *import* `B`.
///
/// When file `B` changes its text, every entry in `rdeps[B]` (and their own
/// rdeps, recursively) must have their HIR caches invalidated, because those
/// files transitively depend on `B`'s exported names (M6).
#[derive(Default)]
struct FileDeps {
    /// file_id → set of file_ids it directly depends on
    deps: FxHashMap<u32, FxHashSet<u32>>,
    /// file_id → set of file_ids that directly depend on it
    rdeps: FxHashMap<u32, FxHashSet<u32>>,
}

impl FileDeps {
    /// Record that `importer` now depends on exactly the files in `new_deps`,
    /// updating the reverse-dependency index accordingly.
    fn set_deps(&mut self, importer: u32, new_deps: FxHashSet<u32>) {
        // Remove old reverse edges.
        if let Some(old) = self.deps.remove(&importer) {
            for dep in old {
                if let Some(set) = self.rdeps.get_mut(&dep) {
                    set.remove(&importer);
                }
            }
        }
        // Insert new reverse edges.
        for &dep in &new_deps {
            self.rdeps.entry(dep).or_default().insert(importer);
        }
        if !new_deps.is_empty() {
            self.deps.insert(importer, new_deps);
        }
    }

    /// Remove all dependency/reverse-dependency edges for a file that is
    /// being dropped.
    #[allow(dead_code)]
    fn remove_file(&mut self, id: u32) {
        self.set_deps(id, FxHashSet::default());
        self.rdeps.remove(&id);
    }

    /// Collect the transitive closure of all files that (directly or
    /// indirectly) import `changed`.  These files must have their HIR caches
    /// invalidated when `changed` is modified.
    fn transitive_rdeps(&self, changed: u32) -> FxHashSet<u32> {
        let mut visited = FxHashSet::default();
        let mut queue = vec![changed];
        while let Some(current) = queue.pop() {
            if let Some(set) = self.rdeps.get(&current) {
                for &rdep in set {
                    if visited.insert(rdep) {
                        queue.push(rdep);
                    }
                }
            }
        }
        visited
    }
}

#[derive(Default)]
struct DbState {
    next_id: u32,
    files: FxHashMap<u32, SourceInput>,
    paths: FxHashMap<PathBuf, SourceFile>,
    parsed: FxHashMap<u32, Cached<ParsedFileResult>>,
    hir: FxHashMap<u32, Cached<HirModuleResult>>,
    file_deps: FileDeps,
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
                // Transitively invalidate all files that (directly or
                // indirectly) import this file (M6).
                let rdeps = state.file_deps.transitive_rdeps(file.id);
                for rdep in rdeps {
                    state.hir.remove(&rdep);
                }
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
            // Transitively invalidate all files that import this file (M6).
            let rdeps = state.file_deps.transitive_rdeps(file.id);
            for rdep in rdeps {
                state.hir.remove(&rdep);
            }
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

    /// Register the set of files that `importer` directly depends on.
    ///
    /// Call this after a successful HIR compilation for `importer`, passing
    /// the `SourceFile` handles of every file whose exported names the
    /// compiled module references.  Future changes to any of those files will
    /// cause `importer`'s HIR cache entry to be evicted transitively (M6).
    ///
    /// Calling this with an empty `deps` slice removes all previously recorded
    /// dependencies for `importer`.
    pub fn register_file_deps(&self, importer: SourceFile, deps: &[SourceFile]) {
        let dep_ids: FxHashSet<u32> = deps.iter().map(|f| f.id).collect();
        self.state
            .write()
            .file_deps
            .set_deps(importer.id, dep_ids);
    }
}
