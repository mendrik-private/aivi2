use aivi_base::{Diagnostic, DiagnosticCode, SourceFile, Span};

const UNEXPECTED_CHARACTER: DiagnosticCode = DiagnosticCode::new("syntax", "unexpected-character");
const UNTERMINATED_STRING: DiagnosticCode = DiagnosticCode::new("syntax", "unterminated-string");
const UNTERMINATED_REGEX: DiagnosticCode = DiagnosticCode::new("syntax", "unterminated-regex");
const INVALID_ESCAPE_SEQUENCE: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-escape-sequence");

/// Token kinds required for the Milestone 1 surface grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Whitespace,
    Newline,
    /// Line comment (`// ...`), trivia.
    LineComment,
    /// Block comment (`/* ... */`), trivia.
    BlockComment,
    /// Doc comment (`/** ... **/`), trivia.
    DocComment,
    Identifier,
    Integer,
    Float,
    Decimal,
    BigInt,
    StringLiteral,
    RegexLiteral,
    At,
    Hash,
    Colon,
    Equals,
    EqualEqual,
    BangEqual,
    Ellipsis,
    DotDot,
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
    Percent,
    CloseTagStart,
    SelfCloseTagEnd,
    Arrow,
    ThinArrow,
    TypeKw,
    DataKw,
    FunKw,
    ValueKw,
    SignalKw,
    SourceKw,
    ResultDeclKw,
    ViewKw,
    AdapterKw,
    ClassKw,
    InstanceKw,
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
    PipeValidate,
    PipePrevious,
    PipeAccumulate,
    PipeDiff,
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
                | TokenKind::BlockComment
                | TokenKind::DocComment
        )
    }

    pub const fn is_top_level_keyword(self) -> bool {
        matches!(
            self,
            TokenKind::TypeKw
                | TokenKind::DataKw
                | TokenKind::FunKw
                | TokenKind::ValueKw
                | TokenKind::SignalKw
                | TokenKind::SourceKw
                | TokenKind::ResultDeclKw
                | TokenKind::ViewKw
                | TokenKind::AdapterKw
                | TokenKind::ClassKw
                | TokenKind::InstanceKw
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
                | TokenKind::PipeValidate
                | TokenKind::PipePrevious
                | TokenKind::PipeAccumulate
                | TokenKind::PipeDiff
                | TokenKind::TruthyBranch
                | TokenKind::FalsyBranch
        )
    }

    pub const fn is_keyword(self) -> bool {
        matches!(
            self,
            TokenKind::TypeKw
                | TokenKind::DataKw
                | TokenKind::FunKw
                | TokenKind::ValueKw
                | TokenKind::SignalKw
                | TokenKind::SourceKw
                | TokenKind::ResultDeclKw
                | TokenKind::ViewKw
                | TokenKind::AdapterKw
                | TokenKind::ClassKw
                | TokenKind::InstanceKw
                | TokenKind::DomainKw
                | TokenKind::ProviderKw
                | TokenKind::UseKw
                | TokenKind::ExportKw
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

        // Handle doc comments (`/** ... **/`) before block comments (`/* ... */`).
        if bytes[cursor..range.end].starts_with(b"/**") {
            let start = cursor;
            cursor += 3;
            while cursor < range.end {
                if bytes[cursor..range.end].starts_with(b"**/") {
                    cursor += 3;
                    break;
                }
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

        // Handle block comments (`/* ... */`).
        if bytes[cursor..range.end].starts_with(b"/*") {
            let start = cursor;
            cursor += 2;
            while cursor < range.end {
                if bytes[cursor..range.end].starts_with(b"*/") {
                    cursor += 2;
                    break;
                }
                cursor += 1;
            }
            tokens.push(Token::new(
                TokenKind::BlockComment,
                source.span(start..cursor),
                line_start,
            ));
            at_line_start = false;
            continue;
        }

        // Handle line comments (`//`).
        if bytes[cursor..range.end].starts_with(b"//") {
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
            let (end, terminated, invalid_escapes) =
                scan_quoted_body(text, bytes, cursor + 2, range.end, EscapeMode::Regex);
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
            for esc_offset in invalid_escapes {
                let esc_end = text[esc_offset..]
                    .chars()
                    .nth(1)
                    .map(|c| esc_offset + 1 + c.len_utf8())
                    .unwrap_or(esc_offset + 1);
                diagnostics.push(
                    Diagnostic::error(format!(
                        "invalid escape sequence `\\{}` in regex literal",
                        &text[esc_offset + 1..esc_end]
                    ))
                    .with_code(INVALID_ESCAPE_SEQUENCE)
                    .with_primary_label(
                        source.source_span(esc_offset..esc_end),
                        "unrecognised escape sequence",
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
            let kind = if cursor < range.end && bytes[cursor] == b'.' {
                let fractional_start = cursor + 1;
                if fractional_start < range.end && bytes[fractional_start].is_ascii_digit() {
                    cursor = fractional_start + 1;
                    while cursor < range.end && bytes[cursor].is_ascii_digit() {
                        cursor += 1;
                    }
                    if cursor < range.end
                        && bytes[cursor] == b'd'
                        && !starts_identifier_continue(text, cursor + 1, range.end)
                    {
                        cursor += 1;
                        TokenKind::Decimal
                    } else {
                        TokenKind::Float
                    }
                } else {
                    TokenKind::Integer
                }
            } else if cursor < range.end
                && bytes[cursor] == b'd'
                && !starts_identifier_continue(text, cursor + 1, range.end)
            {
                cursor += 1;
                TokenKind::Decimal
            } else if cursor < range.end
                && bytes[cursor] == b'n'
                && !starts_identifier_continue(text, cursor + 1, range.end)
            {
                cursor += 1;
                TokenKind::BigInt
            } else {
                TokenKind::Integer
            };
            tokens.push(Token::new(kind, source.span(start..cursor), line_start));
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
            let (end, terminated, invalid_escapes) =
                scan_quoted_body(text, bytes, cursor, range.end, EscapeMode::String);
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
            for esc_offset in invalid_escapes {
                let esc_end = text[esc_offset..]
                    .chars()
                    .nth(1)
                    .map(|c| esc_offset + 1 + c.len_utf8())
                    .unwrap_or(esc_offset + 1);
                diagnostics.push(
                    Diagnostic::error(format!(
                        "invalid escape sequence `\\{}` in string literal",
                        &text[esc_offset + 1..esc_end]
                    ))
                    .with_code(INVALID_ESCAPE_SEQUENCE)
                    .with_primary_label(
                        source.source_span(esc_offset..esc_end),
                        "unrecognised escape sequence",
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
            '%' => TokenKind::Percent,
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
    // Patterns ordered longest-first so no short prefix shadows a longer match.
    const PATTERNS: [(&[u8], TokenKind); 21] = [
        (b"<|@", TokenKind::PipeRecurStep),
        (b"<|*", TokenKind::PipeFanIn),
        (b"</", TokenKind::CloseTagStart),
        (b"/>", TokenKind::SelfCloseTagEnd),
        (b"@|>", TokenKind::PipeRecurStart),
        (b"&|>", TokenKind::PipeApply),
        (b"||>", TokenKind::PipeCase),
        (b"?|>", TokenKind::PipeGate),
        (b"*|>", TokenKind::PipeMap),
        (b"!|>", TokenKind::PipeValidate),
        (b"~|>", TokenKind::PipePrevious),
        (b"+|>", TokenKind::PipeAccumulate),
        (b"-|>", TokenKind::PipeDiff),
        (b"T|>", TokenKind::TruthyBranch),
        (b"F|>", TokenKind::FalsyBranch),
        (b"...", TokenKind::Ellipsis),
        (b"..", TokenKind::DotDot),
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
        "data" => Some(TokenKind::DataKw),
        "fun" => Some(TokenKind::FunKw),
        "value" => Some(TokenKind::ValueKw),
        "signal" => Some(TokenKind::SignalKw),
        "source" => Some(TokenKind::SourceKw),
        "result" => Some(TokenKind::ResultDeclKw),
        "view" => Some(TokenKind::ViewKw),
        "adapter" => Some(TokenKind::AdapterKw),
        "class" => Some(TokenKind::ClassKw),
        "instance" => Some(TokenKind::InstanceKw),
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

fn starts_identifier_continue(text: &str, cursor: usize, end: usize) -> bool {
    if cursor >= end {
        return false;
    }
    text[cursor..]
        .chars()
        .next()
        .map(is_identifier_continue)
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscapeMode {
    String,
    Regex,
}

/// Scans a quoted body starting at `start` (which must point at the opening
/// `"`). Returns `(end_cursor, terminated, invalid_escape_offsets)`.
/// `invalid_escape_offsets` contains the byte offset of each `\` that begins
/// an unrecognised escape sequence for the selected `escape_mode`.
fn scan_quoted_body(
    text: &str,
    bytes: &[u8],
    start: usize,
    end: usize,
    escape_mode: EscapeMode,
) -> (usize, bool, Vec<usize>) {
    let mut cursor = start;
    let mut terminated = false;
    let mut invalid_escapes: Vec<usize> = Vec::new();

    if cursor >= bytes.len() || bytes[cursor] != b'"' {
        return (cursor, false, invalid_escapes);
    }

    cursor += 1;
    while cursor < end {
        let next = text[cursor..]
            .chars()
            .next()
            .expect("quoted scan must stay on a UTF-8 boundary");
        match next {
            '\\' => {
                let backslash_pos = cursor;
                cursor += 1;
                if cursor < end {
                    let escaped = text[cursor..]
                        .chars()
                        .next()
                        .expect("escaped codepoint must stay on a UTF-8 boundary");
                    match escaped {
                        'n' | 't' | 'r' | '\\' | '"' | '\'' | '0' => {
                            cursor += 1;
                        }
                        '{' | '}' if escape_mode == EscapeMode::String => {
                            cursor += 1;
                        }
                        'u' => {
                            // \u{XXXX} unicode escape
                            cursor += 1;
                            if cursor < end && bytes[cursor] == b'{' {
                                cursor += 1;
                                while cursor < end && bytes[cursor] != b'}' {
                                    cursor += 1;
                                }
                                if cursor < end {
                                    cursor += 1; // consume `}`
                                }
                            } else {
                                invalid_escapes.push(backslash_pos);
                            }
                        }
                        'x' => {
                            // \xNN hex escape — consume up to two hex digits
                            cursor += 1;
                            let mut hex_digits = 0usize;
                            while hex_digits < 2
                                && cursor < end
                                && bytes[cursor].is_ascii_hexdigit()
                            {
                                cursor += 1;
                                hex_digits += 1;
                            }
                            if hex_digits == 0 {
                                invalid_escapes.push(backslash_pos);
                            }
                        }
                        _ => {
                            if escape_mode == EscapeMode::Regex {
                                cursor += escaped.len_utf8();
                            } else {
                                // Unrecognised escape — record position, skip
                                // the character so we don't stall the lexer.
                                invalid_escapes.push(backslash_pos);
                                cursor += escaped.len_utf8();
                            }
                        }
                    }
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

    (cursor, terminated, invalid_escapes)
}
