impl<'a> Parser<'a> {
    fn make_apply_expr(&self, callee: Expr, argument: Expr) -> Expr {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            ExprKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                Expr {
                    span,
                    kind: ExprKind::Apply { callee, arguments },
                }
            }
            kind => Expr {
                span,
                kind: ExprKind::Apply {
                    callee: Box::new(Expr {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn make_binary_expr(&self, left: Expr, operator: BinaryOperator, right: Expr) -> Expr {
        Expr {
            span: self.join_spans(left.span, right.span),
            kind: ExprKind::Binary {
                left: Box::new(left),
                operator,
                right: Box::new(right),
            },
        }
    }

    fn make_type_apply(&self, callee: TypeExpr, argument: TypeExpr) -> TypeExpr {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            TypeExprKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                TypeExpr {
                    span,
                    kind: TypeExprKind::Apply { callee, arguments },
                }
            }
            kind => TypeExpr {
                span,
                kind: TypeExprKind::Apply {
                    callee: Box::new(TypeExpr {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn make_type_arrow(&self, parameter: TypeExpr, result: TypeExpr) -> TypeExpr {
        TypeExpr {
            span: self.join_spans(parameter.span, result.span),
            kind: TypeExprKind::Arrow {
                parameter: Box::new(parameter),
                result: Box::new(result),
            },
        }
    }

    fn make_pattern_apply(&self, callee: Pattern, argument: Pattern) -> Pattern {
        let span = self.join_spans(callee.span, argument.span);
        match callee.kind {
            PatternKind::Apply {
                callee,
                mut arguments,
            } => {
                arguments.push(argument);
                Pattern {
                    span,
                    kind: PatternKind::Apply { callee, arguments },
                }
            }
            kind => Pattern {
                span,
                kind: PatternKind::Apply {
                    callee: Box::new(Pattern {
                        span: callee.span,
                        kind,
                    }),
                    arguments: vec![argument],
                },
            },
        }
    }

    fn binary_operator(&self, index: usize) -> Option<(BinaryOperator, u8)> {
        match self.tokens[index].kind() {
            TokenKind::Plus => Some((BinaryOperator::Add, 4)),
            TokenKind::Minus => Some((BinaryOperator::Subtract, 4)),
            TokenKind::Star => Some((BinaryOperator::Multiply, 5)),
            TokenKind::Slash => Some((BinaryOperator::Divide, 5)),
            TokenKind::Percent => Some((BinaryOperator::Modulo, 5)),
            TokenKind::Greater => Some((BinaryOperator::GreaterThan, 3)),
            TokenKind::Less => Some((BinaryOperator::LessThan, 3)),
            TokenKind::GreaterEqual => Some((BinaryOperator::GreaterThanOrEqual, 3)),
            TokenKind::LessEqual => Some((BinaryOperator::LessThanOrEqual, 3)),
            TokenKind::EqualEqual => Some((BinaryOperator::Equals, 3)),
            TokenKind::BangEqual => Some((BinaryOperator::NotEquals, 3)),
            TokenKind::Identifier if self.is_identifier_text(index, "and") => {
                Some((BinaryOperator::And, 2))
            }
            TokenKind::Identifier if self.is_identifier_text(index, "or") => {
                Some((BinaryOperator::Or, 1))
            }
            _ => None,
        }
    }

    fn starts_expr(&self, index: usize) -> bool {
        let kind = self.tokens[index].kind();
        if kind == TokenKind::Minus {
            return self
                .negative_numeric_literal_token(index, self.tokens.len())
                .is_some();
        }
        kind.is_keyword()
            || matches!(
                kind,
                TokenKind::Identifier
                    | TokenKind::Integer
                    | TokenKind::Float
                    | TokenKind::Decimal
                    | TokenKind::BigInt
                    | TokenKind::StringLiteral
                    | TokenKind::RegexLiteral
                    | TokenKind::Dot
                    | TokenKind::LParen
                    | TokenKind::LBracket
                    | TokenKind::LBrace
                    | TokenKind::Less
            )
    }

    fn starts_function_param(&self, start: usize, end: usize) -> bool {
        self.peek_nontrivia(start, end).is_some_and(|index| {
            !self.tokens[index].line_start() && self.is_function_param_token(index)
        })
    }

    fn find_last_same_line_arrow(&self, start: usize, end: usize) -> Option<usize> {
        let mut scan = start;
        let mut found = None;
        let mut saw_token = false;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            if self.tokens[index].kind() == TokenKind::Arrow {
                found = Some(index);
            }
            scan = index + 1;
        }
        found
    }

    fn find_same_line_top_level_equals(&self, start: usize, end: usize) -> Option<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return Some(index),
                _ => {}
            }
            scan = index + 1;
        }
        None
    }

    fn find_top_level_equals(&self, start: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut scan = start;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return Some(index),
                _ => {}
            }
            scan = index + 1;
        }
        None
    }

    fn parameter_annotation_end(&self, start: usize, end: usize) -> usize {
        let Some(body_arrow) = self.find_last_same_line_arrow(start, end) else {
            return end;
        };
        self.same_line_top_level_typed_param_starts(start, body_arrow)
            .into_iter()
            .next()
            .unwrap_or(body_arrow)
    }

    fn function_signature_split_candidates(&self, start: usize, body_arrow: usize) -> Vec<usize> {
        let mut candidates = self.same_line_top_level_param_starts(start, body_arrow);
        if !candidates.contains(&body_arrow) {
            candidates.push(body_arrow);
        }
        candidates
    }

    fn same_line_top_level_param_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        let mut starts = Vec::new();
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                _ if depth == 0 && self.is_function_param_token(index) => starts.push(index),
                _ => {}
            }
            scan = index + 1;
        }
        starts
    }

    fn same_line_top_level_typed_param_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut scan = start;
        let mut saw_token = false;
        let mut depth = 0usize;
        let mut starts = Vec::new();
        while let Some(index) = self.peek_nontrivia(scan, end) {
            if saw_token && self.tokens[index].line_start() {
                break;
            }
            saw_token = true;
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Identifier
                    if depth == 0 && self.peek_kind(index + 1, end) == Some(TokenKind::Colon) =>
                {
                    starts.push(index);
                }
                kind if kind.is_keyword()
                    && depth == 0
                    && self.peek_kind(index + 1, end) == Some(TokenKind::Colon) =>
                {
                    starts.push(index);
                }
                _ => {}
            }
            scan = index + 1;
        }
        starts
    }

    fn probe_function_signature(&self, split: usize, start: usize, body_arrow: usize) -> bool {
        let mut probe = Parser::new(self.source, self.tokens);
        probe.depth = self.depth;

        let mut annotation_cursor = start;
        let (_, annotation) =
            probe.parse_optional_signature_annotation(&mut annotation_cursor, split);
        if annotation.is_none()
            || probe
                .next_significant_in_range(annotation_cursor, split)
                .is_some()
            || probe
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            return false;
        }

        let mut parameter_cursor = split;
        let parameter_end = body_arrow.saturating_add(1);
        while probe.starts_function_param(parameter_cursor, body_arrow) {
            if probe
                .parse_function_param(&mut parameter_cursor, parameter_end)
                .is_none()
            {
                return false;
            }
        }
        probe
            .next_significant_in_range(parameter_cursor, body_arrow)
            .is_none()
            && !probe
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    fn is_function_param_token(&self, index: usize) -> bool {
        match self.tokens[index].kind() {
            kind if kind.is_keyword() => true,
            TokenKind::Identifier => {
                let identifier = self.identifier_from_token(index);
                !identifier.is_uppercase_initial()
            }
            _ => false,
        }
    }

    fn has_top_level_equals(&self, start: usize, end: usize) -> bool {
        let mut depth = 0usize;
        let mut scan = start;
        while let Some(index) = self.peek_nontrivia(scan, end) {
            match self.tokens[index].kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Equals if depth == 0 => return true,
                _ => {}
            }
            scan = index + 1;
        }
        false
    }

    fn pending_type_annotation(item: &Item) -> Option<PendingTypeAnnotation> {
        let Item::Type(item) = item else {
            return None;
        };
        if item.name.is_some() || item.body.is_some() {
            return None;
        }
        Some(PendingTypeAnnotation {
            span: item.base.span,
            constraints: item.constraints.clone(),
            annotation: item.annotation.clone()?,
        })
    }

    fn apply_pending_type_annotation(&mut self, item: &mut Item, pending: PendingTypeAnnotation) {
        match item {
            Item::Fun(named) | Item::Value(named) | Item::Signal(named) => {
                if !named.constraints.is_empty() || named.annotation.is_some() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "standalone `type` annotations cannot be combined with inline annotations",
                        )
                        .with_code(DUPLICATE_STANDALONE_TYPE_ANNOTATION)
                        .with_primary_label(
                            pending.span,
                            "remove this standalone `type` line",
                        )
                        .with_secondary_label(
                            named.name
                                .as_ref()
                                .map(|name| name.span)
                                .unwrap_or(named.keyword_span),
                            "this declaration already carries its own annotation",
                        ),
                    );
                    return;
                }
                named.constraints = pending.constraints;
                named.annotation = Some(pending.annotation);
            }
            _ => self.emit_orphan_standalone_type_annotation(&pending, Some(item.span())),
        }
    }

    fn apply_pending_from_type_annotation(
        &mut self,
        entry: &mut FromEntry,
        pending: PendingTypeAnnotation,
    ) {
        if !entry.constraints.is_empty() || entry.annotation.is_some() {
            self.diagnostics.push(
                Diagnostic::error(
                    "standalone `type` annotations inside `from` blocks cannot be combined with another attached annotation",
                )
                .with_code(DUPLICATE_STANDALONE_TYPE_ANNOTATION)
                .with_primary_label(pending.span, "remove this standalone `type` line")
                .with_secondary_label(entry.span, "this derived entry already carries an annotation"),
            );
            return;
        }
        entry.constraints = pending.constraints;
        entry.annotation = Some(pending.annotation);
    }

    fn emit_orphan_standalone_type_annotation(
        &mut self,
        pending: &PendingTypeAnnotation,
        next_item_span: Option<SourceSpan>,
    ) {
        let diagnostic = Diagnostic::error(
            "standalone `type` annotations must attach to the immediately following `func`, `value`, or `signal` declaration",
        )
        .with_code(ORPHAN_STANDALONE_TYPE_ANNOTATION)
        .with_primary_label(
            pending.span,
            "this `type` line is not attached to a supported declaration",
        );
        self.diagnostics.push(if let Some(span) = next_item_span {
            diagnostic.with_secondary_label(
                span,
                "only the immediately following `func`, `value`, or `signal` can receive a standalone annotation",
            )
        } else {
            diagnostic
        });
    }

    fn emit_orphan_from_type_annotation(
        &mut self,
        pending: &PendingTypeAnnotation,
        next_item_span: Option<SourceSpan>,
    ) {
        let diagnostic = Diagnostic::error(
            "standalone `type` annotations inside `from` blocks must attach to the immediately following derived entry",
        )
        .with_code(ORPHAN_FROM_TYPE_ANNOTATION)
        .with_primary_label(
            pending.span,
            "this `type` line is not attached to a derived entry in the same `from` block",
        );
        self.diagnostics.push(if let Some(span) = next_item_span {
            diagnostic.with_secondary_label(
                span,
                "only the immediately following derived entry can receive this `type` line",
            )
        } else {
            diagnostic
        });
    }

    fn starts_pattern(&self, index: usize) -> bool {
        let kind = self.tokens[index].kind();
        if kind == TokenKind::Minus {
            return self
                .negative_integer_literal_token(index, self.tokens.len())
                .is_some();
        }
        matches!(
            kind,
            TokenKind::Identifier
                | TokenKind::Integer
                | TokenKind::StringLiteral
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
        ) || kind.is_keyword()
    }

    fn starts_type_atom(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::LBrace
        )
    }

    fn expr_should_stop(&self, index: usize, stop: ExprStop) -> bool {
        if stop.patch_entry
            && self.tokens[index].line_start()
            && self.token_starts_patch_selector(index)
        {
            return true;
        }
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::RBracket => stop.rbracket,
            TokenKind::Arrow => stop.arrow,
            TokenKind::Hash => stop.hash,
            kind if kind.is_pipe_operator() => {
                stop.pipe_stage
                    || (stop.pipe_stage_line_start_only && self.tokens[index].line_start())
            }
            _ => false,
        }
    }

    fn token_starts_patch_selector(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Dot | TokenKind::LBracket | TokenKind::Identifier
        ) || self.tokens[index].kind().is_keyword()
    }

    fn consume_constraint_separator(&self, cursor: &mut usize, end: usize) -> bool {
        self.consume_kind(cursor, end, TokenKind::Arrow).is_some()
    }

    fn negative_numeric_literal_token(&self, minus_index: usize, end: usize) -> Option<usize> {
        if self.tokens.get(minus_index)?.kind() != TokenKind::Minus {
            return None;
        }
        let literal_index = self.peek_nontrivia(minus_index + 1, end)?;
        matches!(
            self.tokens[literal_index].kind(),
            TokenKind::Integer | TokenKind::Float | TokenKind::Decimal | TokenKind::BigInt
        )
        .then_some(())?;
        self.tokens_are_adjacent(minus_index, literal_index)
            .then_some(literal_index)
    }

    fn negative_integer_literal_token(&self, minus_index: usize, end: usize) -> Option<usize> {
        let literal_index = self.negative_numeric_literal_token(minus_index, end)?;
        (self.tokens[literal_index].kind() == TokenKind::Integer).then_some(literal_index)
    }

    fn numeric_literal_raw(&self, minus_index: Option<usize>, index: usize) -> String {
        let raw = self.tokens[index].text(self.source);
        match minus_index {
            Some(_) => format!("-{raw}"),
            None => raw.to_owned(),
        }
    }

    fn numeric_literal_span(&self, minus_index: Option<usize>, index: usize) -> SourceSpan {
        match minus_index {
            Some(start) => self.source_span_for_range(start, index + 1),
            None => self.source_span_of_token(index),
        }
    }

    fn reject_invalid_markup_child_content(
        &mut self,
        cursor: &mut usize,
        end: usize,
        parent: &QualifiedName,
    ) {
        let Some(start) = self.peek_nontrivia(*cursor, end) else {
            return;
        };
        let mut next = start + 1;
        while let Some(index) = self.peek_nontrivia(next, end) {
            match self.tokens[index].kind() {
                TokenKind::CloseTagStart | TokenKind::Less => break,
                _ => next = index + 1,
            }
        }
        self.diagnostics.push(
            Diagnostic::error(format!(
                "markup children inside `<{}>` must be provided through attributes",
                parent.as_dotted()
            ))
            .with_code(INVALID_MARKUP_CHILD_CONTENT)
            .with_primary_label(
                self.source_span_for_range(start, next),
                "use an attribute such as `text={...}` instead of child text or interpolation",
            ),
        );
        *cursor = next;
    }

    fn pattern_should_stop(&self, index: usize, stop: PatternStop) -> bool {
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::RBracket => stop.rbracket,
            // Pattern arms use `->` (ThinArrow); `=>` (Arrow) is for lambdas/function bodies.
            TokenKind::ThinArrow => stop.arrow,
            TokenKind::Arrow => stop.fat_arrow,
            _ => false,
        }
    }

    fn type_should_stop(&self, index: usize, stop: TypeStop) -> bool {
        match self.tokens[index].kind() {
            TokenKind::Comma => stop.comma,
            TokenKind::RParen => stop.rparen,
            TokenKind::RBrace => stop.rbrace,
            TokenKind::Arrow => stop.arrow,
            TokenKind::ThinArrow => stop.thin_arrow,
            TokenKind::PipeTransform => stop.pipe_transform,
            _ => false,
        }
    }

    fn missing_body_diagnostic(&mut self, keyword_index: usize, message: &str, label: &str) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(MISSING_DECLARATION_BODY)
                .with_primary_label(self.source_span_of_token(keyword_index), label)
                .with_help("expected `=` followed by the function body"),
        );
    }

    fn parse_expression_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
        missing_message: &str,
        missing_label: &str,
    ) -> Option<NamedItemBody> {
        let expr = self
            .parse_expr(cursor, end, ExprStop::default())
            .or_else(|| {
                self.missing_body_diagnostic(keyword_index, missing_message, missing_label);
                None
            })?;
        self.finish_expression_body(cursor, end, declaration_name, expr)
    }

    fn finish_expression_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
        expr: Expr,
    ) -> Option<NamedItemBody> {
        if let Some(trailing_index) = self.next_significant_in_range(*cursor, end) {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "{declaration_name} body must contain exactly one expression"
                ))
                .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                .with_primary_label(
                    self.source_span_of_token(trailing_index),
                    "this token is outside the declaration body",
                ),
            );
        }
        Some(NamedItemBody::Expr(expr))
    }

    fn text_literal_from_token(&mut self, index: usize) -> TextLiteral {
        let span = self.source_span_of_token(index);
        let raw = self.tokens[index].text(self.source);
        self.parse_text_literal(raw, span)
    }

    fn parse_text_literal(&mut self, raw: &str, span: SourceSpan) -> TextLiteral {
        let start = span.span().start().as_usize();
        let end = span.span().end().as_usize();
        let body_start = if raw.starts_with('"') {
            start + 1
        } else {
            start
        };
        let body_end = if raw.ends_with('"') && end > body_start {
            end - 1
        } else {
            end
        };

        let mut segments = Vec::new();
        let mut cursor = body_start;
        let mut fragment_start = body_start;
        let text = self.source.text();

        while cursor < body_end {
            let next = text[cursor..body_end]
                .chars()
                .next()
                .expect("text literal scan must stay on a UTF-8 boundary");
            match next {
                '\\' => {
                    cursor = text_escape_end(text, cursor, body_end);
                }
                '{' => {
                    self.push_text_fragment(&mut segments, fragment_start, cursor, false);
                    let Some(close_start) = self.find_text_interpolation_close(cursor, body_end)
                    else {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "text interpolation is not terminated before the end of the string literal",
                            )
                            .with_code(UNTERMINATED_TEXT_INTERPOLATION)
                            .with_primary_label(
                                SourceSpan::new(self.source.id(), Span::from(cursor..body_end)),
                                "expected a closing `}` for this interpolation",
                            )
                            .with_help("close the interpolation with `}`"),
                        );
                        let allow_empty = segments.is_empty();
                        self.push_text_fragment(&mut segments, cursor, body_end, allow_empty);
                        fragment_start = body_end;
                        break;
                    };
                    let interpolation_end = close_start + 1;
                    let interpolation_span =
                        SourceSpan::new(self.source.id(), Span::from(cursor..interpolation_end));
                    let expr_range = cursor + 1..close_start;
                    if self.source.text()[expr_range.clone()].trim().is_empty() {
                        self.diagnostics.push(
                            Diagnostic::error("text interpolation must contain an expression")
                                .with_code(INVALID_TEXT_INTERPOLATION)
                                .with_primary_label(
                                    interpolation_span,
                                    "add an expression between `{` and `}`",
                                )
                                .with_help(
                                    "text interpolation expects an expression inside `\\{...}`",
                                ),
                        );
                        self.push_text_fragment(&mut segments, cursor, interpolation_end, false);
                    } else if let Some(expr) =
                        self.parse_text_interpolation_expr(expr_range, interpolation_span)
                    {
                        segments.push(TextSegment::Interpolation(TextInterpolation {
                            expr: Box::new(expr),
                            span: interpolation_span,
                        }));
                    } else {
                        self.push_text_fragment(&mut segments, cursor, interpolation_end, false);
                    }
                    cursor = interpolation_end;
                    fragment_start = interpolation_end;
                }
                _ => cursor += next.len_utf8(),
            }
        }

        let allow_empty = segments.is_empty();
        self.push_text_fragment(&mut segments, fragment_start, body_end, allow_empty);
        TextLiteral { span, segments }
    }

    fn push_text_fragment(
        &self,
        segments: &mut Vec<TextSegment>,
        start: usize,
        end: usize,
        allow_empty: bool,
    ) {
        if start == end && !allow_empty {
            return;
        }
        let span = self.source.source_span(start..end);
        segments.push(TextSegment::Text(TextFragment {
            raw: decode_text_fragment(self.source.slice(span.span())),
            span,
        }));
    }

    fn find_text_interpolation_close(&self, open_brace: usize, body_end: usize) -> Option<usize> {
        let lexed = lex_fragment(self.source, open_brace + 1..body_end);
        let mut brace_depth = 0usize;
        for token in lexed.tokens() {
            match token.kind() {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => {
                    if brace_depth == 0 {
                        return Some(token.span().start().as_usize());
                    }
                    brace_depth -= 1;
                }
                _ => {}
            }
        }
        None
    }

    fn parse_text_interpolation_expr(
        &mut self,
        range: std::ops::Range<usize>,
        interpolation_span: SourceSpan,
    ) -> Option<Expr> {
        let lexed = lex_fragment(self.source, range);
        self.diagnostics.extend(lexed.diagnostics().iter().cloned());

        let mut parser = Parser::new(self.source, lexed.tokens());
        parser.depth = self.depth;
        let mut cursor = 0usize;
        let expr = parser.parse_expr(&mut cursor, lexed.tokens().len(), ExprStop::default());
        let trailing = parser.next_significant_from(cursor);
        self.diagnostics.extend(parser.diagnostics);

        match expr {
            Some(expr) if trailing.is_none() => Some(expr),
            Some(_) => {
                let trailing_index = trailing.expect("checked above");
                self.diagnostics.push(
                    Diagnostic::error("text interpolation must contain exactly one expression")
                        .with_code(INVALID_TEXT_INTERPOLATION)
                        .with_primary_label(
                            SourceSpan::new(
                                self.source.id(),
                                lexed.tokens()[trailing_index].span(),
                            ),
                            "this token is outside the interpolation expression",
                        )
                        .with_help("text interpolation expects an expression inside `\\{...}`"),
                );
                None
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("text interpolation must contain a valid expression")
                        .with_code(INVALID_TEXT_INTERPOLATION)
                        .with_primary_label(
                            interpolation_span,
                            "could not parse the expression inside this interpolation",
                        )
                        .with_help("text interpolation expects an expression inside `\\{...}`"),
                );
                None
            }
        }
    }

    fn parse_identifier(&self, cursor: &mut usize, end: usize) -> Option<Identifier> {
        let index = self.peek_nontrivia(*cursor, end)?;
        let kind = self.tokens[index].kind();
        if kind != TokenKind::Identifier && !kind.is_keyword() {
            return None;
        }
        *cursor = index + 1;
        Some(self.identifier_from_token(index))
    }

    fn parse_qualified_name(&self, cursor: &mut usize, end: usize) -> Option<QualifiedName> {
        let first = self.peek_nontrivia(*cursor, end)?;
        let first_kind = self.tokens[first].kind();
        if first_kind != TokenKind::Identifier && !first_kind.is_keyword() {
            return None;
        }

        let mut segments = vec![self.identifier_from_token(first)];
        let mut scan = first + 1;

        while let Some(dot_index) = self.peek_nontrivia(scan, end) {
            if self.tokens[dot_index].kind() != TokenKind::Dot {
                break;
            }
            let Some(segment_index) = self.peek_nontrivia(dot_index + 1, end) else {
                break;
            };
            let seg_kind = self.tokens[segment_index].kind();
            if seg_kind != TokenKind::Identifier && !seg_kind.is_keyword() {
                break;
            }
            segments.push(self.identifier_from_token(segment_index));
            scan = segment_index + 1;
        }

        *cursor = scan;
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(
                segments
                    .first()
                    .expect("qualified name has a first segment")
                    .span
                    .span()
                    .start(),
                segments
                    .last()
                    .expect("qualified name has a last segment")
                    .span
                    .span()
                    .end(),
            ),
        );
        Some(QualifiedName { segments, span })
    }

    fn line_indent_of_token(&self, index: usize) -> usize {
        let start = self.tokens[index].span().start().as_usize();
        let line_start = self.source.text()[..start]
            .rfind('\n')
            .map(|position| position + 1)
            .unwrap_or(0);
        self.source.text()[line_start..start].chars().count()
    }

    fn starts_instance_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen
        )
    }

    fn starts_domain_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::TypeKw
        )
    }

    fn starts_type_companion_member(&self, index: usize) -> bool {
        matches!(
            self.tokens[index].kind(),
            TokenKind::Identifier | TokenKind::TypeKw
        )
    }

    fn find_next_instance_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_instance_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    fn find_next_domain_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_domain_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    fn find_next_type_companion_member_start(
        &self,
        from: usize,
        end: usize,
        member_indent: usize,
    ) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia()
                || !token.line_start()
                || self.line_indent_of_token(index) != member_indent
                || !self.starts_type_companion_member(index)
            {
                continue;
            }
            return Some(index);
        }
        None
    }

    /// Parses a qualified name in decorator position. Accepts keyword tokens as identifiers
    /// (e.g. `@source`) since both keywords and identifiers are valid decorator names.
    fn parse_decorator_qualified_name(
        &self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<QualifiedName> {
        self.parse_qualified_name(cursor, end)
    }

    fn identifier_from_token(&self, index: usize) -> Identifier {
        let token = self.tokens[index];
        Identifier {
            text: token.text(self.source).to_owned(),
            span: SourceSpan::new(self.source.id(), token.span()),
        }
    }

    fn tokens_are_adjacent(&self, left: usize, right: usize) -> bool {
        self.tokens[left].span().end() == self.tokens[right].span().start()
    }

    fn make_base(&self, start: usize, end: usize, decorators: Vec<Decorator>) -> ItemBase {
        ItemBase {
            span: self.source_span_for_range(start, end),
            token_range: TokenRange::new(start, end),
            decorators,
            leading_comments: Vec::new(),
        }
    }

    fn source_span_for_range(&self, start: usize, end: usize) -> SourceSpan {
        let first = self
            .next_significant_in_range(start, end)
            .unwrap_or(start.min(self.tokens.len().saturating_sub(1)));
        let last = self.prev_significant_in_range(start, end).unwrap_or(first);
        SourceSpan::new(
            self.source.id(),
            Span::new(
                self.tokens[first].span().start(),
                self.tokens[last].span().end(),
            ),
        )
    }

    fn source_span_of_token(&self, index: usize) -> SourceSpan {
        SourceSpan::new(self.source.id(), self.tokens[index].span())
    }

    fn join_spans(&self, left: SourceSpan, right: SourceSpan) -> SourceSpan {
        left.join(right).unwrap_or(left)
    }

    fn find_declaration_keyword(&self, start: usize) -> DecoratorSearch {
        let mut depth = 0usize;
        for index in start..self.tokens.len() {
            let token = self.tokens[index];
            if !token.kind().is_trivia() && token.line_start() && depth == 0 {
                match token.kind() {
                    kind if kind.is_top_level_keyword() && self.is_at_column_zero(index) => {
                        return DecoratorSearch {
                            keyword: Some(index),
                            offending: None,
                        };
                    }
                    TokenKind::At => {}
                    _ if index != start => {
                        return DecoratorSearch {
                            keyword: None,
                            offending: Some(index),
                        };
                    }
                    _ => {}
                }
            }

            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }

        DecoratorSearch {
            keyword: None,
            offending: None,
        }
    }

    /// Returns true when `index` token is at column 0 (no leading indentation on its line).
    /// This distinguishes top-level declarations from identically-named keywords used as
    /// variable references in indented expression bodies.
    fn is_at_column_zero(&self, index: usize) -> bool {
        let start = self.tokens[index].span().start().as_usize();
        if start == 0 {
            return true;
        }
        let text = self.source.text().as_bytes();
        // The token is at column 0 iff the immediately preceding byte is a newline.
        // Any whitespace (spaces/tabs) before the token means it is indented.
        matches!(text[start - 1], b'\n' | b'\r')
    }

    fn find_next_item_start(&self, from: usize) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..self.tokens.len() {
            let token = self.tokens[index];
            if !token.kind().is_trivia()
                && token.line_start()
                && depth == 0
                && (token.kind() == TokenKind::At || token.kind().is_top_level_keyword())
                && self.is_at_column_zero(index)
            {
                return Some(index);
            }

            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        None
    }

    fn next_significant_from(&self, start: usize) -> Option<usize> {
        self.next_significant_in_range(start, self.tokens.len())
    }

    /// Collect line comments from tokens in `[from, to)` that appear at the
    /// start of a line. Only the contiguous block of comments immediately
    /// before the item (index `to`) is kept; any blank line (span gap larger
    /// than a single newline) between comment groups resets the accumulator so
    /// we don't attach comments that belong to the previous item.
    fn collect_leading_comments(&self, from: usize, to: usize) -> Vec<String> {
        // Walk forward over [from, to), collecting LineComment tokens.
        // When we encounter a blank line we reset so only the final group
        // adjacent to the item is kept.
        let mut candidates: Vec<String> = Vec::new();
        let mut last_end: Option<u32> = None;
        for index in from..to {
            let token = self.tokens[index];
            if token.kind() == TokenKind::LineComment {
                // Check for blank line gap: if the byte distance from the
                // previous token's end to this token's start is more than one
                // newline, consider this a fresh block.
                if let Some(prev_end) = last_end {
                    let gap = token.span().start().as_u32().saturating_sub(prev_end);
                    if gap > 1 {
                        // There is at least one blank line — reset.
                        candidates.clear();
                    }
                }
                candidates.push(token.text(self.source).to_owned());
                last_end = Some(token.span().end().as_u32());
            } else if !token.kind().is_trivia() {
                // Unexpected non-trivia in this range — clear and stop.
                candidates.clear();
            } else {
                // Whitespace / block comment trivia: track position for gap detection.
                if last_end.is_none() {
                    last_end = Some(token.span().end().as_u32());
                }
            }
        }
        candidates
    }

    fn next_significant_in_range(&self, start: usize, end: usize) -> Option<usize> {
        self.tokens[start..end]
            .iter()
            .position(|token| !token.kind().is_trivia())
            .map(|offset| start + offset)
    }

    fn prev_significant_in_range(&self, start: usize, end: usize) -> Option<usize> {
        self.tokens[start..end]
            .iter()
            .rposition(|token| !token.kind().is_trivia())
            .map(|offset| start + offset)
    }

    fn peek_nontrivia(&self, start: usize, end: usize) -> Option<usize> {
        self.next_significant_in_range(start, end)
    }

    fn peek_kind(&self, start: usize, end: usize) -> Option<TokenKind> {
        self.peek_nontrivia(start, end)
            .map(|index| self.tokens[index].kind())
    }

    fn consume_kind(&self, cursor: &mut usize, end: usize, kind: TokenKind) -> Option<usize> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[index].kind() != kind {
            return None;
        }
        *cursor = index + 1;
        Some(index)
    }

    fn consume_identifier_text(
        &self,
        cursor: &mut usize,
        end: usize,
        expected: &str,
    ) -> Option<usize> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_identifier_text(index, expected) {
            return None;
        }
        *cursor = index + 1;
        Some(index)
    }

    fn is_identifier_text(&self, index: usize, expected: &str) -> bool {
        self.tokens[index].kind() == TokenKind::Identifier
            && self.tokens[index].text(self.source) == expected
    }

    fn starts_prefixed_collection_literal(
        &self,
        index: usize,
        end: usize,
        prefix: &str,
        opener: TokenKind,
    ) -> bool {
        self.is_identifier_text(index, prefix)
            && self
                .peek_nontrivia(index + 1, end)
                .is_some_and(|next| self.tokens[next].kind() == opener)
    }
}
