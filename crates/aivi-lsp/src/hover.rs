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

    let (symbols, text, path) = {
        let mut db = state.db.write();
        let symbols = aivi_query::symbol_index(&mut db, file);
        let text = file.text(&db).to_owned();
        let path = file.path(&db).to_path_buf();
        (symbols, text, path)
    };

    let mut source_db = aivi_base::SourceDatabase::new();
    let file_id = source_db.add_file(path, text);
    let source_file = &source_db[file_id];

    let cursor = source_file.lsp_position_to_offset(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    });

    // Find the innermost symbol whose full span contains the cursor.
    let sym = symbols.iter().find(|s| {
        let sp = s.span.span();
        sp.start() <= cursor && cursor <= sp.end()
    })?;

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

    // Compute range for the name span so VSCode highlights the hovered word.
    let name_range = source_file.span_to_lsp_range(sym.selection_span.span());
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
