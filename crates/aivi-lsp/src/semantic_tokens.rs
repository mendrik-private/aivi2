use tower_lsp::lsp_types::{SemanticTokens, SemanticTokensParams, SemanticTokensResult};

/// Handle a semantic tokens request (stub).
pub async fn semantic_tokens_full(_params: SemanticTokensParams) -> Option<SemanticTokensResult> {
    Some(SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: Vec::new(),
    }))
}
