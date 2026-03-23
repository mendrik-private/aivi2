use tower_lsp::lsp_types::{SemanticTokensParams, SemanticTokensResult};

/// Semantic tokens not yet implemented — returning None lets VSCode fall back
/// to the TextMate grammar for all coloring.
pub async fn semantic_tokens_full(_params: SemanticTokensParams) -> Option<SemanticTokensResult> {
    None
}
