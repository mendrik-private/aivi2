use std::{collections::HashMap, sync::Arc};

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{
    PrepareRenameResponse, RenameParams, TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit,
};

use crate::{
    navigation::{NavigationAnalysis, NavigationLookup},
    state::ServerState,
};

/// Confirm the cursor is on a renameable position (any site that resolves to
/// definition targets).  Returns `DefaultBehavior` if valid.
pub async fn prepare_rename(
    params: TextDocumentPositionParams,
    state: Arc<ServerState>,
) -> Option<PrepareRenameResponse> {
    let uri = &params.text_document.uri;
    let lsp_pos = params.position;

    let file = *state.files.get(uri)?;
    let navigation = NavigationAnalysis::load(&state.db, file);
    match navigation.definition_targets_at_lsp_position(
        &state.db,
        LspPosition {
            line: lsp_pos.line,
            character: lsp_pos.character,
        },
    ) {
        NavigationLookup::Targets(_) => {
            Some(PrepareRenameResponse::DefaultBehavior { default_behavior: true })
        }
        NavigationLookup::NoSite | NavigationLookup::NoTargets => None,
    }
}

/// Collect all reference locations (same algorithm as find-all-references) and
/// produce a `WorkspaceEdit` that replaces every occurrence with `new_name`.
pub async fn rename(params: RenameParams, state: Arc<ServerState>) -> Option<WorkspaceEdit> {
    let uri = &params.text_document_position.text_document.uri;
    let lsp_pos = params.text_document_position.position;
    let new_name = &params.new_name;

    let file = *state.files.get(uri)?;
    let navigation = NavigationAnalysis::load(&state.db, file);

    let targets = match navigation.definition_targets_at_lsp_position(
        &state.db,
        LspPosition {
            line: lsp_pos.line,
            character: lsp_pos.character,
        },
    ) {
        NavigationLookup::Targets(t) => t,
        NavigationLookup::NoSite | NavigationLookup::NoTargets => return None,
    };

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

    for entry in state.files.iter() {
        let (file_uri, &candidate_file) = (entry.key(), entry.value());
        let nav = NavigationAnalysis::load(&state.db, candidate_file);
        let locs = nav.all_reference_locations_for_targets(&state.db, &targets);
        if !locs.is_empty() {
            let edits = locs
                .into_iter()
                .map(|loc| TextEdit {
                    range: loc.range,
                    new_text: new_name.clone(),
                })
                .collect::<Vec<_>>();
            changes
                .entry(file_uri.clone())
                .or_default()
                .extend(edits);
        }
    }

    if changes.is_empty() {
        None
    } else {
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }
}
