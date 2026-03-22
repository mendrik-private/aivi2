use aivi_base::{Diagnostic, DiagnosticCode, SourceFile, Span};

const UNEXPECTED_CHARACTER: DiagnosticCode = DiagnosticCode::new("syntax", "unexpected-character");
const UNTERMINATED_STRING: DiagnosticCode = DiagnosticCode::new("syntax", "unterminated-string");
const UNTERMINATED_REGEX: DiagnosticCode = DiagnosticCode::new("syntax", "unterminated-regex");

/// Token kinds required for the Milestone 1 surface grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    Newline,
    /// Line comment (`-- ...`), trivia.
    LineComment,
    /// Doc comment (`--- ...`), trivia.
    DocComment,
    Identifier,
    Integer,
    StringLiteral,
    RegexLiteral,
    At,
    Hash,
    Colon,
    Equals,
    EqualEqual,
    BangEqual,
    Dot,
    Comma,
    Plus,
    Minus,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Less,
    Greater,
    Slash,
    CloseTagStart,
    SelfCloseTagEnd,
    Arrow,
    ThinArrow,
    TypeKw,
    ValKw,
    FunKw,
    SigKw,
    ClassKw,
    DomainKw,
    ProviderKw,
    UseKw,
    ExportKw,
    Star,
    PipeTransform,
    PipeGate,
    PipeCase,
    PipeMap,
    PipeApply,
    PipeRecurStart,
    PipeRecurStep,
    PipeTap,
    PipeFanIn,
    TruthyBranch,
    FalsyBranch,
    Unknown,
}

impl TokenKind {
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::LineComment
                | TokenKind::DocComment
        )
    }

    pub const fn is_top_level_keyword(self) -> bool {
        matches!(
            self,
            TokenKind::TypeKw
                | TokenKind::ValKw
                | TokenKind::FunKw
                | TokenKind::SigKw
                | TokenKind::ClassKw
                | TokenKind::DomainKw
                | TokenKind::ProviderKw
                | TokenKind::UseKw
                | TokenKind::ExportKw
        )
    }

    pub const fn is_pipe_operator(self) -> bool {
        matches!(
            self,
            TokenKind::PipeTransform
                | TokenKind::PipeGate
                | TokenKind::PipeCase
                | TokenKind::PipeMap
                | TokenKind::PipeApply
                | TokenKind::PipeRecurStart
                | TokenKind::PipeRecurStep
                | TokenKind::PipeTap
                | TokenKind::PipeFanIn
                | TokenKind::TruthyBranch
                | TokenKind::FalsyBranch
        )
    }
}

/// Single lexed token with source span and line-start marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    kind: TokenKind,
    span: Span,
    line_start: bool,
}

impl Token {
    pub const fn new(kind: TokenKind, span: Span, line_start: bool) -> Self {
        Self {
            kind,
            span,
            line_start,
        }
    }

    pub const fn kind(self) -> TokenKind {
        self.kind
    }

    pub const fn span(self) -> Span {
        self.span
    }

    pub const fn line_start(self) -> bool {
        self.line_start
    }

    pub fn text<'a>(self, source: &'a SourceFile) -> &'a str {
        source.slice(self.span)
    }
}

/// Lossless token buffer plus lexical diagnostics.
#[derive(Clone, Debug, Default)]
pub struct LexedModule {
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl LexedModule {
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    pub fn replay(&self, source: &SourceFile) -> String {
        let mut rendered = String::with_capacity(source.text().len());
        for token in &self.tokens {
            rendered.push_str(source.slice(token.span()));
        }
        rendered
    }
}

pub fn lex_module(source: &SourceFile) -> LexedModule {
    lex_range(source, 0..source.len())
}

pub(crate) fn lex_fragment(source: &SourceFile, range: std::ops::Range<usize>) -> LexedModule {
    lex_range(source, range)
}

fn lex_range(source: &SourceFile, range: std::ops::Range<usize>) -> LexedModule {
    let text = source.text();
    let bytes = text.as_bytes();
    let mut cursor = range.start;
    let mut at_line_start = range.start == 0
        || bytes
            .get(range.start.saturating_sub(1))
            .is_some_and(|byte| *byte == b'\n');
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();

    while cursor < range.end {
        if bytes[cursor] == b'\n' {
            tokens.push(Token::new(
                TokenKind::Newline,
                source.span(cursor..cursor + 1),
                false,
            ));
            cursor += 1;
            at_line_start = true;
            continue;
        }

        if matches!(bytes[cursor], b' ' | b'\t' | b'\r') {
            let start = cursor;
            while cursor < range.end && matches!(bytes[cursor], b' ' | b'\t' | b'\r') {
                cursor += 1;
            }
            tokens.push(Token::new(
                TokenKind::Whitespace,
                source.span(start..cursor),
                false,
            ));
            continue;
        }

        let line_start = at_line_start;

        // Handle doc comments (`---`) before regular comments (`--`).
        if bytes[cursor..range.end].starts_with(b"---") {
            let start = cursor;
            while cursor < range.end && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            tokens.push(Token::new(
                TokenKind::DocComment,
                source.span(start..cursor),
                line_start,
            ));
            at_line_start = false;
            continue;
        }

        if bytes[cursor..range.end].starts_with(b"--") {
            let start = cursor;
            while cursor < range.end && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            tokens.push(Token::new(
                TokenKind::LineComment,
                source.span(start..cursor),
                line_start,
            ));
            at_line_start = false;
            continue;
        }

        if bytes[cursor..range.end].starts_with(b"rx\"") {
            let start = cursor;
            let (end, terminated) = scan_quoted_body(text, bytes, cursor + 2, range.end);
            cursor = end;
            tokens.push(Token::new(
                TokenKind::RegexLiteral,
                source.span(start..cursor),
                line_start,
            ));
            if !terminated {
                diagnostics.push(
                    Diagnostic::error(
                        "regex literal is not terminated before the end of the line or file",
                    )
                    .with_code(UNTERMINATED_REGEX)
                    .with_primary_label(
                        source.source_span(start..cursor),
                        "expected a closing `\"`",
                    ),
                );
            }
            at_line_start = false;
            continue;
        }

        if let Some((kind, width)) = match_compound(bytes, cursor, range.end) {
            tokens.push(Token::new(
                kind,
                source.span(cursor..cursor + width),
                line_start,
            ));
            cursor += width;
            at_line_start = false;
            continue;
        }

        let character = text[cursor..]
            .chars()
            .next()
            .expect("cursor must stay on a UTF-8 boundary");

        if character.is_ascii_digit() {
            let start = cursor;
            cursor += character.len_utf8();
            while cursor < range.end && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            tokens.push(Token::new(
                TokenKind::Integer,
                source.span(start..cursor),
                line_start,
            ));
            at_line_start = false;
            continue;
        }

        if is_identifier_start(character) {
            let start = cursor;
            cursor += character.len_utf8();
            while cursor < range.end {
                let next = text[cursor..]
                    .chars()
                    .next()
                    .expect("identifier scan must stay on a UTF-8 boundary");
                if is_identifier_continue(next) {
                    cursor += next.len_utf8();
                } else {
                    break;
                }
            }
            let slice = &text[start..cursor];
            let kind = keyword_kind(slice).unwrap_or(TokenKind::Identifier);
            tokens.push(Token::new(kind, source.span(start..cursor), line_start));
            at_line_start = false;
            continue;
        }

        if character == '"' {
            let start = cursor;
            let (end, terminated) = scan_quoted_body(text, bytes, cursor, range.end);
            cursor = end;
            tokens.push(Token::new(
                TokenKind::StringLiteral,
                source.span(start..cursor),
                line_start,
            ));
            if !terminated {
                diagnostics.push(
                    Diagnostic::error(
                        "string literal is not terminated before the end of the line or file",
                    )
                    .with_code(UNTERMINATED_STRING)
                    .with_primary_label(
                        source.source_span(start..cursor),
                        "expected a closing `\"`",
                    ),
                );
            }
            at_line_start = false;
            continue;
        }

        let kind = match character {
            '@' => TokenKind::At,
            '#' => TokenKind::Hash,
            ':' => TokenKind::Colon,
            '=' => TokenKind::Equals,
            '.' => TokenKind::Dot,
            ',' => TokenKind::Comma,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            '<' => TokenKind::Less,
            '>' => TokenKind::Greater,
            '/' => TokenKind::Slash,
            '|' => TokenKind::PipeTap,
            _ => TokenKind::Unknown,
        };
        let next_cursor = cursor + character.len_utf8();
        tokens.push(Token::new(
            kind,
            source.span(cursor..next_cursor),
            line_start,
        ));
        if kind == TokenKind::Unknown {
            diagnostics.push(
                Diagnostic::error(format!("unexpected character `{character}` in source text"))
                    .with_code(UNEXPECTED_CHARACTER)
                    .with_primary_label(
                        source.source_span(cursor..next_cursor),
                        "this character is outside the Milestone 1 token set",
                    ),
            );
        }
        cursor = next_cursor;
        at_line_start = false;
    }

    LexedModule {
        tokens,
        diagnostics,
    }
}

fn match_compound(bytes: &[u8], cursor: usize, end: usize) -> Option<(TokenKind, usize)> {
    const PATTERNS: [(&[u8], TokenKind); 15] = [
        (b"<|@", TokenKind::PipeRecurStep),
        (b"<|*", TokenKind::PipeFanIn),
        (b"</", TokenKind::CloseTagStart),
        (b"/>", TokenKind::SelfCloseTagEnd),
        (b"@|>", TokenKind::PipeRecurStart),
        (b"&|>", TokenKind::PipeApply),
        (b"||>", TokenKind::PipeCase),
        (b"?|>", TokenKind::PipeGate),
        (b"*|>", TokenKind::PipeMap),
        (b"T|>", TokenKind::TruthyBranch),
        (b"F|>", TokenKind::FalsyBranch),
        (b"->", TokenKind::ThinArrow),
        (b"!=", TokenKind::BangEqual),
        (b"==", TokenKind::EqualEqual),
        (b"|>", TokenKind::PipeTransform),
    ];

    let fragment = &bytes[cursor..end];
    for (pattern, kind) in PATTERNS {
        if fragment.starts_with(pattern) {
            return Some((kind, pattern.len()));
        }
    }

    fragment.starts_with(b"=>").then_some((TokenKind::Arrow, 2))
}

fn keyword_kind(text: &str) -> Option<TokenKind> {
    match text {
        "type" => Some(TokenKind::TypeKw),
        "val" => Some(TokenKind::ValKw),
        "fun" => Some(TokenKind::FunKw),
        "sig" => Some(TokenKind::SigKw),
        "class" => Some(TokenKind::ClassKw),
        "domain" => Some(TokenKind::DomainKw),
        "provider" => Some(TokenKind::ProviderKw),
        "use" => Some(TokenKind::UseKw),
        "export" => Some(TokenKind::ExportKw),
        _ => None,
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    is_identifier_start(character) || character.is_ascii_digit()
}

fn scan_quoted_body(text: &str, bytes: &[u8], start: usize, end: usize) -> (usize, bool) {
    let mut cursor = start;
    let mut terminated = false;

    if cursor >= bytes.len() || bytes[cursor] != b'"' {
        return (cursor, false);
    }

    cursor += 1;
    while cursor < end {
        let next = text[cursor..]
            .chars()
            .next()
            .expect("quoted scan must stay on a UTF-8 boundary");
        match next {
            '\\' => {
                cursor += 1;
                if cursor < end {
                    let escaped = text[cursor..]
                        .chars()
                        .next()
                        .expect("escaped codepoint must stay on a UTF-8 boundary");
                    cursor += escaped.len_utf8();
                }
            }
            '"' => {
                cursor += 1;
                terminated = true;
                break;
            }
            '\n' => break,
            _ => cursor += next.len_utf8(),
        }
    }

    (cursor, terminated)
}
