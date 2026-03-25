use std::path::PathBuf;

use tower_lsp::lsp_types::Url;

use crate::state::ServerState;

/// Open or update a document in the database.
pub fn open_document(state: &ServerState, uri: &Url, text: String) {
    let path = uri_to_path(uri);
    let file = state.db.open_file(path, text);
    state.files.insert(uri.clone(), file);
}

/// Update an existing document's text.
pub fn change_document(state: &ServerState, uri: &Url, text: String) {
    if let Some(file) = state.files.get(uri) {
        file.set_text(&state.db, text);
    } else {
        open_document(state, uri, text);
    }
}

/// Remove a document from tracking and from the database.
pub fn close_document(state: &ServerState, uri: &Url) {
    if let Some((_, file)) = state.files.remove(uri) {
        state.db.remove_file(file);
    }
}

fn uri_to_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|_| PathBuf::from(uri.as_str()))
}
