use std::sync::RwLock;

use aivi_query::{RootDatabase, SourceFile};
use dashmap::DashMap;
use serde::Deserialize;
use tokio::task::JoinHandle;
use tower_lsp::lsp_types::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerConfig {
    pub diagnostics_debounce_ms: u64,
    pub inlay_hints_enabled: bool,
    pub inlay_hints_max_length: usize,
    pub code_lens_enabled: bool,
}

impl ServerConfig {
    pub fn from_initialization_options(raw: Option<serde_json::Value>) -> Self {
        let defaults = Self::default();
        let options = raw
            .and_then(|value| serde_json::from_value::<InitializationOptions>(value).ok())
            .unwrap_or_default();
        Self {
            diagnostics_debounce_ms: options
                .diagnostics_debounce_ms
                .unwrap_or(defaults.diagnostics_debounce_ms),
            inlay_hints_enabled: options
                .inlay_hints_enabled
                .unwrap_or(defaults.inlay_hints_enabled),
            inlay_hints_max_length: options
                .inlay_hints_max_length
                .unwrap_or(defaults.inlay_hints_max_length)
                .max(4),
            code_lens_enabled: options.code_lens_enabled.unwrap_or(defaults.code_lens_enabled),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            diagnostics_debounce_ms: 200,
            inlay_hints_enabled: true,
            inlay_hints_max_length: 30,
            code_lens_enabled: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializationOptions {
    diagnostics_debounce_ms: Option<u64>,
    inlay_hints_enabled: Option<bool>,
    inlay_hints_max_length: Option<usize>,
    code_lens_enabled: Option<bool>,
}

/// Shared state for the language server.
pub struct ServerState {
    pub db: RootDatabase,
    pub files: DashMap<Url, SourceFile>,
    /// Pending debounced diagnostics tasks, keyed by document URI.
    pub pending_diagnostics: DashMap<Url, JoinHandle<()>>,
    config: RwLock<ServerConfig>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            db: RootDatabase::new(),
            files: DashMap::new(),
            pending_diagnostics: DashMap::new(),
            config: RwLock::new(ServerConfig::default()),
        }
    }

    pub fn config(&self) -> ServerConfig {
        *self
            .config
            .read()
            .expect("server config lock should not be poisoned")
    }

    pub fn set_config(&self, config: ServerConfig) {
        *self
            .config
            .write()
            .expect("server config lock should not be poisoned") = config;
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ServerConfig;

    #[test]
    fn initialization_options_override_defaults() {
        let config = ServerConfig::from_initialization_options(Some(serde_json::json!({
            "diagnosticsDebounceMs": 75,
            "inlayHintsEnabled": false,
            "inlayHintsMaxLength": 12,
            "codeLensEnabled": false
        })));

        assert_eq!(config.diagnostics_debounce_ms, 75);
        assert!(!config.inlay_hints_enabled);
        assert_eq!(config.inlay_hints_max_length, 12);
        assert!(!config.code_lens_enabled);
    }
}
