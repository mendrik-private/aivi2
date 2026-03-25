use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{
    Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Position, Range,
};

use crate::state::ServerState;

pub async fn hover(params: HoverParams, state: Arc<ServerState>) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let lsp_pos = params.text_document_position_params.position;

    let file = *state.files.get(uri)?;
    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);
    // TODO(aivi-base): lsp_position_to_offset now returns Option<ByteIndex>; update this
    // call site to handle None (out-of-range column) instead of relying on the old
    // silent-clamping behaviour.
    let cursor = analysis.source.lsp_position_to_offset(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    })?;
    let sym = analysis.tightest_symbol_at_offset(cursor)?;

    let kind_label = match sym.kind {
        aivi_hir::LspSymbolKind::Function => "fun",
        aivi_hir::LspSymbolKind::Variable => "val",
        aivi_hir::LspSymbolKind::Event => "sig",
        aivi_hir::LspSymbolKind::Struct => "type",
        aivi_hir::LspSymbolKind::Interface => "class",
        aivi_hir::LspSymbolKind::Namespace => "domain",
        _ => "symbol",
    };

    let header = if let Some(detail) = &sym.detail {
        format!("{} {}: {}", kind_label, sym.name, detail)
    } else {
        format!("{} {}", kind_label, sym.name)
    };

    let name_range = analysis.source.span_to_lsp_range(sym.selection_span.span());
    let range = Range {
        start: Position {
            line: name_range.start.line,
            character: name_range.start.character,
        },
        end: Position {
            line: name_range.end.line,
            character: name_range.end.character,
        },
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```aivi\n{}\n```", header),
        }),
        range: Some(range),
    })
}
