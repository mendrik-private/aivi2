use std::sync::Arc;

use aivi_base::Diagnostic;
use aivi_syntax::{ParsedModule, parse_module};

use crate::{RootDatabase, SourceFile};

/// Result of parsing a source file.
#[derive(Clone, Debug)]
pub struct ParsedFileResult {
    revision: u64,
    source: Arc<aivi_base::SourceFile>,
    parsed: ParsedModule,
    diagnostics: Arc<[Diagnostic]>,
}

impl ParsedFileResult {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn source(&self) -> &aivi_base::SourceFile {
        self.source.as_ref()
    }

    pub fn source_arc(&self) -> Arc<aivi_base::SourceFile> {
        Arc::clone(&self.source)
    }

    pub fn parsed(&self) -> &ParsedModule {
        &self.parsed
    }

    pub fn cst(&self) -> &aivi_syntax::Module {
        &self.parsed.module
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn diagnostics_arc(&self) -> Arc<[Diagnostic]> {
        Arc::clone(&self.diagnostics)
    }
}

/// Parse the given source file and memoise the CST by file revision.
pub fn parsed_file(db: &RootDatabase, file: SourceFile) -> Arc<ParsedFileResult> {
    loop {
        let input = db.source_input(file);
        if let Some(cached) = db.cached_parsed(file, input.revision) {
            db.record_parsed_hit();
            return cached;
        }
        db.record_parsed_miss();

        let source = db.make_source_file(file);
        let parsed = parse_module(source.as_ref());
        let diagnostics =
            Arc::<[Diagnostic]>::from(parsed.all_diagnostics().cloned().collect::<Vec<_>>());
        let computed = Arc::new(ParsedFileResult {
            revision: input.revision,
            source,
            parsed,
            diagnostics,
        });

        if let Some(current) = db.store_parsed(file, computed.revision(), computed) {
            return current;
        }
    }
}
