use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::request::{GotoImplementationParams, GotoImplementationResponse};

use crate::{
    navigation::{NavigationAnalysis, NavigationLookup, goto_response},
    state::ServerState,
};

pub async fn implementation(
    params: GotoImplementationParams,
    state: Arc<ServerState>,
) -> Option<GotoImplementationResponse> {
    let uri = &params.text_document_position_params.text_document.uri;
    let lsp_pos = params.text_document_position_params.position;

    let file = *state.files.get(uri)?;
    let analysis = NavigationAnalysis::load(&state.db, file);
    match analysis.implementation_targets_at_lsp_position(
        &state.db,
        LspPosition {
            line: lsp_pos.line,
            character: lsp_pos.character,
        },
    ) {
        NavigationLookup::Targets(targets) => goto_response(&state.db, targets),
        NavigationLookup::NoSite | NavigationLookup::NoTargets => None,
    }
}
