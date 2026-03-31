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
    let navigation = crate::navigation::NavigationAnalysis::load(&state.db, file);
    match navigation.definition_targets_at_lsp_position(
        &state.db,
        LspPosition {
            line: lsp_pos.line,
            character: lsp_pos.character,
        },
    ) {
        crate::navigation::NavigationLookup::Targets(targets) => {
            return crate::navigation::goto_response(&state.db, targets);
        }
        crate::navigation::NavigationLookup::NoTargets => return None,
        crate::navigation::NavigationLookup::NoSite => {}
    }

    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);
    let sym = analysis.tightest_symbol_at_lsp_position(LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character,
    })?;

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
