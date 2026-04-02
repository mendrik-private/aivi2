use std::path::PathBuf;

use aivi_lsp::{
    analysis::FileAnalysis, documents::open_document, state::ServerState, symbols::convert_symbols,
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
fn document_with_declarations_has_symbols() {
    let (state, uri) = open_inline("symbols-decls.aivi", "value answer = 42\n");
    let file = *state.files.get(&uri).expect("file should be open");
    let analysis = FileAnalysis::load(&state.db, file);
    let symbols = convert_symbols(&analysis.symbols, &analysis.source);

    assert!(
        !symbols.is_empty(),
        "a document with a value declaration should produce at least one symbol"
    );
}

#[test]
fn symbol_list_contains_declared_value_name() {
    let (state, uri) = open_inline("symbols-names.aivi", "value answer = 42\n");
    let file = *state.files.get(&uri).expect("file should be open");
    let analysis = FileAnalysis::load(&state.db, file);
    let symbols = convert_symbols(&analysis.symbols, &analysis.source);

    assert!(
        symbols.iter().any(|s| s.name == "answer"),
        "symbol list should contain 'answer'; got: {:?}",
        symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
