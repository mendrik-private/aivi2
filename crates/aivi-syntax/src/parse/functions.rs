impl<'a> Parser<'a> {
    fn parse_function_param(&mut self, cursor: &mut usize, end: usize) -> Option<FunctionParam> {
        let name_index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_function_param_token(name_index) {
            return None;
        }
        *cursor = name_index + 1;
        let identifier = self.identifier_from_token(name_index);
        let name = (identifier.text != "_").then_some(identifier);
        let annotation = if self.peek_kind(*cursor, end) == Some(TokenKind::Colon) {
            let annotation_end = self.parameter_annotation_end(*cursor, end);
            let annotation = self.parse_optional_type_annotation(cursor, annotation_end);
            let annotation_span = annotation
                .as_ref()
                .map(|annotation| annotation.span)
                .unwrap_or_else(|| self.source_span_of_token(name_index));
            self.diagnostics.push(
                Diagnostic::warning("function parameters no longer accept inline type annotations")
                    .with_code(DIRECT_FUNCTION_PARAMETER_ANNOTATION)
                    .with_primary_label(
                        annotation_span,
                        "prefer moving this type into the function signature annotation",
                    )
                    .with_secondary_label(
                        self.source_span_of_token(name_index),
                        "leave the parameter name unannotated here",
                    ),
            );
            annotation
        } else {
            None
        };
        Some(FunctionParam {
            name,
            annotation,
            span: self.source_span_for_range(name_index, *cursor),
        })
    }

    fn starts_unary_subject_function(&self, start: usize, end: usize) -> bool {
        let Some(index) = self.peek_nontrivia(start, end) else {
            return false;
        };
        match self.tokens[index].kind() {
            TokenKind::Dot => true,
            TokenKind::StringLiteral => self.tokens[index].text(self.source).contains('{'),
            _ => false,
        }
    }

    fn parse_unary_subject_function_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<(Vec<FunctionParam>, Option<NamedItemBody>)> {
        let head_start = self.peek_nontrivia(*cursor, end)?;
        if !matches!(
            self.tokens[head_start].kind(),
            TokenKind::Dot | TokenKind::StringLiteral | TokenKind::LBrace
        ) {
            return None;
        }
        let checkpoint = *cursor;
        let parameter =
            self.implicit_function_subject_parameter_at(self.source_span_of_token(head_start));
        let head = self.with_implicit_lambda_disabled(|parser| {
            parser.parse_range_expr(cursor, end, ExprStop::default().with_pipe_stage())
        })?;
        let (head, rewrote_subject) =
            self.rewrite_free_function_subject_expr(head, &parameter, false);
        if !rewrote_subject {
            *cursor = checkpoint;
            return None;
        }
        let body = self
            .with_implicit_lambda_disabled(|parser| {
                parser.parse_subject_root_expr_from_head(
                    head_start,
                    head,
                    cursor,
                    end,
                    ExprStop::default(),
                )
            })
            .and_then(|expr| self.finish_expression_body(cursor, end, "func declaration", expr))
            .or_else(|| {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body after `=`",
                    "expected an expression using `.` or parameters followed by `=>`",
                );
                None
            });
        Some((vec![parameter], body))
    }

    fn parse_selected_subject_function_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
        declaration_name: &str,
    ) -> Option<(Vec<FunctionParam>, Option<NamedItemBody>)> {
        let checkpoint = *cursor;
        let mut parameters = Vec::new();
        let mut parameter_starts = Vec::new();
        let mut selected_head: Option<SubjectPickHead> = None;

        while self.starts_function_param(*cursor, end) {
            let Some((parameter, start_index, selected)) =
                self.parse_subject_pick_function_param(cursor, end)
            else {
                break;
            };
            if selected {
                if selected_head.is_some() {
                    self.emit_invalid_subject_pick(
                        parameter.span,
                        "function headers can only choose one subject with `!`",
                        "remove this `!` or the earlier subject pick",
                    );
                } else if let Some(name) = parameter.name.as_ref() {
                    selected_head = Some(SubjectPickHead {
                        expr: Expr {
                            span: name.span,
                            kind: ExprKind::Name(name.clone()),
                        },
                        start_index,
                    });
                } else {
                    self.emit_invalid_subject_pick(
                        parameter.span,
                        "the discard `_` cannot be the selected subject",
                        "choose a named parameter before `!`",
                    );
                }
            }
            parameter_starts.push(start_index);
            parameters.push(parameter);
        }

        if let Some(selector) = self.parse_subject_pick_record_selector(cursor, end) {
            if selected_head.is_some() {
                self.emit_invalid_subject_pick(
                    selector.span,
                    "function headers can only choose one subject with `!`",
                    "remove either this `{ ...! }` selector or the earlier `!` marker",
                );
            } else if let Some((base_index, base_parameter)) = parameters
                .iter()
                .enumerate()
                .rev()
                .find(|(_, parameter)| parameter.name.is_some())
            {
                let base_name = base_parameter
                    .name
                    .as_ref()
                    .expect("filtered parameter must keep its name")
                    .clone();
                let base_expr = Expr {
                    span: base_name.span,
                    kind: ExprKind::Name(base_name),
                };
                let head_span = self.join_spans(base_expr.span, selector.span);
                selected_head = Some(SubjectPickHead {
                    expr: Expr {
                        span: head_span,
                        kind: ExprKind::Projection {
                            base: Box::new(base_expr),
                            path: selector,
                        },
                    },
                    start_index: parameter_starts[base_index],
                });
            } else {
                self.emit_invalid_subject_pick(
                    selector.span,
                    "record subject selectors need a preceding named parameter",
                    "write a named parameter before this `{ ...! }` selector",
                );
                *cursor = checkpoint;
                return None;
            }
        }

        let Some(head) = selected_head else {
            *cursor = checkpoint;
            return None;
        };

        let body = self
            .with_implicit_lambda_disabled(|parser| {
                parser.parse_subject_root_expr_from_head(
                    head.start_index,
                    head.expr,
                    cursor,
                    end,
                    ExprStop::default(),
                )
            })
            .and_then(|expr| self.finish_expression_body(cursor, end, declaration_name, expr));
        Some((parameters, body))
    }

    fn parse_subject_pick_function_param(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<(FunctionParam, usize, bool)> {
        let start_index = self.peek_nontrivia(*cursor, end)?;
        let parameter = self.parse_function_param(cursor, end)?;
        let selected = self.consume_kind(cursor, end, TokenKind::Bang).is_some();
        Some((parameter, start_index, selected))
    }

    fn parse_subject_pick_record_selector(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ProjectionPath> {
        let checkpoint = *cursor;
        let open_brace = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[open_brace].line_start()
            || self.tokens[open_brace].kind() != TokenKind::LBrace
        {
            return None;
        }
        *cursor = open_brace + 1;
        let first = match self.parse_identifier(cursor, end) {
            Some(first) => first,
            None => {
                *cursor = checkpoint;
                return None;
            }
        };
        let mut fields = vec![first];
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            let Some(segment) = self.parse_identifier(cursor, end) else {
                *cursor = checkpoint;
                return None;
            };
            fields.push(segment);
        }
        let Some(bang_index) = self.consume_kind(cursor, end, TokenKind::Bang) else {
            *cursor = checkpoint;
            return None;
        };
        let Some(close_brace) = self.consume_kind(cursor, end, TokenKind::RBrace) else {
            *cursor = checkpoint;
            return None;
        };
        let path_start = fields
            .first()
            .map(|field| field.span.span().start())
            .unwrap_or_else(|| self.tokens[open_brace].span().start());
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(path_start, self.tokens[bang_index].span().end()),
        );
        let _ = self.source_span_for_range(open_brace, close_brace + 1);
        Some(ProjectionPath { span, fields })
    }

    fn emit_invalid_subject_pick(&mut self, span: SourceSpan, message: &str, label: &str) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(INVALID_SUBJECT_PICK)
                .with_primary_label(span, label),
        );
    }

    fn implicit_function_subject_parameter_at(&self, span: SourceSpan) -> FunctionParam {
        FunctionParam {
            name: Some(Identifier {
                text: IMPLICIT_FUNCTION_SUBJECT_NAME.to_owned(),
                span,
            }),
            annotation: None,
            span,
        }
    }

    fn implicit_function_subject_expr_at(
        &self,
        parameter: &FunctionParam,
        span: SourceSpan,
    ) -> Expr {
        let name = parameter
            .name
            .clone()
            .expect("implicit unary-function parameter should keep its name");
        Expr {
            span,
            kind: ExprKind::Name(name),
        }
    }

    fn rewrite_free_function_subject_expr(
        &self,
        expr: Expr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (Expr, bool) {
        let span = expr.span;
        match expr.kind {
            ExprKind::SubjectPlaceholder if !ambient_allowed => (
                self.implicit_function_subject_expr_at(parameter, span),
                true,
            ),
            ExprKind::AmbientProjection(path) if !ambient_allowed => (
                Expr {
                    span,
                    kind: ExprKind::Projection {
                        base: Box::new(self.implicit_function_subject_expr_at(parameter, span)),
                        path,
                    },
                },
                true,
            ),
            ExprKind::Group(inner) => {
                let (inner, changed) =
                    self.rewrite_free_function_subject_expr(*inner, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Group(Box::new(inner)),
                    },
                    changed,
                )
            }
            ExprKind::Tuple(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Tuple(elements),
                    },
                    changed,
                )
            }
            ExprKind::List(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::List(elements),
                    },
                    changed,
                )
            }
            ExprKind::Map(map) => {
                let (map, changed) =
                    self.rewrite_free_function_subject_map_expr(map, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Map(map),
                    },
                    changed,
                )
            }
            ExprKind::Set(elements) => {
                let mut changed = false;
                let elements = elements
                    .into_iter()
                    .map(|element| {
                        let (element, element_changed) = self.rewrite_free_function_subject_expr(
                            element,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= element_changed;
                        element
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Set(elements),
                    },
                    changed,
                )
            }
            ExprKind::Record(record) => {
                // Detect record projection pattern: { field: . } or { a.b.c: . }
                // When a field value is SubjectPlaceholder, this extracts a field from
                // the subject rather than constructing a record.
                if !ambient_allowed
                    && let Some(proj_field) = record.fields.iter().find(|f| {
                        matches!(
                            f.value.as_ref().map(|v| &v.kind),
                            Some(ExprKind::SubjectPlaceholder)
                        )
                    }) {
                        let mut fields = vec![proj_field.label.clone()];
                        fields.extend(proj_field.label_path.iter().cloned());
                        let path = ProjectionPath {
                            span: proj_field.span,
                            fields,
                        };
                        return (
                            Expr {
                                span,
                                kind: ExprKind::Projection {
                                    base: Box::new(
                                        self.implicit_function_subject_expr_at(parameter, span),
                                    ),
                                    path,
                                },
                            },
                            true,
                        );
                    }
                let (record, changed) = self.rewrite_free_function_subject_record_expr(
                    record,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Record(record),
                    },
                    changed,
                )
            }
            ExprKind::Text(text) => {
                let (text, changed) = self.rewrite_free_function_subject_text_literal(
                    text,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Text(text),
                    },
                    changed,
                )
            }
            ExprKind::Range { start, end } => {
                let (start, start_changed) =
                    self.rewrite_free_function_subject_expr(*start, parameter, ambient_allowed);
                let (end, end_changed) =
                    self.rewrite_free_function_subject_expr(*end, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Range {
                            start: Box::new(start),
                            end: Box::new(end),
                        },
                    },
                    start_changed || end_changed,
                )
            }
            ExprKind::Projection { base, path } => {
                let (base, changed) =
                    self.rewrite_free_function_subject_expr(*base, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Projection {
                            base: Box::new(base),
                            path,
                        },
                    },
                    changed,
                )
            }
            ExprKind::Apply { callee, arguments } => {
                let (callee, callee_changed) =
                    self.rewrite_free_function_subject_expr(*callee, parameter, ambient_allowed);
                let mut changed = callee_changed;
                let arguments = arguments
                    .into_iter()
                    .map(|argument| {
                        let (argument, argument_changed) = self.rewrite_free_function_subject_expr(
                            argument,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= argument_changed;
                        argument
                    })
                    .collect();
                (
                    Expr {
                        span,
                        kind: ExprKind::Apply {
                            callee: Box::new(callee),
                            arguments,
                        },
                    },
                    changed,
                )
            }
            ExprKind::Unary { operator, expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(*expr, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Unary {
                            operator,
                            expr: Box::new(expr),
                        },
                    },
                    changed,
                )
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let (left, left_changed) =
                    self.rewrite_free_function_subject_expr(*left, parameter, ambient_allowed);
                let (right, right_changed) =
                    self.rewrite_free_function_subject_expr(*right, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Binary {
                            left: Box::new(left),
                            operator,
                            right: Box::new(right),
                        },
                    },
                    left_changed || right_changed,
                )
            }
            ExprKind::ResultBlock(block) => {
                let (block, changed) = self.rewrite_free_function_subject_result_block(
                    block,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::ResultBlock(block),
                    },
                    changed,
                )
            }
            ExprKind::PatchApply { target, patch } => {
                let (target, target_changed) =
                    self.rewrite_free_function_subject_expr(*target, parameter, ambient_allowed);
                let (patch, patch_changed) = self.rewrite_free_function_subject_patch_block(
                    patch,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::PatchApply {
                            target: Box::new(target),
                            patch,
                        },
                    },
                    target_changed || patch_changed,
                )
            }
            ExprKind::PatchLiteral(patch) => {
                let (patch, changed) = self.rewrite_free_function_subject_patch_block(
                    patch,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::PatchLiteral(patch),
                    },
                    changed,
                )
            }
            ExprKind::Pipe(pipe) => {
                let (pipe, changed) =
                    self.rewrite_free_function_subject_pipe_expr(pipe, parameter, ambient_allowed);
                (
                    Expr {
                        span,
                        kind: ExprKind::Pipe(pipe),
                    },
                    changed,
                )
            }
            ExprKind::Markup(node) => {
                let (node, changed) = self.rewrite_free_function_subject_markup_node(
                    node,
                    parameter,
                    ambient_allowed,
                );
                (
                    Expr {
                        span,
                        kind: ExprKind::Markup(node),
                    },
                    changed,
                )
            }
            kind => (Expr { span, kind }, false),
        }
    }

    fn rewrite_free_function_subject_text_literal(
        &self,
        text: TextLiteral,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (TextLiteral, bool) {
        let mut changed = false;
        let segments = text
            .segments
            .into_iter()
            .map(|segment| match segment {
                TextSegment::Text(fragment) => TextSegment::Text(fragment),
                TextSegment::Interpolation(interpolation) => {
                    let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                        *interpolation.expr,
                        parameter,
                        ambient_allowed,
                    );
                    changed |= expr_changed;
                    TextSegment::Interpolation(TextInterpolation {
                        expr: Box::new(expr),
                        span: interpolation.span,
                    })
                }
            })
            .collect();
        (
            TextLiteral {
                span: text.span,
                segments,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_record_expr(
        &self,
        record: RecordExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (RecordExpr, bool) {
        let mut changed = false;
        let fields = record
            .fields
            .into_iter()
            .map(|field| {
                let value = field.value.map(|value| {
                    let (value, value_changed) =
                        self.rewrite_free_function_subject_expr(value, parameter, ambient_allowed);
                    changed |= value_changed;
                    value
                });
                RecordField {
                    label: field.label,
                    label_path: field.label_path,
                    value,
                    span: field.span,
                }
            })
            .collect();
        (
            RecordExpr {
                fields,
                span: record.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_map_expr(
        &self,
        map: MapExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (MapExpr, bool) {
        let mut changed = false;
        let entries = map
            .entries
            .into_iter()
            .map(|entry| {
                let (key, key_changed) =
                    self.rewrite_free_function_subject_expr(entry.key, parameter, ambient_allowed);
                let (value, value_changed) = self.rewrite_free_function_subject_expr(
                    entry.value,
                    parameter,
                    ambient_allowed,
                );
                changed |= key_changed || value_changed;
                MapExprEntry {
                    key,
                    value,
                    span: entry.span,
                }
            })
            .collect();
        (
            MapExpr {
                entries,
                span: map.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_result_block(
        &self,
        block: ResultBlockExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (ResultBlockExpr, bool) {
        let mut changed = false;
        let bindings = block
            .bindings
            .into_iter()
            .map(|binding| {
                let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                    binding.expr,
                    parameter,
                    ambient_allowed,
                );
                changed |= expr_changed;
                ResultBinding {
                    name: binding.name,
                    expr,
                    span: binding.span,
                }
            })
            .collect();
        let tail = block.tail.map(|tail| {
            let (tail, tail_changed) =
                self.rewrite_free_function_subject_expr(*tail, parameter, ambient_allowed);
            changed |= tail_changed;
            Box::new(tail)
        });
        (
            ResultBlockExpr {
                bindings,
                tail,
                span: block.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_patch_block(
        &self,
        patch: PatchBlock,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (PatchBlock, bool) {
        let mut changed = false;
        let entries = patch
            .entries
            .into_iter()
            .map(|entry| {
                let segments = entry
                    .selector
                    .segments
                    .into_iter()
                    .map(|segment| match segment {
                        PatchSelectorSegment::BracketExpr { expr, span } => {
                            let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                                *expr,
                                parameter,
                                ambient_allowed,
                            );
                            changed |= expr_changed;
                            PatchSelectorSegment::BracketExpr {
                                expr: Box::new(expr),
                                span,
                            }
                        }
                        other => other,
                    })
                    .collect();
                let instruction_kind = match entry.instruction.kind {
                    PatchInstructionKind::Replace(expr) => {
                        let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                            *expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= expr_changed;
                        PatchInstructionKind::Replace(Box::new(expr))
                    }
                    PatchInstructionKind::Store(expr) => {
                        let (expr, expr_changed) = self.rewrite_free_function_subject_expr(
                            *expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= expr_changed;
                        PatchInstructionKind::Store(Box::new(expr))
                    }
                    PatchInstructionKind::Remove => PatchInstructionKind::Remove,
                };
                PatchEntry {
                    selector: PatchSelector {
                        segments,
                        span: entry.selector.span,
                    },
                    instruction: PatchInstruction {
                        kind: instruction_kind,
                        span: entry.instruction.span,
                    },
                    span: entry.span,
                }
            })
            .collect();
        (
            PatchBlock {
                entries,
                span: patch.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_pipe_expr(
        &self,
        pipe: PipeExpr,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (PipeExpr, bool) {
        let mut changed = false;
        let head = pipe.head.map(|head| {
            let (head, head_changed) =
                self.rewrite_free_function_subject_expr(*head, parameter, ambient_allowed);
            changed |= head_changed;
            Box::new(head)
        });
        let stages = pipe
            .stages
            .into_iter()
            .map(|stage| {
                let (stage, stage_changed) =
                    self.rewrite_free_function_subject_pipe_stage(stage, parameter);
                changed |= stage_changed;
                stage
            })
            .collect();
        (
            PipeExpr {
                head,
                stages,
                span: pipe.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_pipe_stage(
        &self,
        stage: PipeStage,
        parameter: &FunctionParam,
    ) -> (PipeStage, bool) {
        let (kind, changed) = match stage.kind {
            PipeStageKind::Transform { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Transform { expr }, changed)
            }
            PipeStageKind::Gate { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Gate { expr }, changed)
            }
            PipeStageKind::Case(arm) => {
                let (body, changed) =
                    self.rewrite_free_function_subject_expr(arm.body, parameter, true);
                (
                    PipeStageKind::Case(PipeCaseArm {
                        pattern: arm.pattern,
                        body,
                        span: arm.span,
                    }),
                    changed,
                )
            }
            PipeStageKind::Map { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Map { expr }, changed)
            }
            PipeStageKind::Apply { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Apply { expr }, changed)
            }
            PipeStageKind::ClusterFinalizer { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::ClusterFinalizer { expr }, changed)
            }
            PipeStageKind::RecurStart { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::RecurStart { expr }, changed)
            }
            PipeStageKind::RecurStep { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::RecurStep { expr }, changed)
            }
            PipeStageKind::Tap { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Tap { expr }, changed)
            }
            PipeStageKind::FanIn { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::FanIn { expr }, changed)
            }
            PipeStageKind::Truthy { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Truthy { expr }, changed)
            }
            PipeStageKind::Falsy { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Falsy { expr }, changed)
            }
            PipeStageKind::Validate { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Validate { expr }, changed)
            }
            PipeStageKind::Previous { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Previous { expr }, changed)
            }
            PipeStageKind::Accumulate { seed, step } => {
                let (seed, seed_changed) =
                    self.rewrite_free_function_subject_expr(seed, parameter, true);
                let (step, step_changed) =
                    self.rewrite_free_function_subject_expr(step, parameter, true);
                (
                    PipeStageKind::Accumulate { seed, step },
                    seed_changed || step_changed,
                )
            }
            PipeStageKind::Diff { expr } => {
                let (expr, changed) =
                    self.rewrite_free_function_subject_expr(expr, parameter, true);
                (PipeStageKind::Diff { expr }, changed)
            }
            PipeStageKind::Delay { duration } => {
                let (duration, changed) =
                    self.rewrite_free_function_subject_expr(duration, parameter, true);
                (PipeStageKind::Delay { duration }, changed)
            }
            PipeStageKind::Burst { every, count } => {
                let (every, every_changed) =
                    self.rewrite_free_function_subject_expr(every, parameter, true);
                let (count, count_changed) =
                    self.rewrite_free_function_subject_expr(count, parameter, true);
                (
                    PipeStageKind::Burst { every, count },
                    every_changed || count_changed,
                )
            }
        };
        (
            PipeStage {
                subject_memo: stage.subject_memo,
                result_memo: stage.result_memo,
                kind,
                span: stage.span,
            },
            changed,
        )
    }

    fn rewrite_free_function_subject_markup_node(
        &self,
        node: MarkupNode,
        parameter: &FunctionParam,
        ambient_allowed: bool,
    ) -> (MarkupNode, bool) {
        let mut changed = false;
        let attributes = node
            .attributes
            .into_iter()
            .map(|attribute| {
                let value = attribute.value.map(|value| match value {
                    MarkupAttributeValue::Text(text) => {
                        let (text, value_changed) = self
                            .rewrite_free_function_subject_text_literal(
                                text,
                                parameter,
                                ambient_allowed,
                            );
                        changed |= value_changed;
                        MarkupAttributeValue::Text(text)
                    }
                    MarkupAttributeValue::Expr(expr) => {
                        let (expr, value_changed) = self.rewrite_free_function_subject_expr(
                            expr,
                            parameter,
                            ambient_allowed,
                        );
                        changed |= value_changed;
                        MarkupAttributeValue::Expr(expr)
                    }
                    MarkupAttributeValue::Pattern(pattern) => {
                        MarkupAttributeValue::Pattern(pattern)
                    }
                });
                MarkupAttribute {
                    name: attribute.name,
                    value,
                    span: attribute.span,
                }
            })
            .collect();
        let children = node
            .children
            .into_iter()
            .map(|child| {
                let (child, child_changed) = self.rewrite_free_function_subject_markup_node(
                    child,
                    parameter,
                    ambient_allowed,
                );
                changed |= child_changed;
                child
            })
            .collect();
        (
            MarkupNode {
                name: node.name,
                attributes,
                children,
                close_name: node.close_name,
                self_closing: node.self_closing,
                span: node.span,
            },
            changed,
        )
    }

}
