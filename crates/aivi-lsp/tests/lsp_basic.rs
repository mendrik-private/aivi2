use std::path::PathBuf;

use aivi_lsp::{
    diagnostics::collect_lsp_diagnostics,
    documents::{change_document, close_document, open_document},
    state::ServerState,
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

fn test_uri(name: &str) -> Url {
    Url::from_file_path(PathBuf::from("/test-documents").join(name))
        .expect("test URI should be valid")
}

#[test]
fn open_change_close_document_lifecycle() {
    let state = ServerState::new();
    let uri = test_uri("lifecycle.aivi");

    open_document(&state, &uri, "value answer = 42\n".to_owned());
    assert!(
        state.files.get(&uri).is_some(),
        "document should be tracked after open"
    );

    change_document(&state, &uri, "value answer = 43\n".to_owned());
    assert!(
        state.files.get(&uri).is_some(),
        "document should still be tracked after change"
    );

    close_document(&state, &uri);
    assert!(
        state.files.get(&uri).is_none(),
        "document should not be tracked after close"
    );
}

#[test]
fn valid_document_has_no_error_diagnostics() {
    let state = ServerState::new();
    let uri = test_uri("valid.aivi");
    open_document(&state, &uri, "value answer = 42\n".to_owned());
    let file = *state.files.get(&uri).expect("file should be open");

    let diagnostics = collect_lsp_diagnostics(&state.db, file, &uri);
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();

    assert!(
        errors.is_empty(),
        "a valid document should produce no error diagnostics; got: {errors:#?}"
    );
}

#[test]
fn invalid_document_has_error_diagnostics() {
    let state = ServerState::new();
    let uri = test_uri("invalid.aivi");
    // "val = 42" is not valid AIVI syntax; a valid declaration is "value val = 42"
    open_document(&state, &uri, "val = 42\n".to_owned());
    let file = *state.files.get(&uri).expect("file should be open");

    let diagnostics = collect_lsp_diagnostics(&state.db, file, &uri);
    assert!(
        !diagnostics.is_empty(),
        "an invalid document should produce at least one diagnostic"
    );
}
