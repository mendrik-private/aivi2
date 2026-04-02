use std::path::PathBuf;

use aivi_lsp::{
    documents::{change_document, open_document},
    formatting::format_document,
    state::ServerState,
};
use tower_lsp::lsp_types::Url;

fn test_uri(name: &str) -> Url {
    Url::from_file_path(PathBuf::from("/test-documents").join(name))
        .expect("test URI should be valid")
}

fn open_inline(name: &str, text: &str) -> (ServerState, Url) {
    let state = ServerState::new();
    let uri = test_uri(name);
    open_document(&state, &uri, text.to_owned());
    (state, uri)
}

#[test]
fn formatting_valid_document_returns_result() {
    // Compact form without spaces — formatter must add them
    let (state, uri) = open_inline("format-compact.aivi", "value answer=42\n");
    let file = *state.files.get(&uri).expect("file should be open");

    let result = format_document(&state.db, file);
    assert!(
        result.is_some(),
        "formatting a valid document should return Some"
    );
}

#[test]
fn formatting_is_idempotent() {
    let source = "value answer=42\n";
    let (state, uri) = open_inline("format-idempotent.aivi", source);
    let file = *state.files.get(&uri).expect("file should be open");

    // Obtain the canonical formatted text from the first pass
    let first_edits = format_document(&state.db, file).expect("first format should succeed");
    let formatted = if first_edits.is_empty() {
        source.to_owned()
    } else {
        first_edits[0].new_text.clone()
    };

    // Update the document to its formatted state
    change_document(&state, &uri, formatted);

    // A second format pass on an already-formatted document should produce no edits
    let second_edits = format_document(&state.db, file).expect("second format should succeed");
    assert!(
        second_edits.is_empty(),
        "formatting an already-formatted document should produce no edits"
    );
}
