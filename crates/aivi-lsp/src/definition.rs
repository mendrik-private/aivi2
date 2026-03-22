use tower_lsp::lsp_types::{GotoDefinitionParams, GotoDefinitionResponse};

/// Handle a go-to-definition request (stub).
pub async fn definition(_params: GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
    None
}
