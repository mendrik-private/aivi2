#[derive(Clone, Copy, Debug, Default)]
struct DecoratorSearch {
    keyword: Option<usize>,
    offending: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ExprStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    rbracket: bool,
    arrow: bool,
    pipe_stage: bool,
    /// Like `pipe_stage`, but only stops at pipe operators that begin a new line.
    /// Used for pipe-case arm bodies so inline `||>` continuations are allowed
    /// (e.g. `||> [first, ...rest] -> first ||> { email } -> email`).
    pipe_stage_line_start_only: bool,
    hash: bool,
    patch_entry: bool,
}

impl ExprStop {
    fn with_pipe_stage(mut self) -> Self {
        self.pipe_stage = true;
        self
    }

    fn with_hash(mut self) -> Self {
        self.hash = true;
        self
    }

    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn list_context() -> Self {
        Self {
            comma: true,
            rbracket: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn brace_context() -> Self {
        Self {
            rbrace: true,
            ..Self::default()
        }
    }

    fn patch_entry_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            patch_entry: true,
            ..Self::default()
        }
    }
}

fn patch_selector_segment_span(segment: &PatchSelectorSegment) -> SourceSpan {
    match segment {
        PatchSelectorSegment::Named { span, .. }
        | PatchSelectorSegment::BracketTraverse { span }
        | PatchSelectorSegment::BracketExpr { span, .. } => *span,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PatternStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    rbracket: bool,
    arrow: bool,
    fat_arrow: bool,
}

impl PatternStop {
    fn arrow_context() -> Self {
        Self {
            arrow: true,
            ..Self::default()
        }
    }

    fn signal_reactive_arm_context() -> Self {
        Self {
            fat_arrow: true,
            ..Self::default()
        }
    }

    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn list_context() -> Self {
        Self {
            comma: true,
            rbracket: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn brace_context() -> Self {
        Self {
            rbrace: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TypeStop {
    comma: bool,
    rparen: bool,
    rbrace: bool,
    arrow: bool,
    thin_arrow: bool,
    pipe_transform: bool,
}

impl TypeStop {
    fn paren_context() -> Self {
        Self {
            comma: true,
            rparen: true,
            ..Self::default()
        }
    }

    fn record_context() -> Self {
        Self {
            comma: true,
            rbrace: true,
            ..Self::default()
        }
    }

    fn constraint_attempt() -> Self {
        Self {
            arrow: true,
            thin_arrow: true,
            ..Self::default()
        }
    }

    fn with_pipe_transform(mut self) -> Self {
        self.pipe_transform = true;
        self
    }
}

fn is_record_row_transform_name(name: &str) -> bool {
    matches!(
        name,
        "Pick" | "Omit" | "Optional" | "Required" | "Defaulted" | "Rename"
    )
}

fn text_escape_end(text: &str, start: usize, end: usize) -> usize {
    let mut cursor = start + 1;
    if cursor >= end {
        return cursor;
    }
    let escaped = text[cursor..end]
        .chars()
        .next()
        .expect("escaped text segment must stay on a UTF-8 boundary");
    cursor += escaped.len_utf8();
    match escaped {
        'u' => {
            let bytes = text.as_bytes();
            if cursor < end && bytes[cursor] == b'{' {
                cursor += 1;
                while cursor < end && bytes[cursor] != b'}' {
                    cursor += 1;
                }
                if cursor < end {
                    cursor += 1;
                }
            }
            cursor
        }
        'x' => {
            let bytes = text.as_bytes();
            let mut hex_digits = 0usize;
            while hex_digits < 2 && cursor < end && bytes[cursor].is_ascii_hexdigit() {
                cursor += 1;
                hex_digits += 1;
            }
            cursor
        }
        _ => cursor,
    }
}

fn domain_member_surface_name_str(name: &DomainMemberName) -> String {
    match name {
        DomainMemberName::Signature(ClassMemberName::Identifier(id)) => id.text.clone(),
        DomainMemberName::Signature(ClassMemberName::Operator(op)) => op.text.clone(),
        DomainMemberName::Literal(id) => id.text.clone(),
    }
}

fn decode_text_fragment(raw: &str) -> String {
    let mut decoded = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        let Some(escaped) = chars.next() else {
            decoded.push('\\');
            break;
        };
        match escaped {
            'n' => decoded.push('\n'),
            't' => decoded.push('\t'),
            'r' => decoded.push('\r'),
            '\\' => decoded.push('\\'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '0' => decoded.push('\0'),
            '{' => decoded.push('{'),
            '}' => decoded.push('}'),
            'u' => {
                let mut consumed = String::from("\\u");
                let Some('{') = chars.peek().copied() else {
                    decoded.push_str(&consumed);
                    continue;
                };
                consumed.push(chars.next().expect("peeked opening brace must exist"));
                let mut digits = String::new();
                let mut terminated = false;
                for next in chars.by_ref() {
                    consumed.push(next);
                    if next == '}' {
                        terminated = true;
                        break;
                    }
                    digits.push(next);
                }
                match terminated
                    .then(|| u32::from_str_radix(&digits, 16).ok())
                    .flatten()
                    .and_then(char::from_u32)
                {
                    Some(ch) => decoded.push(ch),
                    None => decoded.push_str(&consumed),
                }
            }
            'x' => {
                let mut consumed = String::from("\\x");
                let mut digits = String::new();
                for _ in 0..2 {
                    let Some(next) = chars.peek().copied() else {
                        break;
                    };
                    if !next.is_ascii_hexdigit() {
                        break;
                    }
                    digits.push(next);
                    consumed.push(next);
                    chars.next();
                }
                match u8::from_str_radix(&digits, 16).ok() {
                    Some(value) => decoded.push(char::from(value)),
                    None => decoded.push_str(&consumed),
                }
            }
            other => {
                decoded.push('\\');
                decoded.push(other);
            }
        }
    }
    decoded
}
