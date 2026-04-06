use std::sync::Arc;

use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, Position};

use crate::{analysis::FileAnalysis, state::ServerState};

/// Produce inlay hints for the visible range of the document.
///
/// We emit a `TYPE`-kind hint at the end of each symbol's `selection_span` for:
/// - Top-level `Variable` and `Function` symbols that have a known type detail.
/// - Parameter children of `Function` symbols that have a detail.
pub fn inlay_hints(params: InlayHintParams, state: Arc<ServerState>) -> Option<Vec<InlayHint>> {
    let uri = &params.text_document.uri;
    let file = *state.files.get(uri)?;
    let analysis = FileAnalysis::load(&state.db, file);
    let source = &analysis.source;

    let mut hints = Vec::new();

    for sym in analysis.symbols.iter() {
        if matches!(
            sym.kind,
            aivi_hir::LspSymbolKind::Variable | aivi_hir::LspSymbolKind::Function
        ) {
            if let Some(detail) = &sym.detail {
                let lsp_range = source.span_to_lsp_range(sym.selection_span.span());
                hints.push(InlayHint {
                    position: Position {
                        line: lsp_range.end.line,
                        character: lsp_range.end.character,
                    },
                    label: InlayHintLabel::String(format!(": {}", detail)),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                });
            }

            // Emit hints for parameter children of Function symbols.
            if sym.kind == aivi_hir::LspSymbolKind::Function {
                for child in sym.children.iter() {
                    if let Some(child_detail) = &child.detail {
                        let child_range = source.span_to_lsp_range(child.selection_span.span());
                        hints.push(InlayHint {
                            position: Position {
                                line: child_range.end.line,
                                character: child_range.end.character,
                            },
                            label: InlayHintLabel::String(format!(": {}", child_detail)),
                            kind: Some(InlayHintKind::TYPE),
                            text_edits: None,
                            tooltip: None,
                            padding_left: Some(true),
                            padding_right: None,
                            data: None,
                        });
                    }
                }
            }
        }
    }

    if hints.is_empty() {
        None
    } else {
        Some(hints)
    }
}
