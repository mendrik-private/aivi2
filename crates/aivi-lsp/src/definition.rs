use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, Position, Range,
};

use crate::state::ServerState;

pub async fn definition(
    params: GotoDefinitionParams,
    state: Arc<ServerState>,
) -> Option<GotoDefinitionResponse> {
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

    // Find the symbol whose full span contains the cursor and whose name is
    // at or nearest to the cursor — prefer the tightest (selection_span) match.
    let sym = symbols
        .iter()
        .filter(|s| {
            let sp = s.span.span();
            sp.start() <= cursor && cursor <= sp.end()
        })
        .min_by_key(|s| s.span.span().len())?;

    let lsp_range = source_file.span_to_lsp_range(sym.selection_span.span());
    let range = Range {
        start: Position {
            line: lsp_range.start.line,
            character: lsp_range.start.character,
        },
        end: Position {
            line: lsp_range.end.line,
            character: lsp_range.end.character,
        },
    };

    Some(GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range,
    }))
}
