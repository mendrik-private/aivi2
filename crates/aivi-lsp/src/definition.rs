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
    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);
    let cursor = analysis.source.lsp_position_to_offset(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    })?;
    let sym = analysis.tightest_symbol_at_offset(cursor)?;

    let lsp_range = analysis.source.span_to_lsp_range(sym.selection_span.span());
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
