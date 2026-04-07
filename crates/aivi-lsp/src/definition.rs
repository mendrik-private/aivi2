use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{GotoDefinitionParams, GotoDefinitionResponse};

use crate::state::ServerState;

pub async fn definition(
    params: GotoDefinitionParams,
    state: Arc<ServerState>,
) -> Option<GotoDefinitionResponse> {
    let uri = &params.text_document_position_params.text_document.uri;
    let lsp_pos = params.text_document_position_params.position;

    let file = *state.files.get(uri)?;
    let navigation = crate::navigation::NavigationAnalysis::load(&state.db, file);
    match navigation.preferred_definition_targets_at_lsp_position(
        &state.db,
        LspPosition {
            line: lsp_pos.line,
            character: lsp_pos.character,
        },
    ) {
        crate::navigation::NavigationLookup::Targets(targets) => {
            crate::navigation::goto_response(&state.db, targets)
        }
        crate::navigation::NavigationLookup::NoTargets
        | crate::navigation::NavigationLookup::NoSite => None,
    }
}
