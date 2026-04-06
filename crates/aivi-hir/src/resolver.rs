use crate::exports::ExportedNames;
use crate::hir::HoistKindFilter;

/// One explicit import cycle discovered during module resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportCycle {
    modules: Box<[Box<str>]>,
}

impl ImportCycle {
    pub fn new(modules: Vec<String>) -> Self {
        Self {
            modules: modules
                .into_iter()
                .map(|module| module.into_boxed_str())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    pub fn modules(&self) -> &[Box<str>] {
        &self.modules
    }
}

/// Resolution result for one imported module path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportModuleResolution {
    Resolved(ExportedNames),
    Missing,
    Cycle(ImportCycle),
}

/// A hoist declaration extracted from a module's syntax tree, used to
/// propagate workspace-wide hoists without full HIR lowering of the source
/// module.
#[derive(Clone, Debug)]
pub struct RawHoistItem {
    pub module_path: Vec<String>,
    pub kind_filters: Vec<HoistKindFilter>,
    pub hiding: Vec<String>,
}

/// Resolves an import path to the exported names of the referenced module.
///
/// Implementors inject cross-file resolution into the HIR lowering pipeline
/// without creating a direct dependency on the incremental database layer.
pub trait ImportResolver {
    /// Resolve a dotted module path (e.g. `["aivi", "network"]`) to the set of
    /// names exported by that module.
    fn resolve(&self, path: &[&str]) -> ImportModuleResolution;

    /// Resolve a module path for workspace-hoist registration purposes.
    ///
    /// Unlike `resolve`, this method compiles the target module in an isolated
    /// context (fresh import stack) so that hoist-induced transitive resolution
    /// chains do not create false-positive import cycle errors. Real cycles
    /// within the target module are still detected within its own compilation.
    ///
    /// The default implementation delegates to `resolve`.
    fn resolve_for_hoist(&self, path: &[&str]) -> ImportModuleResolution {
        self.resolve(path)
    }

    /// Return all hoist declarations from other modules in the same workspace.
    ///
    /// These are injected into every module's namespace after its own local
    /// `hoist` items are processed, making the hoisted names globally available
    /// across the entire project without per-file `use` imports.
    ///
    /// The default implementation returns an empty list (no workspace hoists).
    fn workspace_hoist_items(&self) -> Vec<RawHoistItem> {
        vec![]
    }

    /// Return the dotted module path of the module currently being compiled
    /// (e.g. `"libs.time_util"`).  Used to skip self-hoists so a module
    /// declaring `hoist libs.time_util` inside itself doesn't create a cycle.
    fn current_module_path(&self) -> Option<String> {
        None
    }
}

/// A no-op resolver that never resolves any import. Used when cross-file
/// resolution is not available (e.g. single-file analysis or cycle recovery).
pub struct NullImportResolver;

impl ImportResolver for NullImportResolver {
    fn resolve(&self, _path: &[&str]) -> ImportModuleResolution {
        ImportModuleResolution::Missing
    }
}
