use std::sync::Arc;

use aivi_query::{RootDatabase, SourceFile};
use dashmap::DashMap;
use parking_lot::RwLock;
use tower_lsp::lsp_types::Url;

/// Shared state for the language server.
pub struct ServerState {
    pub db: Arc<RwLock<RootDatabase>>,
    pub files: DashMap<Url, SourceFile>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            db: Arc::new(RwLock::new(RootDatabase::new())),
            files: DashMap::new(),
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
