#![forbid(unsafe_code)]

//! Incremental query infrastructure for AIVI.
//!
//! Implements a simple caching layer over the parse/HIR pipeline.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use aivi_base::{Diagnostic, SourceDatabase};
use aivi_hir::{ExportedNames, LoweringResult, LspSymbol, exports, extract_symbols, lower_module};
use aivi_syntax::{Formatter, ParsedModule, parse_module};

/// Result of parsing a source file.
#[derive(Clone, Debug)]
pub struct ParsedFileResult {
    pub parsed: ParsedModule,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedFileResult {
    /// Returns the syntax CST module.
    pub fn cst(&self) -> &aivi_syntax::Module {
        &self.parsed.module
    }
}

/// Result of lowering a source file to HIR.
#[derive(Clone, Debug)]
pub struct HirModuleResult {
    pub module: aivi_hir::Module,
    pub diagnostics: Vec<Diagnostic>,
}

/// A handle to a source file in the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceFile {
    id: u64,
}

impl SourceFile {
    /// Create a new SourceFile handle in the database.
    pub fn new(db: &mut RootDatabase, path: PathBuf, text: String) -> Self {
        let id = db.next_id;
        db.next_id += 1;
        let handle = SourceFile { id };
        db.files.insert(id, FileEntry { path, text });
        db.invalidate(id);
        handle
    }

    /// Set the text content of this file.
    pub fn set_text(self, db: &mut RootDatabase, text: String) {
        if let Some(entry) = db.files.get_mut(&self.id) {
            entry.text = text;
        }
        db.invalidate(self.id);
    }

    /// Get the text of this file.
    pub fn text(self, db: &RootDatabase) -> &str {
        db.files
            .get(&self.id)
            .map(|e| e.text.as_str())
            .unwrap_or("")
    }

    /// Get the path of this file.
    pub fn path(self, db: &RootDatabase) -> &Path {
        db.files
            .get(&self.id)
            .map(|e| e.path.as_path())
            .unwrap_or(Path::new(""))
    }
}

struct FileEntry {
    path: PathBuf,
    text: String,
}

/// The root query database.
pub struct RootDatabase {
    next_id: u64,
    files: HashMap<u64, FileEntry>,
    parse_cache: HashMap<u64, ParsedFileResult>,
    hir_cache: HashMap<u64, HirModuleResult>,
}

impl Default for RootDatabase {
    fn default() -> Self {
        Self {
            next_id: 0,
            files: HashMap::new(),
            parse_cache: HashMap::new(),
            hir_cache: HashMap::new(),
        }
    }
}

impl RootDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    fn invalidate(&mut self, id: u64) {
        self.parse_cache.remove(&id);
        self.hir_cache.remove(&id);
    }

    fn ensure_parsed(&mut self, id: u64) {
        if self.parse_cache.contains_key(&id) {
            return;
        }
        let Some(entry) = self.files.get(&id) else {
            return;
        };
        let mut source_db = SourceDatabase::new();
        let file_id = source_db.add_file(entry.path.clone(), entry.text.clone());
        let source_file = &source_db[file_id];
        let parsed = parse_module(source_file);
        let diagnostics: Vec<Diagnostic> = parsed.all_diagnostics().cloned().collect();
        self.parse_cache.insert(
            id,
            ParsedFileResult {
                parsed,
                diagnostics,
            },
        );
    }

    fn ensure_hir(&mut self, id: u64) {
        self.ensure_parsed(id);
        if self.hir_cache.contains_key(&id) {
            return;
        }
        let Some(parsed_result) = self.parse_cache.get(&id) else {
            return;
        };
        let lowered: LoweringResult = lower_module(&parsed_result.parsed.module);
        let mut diagnostics: Vec<Diagnostic> = parsed_result.diagnostics.clone();
        diagnostics.extend_from_slice(lowered.diagnostics());
        let module = lowered.into_parts().0;
        self.hir_cache.insert(
            id,
            HirModuleResult {
                module,
                diagnostics,
            },
        );
    }
}

/// Parse the given source file and return the result.
pub fn parsed_file(db: &mut RootDatabase, file: SourceFile) -> ParsedFileResult {
    db.ensure_parsed(file.id);
    db.parse_cache
        .get(&file.id)
        .cloned()
        .unwrap_or_else(|| ParsedFileResult {
            parsed: {
                let mut source_db = SourceDatabase::new();
                let file_id = source_db.add_file("<empty>", "");
                let source_file = &source_db[file_id];
                parse_module(source_file)
            },
            diagnostics: Vec::new(),
        })
}

/// Lower the source file to HIR and return the result.
pub fn hir_module(db: &mut RootDatabase, file: SourceFile) -> HirModuleResult {
    db.ensure_hir(file.id);
    db.hir_cache.get(&file.id).cloned().unwrap_or_else(|| {
        let mut source_db = SourceDatabase::new();
        let file_id = source_db.add_file("<empty>", "");
        let source_file = &source_db[file_id];
        let parsed = parse_module(source_file);
        let lowered = lower_module(&parsed.module);
        let module = lowered.into_parts().0;
        HirModuleResult {
            module,
            diagnostics: Vec::new(),
        }
    })
}

/// Collect all diagnostics (parse + HIR) for the given file.
pub fn all_diagnostics(db: &mut RootDatabase, file: SourceFile) -> Vec<Diagnostic> {
    db.ensure_hir(file.id);
    db.hir_cache
        .get(&file.id)
        .map(|r| r.diagnostics.clone())
        .unwrap_or_default()
}

/// Extract LSP symbols from the HIR module.
pub fn symbol_index(db: &mut RootDatabase, file: SourceFile) -> Vec<LspSymbol> {
    db.ensure_hir(file.id);
    db.hir_cache
        .get(&file.id)
        .map(|r| extract_symbols(&r.module))
        .unwrap_or_default()
}

/// Extract exported names from the HIR module.
pub fn exported_names(db: &mut RootDatabase, file: SourceFile) -> ExportedNames {
    db.ensure_hir(file.id);
    db.hir_cache
        .get(&file.id)
        .map(|r| exports(&r.module))
        .unwrap_or_default()
}

/// Format the source file using the aivi formatter.
pub fn format_file(db: &mut RootDatabase, file: SourceFile) -> Option<String> {
    db.ensure_parsed(file.id);
    db.parse_cache.get(&file.id).map(|r| {
        let formatter = Formatter;
        formatter.format(r.cst())
    })
}
