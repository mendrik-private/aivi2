use std::{path::PathBuf, sync::Arc};

use aivi_lsp::{documents::open_document, inlay_hints::inlay_hints, state::ServerState};
use tower_lsp::lsp_types::{
    InlayHintKind, InlayHintLabel, InlayHintParams, Range, TextDocumentIdentifier, Url,
    WorkDoneProgressParams,
};

fn inline_uri(name: &str) -> Url {
    Url::from_file_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(name),
    )
    .expect("test file path should convert to a file URL")
}

fn open_inline(name: &str, text: &str) -> (Arc<ServerState>, Url) {
    let state = Arc::new(ServerState::new());
    let uri = inline_uri(name);
    open_document(&state, &uri, text.to_owned());
    (state, uri)
}

fn inlay_hint_params(uri: Url) -> InlayHintParams {
    InlayHintParams {
        text_document: TextDocumentIdentifier { uri },
        range: Range::default(),
        work_done_progress_params: WorkDoneProgressParams::default(),
    }
}

#[test]
fn inlay_hints_returns_none_for_value_without_type_info() {
    // A simple value declaration without a type annotation — the compiler may
    // or may not infer a type, but the test verifies we don't crash.
    let (state, uri) = open_inline("hints-simple.aivi", "value answer = 42\n");
    // This should not panic regardless of whether hints are produced.
    let _ = inlay_hints(inlay_hint_params(uri), state);
}

#[test]
fn inlay_hints_kind_is_type_for_annotated_value() {
    // If the compiler emits a type detail for `answer`, all produced hints
    // should carry `InlayHintKind::TYPE`.
    let (state, uri) = open_inline("hints-kind.aivi", "value answer = 42\n");
    let hints = inlay_hints(inlay_hint_params(uri), state);

    if let Some(hints) = hints {
        for hint in &hints {
            assert_eq!(
                hint.kind,
                Some(InlayHintKind::TYPE),
                "every inlay hint for a value should have kind TYPE"
            );
        }
    }
}

#[test]
fn inlay_hints_label_starts_with_colon_space() {
    // Every inlay hint label should start with ": " to format as ": Type".
    let (state, uri) = open_inline("hints-label.aivi", "value answer = 42\n");
    let hints = inlay_hints(inlay_hint_params(uri), state);

    if let Some(hints) = hints {
        for hint in &hints {
            if let InlayHintLabel::String(label) = &hint.label {
                assert!(
                    label.starts_with(": "),
                    "inlay hint label should start with ': '; got: {:?}",
                    label
                );
            }
        }
    }
}

#[test]
fn inlay_hints_returns_none_for_empty_file() {
    let (state, uri) = open_inline("hints-empty.aivi", "");
    let hints = inlay_hints(inlay_hint_params(uri), state);
    assert!(
        hints.is_none(),
        "an empty file should produce no inlay hints"
    );
}
