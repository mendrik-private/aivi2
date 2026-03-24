use crate::exports::ExportedNames;

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

/// Resolves an import path to the exported names of the referenced module.
///
/// Implementors inject cross-file resolution into the HIR lowering pipeline
/// without creating a direct dependency on the incremental database layer.
pub trait ImportResolver {
    /// Resolve a dotted module path (e.g. `["aivi", "network"]`) to the set of
    /// names exported by that module.
    fn resolve(&self, path: &[&str]) -> ImportModuleResolution;
}

/// A no-op resolver that never resolves any import. Used when cross-file
/// resolution is not available (e.g. single-file analysis or cycle recovery).
pub struct NullImportResolver;

impl ImportResolver for NullImportResolver {
    fn resolve(&self, _path: &[&str]) -> ImportModuleResolution {
        ImportModuleResolution::Missing
    }
}
