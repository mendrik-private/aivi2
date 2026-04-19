impl<'a> Parser<'a> {
    fn parse_decorators(&mut self, start: usize, keyword_index: usize) -> Vec<Decorator> {
        let starts = self.collect_decorator_starts(start, keyword_index);
        let mut decorators = Vec::new();
        for (index, decorator_start) in starts.iter().copied().enumerate() {
            let decorator_end = starts.get(index + 1).copied().unwrap_or(keyword_index);
            if let Some(decorator) = self.parse_decorator_range(decorator_start, decorator_end) {
                decorators.push(decorator);
            }
        }
        decorators
    }

    fn collect_decorator_starts(&self, start: usize, end: usize) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut depth = 0usize;
        for index in start..end {
            let token = self.tokens[index];
            if !token.kind().is_trivia()
                && token.line_start()
                && depth == 0
                && token.kind() == TokenKind::At
            {
                starts.push(index);
            }
            match token.kind() {
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                _ => {}
            }
        }
        starts
    }

    fn parse_decorator_range(&mut self, start: usize, end: usize) -> Option<Decorator> {
        let mut cursor = start;
        let _ = self.consume_kind(&mut cursor, end, TokenKind::At)?;
        let name = match self.parse_decorator_qualified_name(&mut cursor, end) {
            Some(name) => name,
            None => {
                self.diagnostics.push(
                    Diagnostic::error("decorator name is missing after `@`")
                        .with_code(MISSING_DECORATOR_NAME)
                        .with_primary_label(
                            self.source_span_of_token(start),
                            "expected a decorator name such as `source`",
                        ),
                );
                return None;
            }
        };
        let payload = if name.as_dotted() == "source" {
            DecoratorPayload::Source(self.parse_source_decorator_payload(&mut cursor, end))
        } else {
            self.parse_generic_decorator_payload(&mut cursor, end)
        };
        Some(Decorator {
            name,
            span: self.source_span_for_range(start, end),
            payload,
        })
    }

    fn parse_source_decorator_payload(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> SourceDecorator {
        let provider = self.parse_qualified_name(cursor, end);
        let mut arguments = Vec::new();
        let mut options = None;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with") {
                *cursor = index + 1;
                options = self.parse_record_expr(cursor, end);
                break;
            }
            let Some(argument) = self.parse_decorator_argument(cursor, end) else {
                break;
            };
            arguments.push(argument);
            let _ = self.consume_kind(cursor, end, TokenKind::Comma);
        }

        SourceDecorator {
            provider,
            arguments,
            options,
        }
    }

    fn parse_generic_decorator_payload(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> DecoratorPayload {
        let mut arguments = Vec::new();
        let mut options = None;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with") {
                *cursor = index + 1;
                options = self.parse_record_expr(cursor, end);
                break;
            }
            let Some(argument) = self.parse_decorator_argument(cursor, end) else {
                break;
            };
            arguments.push(argument);
            let _ = self.consume_kind(cursor, end, TokenKind::Comma);
        }

        if arguments.is_empty() && options.is_none() {
            DecoratorPayload::Bare
        } else {
            DecoratorPayload::Arguments(DecoratorArguments { arguments, options })
        }
    }

    fn parse_decorator_argument(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let mut expr = self.parse_atomic_expr(cursor, end, ExprStop::default())?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.is_identifier_text(index, "with")
                || self.tokens[index].line_start()
                || self.tokens[index].kind() == TokenKind::Comma
                || self.tokens[index].kind().is_pipe_operator()
                || !self.starts_expr(index)
            {
                break;
            }
            let argument = self.parse_atomic_expr(cursor, end, ExprStop::default())?;
            expr = self.make_apply_expr(expr, argument);
        }
        Some(expr)
    }

    fn parse_named_item_name(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
        description: &str,
    ) -> Option<Identifier> {
        let name = self.parse_identifier(cursor, end);
        if name.is_none() {
            self.diagnostics.push(
                Diagnostic::error(format!("{description} is missing its name"))
                    .with_code(MISSING_ITEM_NAME)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        format!(
                            "expected a name after `{}`",
                            self.tokens[keyword_index].text(self.source)
                        ),
                    ),
            );
        }
        name
    }

    fn parse_optional_type_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<TypeExpr> {
        self.consume_kind(cursor, end, TokenKind::Colon)?;
        self.parse_type_expr(cursor, end, TypeStop::default())
    }

    fn parse_optional_signature_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        if self.consume_kind(cursor, end, TokenKind::Colon).is_none() {
            return (Vec::new(), None);
        }
        self.parse_constrained_type(cursor, end)
    }

    fn parse_function_signature_annotation(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        if self.peek_kind(*cursor, end) != Some(TokenKind::Colon) {
            return (Vec::new(), None);
        }
        let Some(body_arrow) = self.find_last_same_line_arrow(*cursor, end) else {
            return self.parse_optional_signature_annotation(cursor, end);
        };
        for split in self.function_signature_split_candidates(*cursor, body_arrow) {
            if self.probe_function_signature(split, *cursor, body_arrow) {
                return self.parse_optional_signature_annotation(cursor, split);
            }
        }
        if let Some(body_equals) = self.find_same_line_top_level_equals(*cursor, body_arrow) {
            return self.parse_optional_signature_annotation(cursor, body_equals);
        }
        if let Some(body_equals) = self.find_same_line_top_level_equals(*cursor, end) {
            return self.parse_optional_signature_annotation(cursor, body_equals);
        }
        self.parse_optional_signature_annotation(cursor, body_arrow)
    }

    fn parse_constrained_type(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> (Vec<TypeExpr>, Option<TypeExpr>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self.consume_constraint_separator(cursor, end)
        {
            return (
                constraints,
                self.parse_type_expr(cursor, end, TypeStop::default()),
            );
        }
        *cursor = checkpoint;
        (
            Vec::new(),
            self.parse_type_expr(cursor, end, TypeStop::default()),
        )
    }

    fn parse_constraint_list(&mut self, cursor: &mut usize, end: usize) -> Option<Vec<TypeExpr>> {
        if self.peek_kind(*cursor, end) == Some(TokenKind::LParen) {
            self.consume_kind(cursor, end, TokenKind::LParen)?;
            let mut constraints = Vec::new();
            loop {
                constraints.push(self.parse_type_expr(cursor, end, TypeStop::paren_context())?);
                if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                    continue;
                }
                self.consume_kind(cursor, end, TokenKind::RParen)?;
                break;
            }
            return Some(constraints);
        }
        Some(vec![self.parse_type_expr(
            cursor,
            end,
            TypeStop::constraint_attempt(),
        )?])
    }

    /// Returns `true` when `expr` looks like a class constraint — i.e. a type
    /// application whose callee is a multi-character identifier.  Single-letter
    /// identifiers are type variables by convention in AIVI and therefore not
    /// valid class names.  Parenthesised constraint tuples are handled by the
    /// caller and do not reach this predicate.
    fn is_constraint_expr(expr: &TypeExpr) -> bool {
        match &expr.kind {
            TypeExprKind::Apply { callee, .. } => match &callee.kind {
                TypeExprKind::Name(name) => name.text.len() > 1,
                _ => false,
            },
            _ => false,
        }
    }

    fn parse_optional_constraint_prefix(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<TypeExpr> {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self.consume_constraint_separator(cursor, end)
        {
            return constraints;
        }
        *cursor = checkpoint;
        Vec::new()
    }

    fn parse_type_parameters_same_line(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<Identifier> {
        let mut type_parameters = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() || self.tokens[index].kind() != TokenKind::Identifier
            {
                break;
            }
            type_parameters.push(self.identifier_from_token(index));
            *cursor = index + 1;
        }
        type_parameters
    }

    fn parse_class_head(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> (Option<Identifier>, Vec<Identifier>) {
        let checkpoint = *cursor;
        if let Some(constraints) = self.parse_constraint_list(cursor, end)
            && constraints.iter().all(Self::is_constraint_expr)
            && self
                .consume_kind(cursor, end, TokenKind::ThinArrow)
                .is_some()
        {
            self.diagnostics.push(
                Diagnostic::error("class declarations do not accept head constraint prefixes")
                    .with_code(UNSUPPORTED_CLASS_HEAD_CONSTRAINTS)
                    .with_primary_label(
                        self.source_span_for_range(checkpoint, *cursor),
                        "move these constraints into indented `with ...` lines inside the class body",
                    )
                    .with_note("supported form: `class Applicative F` followed by `with Functor F`"),
            );
            let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
            let type_parameters = self.parse_type_parameters_same_line(cursor, end);
            return (name, type_parameters);
        }
        *cursor = checkpoint;
        let name = self.parse_named_item_name(keyword_index, cursor, end, "class declaration");
        let type_parameters = self.parse_type_parameters_same_line(cursor, end);
        (name, type_parameters)
    }

}
