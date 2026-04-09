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

const IDX_KEYWORD: u32 = 3;
const IDX_STRING: u32 = 4;
const IDX_NUMBER: u32 = 5;
const IDX_COMMENT: u32 = 7;

fn token_type_index(kind: TokenKind) -> Option<u32> {
    match kind {
        // Keywords
        // Keep `type` on TextMate scopes so type signatures/declarations can
        // opt into a uniform line color without a semantic-keyword override.
        TokenKind::TypeKw => None,
        TokenKind::FuncKw
        | TokenKind::ValueKw
        | TokenKind::SignalKw
        | TokenKind::FromKw
        | TokenKind::ClassKw
        | TokenKind::InstanceKw
        | TokenKind::DomainKw
        | TokenKind::ProviderKw
        | TokenKind::UseKw
        | TokenKind::ExportKw
        | TokenKind::HoistKw
        | TokenKind::PatchKw => Some(IDX_KEYWORD),

        // Identifiers: deferred to soft_or_hard_token_type_index.
        TokenKind::Identifier => None,

        // Literals
        TokenKind::StringLiteral | TokenKind::RegexLiteral => Some(IDX_STRING),
        TokenKind::Integer | TokenKind::Float | TokenKind::Decimal | TokenKind::BigInt => {
            Some(IDX_NUMBER)
        }

        // Operators and punctuation: let TextMate grammar handle these so that
        // per-operator colors (pipe variants, arrows, etc.) are preserved.
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
        | TokenKind::Bang
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
        | TokenKind::PipeDelay
        | TokenKind::PipeBurst
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
        | TokenKind::SelfCloseTagEnd => None,

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

fn soft_or_hard_token_type_index(
    token: aivi_syntax::Token,
    source: &aivi_base::SourceFile,
) -> Option<u32> {
    match token.kind() {
        TokenKind::Identifier if token.text(source) == "when" => Some(IDX_KEYWORD),
        // Interpolated string literals need TextMate's nested scopes so the
        // interpolation braces and body can be themed independently.
        TokenKind::StringLiteral if string_literal_has_interpolation(token.text(source)) => None,
        // Let TextMate grammar handle identifier coloring — it uses specific scopes
        // (e.g. variable.parameter.labeled, variable.other.field) that carry more
        // precise color intent than a blanket `variable` semantic token.
        TokenKind::Identifier => None,
        kind => token_type_index(kind),
    }
}

fn string_literal_has_interpolation(literal: &str) -> bool {
    let mut chars = literal.chars();
    if chars.next() != Some('"') {
        return false;
    }

    let mut escaped = false;
    for ch in chars {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => return false,
            '{' => return true,
            _ => {}
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        IDX_KEYWORD, IDX_STRING, soft_or_hard_token_type_index, string_literal_has_interpolation,
        token_type_index,
    };
    use aivi_base::{FileId, SourceFile};
    use aivi_syntax::{TokenKind, lex_module};

    #[test]
    fn leaves_patch_surface_operators_to_textmate() {
        assert_eq!(token_type_index(TokenKind::PatchKw), Some(IDX_KEYWORD));
        assert_eq!(token_type_index(TokenKind::PatchApply), None);
        assert_eq!(token_type_index(TokenKind::ColonEquals), None);
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

    #[test]
    fn leaves_type_keyword_to_textmate() {
        assert_eq!(token_type_index(TokenKind::TypeKw), None);
    }

    #[test]
    fn treats_from_as_a_keyword() {
        assert_eq!(token_type_index(TokenKind::FromKw), Some(IDX_KEYWORD));
    }

    #[test]
    fn detects_unescaped_text_interpolation_holes() {
        assert!(string_literal_has_interpolation(
            r#""Final score: {game.score}""#
        ));
        assert!(!string_literal_has_interpolation(
            r#""use \{literal\} braces""#
        ));
    }

    #[test]
    fn leaves_interpolated_string_literals_to_textmate() {
        let source = SourceFile::new(
            FileId::new(0),
            "test.aivi",
            r#"value label = "Final score: {game.score}""#,
        );
        let lexed = lex_module(&source);
        let string = lexed
            .tokens()
            .iter()
            .find(|token| token.kind() == TokenKind::StringLiteral)
            .copied()
            .expect("expected a string literal token");

        assert_eq!(soft_or_hard_token_type_index(string, &source), None);
    }

    #[test]
    fn keeps_plain_string_literals_as_semantic_strings() {
        let source = SourceFile::new(
            FileId::new(0),
            "test.aivi",
            r#"value label = "use \{literal\} braces""#,
        );
        let lexed = lex_module(&source);
        let string = lexed
            .tokens()
            .iter()
            .find(|token| token.kind() == TokenKind::StringLiteral)
            .copied()
            .expect("expected a string literal token");

        assert_eq!(
            soft_or_hard_token_type_index(string, &source),
            Some(IDX_STRING)
        );
    }
}
