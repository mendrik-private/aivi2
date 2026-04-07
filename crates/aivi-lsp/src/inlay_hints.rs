use std::sync::Arc;

use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, Position};

use crate::{analysis::FileAnalysis, state::ServerState};

/// Produce inlay hints for the visible range of the document.
///
/// We emit a `TYPE`-kind hint at the end of each symbol's `selection_span` for:
/// - Top-level `Variable` and `Function` symbols that have a known type detail.
/// - Parameter children of `Function` symbols that have a detail.
pub fn inlay_hints(params: InlayHintParams, state: Arc<ServerState>) -> Option<Vec<InlayHint>> {
    let config = state.config();
    if !config.inlay_hints_enabled {
        return None;
    }

    let uri = &params.text_document.uri;
    let file = *state.files.get(uri)?;
    let analysis = FileAnalysis::load(&state.db, file);
    let source = &analysis.source;

    let mut hints = Vec::new();

    for declaration in analysis.typed_declarations.iter() {
        if declaration.annotation.is_some() {
            continue;
        }
        let Some(inferred) = &declaration.inferred_type else {
            continue;
        };
        let lsp_range = source.span_to_lsp_range(declaration.name_span.span());
        hints.push(InlayHint {
            position: Position {
                line: lsp_range.end.line,
                character: lsp_range.end.character,
            },
            label: InlayHintLabel::String(truncate_inlay_hint_label(
                inferred,
                config.inlay_hints_max_length,
            )),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: Some(true),
            padding_right: None,
            data: None,
        });
    }

    if hints.is_empty() { None } else { Some(hints) }
}

fn truncate_inlay_hint_label(inferred: &str, max_length: usize) -> String {
    let label = format!(": {}", inferred);
    if label.chars().count() <= max_length {
        return label;
    }

    let truncated: String = label
        .chars()
        .take(max_length.saturating_sub(1))
        .collect::<String>();
    format!("{truncated}…")
}
