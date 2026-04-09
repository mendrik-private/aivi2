impl<'a> Parser<'a> {
    fn parse_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_patch_apply_expr(cursor, end, stop);
        self.depth_exit();
        result
    }

    fn parse_patch_apply_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let expr = self.parse_pipe_expr(cursor, end, stop)?;
        Some(self.parse_patch_apply_suffix(expr, cursor, end, stop))
    }

    fn parse_patch_apply_suffix(
        &mut self,
        mut expr: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Expr {
        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop)
                || self.tokens[index].kind() != TokenKind::PatchApply
            {
                break;
            }
            *cursor = index + 1;
            let Some(patch) = self.parse_patch_block(cursor, end) else {
                break;
            };
            let span = self.join_spans(expr.span, patch.span);
            expr = Expr {
                span,
                kind: ExprKind::PatchApply {
                    target: Box::new(expr),
                    patch,
                },
            };
        }
        expr
    }

    fn parse_subject_root_expr_from_head(
        &mut self,
        start: usize,
        head: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let expr = self.parse_pipe_expr_from_head(start, Some(head), cursor, end, stop)?;
        Some(self.parse_patch_apply_suffix(expr, cursor, end, stop))
    }

    fn parse_pipe_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        let start = *cursor;
        let head = if self.peek_kind(*cursor, end) == Some(TokenKind::PipeApply) {
            None
        } else {
            Some(self.parse_range_expr(cursor, end, stop.with_pipe_stage())?)
        };
        self.parse_pipe_expr_from_head(start, head, cursor, end, stop)
    }

    fn parse_pipe_expr_from_head(
        &mut self,
        start: usize,
        head: Option<Expr>,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut head = head.map(Box::new);
        let mut stages = Vec::new();
        let mut cluster_active = false;

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.expr_should_stop(index, stop) {
                break;
            }
            let kind = self.tokens[index].kind();
            if !kind.is_pipe_operator() {
                break;
            }
            *cursor = index + 1;
            let (subject_memo, stage_kind, result_memo) = match kind {
                TokenKind::PipeTransform => {
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let temporal_stage = if !cluster_active {
                        self.try_parse_prefixed_temporal_pipe_stage(
                            cursor,
                            end,
                            stop.with_pipe_stage().with_hash(),
                        )?
                    } else {
                        None
                    };
                    if let Some(stage_kind) = temporal_stage {
                        let result_memo = self.parse_optional_pipe_memo(cursor, end);
                        (subject_memo, stage_kind, result_memo)
                    } else {
                        let expr = self.parse_patch_apply_expr(
                            cursor,
                            end,
                            stop.with_pipe_stage().with_hash(),
                        )?;
                        let result_memo = self.parse_optional_pipe_memo(cursor, end);
                        if cluster_active {
                            cluster_active = false;
                            (
                                subject_memo,
                                PipeStageKind::ClusterFinalizer { expr },
                                result_memo,
                            )
                        } else {
                            (subject_memo, PipeStageKind::Transform { expr }, result_memo)
                        }
                    }
                }
                TokenKind::PipeGate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Gate { expr }, result_memo)
                }
                TokenKind::PipeCase => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let arm =
                        self.parse_pipe_case_arm(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Case(arm), result_memo)
                }
                TokenKind::PipeMap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Map { expr }, result_memo)
                }
                TokenKind::PipeApply => {
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    cluster_active = true;
                    (subject_memo, PipeStageKind::Apply { expr }, result_memo)
                }
                TokenKind::PipeRecurStart => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (
                        subject_memo,
                        PipeStageKind::RecurStart { expr },
                        result_memo,
                    )
                }
                TokenKind::PipeRecurStep => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::RecurStep { expr }, result_memo)
                }
                TokenKind::PipeTap => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Tap { expr }, result_memo)
                }
                TokenKind::PipeFanIn => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::FanIn { expr }, result_memo)
                }
                TokenKind::TruthyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Truthy { expr }, result_memo)
                }
                TokenKind::FalsyBranch => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Falsy { expr }, result_memo)
                }
                TokenKind::PipeValidate => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Validate { expr }, result_memo)
                }
                TokenKind::PipePrevious => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Previous { expr }, result_memo)
                }
                TokenKind::PipeAccumulate => {
                    // `signal +|> seed (state input => next)`
                    // The seed expression comes first, then the step function expression.
                    // Parse the seed with atomic-expression boundaries so an adjacent step name
                    // does not get swallowed as an application (`+|> 0 step`).
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let seed =
                        self.parse_atomic_expr(cursor, end, stop.with_pipe_stage().with_hash())?;
                    let step = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (
                        subject_memo,
                        PipeStageKind::Accumulate { seed, step },
                        result_memo,
                    )
                }
                TokenKind::PipeDiff => {
                    cluster_active = false;
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let expr = self.parse_patch_apply_expr(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, PipeStageKind::Diff { expr }, result_memo)
                }
                TokenKind::PipeDelay => {
                    cluster_active = false;
                    self.emit_removed_temporal_pipe_operator(index, "|> delay <duration>");
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let stage_kind = self.parse_delay_pipe_stage(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, stage_kind, result_memo)
                }
                TokenKind::PipeBurst => {
                    cluster_active = false;
                    self.emit_removed_temporal_pipe_operator(index, "|> burst <duration> <count>");
                    let subject_memo = self.parse_optional_pipe_memo(cursor, end);
                    let stage_kind = self.parse_burst_pipe_stage(
                        cursor,
                        end,
                        stop.with_pipe_stage().with_hash(),
                    )?;
                    let result_memo = self.parse_optional_pipe_memo(cursor, end);
                    (subject_memo, stage_kind, result_memo)
                }
                _ => break,
            };
            stages.push(PipeStage {
                subject_memo,
                result_memo,
                span: self.source_span_for_range(index, *cursor),
                kind: stage_kind,
            });
        }

        if stages.is_empty() {
            return head.map(|expr| *expr);
        }

        let span = self.source_span_for_range(start, *cursor);
        Some(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head: head.take(),
                stages,
                span,
            }),
        })
    }

    fn try_parse_prefixed_temporal_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Option<PipeStageKind>> {
        let Some(index) = self.peek_nontrivia(*cursor, end) else {
            return Some(None);
        };
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Identifier
        {
            return Some(None);
        }
        if self.is_identifier_text(index, "delay") {
            *cursor = index + 1;
            return Some(Some(self.parse_delay_pipe_stage(cursor, end, stop)?));
        }
        if self.is_identifier_text(index, "burst") {
            *cursor = index + 1;
            return Some(Some(self.parse_burst_pipe_stage(cursor, end, stop)?));
        }
        Some(None)
    }

    fn parse_delay_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<PipeStageKind> {
        let duration = self.parse_patch_apply_expr(cursor, end, stop)?;
        Some(PipeStageKind::Delay { duration })
    }

    fn parse_burst_pipe_stage(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<PipeStageKind> {
        let every = self.parse_atomic_expr(cursor, end, stop)?;
        let count = self.parse_atomic_expr(cursor, end, stop)?;
        Some(PipeStageKind::Burst { every, count })
    }

    fn emit_removed_temporal_pipe_operator(&mut self, index: usize, replacement: &str) {
        let operator = self.tokens[index].text(self.source);
        self.diagnostics.push(
            Diagnostic::error(format!("`{operator}` has been removed"))
                .with_code(REMOVED_TEMPORAL_PIPE_OPERATOR)
                .with_primary_label(
                    self.source_span_of_token(index),
                    "rewrite this temporal stage using the new prefix form",
                )
                .with_help(format!("use `{replacement}` instead")),
        );
    }

    fn parse_range_expr(&mut self, cursor: &mut usize, end: usize, stop: ExprStop) -> Option<Expr> {
        let start = self.parse_binary_expr(cursor, end, stop)?;
        let Some(index) = self.peek_nontrivia(*cursor, end) else {
            return Some(start);
        };
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::DotDot {
            return Some(start);
        }

        *cursor = index + 1;
        let end_expr = self.parse_binary_expr(cursor, end, stop)?;
        let span = self.join_spans(start.span, end_expr.span);
        Some(Expr {
            span,
            kind: ExprKind::Range {
                start: Box::new(start),
                end: Box::new(end_expr),
            },
        })
    }

    fn parse_optional_pipe_memo(&mut self, cursor: &mut usize, end: usize) -> Option<Identifier> {
        let Some(hash_index) = self.consume_kind(cursor, end, TokenKind::Hash) else {
            return None;
        };
        match self.parse_identifier(cursor, end) {
            Some(identifier) => Some(identifier),
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`#` in a pipe stage must be followed by a memo name")
                        .with_code(MISSING_PIPE_MEMO_NAME)
                        .with_primary_label(
                            self.source_span_of_token(hash_index),
                            "add an identifier such as `#value` here",
                        ),
                );
                None
            }
        }
    }

    fn parse_binary_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        self.parse_binary_expr_prec(cursor, end, stop, 0)
    }

    fn parse_binary_expr_prec(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
        min_precedence: u8,
    ) -> Option<Expr> {
        let mut left = self.parse_application_expr(cursor, end, stop)?;

        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop) {
                break;
            }
            let Some((operator, precedence)) = self.binary_operator(index) else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            *cursor = index + 1;
            let right = self.parse_binary_expr_prec(cursor, end, stop, precedence + 1)?;
            left = self.make_binary_expr(left, operator, right);
        }

        Some(left)
    }

    fn parse_application_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut expr = self.parse_atomic_expr(cursor, end, stop)?;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.expr_should_stop(index, stop)
                || self.tokens[index].kind().is_pipe_operator()
                || (self.binary_operator(index).is_some()
                    && !self.starts_negative_numeric_application_argument(&expr, index, end))
            {
                break;
            }
            if self.tokens[index].line_start() || !self.starts_expr(index) {
                break;
            }
            let argument = self.parse_atomic_expr(cursor, end, stop)?;
            expr = self.make_apply_expr(expr, argument);
        }
        Some(expr)
    }

    fn starts_negative_numeric_application_argument(
        &self,
        callee: &Expr,
        index: usize,
        end: usize,
    ) -> bool {
        self.tokens[index].kind() == TokenKind::Minus
            && self.negative_numeric_literal_token(index, end).is_some()
            && self.expr_can_accept_negative_numeric_argument(callee)
    }

    fn expr_can_accept_negative_numeric_argument(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Name(_)
            | ExprKind::AmbientProjection(_)
            | ExprKind::Projection { .. }
            | ExprKind::Apply { .. } => true,
            ExprKind::Group(inner) => self.expr_can_accept_negative_numeric_argument(inner),
            _ => false,
        }
    }

    fn parse_atomic_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let mut expr = self.parse_prefix_expr(cursor, end, stop)?;
        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                break;
            };
            if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Dot {
                break;
            }
            expr = self.parse_projection_suffix(expr, cursor, end, stop)?;
        }
        Some(expr)
    }

    fn parse_prefix_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) {
            return None;
        }
        if self.tokens[index].kind() == TokenKind::Identifier
            && self.is_identifier_text(index, "not")
        {
            *cursor = index + 1;
            let expr = self.parse_prefix_expr(cursor, end, stop)?;
            let span = self.source_span_for_range(index, *cursor);
            return Some(Expr {
                span,
                kind: ExprKind::Unary {
                    operator: UnaryOperator::Not,
                    expr: Box::new(expr),
                },
            });
        }
        if let Some(expr) = self.parse_negative_numeric_expr(cursor, end, stop) {
            return Some(expr);
        }
        self.parse_primary_expr(cursor, end, stop)
    }

    fn parse_negative_numeric_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let minus_index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(minus_index, stop)
            || self.tokens[minus_index].kind() != TokenKind::Minus
        {
            return None;
        }
        let literal_index = self.negative_numeric_literal_token(minus_index, end)?;
        *cursor = literal_index + 1;
        match self.tokens[literal_index].kind() {
            TokenKind::Integer => Some(self.parse_integer_expr_with_sign(
                Some(minus_index),
                literal_index,
                cursor,
                end,
            )),
            TokenKind::Float => {
                Some(self.parse_float_expr_with_sign(Some(minus_index), literal_index))
            }
            TokenKind::Decimal => {
                Some(self.parse_decimal_expr_with_sign(Some(minus_index), literal_index))
            }
            TokenKind::BigInt => {
                Some(self.parse_bigint_expr_with_sign(Some(minus_index), literal_index))
            }
            _ => None,
        }
    }

    fn parse_primary_expr(
        &mut self,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) {
            return None;
        }

        match self.tokens[index].kind() {
            TokenKind::PatchKw => self.parse_patch_literal_expr(cursor, end),
            TokenKind::Identifier => {
                if self.is_identifier_text(index, "result")
                    && self
                        .peek_nontrivia(index + 1, end)
                        .is_some_and(|next| self.tokens[next].kind() == TokenKind::LBrace)
                {
                    return self.parse_result_block_expr(cursor, end);
                }
                if self.starts_prefixed_collection_literal(index, end, "Map", TokenKind::LBrace) {
                    return self.parse_map_expr(cursor, end).map(|map| Expr {
                        span: map.span,
                        kind: ExprKind::Map(map),
                    });
                }
                if self.starts_prefixed_collection_literal(index, end, "Set", TokenKind::LBracket) {
                    return self.parse_set_expr(cursor, end);
                }
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                if name.text == "_" {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "the discard `_` is only valid in binder and pattern positions",
                        )
                        .with_code(INVALID_DISCARD_EXPR)
                        .with_primary_label(
                            name.span,
                            "use `.` for the current subject placeholder",
                        ),
                    );
                }
                Some(Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                })
            }
            TokenKind::Integer => {
                *cursor = index + 1;
                Some(self.parse_integer_expr(index, cursor, end))
            }
            TokenKind::Float => {
                *cursor = index + 1;
                Some(self.parse_float_expr(index))
            }
            TokenKind::Decimal => {
                *cursor = index + 1;
                Some(self.parse_decimal_expr(index))
            }
            TokenKind::BigInt => {
                *cursor = index + 1;
                Some(self.parse_bigint_expr(index))
            }
            TokenKind::StringLiteral => {
                *cursor = index + 1;
                let literal = self.text_literal_from_token(index);
                Some(Expr {
                    span: literal.span,
                    kind: ExprKind::Text(literal),
                })
            }
            TokenKind::RegexLiteral => {
                *cursor = index + 1;
                let span = self.source_span_of_token(index);
                Some(Expr {
                    span,
                    kind: ExprKind::Regex(RegexLiteral {
                        raw: self.tokens[index].text(self.source).to_owned(),
                        span,
                    }),
                })
            }
            TokenKind::Dot => self.parse_ambient_projection(cursor, end),
            TokenKind::LParen => self.parse_grouped_expr(cursor, end),
            TokenKind::LBracket => self.parse_list_expr(cursor, end),
            TokenKind::LBrace => self.parse_record_expr(cursor, end).map(|record| Expr {
                span: record.span,
                kind: ExprKind::Record(record),
            }),
            TokenKind::Less => self.parse_markup_expr(cursor, end),
            kind if kind.is_keyword() => {
                *cursor = index + 1;
                let name = self.identifier_from_token(index);
                Some(Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                })
            }
            _ => None,
        }
    }

    fn parse_patch_literal_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::PatchKw)?;
        let patch = self.parse_patch_block(cursor, end)?;
        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::PatchLiteral(patch),
        })
    }

    fn parse_patch_block(&mut self, cursor: &mut usize, end: usize) -> Option<PatchBlock> {
        let start = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut entries = Vec::new();
        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let selector = self.parse_patch_selector(cursor, end)?;
            let _ = self.consume_kind(cursor, end, TokenKind::Colon)?;
            let instruction = self.parse_patch_instruction(cursor, end)?;
            let span = self.join_spans(selector.span, instruction.span);
            entries.push(PatchEntry {
                selector,
                instruction,
                span,
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_some() {
                continue;
            }
            if let Some(index) = self.peek_nontrivia(*cursor, end) {
                if self.tokens[index].kind() == TokenKind::RBrace {
                    let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                    break;
                }
                if self.tokens[index].line_start() && self.token_starts_patch_selector(index) {
                    continue;
                }
            }
            break;
        }

        Some(PatchBlock {
            entries,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_patch_selector(&mut self, cursor: &mut usize, end: usize) -> Option<PatchSelector> {
        let start_index = self.peek_nontrivia(*cursor, end)?;
        let mut segments = Vec::new();
        loop {
            if let Some(start) = self.consume_kind(cursor, end, TokenKind::Dot) {
                let name = self.parse_identifier(cursor, end)?;
                let span = self.source_span_for_range(start, *cursor);
                segments.push(PatchSelectorSegment::Named {
                    name,
                    dotted: true,
                    span,
                });
                continue;
            }
            if self.peek_kind(*cursor, end) == Some(TokenKind::LBracket) {
                segments.push(self.parse_patch_bracket_selector(cursor, end)?);
                continue;
            }
            if segments.is_empty() {
                if let Some(name) = self.parse_identifier(cursor, end) {
                    let span = name.span;
                    segments.push(PatchSelectorSegment::Named {
                        name,
                        dotted: false,
                        span,
                    });
                    continue;
                }
            }
            break;
        }
        if segments.is_empty() {
            return None;
        }
        let end_span = patch_selector_segment_span(segments.last().expect("non-empty segments"));
        Some(PatchSelector {
            segments,
            span: SourceSpan::new(
                self.source.id(),
                Span::new(
                    self.tokens[start_index].span().start(),
                    end_span.span().end(),
                ),
            ),
        })
    }

    fn parse_patch_bracket_selector(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<PatchSelectorSegment> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        if self.consume_kind(cursor, end, TokenKind::Star).is_some() {
            let close = self.consume_kind(cursor, end, TokenKind::RBracket)?;
            return Some(PatchSelectorSegment::BracketTraverse {
                span: self.source_span_for_range(start, close + 1),
            });
        }
        let expr = self.parse_expr(cursor, end, ExprStop::list_context())?;
        let _ = self.consume_kind(cursor, end, TokenKind::RBracket)?;
        Some(PatchSelectorSegment::BracketExpr {
            span: self.source_span_for_range(start, *cursor),
            expr: Box::new(expr),
        })
    }

    fn parse_patch_instruction(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<PatchInstruction> {
        if let Some(start) = self.consume_kind(cursor, end, TokenKind::ColonEquals) {
            let expr = self.parse_expr(cursor, end, ExprStop::patch_entry_context())?;
            return Some(PatchInstruction {
                span: self.source_span_for_range(start, *cursor),
                kind: PatchInstructionKind::Store(Box::new(expr)),
            });
        }
        if let Some(start) = self.consume_kind(cursor, end, TokenKind::Minus) {
            return Some(PatchInstruction {
                span: self.source_span_of_token(start),
                kind: PatchInstructionKind::Remove,
            });
        }
        let expr = self.parse_expr(cursor, end, ExprStop::patch_entry_context())?;
        Some(PatchInstruction {
            span: expr.span,
            kind: PatchInstructionKind::Replace(Box::new(expr)),
        })
    }

    fn parse_result_block_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let keyword_index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_identifier_text(keyword_index, "result") {
            return None;
        }
        *cursor = keyword_index + 1;
        let start = keyword_index;
        let open_brace = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let Some(close_brace) = self.find_matching_brace(open_brace, end) else {
            self.diagnostics.push(
                Diagnostic::error("`result { ... }` block is missing a closing `}`")
                    .with_code(MISSING_RESULT_BLOCK_TAIL)
                    .with_primary_label(
                        self.source_span_of_token(open_brace),
                        "close this `result` block with `}`",
                    ),
            );
            return None;
        };

        let mut bindings = Vec::new();
        let mut tail = None;

        while let Some(index) = self.peek_nontrivia(*cursor, close_brace) {
            if let Some((name, left_arrow)) = self.result_block_binding_start(index, close_brace) {
                let item_end =
                    self.find_next_result_block_item_boundary(left_arrow + 1, close_brace);
                let mut binding_cursor = left_arrow + 1;
                let expr = self
                    .parse_expr(&mut binding_cursor, item_end, ExprStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "result block bindings must have an expression after `<-`",
                            )
                            .with_code(MISSING_RESULT_BINDING_EXPR)
                            .with_primary_label(
                                self.source_span_of_token(left_arrow),
                                "add a `Result ...` expression after this binding arrow",
                            ),
                        );
                        None
                    })?;
                if let Some(trailing_index) =
                    self.next_significant_in_range(binding_cursor, item_end)
                {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "result block bindings must contain exactly one expression",
                        )
                        .with_code(MISSING_RESULT_BINDING_EXPR)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "move this token into the binding expression or start a new block line",
                        ),
                    );
                }
                let span = SourceSpan::new(
                    self.source.id(),
                    Span::new(name.span.span().start(), expr.span.span().end()),
                );
                bindings.push(ResultBinding { name, expr, span });
                *cursor = item_end;
                continue;
            }

            let mut tail_cursor = index;
            let expr = self
                .parse_expr(&mut tail_cursor, close_brace, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("result blocks must end with a final expression")
                            .with_code(MISSING_RESULT_BLOCK_TAIL)
                            .with_primary_label(
                                self.source_span_of_token(index),
                                "add the final success value here",
                            ),
                    );
                    None
                })?;
            if let Some(trailing_index) = self.next_significant_in_range(tail_cursor, close_brace) {
                self.diagnostics.push(
                    Diagnostic::error("result block tails must contain exactly one expression")
                        .with_code(MISSING_RESULT_BLOCK_TAIL)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "move this token into the tail expression or close the block",
                        ),
                );
            }
            tail = Some(Box::new(expr));
            *cursor = close_brace;
            break;
        }

        *cursor = close_brace;
        let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
        let span = self.source_span_for_range(start, *cursor);
        if bindings.is_empty() && tail.is_none() {
            self.diagnostics.push(
                Diagnostic::error("result blocks cannot be empty")
                    .with_code(EMPTY_RESULT_BLOCK)
                    .with_primary_label(
                        span,
                        "add a binding or final expression inside this block",
                    ),
            );
        }
        Some(Expr {
            span,
            kind: ExprKind::ResultBlock(ResultBlockExpr {
                bindings,
                tail,
                span,
            }),
        })
    }

    fn result_block_binding_start(&self, index: usize, end: usize) -> Option<(Identifier, usize)> {
        if self.tokens.get(index)?.kind() != TokenKind::Identifier {
            return None;
        }
        let left_arrow = self.peek_nontrivia(index + 1, end)?;
        (self.tokens[left_arrow].kind() == TokenKind::LeftArrow)
            .then(|| (self.identifier_from_token(index), left_arrow))
    }

    fn find_matching_brace(&self, open_brace: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        for index in open_brace..end {
            match self.tokens[index].kind() {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn find_next_result_block_item_boundary(&self, start: usize, close_brace: usize) -> usize {
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        for index in start..close_brace {
            match self.tokens[index].kind() {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => {
                    if brace_depth == 0 {
                        return index;
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                }
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }

            if paren_depth == 0
                && brace_depth == 0
                && bracket_depth == 0
                && self.tokens[index].kind() == TokenKind::Newline
            {
                let Some(next) = self.peek_nontrivia(index + 1, close_brace) else {
                    return close_brace;
                };
                if self.result_block_binding_start(next, close_brace).is_some() {
                    return next;
                }
                if !self.tokens[next].kind().is_pipe_operator() {
                    return next;
                }
            }
        }
        close_brace
    }

    fn parse_projection_suffix(
        &mut self,
        base: Expr,
        cursor: &mut usize,
        end: usize,
        stop: ExprStop,
    ) -> Option<Expr> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if self.expr_should_stop(index, stop) || self.tokens[index].kind() != TokenKind::Dot {
            return None;
        }

        let mut fields = Vec::new();
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            let field = self.parse_identifier(cursor, end)?;
            fields.push(field);
            if self.peek_kind(*cursor, end) != Some(TokenKind::Dot) {
                break;
            }
        }

        let path_end = fields
            .last()
            .map(|field| field.span.span().end())
            .unwrap_or_else(|| self.tokens[index].span().end());
        let path_span = SourceSpan::new(
            self.source.id(),
            Span::new(self.tokens[index].span().start(), path_end),
        );
        let projection = ProjectionPath {
            span: path_span,
            fields,
        };
        Some(Expr {
            span: self.join_spans(base.span, projection.span),
            kind: ExprKind::Projection {
                base: Box::new(base),
                path: projection,
            },
        })
    }

    fn parse_integer_expr(&self, index: usize, cursor: &mut usize, end: usize) -> Expr {
        self.parse_integer_expr_with_sign(None, index, cursor, end)
    }

    fn parse_integer_expr_with_sign(
        &self,
        minus_index: Option<usize>,
        index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        let literal = IntegerLiteral {
            raw: self.numeric_literal_raw(minus_index, index),
            span,
        };
        if let Some(suffix_index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[suffix_index].kind() == TokenKind::Identifier
                && self.tokens_are_adjacent(index, suffix_index)
            {
                *cursor = suffix_index + 1;
                let suffix = self.identifier_from_token(suffix_index);
                let span = self.join_spans(literal.span, suffix.span);
                return Expr {
                    span,
                    kind: ExprKind::SuffixedInteger(SuffixedIntegerLiteral {
                        literal,
                        suffix,
                        span,
                    }),
                };
            }
        }

        Expr {
            span,
            kind: ExprKind::Integer(literal),
        }
    }

    fn parse_float_expr(&self, index: usize) -> Expr {
        self.parse_float_expr_with_sign(None, index)
    }

    fn parse_float_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::Float(FloatLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_decimal_expr(&self, index: usize) -> Expr {
        self.parse_decimal_expr_with_sign(None, index)
    }

    fn parse_decimal_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::Decimal(DecimalLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_bigint_expr(&self, index: usize) -> Expr {
        self.parse_bigint_expr_with_sign(None, index)
    }

    fn parse_bigint_expr_with_sign(&self, minus_index: Option<usize>, index: usize) -> Expr {
        let span = self.numeric_literal_span(minus_index, index);
        Expr {
            span,
            kind: ExprKind::BigInt(BigIntLiteral {
                raw: self.numeric_literal_raw(minus_index, index),
                span,
            }),
        }
    }

    fn parse_ambient_projection(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::Dot)?;
        let next_is_ident = matches!(
            self.peek_kind(*cursor, end),
            Some(k) if k == TokenKind::Identifier || k.is_keyword()
        );
        if !next_is_ident {
            return Some(Expr {
                span: self.source_span_of_token(start),
                kind: ExprKind::SubjectPlaceholder,
            });
        }
        let mut fields = vec![self.parse_identifier(cursor, end)?];
        while self.consume_kind(cursor, end, TokenKind::Dot).is_some() {
            fields.push(self.parse_identifier(cursor, end)?);
        }
        let last_end = fields
            .last()
            .map(|field| field.span.span().end())
            .unwrap_or_else(|| self.tokens[start].span().end());
        let span = SourceSpan::new(
            self.source.id(),
            Span::new(self.tokens[start].span().start(), last_end),
        );
        Some(Expr {
            span,
            kind: ExprKind::AmbientProjection(ProjectionPath { span, fields }),
        })
    }

    fn parse_grouped_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let mut elements = Vec::new();
        let mut saw_comma = false;

        if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
            return Some(Expr {
                span: self.source_span_for_range(start, *cursor),
                kind: ExprKind::Tuple(Vec::new()),
            });
        }

        // Detect operator section: (op) where the content is a single binary operator token.
        if let Some(op_index) = self.peek_nontrivia(*cursor, end) {
            if let Some((op, _)) = self.binary_operator(op_index) {
                let after_op = self.peek_nontrivia(op_index + 1, end);
                if after_op.map_or(false, |i| self.tokens[i].kind() == TokenKind::RParen) {
                    *cursor = op_index + 1;
                    let _ = self.consume_kind(cursor, end, TokenKind::RParen);
                    return Some(Expr {
                        span: self.source_span_for_range(start, *cursor),
                        kind: ExprKind::OperatorSection(op),
                    });
                }
            }
        }

        loop {
            let element = self.parse_expr(cursor, end, ExprStop::paren_context())?;
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
            Expr {
                span,
                kind: ExprKind::Tuple(elements),
            }
        } else {
            Expr {
                span,
                kind: ExprKind::Group(Box::new(elements.remove(0))),
            }
        })
    }

    fn parse_list_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::RBracket)
                .is_some()
            {
                break;
            }
            let element = self.parse_expr(cursor, end, ExprStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
                break;
            }
        }

        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::List(elements),
        })
    }

    fn parse_set_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let start = self.consume_identifier_text(cursor, end, "Set")?;
        let _ = self.consume_kind(cursor, end, TokenKind::LBracket)?;
        let mut elements = Vec::new();

        loop {
            if self
                .consume_kind(cursor, end, TokenKind::RBracket)
                .is_some()
            {
                break;
            }
            let element = self.parse_expr(cursor, end, ExprStop::list_context())?;
            elements.push(element);
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBracket);
                break;
            }
        }

        Some(Expr {
            span: self.source_span_for_range(start, *cursor),
            kind: ExprKind::Set(elements),
        })
    }

    fn parse_record_expr(&mut self, cursor: &mut usize, end: usize) -> Option<RecordExpr> {
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
            let value = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_expr(cursor, end, ExprStop::record_context())
            } else {
                None
            };
            let field_end = value
                .as_ref()
                .map(|expr| expr.span.span().end())
                .unwrap_or_else(|| {
                    label_path
                        .last()
                        .map(|p| p.span.span().end())
                        .unwrap_or_else(|| label.span.span().end())
                });
            fields.push(RecordField {
                label,
                label_path,
                value,
                span: SourceSpan::new(self.source.id(), Span::new(field_start, field_end)),
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(RecordExpr {
            fields,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_map_expr(&mut self, cursor: &mut usize, end: usize) -> Option<MapExpr> {
        let start = self.consume_identifier_text(cursor, end, "Map")?;
        let _ = self.consume_kind(cursor, end, TokenKind::LBrace)?;
        let mut entries = Vec::new();

        loop {
            if self.consume_kind(cursor, end, TokenKind::RBrace).is_some() {
                break;
            }
            let key = self.parse_expr(cursor, end, ExprStop::record_context())?;
            let _ = self.consume_kind(cursor, end, TokenKind::Colon)?;
            let value = self.parse_expr(cursor, end, ExprStop::record_context())?;
            entries.push(MapExprEntry {
                span: self.join_spans(key.span, value.span),
                key,
                value,
            });
            if self.consume_kind(cursor, end, TokenKind::Comma).is_none() {
                let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                break;
            }
        }

        Some(MapExpr {
            entries,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_markup_expr(&mut self, cursor: &mut usize, end: usize) -> Option<Expr> {
        let node = self.parse_markup_node(cursor, end)?;
        Some(Expr {
            span: node.span,
            kind: ExprKind::Markup(node),
        })
    }

    fn parse_markup_node(&mut self, cursor: &mut usize, end: usize) -> Option<MarkupNode> {
        if !self.depth_enter(cursor) {
            return None;
        }
        let result = self.parse_markup_node_inner(cursor, end);
        self.depth_exit();
        result
    }

    fn parse_markup_node_inner(&mut self, cursor: &mut usize, end: usize) -> Option<MarkupNode> {
        let start = self.consume_kind(cursor, end, TokenKind::Less)?;
        let name = self.parse_qualified_name(cursor, end)?;
        let case_pattern_attrs = name.as_dotted() == "case";
        let mut attributes = Vec::new();

        loop {
            match self.peek_kind(*cursor, end) {
                Some(TokenKind::SelfCloseTagEnd) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::SelfCloseTagEnd);
                    return Some(MarkupNode {
                        name,
                        attributes,
                        children: Vec::new(),
                        close_name: None,
                        self_closing: true,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
                Some(TokenKind::Greater) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::Greater);
                    break;
                }
                Some(kind) if kind == TokenKind::Identifier || kind.is_keyword() => {
                    let Some(attribute) =
                        self.parse_markup_attribute(cursor, end, case_pattern_attrs)
                    else {
                        break;
                    };
                    attributes.push(attribute);
                }
                _ => break,
            }
        }

        let mut children = Vec::new();
        let mut close_name = None;

        loop {
            let Some(index) = self.peek_nontrivia(*cursor, end) else {
                self.diagnostics.push(
                    Diagnostic::error(
                        "markup node is not closed before the end of the declaration",
                    )
                    .with_code(UNTERMINATED_MARKUP_NODE)
                    .with_primary_label(name.span, "this markup node needs a matching closing tag")
                    .with_help("add a closing tag or use self-closing syntax"),
                );
                break;
            };
            match self.tokens[index].kind() {
                TokenKind::CloseTagStart => {
                    *cursor = index + 1;
                    close_name = self.parse_qualified_name(cursor, end);
                    let _ = self.consume_kind(cursor, end, TokenKind::Greater);
                    if let Some(close_name_value) = close_name.as_ref() {
                        if close_name_value.as_dotted() != name.as_dotted() {
                            self.diagnostics.push(
                                Diagnostic::error("markup closing tag does not match the open tag")
                                    .with_code(MISMATCHED_MARKUP_CLOSE)
                                    .with_primary_label(
                                        close_name_value.span,
                                        format!(
                                            "expected `</{}>` to close this node",
                                            name.as_dotted()
                                        ),
                                    )
                                    .with_secondary_label(
                                        name.span,
                                        format!("`<{}>` was opened here", name.as_dotted()),
                                    )
                                    .with_help("ensure opening and closing tags match"),
                            );
                        }
                    }
                    return Some(MarkupNode {
                        name,
                        attributes,
                        children,
                        close_name,
                        self_closing: false,
                        span: self.source_span_for_range(start, *cursor),
                    });
                }
                TokenKind::Less => {
                    let Some(child) = self.parse_markup_node(cursor, end) else {
                        break;
                    };
                    children.push(child);
                }
                _ => {
                    self.reject_invalid_markup_child_content(cursor, end, &name);
                }
            }
        }

        Some(MarkupNode {
            name,
            attributes,
            children,
            close_name,
            self_closing: false,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_markup_attribute(
        &mut self,
        cursor: &mut usize,
        end: usize,
        case_pattern_attr: bool,
    ) -> Option<MarkupAttribute> {
        let name = self.parse_identifier(cursor, end)?;
        let attribute_start = name.span.span().start();
        let value = if self.consume_kind(cursor, end, TokenKind::Equals).is_some() {
            match self.peek_kind(*cursor, end) {
                Some(TokenKind::StringLiteral) => {
                    let index = self.peek_nontrivia(*cursor, end)?;
                    *cursor = index + 1;
                    Some(MarkupAttributeValue::Text(
                        self.text_literal_from_token(index),
                    ))
                }
                Some(TokenKind::LBrace) => {
                    let _ = self.consume_kind(cursor, end, TokenKind::LBrace);
                    let value = if case_pattern_attr && name.text == "pattern" {
                        self.parse_pattern(cursor, end, PatternStop::brace_context())
                            .map(MarkupAttributeValue::Pattern)
                    } else {
                        self.parse_expr(cursor, end, ExprStop::brace_context())
                            .map(MarkupAttributeValue::Expr)
                    }?;
                    let _ = self.consume_kind(cursor, end, TokenKind::RBrace);
                    Some(value)
                }
                _ => None,
            }
        } else {
            None
        };
        let attribute_end = match &value {
            Some(MarkupAttributeValue::Text(text)) => text.span.span().end(),
            Some(MarkupAttributeValue::Expr(expr)) => expr.span.span().end(),
            Some(MarkupAttributeValue::Pattern(pattern)) => pattern.span.span().end(),
            None => name.span.span().end(),
        };
        Some(MarkupAttribute {
            name,
            value,
            span: SourceSpan::new(self.source.id(), Span::new(attribute_start, attribute_end)),
        })
    }

}
