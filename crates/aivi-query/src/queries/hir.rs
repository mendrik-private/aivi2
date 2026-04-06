use std::{cell::RefCell, sync::Arc};

use aivi_base::Diagnostic;
use aivi_hir::{
    ExportedNames, HoistKindFilter, ImportCycle, ImportModuleResolution, ImportResolver,
    LoweringResult, LspSymbol,
    exports, extract_symbols, lower_module_with_resolver,
};
use aivi_syntax::Formatter;

use crate::{RootDatabase, SourceFile, queries::parsed_file, workspace::Workspace};

/// Result of lowering a source file to HIR.
#[derive(Clone, Debug)]
pub struct HirModuleResult {
    revision: u64,
    source: Arc<aivi_base::SourceFile>,
    module: aivi_hir::Module,
    diagnostics: Arc<[Diagnostic]>,
    hir_diagnostics: Arc<[Diagnostic]>,
    symbols: Arc<[LspSymbol]>,
    exported_names: ExportedNames,
}

impl HirModuleResult {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn source(&self) -> &aivi_base::SourceFile {
        self.source.as_ref()
    }

    pub fn source_arc(&self) -> Arc<aivi_base::SourceFile> {
        Arc::clone(&self.source)
    }

    pub fn module(&self) -> &aivi_hir::Module {
        &self.module
    }

    /// Parse + HIR lowering diagnostics for the current file.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn diagnostics_arc(&self) -> Arc<[Diagnostic]> {
        Arc::clone(&self.diagnostics)
    }

    /// HIR lowering diagnostics only, excluding parse diagnostics.
    pub fn hir_diagnostics(&self) -> &[Diagnostic] {
        &self.hir_diagnostics
    }

    pub fn hir_diagnostics_arc(&self) -> Arc<[Diagnostic]> {
        Arc::clone(&self.hir_diagnostics)
    }

    pub fn symbols(&self) -> &[LspSymbol] {
        &self.symbols
    }

    pub fn symbols_arc(&self) -> Arc<[LspSymbol]> {
        Arc::clone(&self.symbols)
    }

    pub fn exported_names(&self) -> &ExportedNames {
        &self.exported_names
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportStackEntry {
    file: SourceFile,
    module_name: String,
}

struct WorkspaceImportResolver<'a> {
    db: &'a RootDatabase,
    workspace: &'a Workspace,
    stack: &'a [ImportStackEntry],
    dependencies: RefCell<Vec<SourceFile>>,
}

impl<'a> WorkspaceImportResolver<'a> {
    fn new(db: &'a RootDatabase, workspace: &'a Workspace, stack: &'a [ImportStackEntry]) -> Self {
        Self {
            db,
            workspace,
            stack,
            dependencies: RefCell::new(Vec::new()),
        }
    }

    fn dependencies(&self) -> Vec<SourceFile> {
        let mut deps = self.dependencies.borrow().clone();
        deps.sort_by_key(|file| file.id);
        deps.dedup_by_key(|file| file.id);
        deps
    }

    fn record_dependency(&self, file: SourceFile) {
        let mut deps = self.dependencies.borrow_mut();
        if !deps.iter().any(|existing| existing.id == file.id) {
            deps.push(file);
        }
    }

    fn cycle(&self, file: SourceFile, requested_module: &[&str]) -> Option<ImportCycle> {
        let cycle_start = self.stack.iter().position(|entry| entry.file == file)?;
        let mut modules = self.stack[cycle_start..]
            .iter()
            .map(|entry| entry.module_name.clone())
            .collect::<Vec<_>>();
        modules.push(requested_module.join("."));
        Some(ImportCycle::new(modules))
    }
}

impl ImportResolver for WorkspaceImportResolver<'_> {
    fn resolve(&self, path: &[&str]) -> ImportModuleResolution {
        let Some(file) = self.workspace.resolve_module_file(self.db, path) else {
            return ImportModuleResolution::Missing;
        };
        self.record_dependency(file);

        if let Some(cycle) = self.cycle(file, path) {
            return ImportModuleResolution::Cycle(cycle);
        }

        // Propagate the current workspace to sub-module lowering so that
        // transitive imports (e.g. bundled stdlib files) use the user project
        // workspace for both module resolution and hoist collection.  Without
        // this, a bundled stdlib sub-module would rediscover the stdlib dir as
        // its own workspace root and walk all stdlib files as "project files",
        // incorrectly loading files that should remain absent from the db (such
        // as bundledsmoketest.aivi when a workspace override exists).
        let lowered = hir_module_with_stack(self.db, file, self.stack, Some(self.workspace));
        ImportModuleResolution::Resolved(lowered.exported_names().clone())
    }

    fn resolve_for_hoist(&self, path: &[&str]) -> ImportModuleResolution {
        let Some(file) = self.workspace.resolve_module_file(self.db, path) else {
            return ImportModuleResolution::Missing;
        };
        self.record_dependency(file);

        // Detect cycles using the full stack to prevent infinite recursion, but
        // return Missing (silent skip) instead of Cycle. This prevents the
        // false-positive import-cycle errors that arise when hoist-resolution
        // chains cause stdlib modules to appear in each other's compile stacks
        // even though no real circular dependency exists at the import level.
        if self.cycle(file, path).is_some() {
            return ImportModuleResolution::Missing;
        }

        let lowered = hir_module_with_stack(self.db, file, self.stack, Some(self.workspace));
        ImportModuleResolution::Resolved(lowered.exported_names().clone())
    }

    fn workspace_hoist_items(&self) -> Vec<aivi_hir::resolver::RawHoistItem> {
        collect_workspace_hoist_items(self.db, self.workspace)
    }

    fn current_module_path(&self) -> Option<String> {
        self.stack.last().map(|entry| entry.module_name.clone())
    }
}

/// Lower the given source file to HIR and memoise the result by file revision.
pub fn hir_module(db: &RootDatabase, file: SourceFile) -> Arc<HirModuleResult> {
    hir_module_with_stack(db, file, &[], None)
}

/// `parent_workspace` propagates the calling compilation's workspace to
/// transitive imports.  When `Some`, module resolution and hoist collection
/// use the caller's workspace rather than re-discovering one from `file`.
/// This is essential for bundled stdlib sub-modules, which would otherwise
/// discover the stdlib directory as their own workspace and walk it as a
/// project root, loading unrelated files into the database.
fn hir_module_with_stack(
    db: &RootDatabase,
    file: SourceFile,
    parent_stack: &[ImportStackEntry],
    parent_workspace: Option<&Workspace>,
) -> Arc<HirModuleResult> {
    let owned_workspace;
    let workspace = match parent_workspace {
        Some(w) => w,
        None => {
            owned_workspace = Workspace::discover(db, file);
            &owned_workspace
        }
    };
    let module_name = workspace
        .module_name_for_file(db, file)
        .unwrap_or_else(|| file.path(db).display().to_string());
    let mut stack = parent_stack.to_vec();
    stack.push(ImportStackEntry { file, module_name });

    loop {
        let parsed = parsed_file(db, file);
        if let Some(cached) = db.cached_hir(file, parsed.revision()) {
            return cached;
        }

        let resolver = WorkspaceImportResolver::new(db, workspace, &stack);
        let lowered: LoweringResult = lower_module_with_resolver(parsed.cst(), Some(&resolver));
        let hir_diagnostics = Arc::<[Diagnostic]>::from(lowered.diagnostics().to_vec());
        let mut diagnostics = parsed.diagnostics().to_vec();
        diagnostics.extend_from_slice(lowered.diagnostics());
        db.register_file_deps(file, &resolver.dependencies());

        let module = lowered.into_parts().0;
        let symbols = Arc::<[LspSymbol]>::from(extract_symbols(&module));
        let exported_names = exports(&module);
        let computed = Arc::new(HirModuleResult {
            revision: parsed.revision(),
            source: parsed.source_arc(),
            module,
            diagnostics: Arc::<[Diagnostic]>::from(diagnostics),
            hir_diagnostics,
            symbols,
            exported_names,
        });

        if let Some(current) = db.store_hir(file, computed.revision(), computed) {
            return current;
        }
    }
}

/// Collect all diagnostics (currently parse + HIR) for the given file.
pub fn all_diagnostics(db: &RootDatabase, file: SourceFile) -> Arc<[Diagnostic]> {
    hir_module(db, file).diagnostics_arc()
}

/// Extract LSP symbols from the HIR module.
pub fn symbol_index(db: &RootDatabase, file: SourceFile) -> Arc<[LspSymbol]> {
    hir_module(db, file).symbols_arc()
}

/// Extract exported names from the HIR module.
pub fn exported_names(db: &RootDatabase, file: SourceFile) -> ExportedNames {
    hir_module(db, file).exported_names().clone()
}

/// Resolve a module path from the perspective of an already-known source file.
///
/// This reuses the same workspace-root and bundled-stdlib rules as HIR lowering,
/// opening the target source file on demand when it exists on disk.
pub fn resolve_module_file(
    db: &RootDatabase,
    file: SourceFile,
    module: &[&str],
) -> Option<SourceFile> {
    Workspace::discover(db, file).resolve_module_file(db, module)
}

/// Format the source file using the memoised CST.
///
/// Returns `None` when the file has parse errors so the editor does not
/// receive a format edit that might corrupt the file (e.g. by substituting
/// unparseable regions with placeholder text).
pub fn format_file(db: &RootDatabase, file: SourceFile) -> Option<String> {
    let parsed = parsed_file(db, file);
    if !parsed.diagnostics().is_empty() {
        return None;
    }
    let formatter = Formatter;
    Some(formatter.format(parsed.cst()))
}

/// Collect all `hoist` declarations from every `.aivi` file in the workspace.
///
/// Walks the entire workspace root directory on disk so that `hoist`
/// declarations in files not yet imported (e.g. library modules that nothing
/// has explicitly `use`d yet) are still found.  This lets individual modules
/// declare their own hoists without requiring a hub file that imports them all.
///
/// The bundled stdlib prelude (`aivi.aivi`) is always included.
fn collect_workspace_hoist_items(
    db: &RootDatabase,
    workspace: &Workspace,
) -> Vec<aivi_hir::resolver::RawHoistItem> {
    let mut result = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();

    // Scan every .aivi file in the project directory tree.
    for file in workspace.all_project_files(db) {
        if seen.insert(file.id) {
            if let Some(module_name) = workspace.module_name_for_file(db, file) {
                let parsed = parsed_file(db, file);
                collect_hoists_from_module(parsed.cst(), &module_name, &mut result);
            }
        }
    }

    // Scan every bundled stdlib file so individual stdlib modules can declare
    // `hoist` themselves (e.g. `aivi/list.aivi` declaring `hoist`).
    // all_bundled_stdlib_files() already filters out workspace-overridden files.
    for file in workspace.all_bundled_stdlib_files(db) {
        if seen.insert(file.id) {
            if let Some(module_name) = workspace.module_name_for_file(db, file) {
                let parsed = parsed_file(db, file);
                collect_hoists_from_module(parsed.cst(), &module_name, &mut result);
            }
        }
    }

    result
}

/// Walk a module's CST and collect `hoist` declarations as `RawHoistItem`s.
///
/// `declaring_module` is the dotted module path of the file being scanned
/// (e.g. `"libs.time_util"`). Each `hoist` item in the file publishes the
/// declaring module's own exports, so `declaring_module` becomes the
/// `module_path` in every emitted `RawHoistItem`.
fn collect_hoists_from_module(
    module: &aivi_syntax::cst::Module,
    declaring_module: &str,
    result: &mut Vec<aivi_hir::resolver::RawHoistItem>,
) {
    let module_path: Vec<String> = declaring_module.split('.').map(str::to_owned).collect();
    if module_path.is_empty() {
        return;
    }
    for item in &module.items {
        let aivi_syntax::cst::Item::Hoist(hoist) = item else {
            continue;
        };
        let kind_filters: Vec<HoistKindFilter> = hoist
            .kind_filters
            .iter()
            .filter_map(|kf| match kf.text.as_str() {
                "func" => Some(HoistKindFilter::Func),
                "value" => Some(HoistKindFilter::Value),
                "signal" => Some(HoistKindFilter::Signal),
                "type" => Some(HoistKindFilter::Type),
                "domain" => Some(HoistKindFilter::Domain),
                "class" => Some(HoistKindFilter::Class),
                _ => None,
            })
            .collect();
        let hiding: Vec<String> = hoist
            .hiding
            .iter()
            .map(|h: &aivi_syntax::cst::Identifier| h.text.clone())
            .collect();
        result.push(aivi_hir::resolver::RawHoistItem {
            module_path: module_path.clone(),
            kind_filters,
            hiding,
        });
    }
}
