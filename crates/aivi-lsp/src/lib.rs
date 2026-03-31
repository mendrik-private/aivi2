#![forbid(unsafe_code)]

//! AIVI Language Server Protocol server.

pub mod analysis;
pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod documents;
pub mod formatting;
pub mod hover;
pub mod implementation;
mod navigation;
pub mod semantic_tokens;
pub mod server;
pub mod state;
pub mod symbols;

/// Start the LSP server, listening on stdio.
pub async fn run() -> anyhow::Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};

    // Initialize tracing to stderr (LSP uses stdout for protocol traffic).
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = tower_lsp::LspService::new(|client| server::Backend::new(client));

    tower_lsp::Server::new(stdin, stdout, socket)
        .serve(service)
        .await;

    Ok(())
}
