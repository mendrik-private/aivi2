use std::sync::Arc;

use aivi_base::ByteIndex;
use aivi_hir::LspSymbol;

/// Query-backed per-file analysis snapshot used by editor features.
pub struct FileAnalysis {
    pub source: Arc<aivi_base::SourceFile>,
    pub diagnostics: Arc<[aivi_base::Diagnostic]>,
    pub symbols: Arc<[LspSymbol]>,
}

impl FileAnalysis {
    pub fn load(db: &aivi_query::RootDatabase, file: aivi_query::SourceFile) -> Self {
        let hir = aivi_query::hir_module(db, file);
        Self {
            source: hir.source_arc(),
            diagnostics: hir.diagnostics_arc(),
            symbols: hir.symbols_arc(),
        }
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
}

fn symbol_contains(symbol: &LspSymbol, cursor: ByteIndex) -> bool {
    let span = symbol.span.span();
    span.start() <= cursor && cursor <= span.end()
}

#[cfg(test)]
mod tests {
    use super::FileAnalysis;
    use std::sync::Arc;

    use aivi_base::{ByteIndex, FileId, SourceSpan, Span};
    use aivi_hir::LspSymbol;

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

    #[test]
    fn prefers_the_tightest_nested_symbol() {
        let analysis = FileAnalysis {
            source: Arc::new(aivi_base::SourceFile::new(FileId::new(0), "test.aivi", "")),
            diagnostics: Arc::<[aivi_base::Diagnostic]>::from(Vec::new()),
            symbols: Arc::<[LspSymbol]>::from(vec![symbol(
                "outer",
                0..12,
                vec![symbol("inner", 3..7, Vec::new())],
            )]),
        };

        let selected = analysis
            .tightest_symbol_at_offset(ByteIndex::new(4))
            .expect("a nested symbol should be found");

        assert_eq!(selected.name, "inner");
        assert_eq!(selected.kind, LspSymbolKind::Function);
    }
}
