use std::path::PathBuf;

use aivi_query::SourceFile;
use tower_lsp::lsp_types::Url;

use crate::state::ServerState;

/// Open or update a document in the database.
pub fn open_document(state: &ServerState, uri: &Url, text: String) {
    let path = uri_to_path(uri);
    let mut db = state.db.write();
    if let Some(file) = state.files.get(uri) {
        file.set_text(&mut db, text);
    } else {
        drop(db);
        let mut db = state.db.write();
        let file = SourceFile::new(&mut db, path, text);
        drop(db);
        state.files.insert(uri.clone(), file);
    }
}

/// Update an existing document's text.
pub fn change_document(state: &ServerState, uri: &Url, text: String) {
    if let Some(file) = state.files.get(uri) {
        let mut db = state.db.write();
        file.set_text(&mut db, text);
    } else {
        open_document(state, uri, text);
    }
}

/// Remove a document from tracking (it remains in the DB but we forget the handle).
pub fn close_document(state: &ServerState, uri: &Url) {
    state.files.remove(uri);
}

fn uri_to_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|_| PathBuf::from(uri.as_str()))
}
