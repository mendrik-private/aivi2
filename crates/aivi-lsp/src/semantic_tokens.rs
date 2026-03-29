use std::sync::Arc;

use aivi_syntax::{TokenKind, lex_module};
use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokens, SemanticTokensParams, SemanticTokensResult,
};

use crate::state::ServerState;

/// Ordered list of token type names used in the legend.  The index in this
/// array is the `token_type` field emitted for each `SemanticToken`.
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::TYPE,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::COMMENT,
];

const IDX_VARIABLE: u32 = 2;
const IDX_KEYWORD: u32 = 3;
const IDX_STRING: u32 = 4;
const IDX_NUMBER: u32 = 5;
const IDX_OPERATOR: u32 = 6;
const IDX_COMMENT: u32 = 7;

fn token_type_index(kind: TokenKind) -> Option<u32> {
    match kind {
        // Keywords
        TokenKind::TypeKw
        | TokenKind::FunKw
        | TokenKind::ValueKw
        | TokenKind::SignalKw
        | TokenKind::ClassKw
        | TokenKind::InstanceKw
        | TokenKind::DomainKw
        | TokenKind::ProviderKw
        | TokenKind::UseKw
        | TokenKind::ExportKw
        | TokenKind::PatchKw => Some(IDX_KEYWORD),

        // Soft keywords are handled separately; other identifiers remain variables.
        TokenKind::Identifier => None,

        // Literals
        TokenKind::StringLiteral | TokenKind::RegexLiteral => Some(IDX_STRING),
        TokenKind::Integer | TokenKind::Float | TokenKind::Decimal | TokenKind::BigInt => {
            Some(IDX_NUMBER)
        }

        // Operators and punctuation
        TokenKind::Plus
        | TokenKind::Minus
        | TokenKind::Slash
        | TokenKind::Star
        | TokenKind::Percent
        | TokenKind::Less
        | TokenKind::Greater
        | TokenKind::LessEqual
        | TokenKind::GreaterEqual
        | TokenKind::Equals
        | TokenKind::EqualEqual
        | TokenKind::BangEqual
        | TokenKind::Ellipsis
        | TokenKind::Arrow
        | TokenKind::ThinArrow
        | TokenKind::LeftArrow
        | TokenKind::ColonEquals
        | TokenKind::PipeTransform
        | TokenKind::PipeGate
        | TokenKind::PipeCase
        | TokenKind::PipeMap
        | TokenKind::PipeApply
        | TokenKind::PipeRecurStart
        | TokenKind::PipeRecurStep
        | TokenKind::PipeTap
        | TokenKind::PipeFanIn
        | TokenKind::PatchApply
        | TokenKind::TruthyBranch
        | TokenKind::FalsyBranch
        | TokenKind::PipeValidate
        | TokenKind::PipePrevious
        | TokenKind::PipeAccumulate
        | TokenKind::PipeDiff
        | TokenKind::At
        | TokenKind::Hash
        | TokenKind::Colon
        | TokenKind::Dot
        | TokenKind::DotDot
        | TokenKind::Comma
        | TokenKind::LParen
        | TokenKind::RParen
        | TokenKind::LBrace
        | TokenKind::RBrace
        | TokenKind::LBracket
        | TokenKind::RBracket
        | TokenKind::CloseTagStart
        | TokenKind::SelfCloseTagEnd => Some(IDX_OPERATOR),

        // Comments
        TokenKind::LineComment | TokenKind::BlockComment | TokenKind::DocComment => {
            Some(IDX_COMMENT)
        }

        // Whitespace and unknown — not emitted.
        TokenKind::Whitespace | TokenKind::Newline | TokenKind::Unknown => None,
    }
}

pub async fn semantic_tokens_full(
    params: SemanticTokensParams,
    state: Arc<ServerState>,
) -> Option<SemanticTokensResult> {
    let uri = &params.text_document.uri;
    let file = *state.files.get(uri)?;
    let analysis = crate::analysis::FileAnalysis::load(&state.db, file);
    let source = analysis.source.as_ref();

    let lexed = lex_module(source);
    let mut result: Vec<SemanticToken> = Vec::new();
    let mut prev_line: u32 = 0;
    let mut prev_char: u32 = 0;

    for token in lexed.tokens() {
        let Some(type_index) = soft_or_hard_token_type_index(*token, source) else {
            continue;
        };

        let lsp_range = source.span_to_lsp_range(token.span());
        let token_line = lsp_range.start.line;
        let token_char = lsp_range.start.character;
        let token_len = lsp_range
            .end
            .character
            .saturating_sub(lsp_range.start.character);

        // Multi-line tokens (e.g. block comments): skip — the LSP spec
        // requires single-line tokens in the full-tokens response.
        if lsp_range.start.line != lsp_range.end.line {
            continue;
        }

        let delta_line = token_line - prev_line;
        let delta_start = if delta_line == 0 {
            token_char - prev_char
        } else {
            token_char
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: token_len,
            token_type: type_index,
            token_modifiers_bitset: 0,
        });

        prev_line = token_line;
        prev_char = token_char;
    }

    Some(SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: result,
    }))
}

fn soft_or_hard_token_type_index(token: aivi_syntax::Token, source: &aivi_base::SourceFile) -> Option<u32> {
    match token.kind() {
        TokenKind::Identifier if token.text(source) == "when" => Some(IDX_KEYWORD),
        TokenKind::Identifier => Some(IDX_VARIABLE),
        kind => token_type_index(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::{IDX_KEYWORD, IDX_OPERATOR, soft_or_hard_token_type_index, token_type_index};
    use aivi_base::{FileId, SourceFile};
    use aivi_syntax::{TokenKind, lex_module};

    #[test]
    fn classifies_patch_surface_tokens() {
        assert_eq!(token_type_index(TokenKind::PatchKw), Some(IDX_KEYWORD));
        assert_eq!(token_type_index(TokenKind::PatchApply), Some(IDX_OPERATOR));
        assert_eq!(token_type_index(TokenKind::ColonEquals), Some(IDX_OPERATOR));
    }

    #[test]
    fn classifies_when_as_soft_keyword() {
        let source = SourceFile::new(FileId::new(0), "test.aivi", "when ready => total <- 1\n");
        let lexed = lex_module(&source);
        let when = lexed
            .tokens()
            .iter()
            .find(|token| token.kind() == TokenKind::Identifier && token.text(&source) == "when")
            .copied()
            .expect("expected `when` token");
        assert_eq!(
            soft_or_hard_token_type_index(when, &source),
            Some(IDX_KEYWORD)
        );
    }
}
