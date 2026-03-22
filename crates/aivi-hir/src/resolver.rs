use crate::exports::ExportedNames;

/// Resolves an import path to the exported names of the referenced module.
///
/// Implementors inject cross-file resolution into the HIR lowering pipeline
/// without creating a direct dependency on the salsa database.
pub trait ImportResolver {
    /// Resolve a dotted module path (e.g. `["aivi", "network", "http"]`) to the
    /// set of names exported by that module. Returns `None` if the module is not
    /// found or not yet analysed.
    fn resolve(&self, path: &[&str]) -> Option<ExportedNames>;
}

/// A no-op resolver that never resolves any import. Used when cross-file
/// resolution is not available (e.g. single-file analysis or cycle recovery).
pub struct NullImportResolver;

impl ImportResolver for NullImportResolver {
    fn resolve(&self, _path: &[&str]) -> Option<ExportedNames> {
        None
    }
}
