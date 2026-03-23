use aivi_query::{RootDatabase, SourceFile};
use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

/// Shared state for the language server.
pub struct ServerState {
    pub db: RootDatabase,
    pub files: DashMap<Url, SourceFile>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            db: RootDatabase::new(),
            files: DashMap::new(),
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
