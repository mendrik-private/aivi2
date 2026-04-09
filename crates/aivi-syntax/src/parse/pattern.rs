impl<'a> Parser<'a> {
    fn parse_pipe_case_arm(
        &mut self,
        cursor: &mut usize,
        end: usize,
        outer_stop: ExprStop,
    ) -> Option<PipeCaseArm> {
        let start = *cursor;
        let pattern = self.parse_pattern(cursor, end, PatternStop::arrow_context())?;
        let _ = self.consume_kind(cursor, end, TokenKind::ThinArrow)?;
        // The arm body may contain inline pipe expressions (e.g. `first ||> { email } -> email`).
        // Only line-start pipe operators terminate the body — those belong to sibling arms.
        let mut body_stop = outer_stop;
        body_stop.pipe_stage = false;
        body_stop.pipe_stage_line_start_only = true;
        let body = self.parse_expr(cursor, end, body_stop)?;
        Some(PipeCaseArm {
            pattern,
            body,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_pattern(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: PatternStop,
    ) -> Option<Pattern> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let mut pattern = self.parse_pattern_atom(cursor, end, stop);
        let result = pattern.take().and_then(|mut p| {
            while let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.pattern_should_stop(index, stop) || !self.starts_pattern(index) {
                    break;
                }
                if self.tokens[index].line_start() {
                    break;
                }
                let argument = self.parse_pattern_atom(cursor, end, stop)?;
                p = self.make_pattern_apply(p, argument);
            }
            Some(p)
        });
        self.depth_exit();
        result
    }

    fn parse_pattern_atom(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: PatternStop,
    ) -> Option<Pattern> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.pattern_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::Minus => {
                let literal_index = self.negative_integer_literal_token(index, end)?;
                *cursor = literal_index + 1;
                let span = self.numeric_literal_span(Some(index), literal_index);
                Some(Pattern {
                    span,
                    kind: PatternKind::Integer(IntegerLiteral {
                        raw: self.numeric_literal_raw(Some(index), literal_index),
                        span,
                    }),
                })
            }
            TokenKind::Identifier => {
                *cursor = index + 1;
                let identifier = self.identifier_from_token(index);
                if identifier.text == "_" {
                    Some(Pattern {
                        span: identifier.span,
                        kind: PatternKind::Wildcard,
                    })
                } else {
                    Some(Pattern {
                        span: identifier.span,
                        kind: PatternKind::Name(identifier),
                    })
                }
            }
            // Keywords can be used as binding variable names in patterns (contextual keyword).
            kind if kind.is_keyword() => {
                *cursor = index + 1;
                let identifier = self.identifier_from_token(index);
                Some(Pattern {
                    span: identifier.span,
                    kind: PatternKind::Name(identifier),
                })
            }
            TokenKind::Integer => {
                *cursor = index + 1;
                let span = self.source_span_of_token(index);
                Some(Pattern {
                    span,
                    kind: PatternKind::Integer(IntegerLiteral {
                        raw: self.tokens[index].text(self.source).to_owned(),
                        span,
                    }),
                })
            }
            TokenKind::StringLiteral => {
                *cursor = index + 1;
                let literal = self.text_literal_from_token(index);
                Some(Pattern {
                    span: literal.span,
                    kind: PatternKind::Text(literal),
                })
            }
            TokenKind::LParen => self.parse_grouped_pattern(cursor, end),
            TokenKind::LBracket => self.parse_list_pattern(cursor, end),
            TokenKind::LBrace => self.parse_record_pattern(cursor, end),
            _ => None,
        }
    }

    fn parse_grouped_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(Pattern {
                span: self.source_span_for_range(start, *cursor),
                kind: PatternKind::Tuple(Vec::new()),
            });
        }

        loop {
            let element = self.parse_pattern(cursor, end, PatternStop::paren_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                saw_comma = true;
                if self.peek_kind(*cursor, end) == Some(TokenKind::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RParen);
        let span = self.source_span_for_range(start, *cursor);
        Some(if saw_comma || elements.len() != 1 {
            Pattern {
                span,
                kind: PatternKind::Tuple(elements),
            }
        } else {
            Pattern {
                span,
                kind: PatternKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_record_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut fields = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let Some(label) = self.parse_identifier(cursor, end) else {
                break;
            };
            let field_start = label.span.span().start();
            let mut label_path = Vec::new();
            while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
                if let Some(next) = self.parse_identifier(cursor, end) {
                    label_path.push(next);
                } else {
                    break;
                }
            }
            let pattern = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_pattern(cursor, end, PatternStop::record_context())
            } else {
                None
            };
            let field_end = pattern
                .as_ref()
                .map(|pattern| pattern.span.span().end())
                .unwrap_or_else(|| {
                    label_path
                        .last()
                        .map(|p| p.span.span().end())
                        .unwrap_or_else(|| label.span.span().end())
                });
            fields.push(RecordPatternField {
                label,
                label_path,
                pattern,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(Pattern {
            span: self.source_span_for_range(start, *cursor),
            kind: PatternKind::Record(fields),
        })
    }

    fn parse_list_pattern(&mut self, cursor: &mut usize, end: usize) -> Option<Pattern> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();
        let mut rest = None;

        if self
            .consume_kind(cursor, end, TokenKind::RBracket)
            .is_some()
        {
            return Some(Pattern {
                span: self.source_span_for_range(start, *cursor),
                kind: PatternKind::List {
                    elements,
                    rest: None,
                },
            });
        }

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::Ellipsis)
                .is_some()
            {
                rest = Some(Box::new(self.parse_pattern(
                    cursor,
                    end,
                    PatternStop::list_context(),
                )?));
                let _ = self.consume_kind(cursor, end, TokenKind::Comma);
                break;
            }

            let element = self.parse_pattern(cursor, end, PatternStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                if self.peek_kind(*cursor, end) == Some(TokenKind::RBracket) {
                    break;
                }
                continue;
            }
            break;
        }

        let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
        Some(Pattern {
            span: self.source_span_for_range(start, *cursor),
            kind: PatternKind::List { elements, rest },
        })
    }

}
