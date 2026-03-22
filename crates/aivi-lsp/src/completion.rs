use tower_lsp::lsp_types::{CompletionParams, CompletionResponse};

/// Handle a completion request (stub).
pub async fn completion(_params: CompletionParams) -> Option<CompletionResponse> {
    Some(CompletionResponse::Array(Vec::new()))
}
