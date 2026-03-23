use std::sync::Arc;

use aivi_base::Diagnostic;
use aivi_hir::{ExportedNames, LoweringResult, LspSymbol, exports, extract_symbols, lower_module};
use aivi_syntax::Formatter;

use crate::{RootDatabase, SourceFile, queries::parsed_file};

/// Result of lowering a source file to HIR.
#[derive(Clone, Debug)]
pub struct HirModuleResult {
    revision: u64,
    source: Arc<aivi_base::SourceFile>,
    module: aivi_hir::Module,
    diagnostics: Arc<[Diagnostic]>,
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

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn diagnostics_arc(&self) -> Arc<[Diagnostic]> {
        Arc::clone(&self.diagnostics)
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

/// Lower the given source file to HIR and memoise the result by file revision.
pub fn hir_module(db: &RootDatabase, file: SourceFile) -> Arc<HirModuleResult> {
    loop {
        let parsed = parsed_file(db, file);
        if let Some(cached) = db.cached_hir(file, parsed.revision()) {
            return cached;
        }

        let lowered: LoweringResult = lower_module(parsed.cst());
        let mut diagnostics = parsed.diagnostics().to_vec();
        diagnostics.extend_from_slice(lowered.diagnostics());
        let module = lowered.into_parts().0;
        let symbols = Arc::<[LspSymbol]>::from(extract_symbols(&module));
        let exported_names = exports(&module);
        let computed = Arc::new(HirModuleResult {
            revision: parsed.revision(),
            source: parsed.source_arc(),
            module,
            diagnostics: Arc::<[Diagnostic]>::from(diagnostics),
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
