impl<'a> Parser<'a> {
    fn parse_type_decl_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeDeclBody> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[index].kind() == TokenKind::LBrace {
            let inner_end = self.find_matching_brace(index, end).unwrap_or(end);
            if let Some(first_inside) = self.peek_nontrivia(index + 1, inner_end)
                && self.tokens[first_inside].kind() == TokenKind::PipeTap {
                    return self
                        .parse_sum_type_block_body(cursor, end)
                        .map(TypeDeclBody::Sum);
                }
        }
        if self.tokens[index].kind() == TokenKind::PipeTap {
            return self.parse_sum_type_body(cursor, end);
        }
        if self.tokens[index].kind() == TokenKind::Identifier {
            let identifier = self.identifier_from_token(index);
            if identifier.is_uppercase_initial() && !is_record_row_transform_name(&identifier.text)
                && let Some(next_index) = self.peek_nontrivia(index + 1, end)
                    && (self.tokens[next_index].kind() == TokenKind::PipeTap
                        || (self.starts_type_atom(next_index)
                            && !self.tokens[next_index].line_start()))
                    {
                        return self.parse_sum_type_body(cursor, end);
                    }
        }
        self.parse_type_expr(cursor, end, TypeStop::default())
            .map(TypeDeclBody::Alias)
    }

    fn parse_sum_type_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeDeclBody> {
        let start = *cursor;
        let variants = self.parse_sum_type_variants(cursor, end);
        (!variants.is_empty()).then_some(TypeDeclBody::Sum(TypeSumBody {
            variants,
            companions: Vec::new(),
            span: self.source_span_for_range(start, *cursor),
        }))
    }

    fn parse_sum_type_block_body(&mut self, cursor: &mut usize, end: usize) -> Option<TypeSumBody> {
        let lbrace = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let inner_end = self.find_matching_brace(lbrace, end).unwrap_or(end);
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&index| self.tokens[index].line_start())
            .map(|index| self.line_indent_of_token(index))
            .unwrap_or(0);
        let variants = self.parse_sum_type_variants(cursor, inner_end);
        if variants.is_empty() {
            *cursor = inner_end.saturating_add(1);
            return None;
        }

        let mut companions = Vec::new();
        let mut pending_annotation: Option<TypeExpr> = None;
        let mut pending_colon_member: Option<TypeCompanionMember> = None;

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            if self.tokens[index].line_start()
                && member_indent != 0
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if self.tokens[index].kind() == TokenKind::PipeTap {
                self.diagnostics.push(
                    Diagnostic::error("sum constructors must come before companion members")
                        .with_code(MISSING_DECLARATION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(index),
                            "move this constructor above the companion bindings",
                        ),
                );
                break;
            }
            if !self.starts_type_companion_member(index) {
                break;
            }

            if self.tokens[index].kind() == TokenKind::TypeKw {
                if let Some(held) = pending_colon_member.take() {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "type companion member `{}` is missing its binding",
                            held.name.text
                        ))
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            held.span,
                            "expected a binding line after this companion type annotation",
                        ),
                    );
                }
                *cursor = index + 1;
                let type_end = self
                    .find_next_type_companion_member_start(*cursor, inner_end, member_indent)
                    .unwrap_or(inner_end);
                let annotation = self
                    .parse_type_expr(cursor, type_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion annotation is missing its type after `type`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_TYPE)
                            .with_primary_label(
                                self.source_span_for_range(index, *cursor),
                                "expected a type such as `Player -> Text`",
                            ),
                        );
                        None
                    });
                *cursor = type_end;
                pending_annotation = annotation;
                continue;
            }

            let before = *cursor;
            let Some(mut member) =
                self.parse_type_companion_member(cursor, inner_end, member_indent)
            else {
                break;
            };

            if let Some(annotation) = pending_annotation.take() {
                member.annotation = Some(annotation);
            }

            if member.annotation.is_none()
                && let Some(held) = pending_colon_member.take() {
                    if held.name.text == member.name.text {
                        member.annotation = held.annotation;
                    } else {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "type companion member `{}` is missing its binding",
                                held.name.text
                            ))
                            .with_code(MISSING_TYPE_COMPANION_BODY)
                            .with_primary_label(
                                held.span,
                                "expected a binding line with the same companion name",
                            ),
                        );
                    }
                }

            if member.annotation.is_some() && member.body.is_none() && member.parameters.is_empty()
            {
                pending_colon_member = Some(member);
                if *cursor <= before {
                    break;
                }
                continue;
            }

            companions.push(member);
            if *cursor <= before {
                break;
            }
        }

        if let Some(held) = pending_colon_member.take() {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "type companion member `{}` is missing its binding",
                    held.name.text
                ))
                .with_code(MISSING_TYPE_COMPANION_BODY)
                .with_primary_label(
                    held.span,
                    "expected a binding line after this companion type annotation",
                ),
            );
        }

        if let Some(annotation) = pending_annotation {
            self.diagnostics.push(
                Diagnostic::error("type companion annotation has no following member")
                    .with_code(MISSING_TYPE_COMPANION_NAME)
                    .with_primary_label(
                        annotation.span,
                        "expected a companion binding after this type annotation",
                    ),
            );
        }

        *cursor = inner_end.saturating_add(1);
        Some(TypeSumBody {
            variants,
            companions,
            span: self.source_span_for_range(lbrace, *cursor),
        })
    }

    fn parse_sum_type_variants(&mut self, cursor: &mut usize, end: usize) -> Vec<TypeVariant> {
        let mut variants = Vec::new();

        loop {
            let _ = self.consume_kind(cursor, end, TokenKind::PipeTap);
            let variant_start = *cursor;
            let name = self.parse_identifier(cursor, end);
            if name.is_none() {
                break;
            }

            let mut fields = Vec::new();
            while let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.tokens[index].line_start()
                    || self.tokens[index].kind() == TokenKind::PipeTap
                    || !self.starts_type_atom(index)
                {
                    break;
                }
                let Some(field) = self.parse_type_variant_field(cursor, end) else {
                    break;
                };
                fields.push(field);
            }

            variants.push(TypeVariant {
                name,
                fields,
                span: self.source_span_for_range(variant_start, *cursor),
            });

            if self.peek_kind(*cursor, end) != Some(TokenKind::PipeTap) {
                break;
            }
        }

        variants
    }

    fn parse_type_companion_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<TypeCompanionMember> {
        let start = *cursor;
        let name_index = self.peek_nontrivia(*cursor, end).unwrap_or(start);
        let Some(name) = self.parse_identifier(cursor, end) else {
            self.diagnostics.push(
                Diagnostic::error("type companion member is missing its name")
                    .with_code(MISSING_TYPE_COMPANION_NAME)
                    .with_primary_label(
                        self.source_span_for_range(start, *cursor),
                        "expected a companion name such as `label` or `opponent`",
                    ),
            );
            return None;
        };
        let name_span = name.span;
        let mut annotation = None;

        if let Some(colon_idx) = self.peek_nontrivia(*cursor, end)
            && self.tokens[colon_idx].kind() == TokenKind::Colon
                && !self.tokens[colon_idx].line_start()
            {
                *cursor = colon_idx + 1;
                let ann_end = self
                    .find_same_line_top_level_equals(*cursor, end)
                    .or_else(|| {
                        self.find_next_type_companion_member_start(*cursor, end, member_indent)
                    })
                    .unwrap_or(end);
                annotation = self
                    .parse_type_expr(cursor, ann_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion member is missing its type after `:`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_TYPE)
                            .with_primary_label(
                                name_span,
                                "expected a companion type such as `Player -> Player`",
                            ),
                        );
                        None
                    });
                *cursor = ann_end;
                if self.peek_kind(*cursor, end) != Some(TokenKind::Equals) {
                    return Some(TypeCompanionMember {
                        name,
                        annotation,
                        function_form: FunctionSurfaceForm::Explicit,
                        parameters: Vec::new(),
                        body: None,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
            }

        let Some(eq_index) = self.consume_kind(cursor, end, TokenKind::Equals) else {
            self.diagnostics.push(
                Diagnostic::error("type companion member is missing `=` before its body")
                    .with_code(MISSING_TYPE_COMPANION_BODY)
                    .with_primary_label(name.span, "write `name = self => ...` or `name = . ...`"),
            );
            return None;
        };
        let member_end = self
            .find_next_type_companion_member_start(*cursor, end, member_indent)
            .unwrap_or(end);

        let (function_form, parameters, body) = if let Some((parameters, body)) =
            self.parse_unary_subject_function_body(name_index, cursor, member_end)
        {
            (
                FunctionSurfaceForm::UnarySubjectSugar,
                parameters,
                body.and_then(|body| match body {
                    NamedItemBody::Expr(expr) => Some(expr),
                    _ => None,
                }),
            )
        } else if let Some((parameters, body)) =
            self.parse_selected_subject_function_body(cursor, member_end, "type companion member")
        {
            (
                FunctionSurfaceForm::SelectedSubjectSugar,
                parameters,
                body.and_then(|body| match body {
                    NamedItemBody::Expr(expr) => Some(expr),
                    _ => None,
                }),
            )
        } else {
            let mut parameters = Vec::new();
            while self.starts_function_param(*cursor, member_end) {
                let Some(parameter) = self.parse_function_param(cursor, member_end) else {
                    break;
                };
                parameters.push(parameter);
            }

            let body = if let Some(arrow_index) =
                self.consume_kind(cursor, member_end, TokenKind::Arrow)
            {
                let body = self
                    .parse_expr(cursor, member_end, ExprStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "type companion member is missing its body after `=>`",
                            )
                            .with_code(MISSING_TYPE_COMPANION_BODY)
                            .with_primary_label(
                                self.source_span_of_token(arrow_index),
                                "expected an expression body for this companion member",
                            ),
                        );
                        None
                    })
                    .and_then(|expr| {
                        self.finish_expression_body(
                            cursor,
                            member_end,
                            "type companion member",
                            expr,
                        )
                        .and_then(|body| match body {
                            NamedItemBody::Expr(expr) => Some(expr),
                            _ => None,
                        })
                    });
                if parameters.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "type companion members require an explicit parameter before `=>`",
                        )
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(arrow_index),
                            "insert a parameter such as `self` before `=>`",
                        )
                        .with_note(
                            "receiver-only helpers can use the shortcut `name = .field` or `name = .`",
                        ),
                    );
                }
                body
            } else {
                self.diagnostics.push(
                    Diagnostic::error(
                        "type companion member body must use `name = self => ...` or `name = . ...`",
                    )
                        .with_code(MISSING_TYPE_COMPANION_BODY)
                        .with_primary_label(
                            self.source_span_of_token(eq_index),
                            "expected `.` or parameters followed by `=>`",
                        ),
                );
                None
            };

            (FunctionSurfaceForm::Explicit, parameters, body)
        };

        *cursor = member_end;
        Some(TypeCompanionMember {
            name,
            annotation,
            function_form,
            parameters,
            body,
            span: self.source_span_for_range(start, member_end),
        })
    }

    fn parse_type_variant_field(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<TypeVariantField> {
        let field_start = *cursor;
        // Try named field: `label:Type` (lowercase identifier followed by colon)
        let checkpoint = *cursor;
        if let Some(index) = self.peek_nontrivia(*cursor, end)
            && self.tokens[index].kind() == TokenKind::Identifier {
                let ident = self.identifier_from_token(index);
                if !ident.is_uppercase_initial() {
                    // Peek ahead for colon
                    if let Some(colon_index) = self.peek_nontrivia(index + 1, end)
                        && self.tokens[colon_index].kind() == TokenKind::Colon {
                            // Consume label and colon
                            let label = self.parse_identifier(cursor, end)?;
                            let _ = self.consume_kind(cursor, end, TokenKind::Colon);
                            let ty = self.parse_type_atom(cursor, end, TypeStop::default())?;
                            return Some(TypeVariantField {
                                label: Some(label),
                                span: self.source_span_for_range(field_start, *cursor),
                                ty,
                            });
                        }
                }
            }
        *cursor = checkpoint;
        // Fall back to anonymous field
        let ty = self.parse_type_atom(cursor, end, TypeStop::default())?;
        Some(TypeVariantField {
            label: None,
            span: self.source_span_for_range(field_start, *cursor),
            ty,
        })
    }

    fn parse_type_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_type_pipe_expr(cursor, end, stop);
        self.depth_exit();
        result
    }

    fn parse_type_pipe_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let mut ty = self.parse_type_arrow_expr(cursor, end, stop.with_pipe_transform())?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.type_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::PipeTransform
            {
                break;
            }
            *cursor = index + 1;
            let stage = self.parse_type_arrow_expr(cursor, end, stop.with_pipe_transform())?;
            ty = self.make_type_apply(stage, ty);
        }
        Some(ty)
    }

    fn parse_type_arrow_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let parameter = self.parse_type_application_expr(cursor, end, stop);
        parameter.and_then(|parameter| {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                return Some(parameter);
            };
            if self.type_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::ThinArrow
            {
                return Some(parameter);
            }
            *cursor = index + 1;
            let result = self.parse_type_expr(cursor, end, stop)?;
            Some(self.make_type_arrow(parameter, result))
        })
    }

    fn parse_type_application_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let mut ty = self.parse_type_atom(cursor, end, stop)?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.type_should_stop(index, stop) || !self.starts_type_atom(index) {
                break;
            }
            if self.tokens[index].line_start() {
                break;
            }
            let argument = self.parse_type_atom(cursor, end, stop)?;
            ty = self.make_type_apply(ty, argument);
        }
        Some(ty)
    }

    fn parse_type_atom(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: TypeStop,
    ) -> Option<TypeExpr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.type_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::Identifier => {
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                Some(TypeExpr {
                    span: name.span,
                    kind: TypeExprKind::Name(name),
                })
            }
            TokenKind::LParen => self.parse_grouped_type(cursor, end),
            TokenKind::LBrace => self.parse_record_type(cursor, end),
            _ => None,
        }
    }

    fn parse_grouped_type(&mut self, cursor: &mut usize, end: usize) -> Option<TypeExpr> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(TypeExpr {
                span: self.source_span_for_range(start, *cursor),
                kind: TypeExprKind::Tuple(Vec::new()),
            });
        }

        loop {
            let element = self.parse_type_expr(cursor, end, TypeStop::paren_context())?;
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
            TypeExpr {
                span,
                kind: TypeExprKind::Tuple(elements),
            }
        } else {
            TypeExpr {
                span,
                kind: TypeExprKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_record_type(&mut self, cursor: &mut usize, end: usize) -> Option<TypeExpr> {
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
            let ty = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_type_expr(cursor, end, TypeStop::record_context())
            } else {
                None
            };
            let field_end = ty
                .as_ref()
                .map(|ty| ty.span.span().end())
                .unwrap_or_else(|| label.span.span().end());
            fields.push(TypeField {
                label,
                ty,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(TypeExpr {
            span: self.source_span_for_range(start, *cursor),
            kind: TypeExprKind::Record(fields),
        })
    }

}
