use std::{cell::RefCell, sync::Arc};

use aivi_base::Diagnostic;
use aivi_hir::{
    ExportedNames, ImportCycle, ImportModuleResolution, ImportResolver, LoweringResult, LspSymbol,
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

        let lowered = hir_module_with_stack(self.db, file, self.stack);
        ImportModuleResolution::Resolved(lowered.exported_names().clone())
    }
}

/// Lower the given source file to HIR and memoise the result by file revision.
pub fn hir_module(db: &RootDatabase, file: SourceFile) -> Arc<HirModuleResult> {
    hir_module_with_stack(db, file, &[])
}

fn hir_module_with_stack(
    db: &RootDatabase,
    file: SourceFile,
    parent_stack: &[ImportStackEntry],
) -> Arc<HirModuleResult> {
    let workspace = Workspace::discover(db, file);
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

        let resolver = WorkspaceImportResolver::new(db, &workspace, &stack);
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

/// Format the source file using the memoised CST.
pub fn format_file(db: &RootDatabase, file: SourceFile) -> Option<String> {
    let parsed = parsed_file(db, file);
    let formatter = Formatter;
    Some(formatter.format(parsed.cst()))
}
