use tower_lsp::lsp_types::{Hover, HoverParams};

/// Handle a hover request (stub).
pub async fn hover(_params: HoverParams) -> Option<Hover> {
    None
}
