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
    let cursor = LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    };

    if let Some(sym) = analysis.tightest_symbol_at_lsp_position(cursor) {
        return Some(hover_for_symbol(sym, &analysis.source));
    }

    // Fallback: cursor may be on a navigation reference site (not a declaration symbol).
    // Resolve to the declaration and retrieve its type detail.
    let navigation = crate::navigation::NavigationAnalysis::load(&state.db, file);
    if let crate::navigation::NavigationLookup::Targets(targets) =
        navigation.definition_targets_at_lsp_position(&state.db, cursor)
    {
        for target in &targets {
            if let Some(decl_sym) = target.find_symbol_at_target(&state.db) {
                return Some(hover_for_symbol(&decl_sym, &analysis.source));
            }
        }
    }

    None
}

fn hover_for_symbol(sym: &aivi_hir::LspSymbol, source: &aivi_base::SourceFile) -> Hover {
    let kind_label = kind_label(sym.kind);
    let header = if let Some(detail) = &sym.detail {
        format!("{} {} : {}", kind_label, sym.name, detail)
    } else {
        format!("{} {}", kind_label, sym.name)
    };

    let name_range = source.span_to_lsp_range(sym.selection_span.span());
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

    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```aivi\n{}\n```", header),
        }),
        range: Some(range),
    }
}

pub fn kind_label(kind: aivi_hir::LspSymbolKind) -> &'static str {
    match kind {
        aivi_hir::LspSymbolKind::Function => "func",
        aivi_hir::LspSymbolKind::Variable => "value",
        aivi_hir::LspSymbolKind::Event => "signal",
        aivi_hir::LspSymbolKind::Struct => "type",
        aivi_hir::LspSymbolKind::Interface => "class",
        aivi_hir::LspSymbolKind::Namespace => "domain",
        _ => "symbol",
    }
}
