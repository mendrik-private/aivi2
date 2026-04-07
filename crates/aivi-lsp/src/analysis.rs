use std::sync::Arc;

use aivi_base::{ByteIndex, LspPosition, SourceSpan};
use aivi_hir::LspSymbol;

use crate::type_annotations::TypedDeclarationSummary;

/// Query-backed per-file analysis snapshot used by editor features.
pub struct FileAnalysis {
    pub source: Arc<aivi_base::SourceFile>,
    pub diagnostics: Arc<[aivi_base::Diagnostic]>,
    pub symbols: Arc<[LspSymbol]>,
    pub typed_declarations: Arc<[TypedDeclarationSummary]>,
}

impl FileAnalysis {
    pub fn load(db: &aivi_query::RootDatabase, file: aivi_query::SourceFile) -> Self {
        let hir = aivi_query::hir_module(db, file);
        let parsed = aivi_query::parsed_file(db, file);
        Self {
            source: hir.source_arc(),
            diagnostics: hir.diagnostics_arc(),
            symbols: hir.symbols_arc(),
            typed_declarations: Arc::<[TypedDeclarationSummary]>::from(
                crate::type_annotations::collect_typed_declaration_summaries(
                    hir.module(),
                    parsed.cst(),
                    hir.source(),
                ),
            ),
        }
    }

    pub fn tightest_symbol_at_lsp_position(&self, position: LspPosition) -> Option<&LspSymbol> {
        let cursor = self.source.lsp_position_to_offset(position)?;
        self.tightest_symbol_at_offset(cursor)
    }

    pub fn tightest_symbol_at_offset(&self, cursor: ByteIndex) -> Option<&LspSymbol> {
        let mut best = None;
        let mut stack: Vec<&LspSymbol> = self.symbols.iter().rev().collect();

        while let Some(symbol) = stack.pop() {
            if !symbol_contains(symbol, cursor) {
                continue;
            }

            if best.is_none_or(|current: &LspSymbol| {
                symbol.span.span().len() < current.span.span().len()
            }) {
                best = Some(symbol);
            }
            stack.extend(symbol.children.iter().rev());
        }

        best
    }

    pub fn typed_declaration_at_lsp_position(
        &self,
        position: LspPosition,
    ) -> Option<&TypedDeclarationSummary> {
        let cursor = self.source.lsp_position_to_offset(position)?;
        self.typed_declaration_at_offset(cursor)
    }

    pub fn typed_declaration_at_offset(
        &self,
        cursor: ByteIndex,
    ) -> Option<&TypedDeclarationSummary> {
        self.typed_declarations
            .iter()
            .find(|declaration| declaration.name_span.span().contains(cursor))
    }

    pub fn typed_declaration_for_name_span(
        &self,
        name_span: SourceSpan,
    ) -> Option<&TypedDeclarationSummary> {
        self.typed_declarations
            .iter()
            .find(|declaration| declaration.name_span == name_span)
    }
}

fn symbol_contains(symbol: &LspSymbol, cursor: ByteIndex) -> bool {
    symbol.span.span().contains(cursor)
}

#[cfg(test)]
mod tests {
    use super::FileAnalysis;
    use std::sync::Arc;

    use aivi_base::{ByteIndex, FileId, LspPosition, SourceSpan, Span};
    use aivi_hir::{LspSymbol, LspSymbolKind};
    use crate::type_annotations::TypedDeclarationSummary;

    fn symbol(name: &str, span: std::ops::Range<usize>, children: Vec<LspSymbol>) -> LspSymbol {
        let span = SourceSpan::new(FileId::new(0), Span::from(span));
        LspSymbol {
            name: name.to_owned(),
            kind: LspSymbolKind::Function,
            span,
            selection_span: span,
            detail: None,
            children,
        }
    }

    fn analysis(text: &str, symbols: Vec<LspSymbol>) -> FileAnalysis {
        FileAnalysis {
            source: Arc::new(aivi_base::SourceFile::new(
                FileId::new(0),
                "test.aivi",
                text,
            )),
            diagnostics: Arc::<[aivi_base::Diagnostic]>::from(Vec::new()),
            symbols: Arc::<[LspSymbol]>::from(symbols),
            typed_declarations: Arc::<[TypedDeclarationSummary]>::from(Vec::new()),
        }
    }

    #[test]
    fn prefers_the_tightest_nested_symbol() {
        let analysis = analysis(
            "",
            vec![symbol(
                "outer",
                0..12,
                vec![symbol("inner", 3..7, Vec::new())],
            )],
        );

        let selected = analysis
            .tightest_symbol_at_offset(ByteIndex::new(4))
            .expect("a nested symbol should be found");

        assert_eq!(selected.name, "inner");
        assert_eq!(selected.kind, LspSymbolKind::Function);
    }

    #[test]
    fn maps_valid_lsp_positions_before_selecting_symbols() {
        let analysis = analysis(
            "value answer = 42\n",
            vec![symbol("answer", 0..16, Vec::new())],
        );

        let selected = analysis
            .tightest_symbol_at_lsp_position(LspPosition {
                line: 0,
                character: 7,
            })
            .expect("a symbol should be found at a valid LSP position");

        assert_eq!(selected.name, "answer");
    }

    #[test]
    fn rejects_invalid_lsp_columns() {
        let analysis = analysis(
            "value answer = 42\n",
            vec![symbol("answer", 0..16, Vec::new())],
        );

        assert!(
            analysis
                .tightest_symbol_at_lsp_position(LspPosition {
                    line: 0,
                    character: 99,
                })
                .is_none(),
            "out-of-range UTF-16 columns should not be silently clamped",
        );
    }
}
