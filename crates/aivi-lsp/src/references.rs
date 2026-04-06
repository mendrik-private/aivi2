use std::sync::Arc;

use aivi_base::LspPosition;
use tower_lsp::lsp_types::{Location, ReferenceParams};

use crate::{
    navigation::{NavigationAnalysis, NavigationLookup},
    state::ServerState,
};

/// Find all reference locations for the symbol under the cursor.
///
/// The algorithm:
/// 1. Resolve the definition target(s) at the cursor position.
/// 2. For every tracked file, walk all navigation sites and collect those
///    whose definition targets overlap the sought targets.
/// 3. Return deduplicated `Location` values.
pub async fn references(
    params: ReferenceParams,
    state: Arc<ServerState>,
) -> Option<Vec<Location>> {
    let uri = &params.text_document_position.text_document.uri;
    let lsp_pos = params.text_document_position.position;

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

    let mut locations: Vec<Location> = Vec::new();

    for entry in state.files.iter() {
        let (_, &candidate_file) = (entry.key(), entry.value());
        let nav = NavigationAnalysis::load(&state.db, candidate_file);
        let file_locs = nav.all_reference_locations_for_targets(&state.db, &targets);
        for loc in file_locs {
            if !locations.contains(&loc) {
                locations.push(loc);
            }
        }
    }

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}
