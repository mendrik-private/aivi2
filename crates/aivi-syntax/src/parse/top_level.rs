impl<'a> Parser<'a> {
    fn new(source: &'a SourceFile, tokens: &'a [Token]) -> Self {
        Self {
            source,
            tokens,
            cursor: 0,
            diagnostics: Vec::new(),
            depth: 0,
            implicit_lambda_disabled: 0,
        }
    }

    fn implicit_lambda_enabled(&self) -> bool {
        self.implicit_lambda_disabled == 0
    }

    fn with_implicit_lambda_disabled<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.implicit_lambda_disabled += 1;
        let result = f(self);
        self.implicit_lambda_disabled -= 1;
        result
    }

    /// Attempt to enter a recursive parse frame. Returns `true` if the caller
    /// may proceed; returns `false` (and emits a diagnostic) when the nesting
    /// limit has been reached.  The caller **must** call `depth_exit` exactly
    /// once after a successful `depth_enter`.
    fn depth_enter(&mut self, cursor: &mut usize) -> bool {
        if self.depth >= MAX_PARSE_DEPTH {
            let span = if *cursor < self.tokens.len() {
                self.source_span_of_token(*cursor)
            } else if !self.tokens.is_empty() {
                self.source_span_of_token(self.tokens.len() - 1)
            } else {
                self.source.source_span(0..0)
            };
            self.diagnostics.push(
                Diagnostic::error("expression is nested too deeply to parse")
                    .with_code(PARSE_DEPTH_EXCEEDED)
                    .with_primary_label(span, "maximum parse depth exceeded here")
                    .with_help("refactor deeply nested expressions into smaller named values"),
            );
            return false;
        }
        self.depth += 1;
        true
    }

    fn depth_exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse(mut self) -> (Module, Vec<Diagnostic>) {
        let mut items = Vec::new();
        let mut pending_type_annotation = None;
        // Comments collected for a pending standalone `type` annotation are
        // carried forward and prepended to the following declaration.
        let mut carried_comments: Vec<String> = Vec::new();
        while let Some(start) = self.next_significant_from(self.cursor) {
            let leading_comments = self.collect_leading_comments(self.cursor, start);
            let item = match self.tokens[start].kind() {
                TokenKind::At => self.parse_decorated_item(start),
                kind if kind.is_top_level_keyword() => self.parse_item_without_decorators(start),
                _ => self.parse_error_item(start),
            };
            let next_cursor = item.token_range().end();
            self.cursor = if next_cursor > start {
                next_cursor
            } else {
                start + 1
            };

            if let Some(pending) = Self::pending_type_annotation(&item) {
                if let Some(previous) = pending_type_annotation.replace(pending) {
                    self.emit_orphan_standalone_type_annotation(&previous, None);
                }
                // Carry the comments so they prefix the following declaration.
                if !leading_comments.is_empty() {
                    carried_comments = leading_comments;
                }
                continue;
            }

            let mut item = item;
            if let Some(pending) = pending_type_annotation.take() {
                self.apply_pending_type_annotation(&mut item, pending);
            }
            // Merge carried comments (from a preceding type annotation) with
            // any comments directly before this item, preferring carried first.
            let mut all_comments = std::mem::take(&mut carried_comments);
            all_comments.extend(leading_comments);
            if !all_comments.is_empty() {
                item.base_mut().leading_comments = all_comments;
            }
            items.push(item);
        }

        if let Some(pending) = pending_type_annotation.take() {
            self.emit_orphan_standalone_type_annotation(&pending, None);
        }

        (
            Module {
                file: self.source.id(),
                items,
                token_count: self.tokens.len(),
            },
            self.diagnostics,
        )
    }

    fn parse_decorated_item(&mut self, start: usize) -> Item {
        let search = self.find_declaration_keyword(start);
        let Some(keyword_index) = search.keyword else {
            let end = search
                .offending
                .and_then(|index| self.find_next_item_start(index + 1))
                .unwrap_or(self.tokens.len());
            let base = self.make_base(start, end, Vec::new());
            self.diagnostics.push(
                Diagnostic::error("decorators must attach to a following top-level declaration")
                    .with_code(DANGLING_DECORATOR_BLOCK)
                    .with_primary_label(
                        self.source_span_of_token(start),
                        "expected a following top-level declaration after this decorator block",
                    )
                    .with_help("decorators like @source must be followed by a declaration"),
            );
            return Item::Error(ErrorItem {
                base,
                message: "dangling decorator block".to_owned(),
            });
        };

        let end = self
            .find_next_item_start(keyword_index + 1)
            .unwrap_or(self.tokens.len());
        let decorators = self.parse_decorators(start, keyword_index);
        self.finish_item(keyword_index, start, end, decorators)
    }

    fn parse_item_without_decorators(&mut self, start: usize) -> Item {
        let end = self
            .find_next_item_start(start + 1)
            .unwrap_or(self.tokens.len());
        self.finish_item(start, start, end, Vec::new())
    }

    fn finish_item(
        &mut self,
        keyword_index: usize,
        start: usize,
        end: usize,
        decorators: Vec<Decorator>,
    ) -> Item {
        let base = self.make_base(start, end, decorators);
        match self.tokens[keyword_index].kind() {
            TokenKind::TypeKw => {
                Item::Type(self.parse_type_item(base, keyword_index, end, "type declaration"))
            }
            TokenKind::FuncKw => Item::Fun(self.parse_fun_item(base, keyword_index, end)),
            TokenKind::ValueKw => Item::Value(self.parse_value_item(base, keyword_index, end)),
            TokenKind::SignalKw => {
                Item::Signal(self.parse_signal_item(base, keyword_index, end, "signal declaration"))
            }
            TokenKind::FromKw => Item::From(self.parse_from_item(base, keyword_index, end)),
            TokenKind::ClassKw => Item::Class(self.parse_class_item(base, keyword_index, end)),
            TokenKind::InstanceKw => {
                Item::Instance(self.parse_instance_item(base, keyword_index, end))
            }
            TokenKind::DomainKw => Item::Domain(self.parse_domain_item(base, keyword_index, end)),
            TokenKind::ProviderKw => Item::SourceProviderContract(
                self.parse_source_provider_contract_item(base, keyword_index, end),
            ),
            TokenKind::UseKw => Item::Use(self.parse_use_item(base, keyword_index, end)),
            TokenKind::ExportKw => Item::Export(self.parse_export_item(base, keyword_index, end)),
            TokenKind::HoistKw => Item::Hoist(self.parse_hoist_item(base, keyword_index, end)),
            _ => unreachable!("finish_item only accepts top-level declaration keywords"),
        }
    }

    fn parse_type_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
        description: &str,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        if !self.has_top_level_equals(cursor, end) {
            let (constraints, annotation) = self.parse_constrained_type(&mut cursor, end);
            if annotation.is_none() {
                self.diagnostics.push(
                    Diagnostic::error("standalone `type` annotations require a type expression")
                        .with_code(MISSING_STANDALONE_TYPE_ANNOTATION)
                        .with_primary_label(
                            self.source_span_of_token(keyword_index),
                            "expected a type expression after `type`",
                        )
                        .with_help("expected a type expression after the name"),
                );
            }
            return NamedItem {
                base,
                keyword_span: self.source_span_of_token(keyword_index),
                name: None,
                type_parameters: Vec::new(),
                constraints,
                annotation,
                function_form: FunctionSurfaceForm::Explicit,
                parameters: Vec::new(),
                body: None,
            };
        }

        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, description);
        let mut type_parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(cursor, end) {
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    type_parameters.push(self.identifier_from_token(index));
                    cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_type_decl_body(&mut cursor, end)
                .map(NamedItemBody::Type)
                .or_else(|| {
                    self.missing_body_diagnostic(
                        keyword_index,
                        "type declaration is missing its body after `=`",
                        "expected a type body after `=`",
                    );
                    None
                })
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                "type declaration is missing its body",
                "expected `=` followed by a type body",
            );
            None
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            constraints: Vec::new(),
            annotation: None,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_class_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let (name, type_parameters) = self.parse_class_head(keyword_index, &mut cursor, end);

        let body = self
            .parse_class_body(&mut cursor, end)
            .map(NamedItemBody::Class);

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            constraints: Vec::new(),
            annotation: None,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_instance_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> InstanceItem {
        let mut cursor = keyword_index + 1;
        let context = self.parse_optional_constraint_prefix(&mut cursor, end);
        let class = self.parse_qualified_name(&mut cursor, end).or_else(|| {
            self.diagnostics.push(
                Diagnostic::error("instance declaration is missing its class name")
                    .with_code(MISSING_INSTANCE_CLASS)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a class name such as `Eq` or `Default`",
                    )
                    .with_help("syntax: instance <Name> of <ClassName> = { ... }"),
            );
            None
        });
        let target = self
            .parse_type_expr(&mut cursor, end, TypeStop::default())
            .or_else(|| {
                self.diagnostics.push(
                    Diagnostic::error("instance declaration is missing its target type")
                        .with_code(MISSING_INSTANCE_TARGET)
                        .with_primary_label(
                            class
                                .as_ref()
                                .map(|class| class.span)
                                .unwrap_or_else(|| self.source_span_of_token(keyword_index)),
                            "expected one instance target type such as `Blob` or `Result HttpError`",
                        )
                        .with_help("syntax: instance <Name> of <ClassName> = { ... }"),
                );
                None
            });
        let body = self.parse_instance_body(&mut cursor, end);
        InstanceItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            context,
            class,
            target,
            body,
        }
    }

    fn parse_domain_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> DomainItem {
        let mut cursor = keyword_index + 1;
        let name =
            self.parse_named_item_name(keyword_index, &mut cursor, end, "domain declaration");
        let mut type_parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(cursor, end) {
            if self.tokens[index].line_start() || self.is_identifier_text(index, "over") {
                break;
            }
            if self.tokens[index].kind() != TokenKind::Identifier {
                break;
            }
            type_parameters.push(self.identifier_from_token(index));
            cursor = index + 1;
        }

        let over_span = if let Some(index) = self.consume_identifier_text(&mut cursor, end, "over")
        {
            Some(self.source_span_of_token(index))
        } else {
            self.diagnostics.push(
                Diagnostic::error("domain declaration is missing `over` before its carrier type")
                    .with_code(MISSING_DOMAIN_OVER)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected `over` followed by the carrier type",
                    )
                    .with_help("syntax: domain <Name> over <CarrierType> = { ... }"),
            );
            None
        };

        let carrier = self.parse_type_expr(&mut cursor, end, TypeStop::default());
        if carrier.is_none() {
            self.diagnostics.push(
                Diagnostic::error("domain declaration is missing its carrier type")
                    .with_code(MISSING_DOMAIN_CARRIER)
                    .with_primary_label(
                        over_span.unwrap_or_else(|| self.source_span_of_token(keyword_index)),
                        "expected a carrier type such as `Int`, `Text`, or `List A`",
                    )
                    .with_help("expected a carrier type after `over`"),
            );
        }

        let body = self.parse_domain_body(&mut cursor, end);
        DomainItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters,
            carrier,
            body,
        }
    }

    fn parse_source_provider_contract_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> SourceProviderContractItem {
        let mut cursor = keyword_index + 1;
        let provider = self.parse_qualified_name(&mut cursor, end).or_else(|| {
            self.diagnostics.push(
                Diagnostic::error("provider contract declaration is missing its provider name")
                    .with_code(MISSING_PROVIDER_CONTRACT_NAME)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a qualified provider name such as `custom.feed`",
                    ),
            );
            None
        });
        let body = self.parse_source_provider_contract_body(&mut cursor, end);
        SourceProviderContractItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            provider,
            body,
        }
    }

    /// Parse a `value` declaration: constant form only, uses `=`.
    fn parse_value_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, "value declaration");
        let (constraints, annotation) = self.parse_function_signature_annotation(&mut cursor, end);

        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_expression_body(
                keyword_index,
                &mut cursor,
                end,
                "value declaration",
                "value declaration is missing its body after `=`",
                "expected an expression after `=`",
            )
        } else {
            self.missing_body_diagnostic(
                keyword_index,
                "value declaration is missing its body",
                "expected `=` followed by an expression",
            );
            None
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints,
            annotation,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    /// Parse a `func` declaration: `func name = params => body` or `func name = .`.
    fn parse_fun_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, "func declaration");
        let (constraints, annotation) = self.parse_function_signature_annotation(&mut cursor, end);

        let has_equals = self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some();
        if !has_equals {
            if self.peek_nontrivia(cursor, end).is_some_and(|index| {
                self.is_function_param_token(index)
                    || self.tokens[index].kind() == TokenKind::Arrow
                    || self.starts_unary_subject_function(cursor, end)
            }) {
                let anchor_span = name
                    .as_ref()
                    .map(|identifier| identifier.span)
                    .unwrap_or_else(|| self.source_span_of_token(keyword_index));
                self.diagnostics.push(
                    Diagnostic::error(
                        "func declaration is missing `=` before its parameters and body",
                    )
                    .with_code(MISSING_DECLARATION_BODY)
                    .with_primary_label(
                        anchor_span,
                        "insert `=` between the function name and its body",
                    )
                    .with_help("expected `=` followed by the function body"),
                );
            } else {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body",
                    "expected `=` followed by an expression using `.` or parameters and `=>`",
                );
            }
        }

        let (function_form, parameters, body) = if let Some((parameters, body)) =
            self.parse_unary_subject_function_body(keyword_index, &mut cursor, end)
        {
            (FunctionSurfaceForm::UnarySubjectSugar, parameters, body)
        } else if let Some((parameters, body)) =
            self.parse_selected_subject_function_body(&mut cursor, end, "func declaration")
        {
            (FunctionSurfaceForm::SelectedSubjectSugar, parameters, body)
        } else {
            let mut parameters = Vec::new();
            while self.starts_function_param(cursor, end) {
                let Some(parameter) = self.parse_function_param(&mut cursor, end) else {
                    break;
                };
                parameters.push(parameter);
            }

            let body = if let Some(arrow_index) =
                self.consume_kind(&mut cursor, end, TokenKind::Arrow)
            {
                let body = self.parse_expression_body(
                    keyword_index,
                    &mut cursor,
                    end,
                    "func declaration",
                    "func declaration is missing its body after `=>`",
                    "expected a body expression after `=>`",
                );
                if parameters.is_empty() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "func declarations require an explicit parameter before `=>`",
                        )
                        .with_code(NULLARY_FUNCTION_DECLARATION)
                        .with_primary_label(
                            self.source_span_of_token(arrow_index),
                            "insert a parameter such as `_` before `=>`",
                        )
                        .with_note("ignored unary functions are written as `func name = _ => body`")
                        .with_help("functions must have at least one parameter"),
                    );
                }
                body
            } else {
                self.missing_body_diagnostic(
                    keyword_index,
                    "func declaration is missing its body",
                    "expected `=>` followed by a body expression",
                );
                None
            };
            (FunctionSurfaceForm::Explicit, parameters, body)
        };

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints,
            annotation,
            function_form,
            parameters,
            body,
        }
    }

    fn parse_signal_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
        description: &str,
    ) -> NamedItem {
        let mut cursor = keyword_index + 1;
        let name = self.parse_named_item_name(keyword_index, &mut cursor, end, description);
        let annotation = self.parse_optional_type_annotation(&mut cursor, end);
        let body = if self
            .consume_kind(&mut cursor, end, TokenKind::Equals)
            .is_some()
        {
            self.parse_signal_body(keyword_index, &mut cursor, end)
        } else {
            None
        };

        if annotation.is_none() && body.is_none() {
            self.missing_body_diagnostic(
                keyword_index,
                "signal declaration is missing its body",
                "expected either `:` with a type or `=` with an expression",
            );
        }

        NamedItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            name,
            type_parameters: Vec::new(),
            constraints: Vec::new(),
            annotation,
            function_form: FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body,
        }
    }

    fn parse_from_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> FromItem {
        let mut cursor = keyword_index + 1;
        let equals_index = self.find_top_level_equals(cursor, end);
        let source_end = equals_index.unwrap_or(end);

        let mut source_cursor = cursor;
        let source = self.parse_expr(&mut source_cursor, source_end, ExprStop::default());
        match &source {
            Some(_) => {
                if let Some(trailing_index) =
                    self.next_significant_in_range(source_cursor, source_end)
                {
                    self.diagnostics.push(
                        Diagnostic::error("`from` source must contain exactly one expression")
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the shared source expression",
                            ),
                    );
                }
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`from` declaration is missing its shared source expression")
                        .with_code(MISSING_FROM_SOURCE)
                        .with_primary_label(
                            self.source_span_of_token(keyword_index),
                            "expected a source expression such as `state` after `from`",
                        )
                        .with_help("syntax: from <signal-expr> = { derivedName: expr }"),
                );
            }
        }
        cursor = equals_index.map_or(source_cursor, |index| index + 1);

        let open_brace = self.consume_kind(&mut cursor, end, TokenKind::LBrace);
        let entries = if let Some(open_brace) = open_brace {
            let inner_end = self.find_matching_brace(open_brace, end).unwrap_or(end);
            let mut entries_cursor = open_brace + 1;
            self.parse_from_entries(&mut entries_cursor, inner_end)
        } else {
            self.diagnostics.push(
                Diagnostic::error("`from` declaration is missing its fan-out body")
                    .with_code(MISSING_FROM_OPEN_BRACE)
                    .with_primary_label(
                        self.source_span_of_token(equals_index.unwrap_or(keyword_index)),
                        "expected `{` to open the grouped derived signals",
                    )
                    .with_help("syntax: from <signal-expr> = { derivedName: expr }"),
            );
            Vec::new()
        };

        FromItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            source,
            entries,
        }
    }

    fn parse_from_entries(&mut self, cursor: &mut usize, end: usize) -> Vec<FromEntry> {
        let Some(first_index) = self.peek_nontrivia(*cursor, end) else {
            return Vec::new();
        };
        if !self.tokens[first_index].line_start() {
            return Vec::new();
        }
        let entry_indent = self.line_indent_of_token(first_index);
        let mut entries = Vec::new();
        let mut pending_type_annotation = None;
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.starts_from_type_annotation(index, entry_indent) {
                let annotation_end = self
                    .find_next_from_block_item_start(index + 1, end, entry_indent)
                    .unwrap_or(end);
                if let Some(pending) = self.parse_from_type_annotation(index, annotation_end)
                    && let Some(previous) = pending_type_annotation.replace(pending) {
                        self.emit_orphan_from_type_annotation(&previous, None);
                    }
                *cursor = annotation_end;
                continue;
            }
            if !self.starts_from_entry(index, end, entry_indent) {
                break;
            }
            let entry_end = self
                .find_next_from_block_item_start(index + 1, end, entry_indent)
                .unwrap_or(end);
            let mut entry = self.parse_from_entry(index, entry_end);
            if let Some(pending) = pending_type_annotation.take() {
                self.apply_pending_from_type_annotation(&mut entry, pending);
            }
            entries.push(entry);
            *cursor = entry_end;
        }
        if let Some(pending) = pending_type_annotation.take() {
            self.emit_orphan_from_type_annotation(&pending, None);
        }
        entries
    }

    fn starts_from_type_annotation(&self, index: usize, entry_indent: usize) -> bool {
        let token = self.tokens[index];
        token.kind() == TokenKind::TypeKw
            && token.line_start()
            && self.line_indent_of_token(index) == entry_indent
    }

    fn starts_from_entry(&self, index: usize, end: usize, entry_indent: usize) -> bool {
        let token = self.tokens[index];
        token.kind() == TokenKind::Identifier
            && token.line_start()
            && self.line_indent_of_token(index) == entry_indent
            && self.find_from_entry_header_colon(index, end).is_some()
    }

    fn parse_from_entry(&mut self, start: usize, end: usize) -> FromEntry {
        let Some(header_colon) = self.find_from_entry_header_colon(start, end) else {
            return FromEntry {
                name: self.identifier_from_token(start),
                constraints: Vec::new(),
                annotation: None,
                parameters: Vec::new(),
                body: None,
                span: self.source_span_for_range(start, end),
            };
        };
        let mut cursor = start;
        let name = self
            .parse_identifier(&mut cursor, header_colon)
            .unwrap_or_else(|| Identifier {
                text: "<missing>".to_owned(),
                span: self.source_span_of_token(start),
            });
        let mut parameters = Vec::new();
        while let Some(parameter) = self.parse_from_entry_param(&mut cursor, header_colon) {
            parameters.push(parameter);
        }
        if let Some(trailing_index) = self.next_significant_in_range(cursor, header_colon) {
            self.diagnostics.push(
                Diagnostic::error(
                    "`from` entry headers only accept unannotated parameters before `:`",
                )
                .with_primary_label(
                    self.source_span_of_token(trailing_index),
                    "remove this token or move its type into a preceding `type` line",
                ),
            );
        }
        cursor = header_colon + 1;
        let body = self.with_implicit_lambda_disabled(|parser| {
            parser.parse_expr(&mut cursor, end, ExprStop::default())
        });
        match &body {
            Some(_) => {
                if let Some(trailing_index) = self.next_significant_in_range(cursor, end) {
                    self.diagnostics.push(
                        Diagnostic::error("`from` entry body must contain exactly one expression")
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the derived signal body",
                            ),
                    );
                }
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`from` entry is missing its derived expression")
                        .with_code(MISSING_FROM_ENTRY_BODY)
                        .with_primary_label(
                            name.span,
                            format!("expected an expression after `{}:`", name.text),
                        ),
                );
            }
        }

        FromEntry {
            name,
            constraints: Vec::new(),
            annotation: None,
            parameters,
            body,
            span: self.source_span_for_range(start, end),
        }
    }

    fn find_next_from_block_item_start(
        &self,
        from: usize,
        end: usize,
        entry_indent: usize,
    ) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if depth == 0
                && (self.starts_from_type_annotation(index, entry_indent)
                    || self.starts_from_entry(index, end, entry_indent))
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

    fn find_from_entry_header_colon(&self, start: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut saw_token = false;
        for index in start..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if saw_token && token.line_start() && depth == 0 {
                break;
            }
            saw_token = true;
            match token.kind() {
                TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                    depth = depth.saturating_sub(1)
                }
                TokenKind::Colon if depth == 0 => return Some(index),
                _ => {}
            }
        }
        None
    }

    fn parse_from_entry_param(&mut self, cursor: &mut usize, end: usize) -> Option<FunctionParam> {
        let index = self.peek_nontrivia(*cursor, end)?;
        if !self.is_function_param_token(index) {
            return None;
        }
        *cursor = index + 1;
        let identifier = self.identifier_from_token(index);
        let name = (identifier.text != "_").then_some(identifier);
        Some(FunctionParam {
            name,
            annotation: None,
            span: self.source_span_of_token(index),
        })
    }

    fn parse_from_type_annotation(
        &mut self,
        start: usize,
        end: usize,
    ) -> Option<PendingTypeAnnotation> {
        let mut cursor = start + 1;
        let (constraints, annotation) = self.parse_constrained_type(&mut cursor, end);
        let annotation = match annotation {
            Some(annotation) => annotation,
            None => {
                self.diagnostics.push(
                    Diagnostic::error("standalone `type` annotations inside `from` blocks require a type expression")
                        .with_code(MISSING_STANDALONE_TYPE_ANNOTATION)
                        .with_primary_label(
                            self.source_span_of_token(start),
                            "expected a type expression after `type`",
                        )
                        .with_help("write a payload/result type such as `type Bool` or `type Coord -> Bool`"),
                );
                return None;
            }
        };
        if let Some(trailing_index) = self.next_significant_in_range(cursor, end) {
            self.diagnostics.push(
                Diagnostic::error(
                    "`from`-block type annotations must contain exactly one type expression",
                )
                .with_primary_label(
                    self.source_span_of_token(trailing_index),
                    "this token is outside the attached type annotation",
                ),
            );
        }
        Some(PendingTypeAnnotation {
            span: self.source_span_for_range(start, end),
            constraints,
            annotation,
        })
    }

    /// Parse a signal body after `=`. This may be:
    /// - A merge body: `sig1 | sig2 ||> ...` or `sig1 ||> ...`
    /// - A plain expression body: `expr |> pipe`
    fn parse_signal_body(
        &mut self,
        keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<NamedItemBody> {
        // Probe: is this a merge body?
        // A merge body starts with an identifier, optionally followed by `|` ident sequences,
        // and eventually has `||>` arms (either inline or on next lines).
        if let Some(merge) = self.try_parse_signal_merge_body(keyword_index, cursor, end) {
            return Some(NamedItemBody::Merge(merge));
        }
        // Fall back to plain expression body.
        self.parse_expression_body(
            keyword_index,
            cursor,
            end,
            "signal declaration",
            "signal declaration is missing its body after `=`",
            "expected an expression after `=`",
        )
    }

    /// Attempt to parse a signal merge body: `sig1 | sig2 ||> arm1 ||> arm2 ...`
    /// Returns `None` if the token stream doesn't match the merge pattern, leaving cursor unchanged.
    fn try_parse_signal_merge_body(
        &mut self,
        _keyword_index: usize,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SignalMergeBody> {
        let merge_span_start = *cursor;

        // Probe ahead without consuming: look for identifier (| identifier)* followed by ||> arms.
        let mut probe = *cursor;
        let mut source_positions: Vec<usize> = Vec::new();

        // First source must be an identifier.
        let first = self.peek_nontrivia(probe, end)?;
        if self.tokens[first].kind() != TokenKind::Identifier {
            return None;
        }
        source_positions.push(first);
        probe = first + 1;

        // Collect additional `| ident` sources.
        loop {
            let Some(next) = self.peek_nontrivia(probe, end) else {
                break;
            };
            if self.tokens[next].kind() == TokenKind::PipeTap && !self.tokens[next].line_start() {
                let Some(ident_idx) = self.peek_nontrivia(next + 1, end) else {
                    break;
                };
                if self.tokens[ident_idx].kind() == TokenKind::Identifier
                    && !self.tokens[ident_idx].line_start()
                {
                    source_positions.push(ident_idx);
                    probe = ident_idx + 1;
                    continue;
                }
            }
            break;
        }

        // Now check: do we see `||>` arms?
        let has_arms = self.find_signal_reactive_arm_start(probe, end).is_some();
        if !has_arms {
            return None;
        }

        // Commit: parse the sources.
        let mut sources = Vec::new();
        for &pos in &source_positions {
            let ident = Identifier {
                text: self.tokens[pos].text(self.source).to_owned(),
                span: self.source_span_of_token(pos),
            };
            sources.push(ident);
        }
        *cursor = probe;

        // Parse the arms.
        let arms = self.parse_signal_reactive_arms(cursor, end);

        let merge_span =
            self.source_span_for_range(merge_span_start, *cursor.min(&mut end.clone()));
        Some(SignalMergeBody {
            sources,
            arms,
            span: merge_span,
        })
    }

    /// Find the start of the first `||>` reactive arm token within `[from, end)`.
    /// Reactive arms use `=>` (fat arrow). Pipe-case arms use `->` (thin arrow) and are NOT reactive.
    fn find_signal_reactive_arm_start(&self, from: usize, end: usize) -> Option<usize> {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if token.kind() == TokenKind::PipeCase && token.line_start() {
                // Verify this arm uses `=>` (reactive) not `->` (case).
                // Scan ahead on the same line for the arrow token.
                if self.reactive_arm_uses_fat_arrow(index + 1, end) {
                    return Some(index);
                }
                // `||>` with `->` is a pipe-case expression, not a reactive arm.
                return None;
            }
            // If we encounter something else that's non-trivia and on a new line, stop.
            if token.line_start() && token.kind() != TokenKind::PipeCase {
                return None;
            }
        }
        None
    }

    /// Check if a `||>` arm uses `=>` (fat arrow) rather than `->` (thin arrow).
    /// Scans from the token after `||>` up to the next line-start or end.
    fn reactive_arm_uses_fat_arrow(&self, from: usize, end: usize) -> bool {
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            // If we hit a new line-start token, stop scanning this arm.
            if token.line_start() {
                return false;
            }
            if token.kind() == TokenKind::Arrow {
                return true;
            }
            if token.kind() == TokenKind::ThinArrow {
                return false;
            }
        }
        false
    }

    /// Parse all `||>` reactive arms at a consistent indent level.
    fn parse_signal_reactive_arms(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Vec<SignalReactiveArm> {
        let Some(first_index) = self.peek_nontrivia(*cursor, end) else {
            return Vec::new();
        };
        if self.tokens[first_index].kind() != TokenKind::PipeCase
            || !self.tokens[first_index].line_start()
        {
            return Vec::new();
        }
        let arm_indent = self.line_indent_of_token(first_index);
        let mut arms = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].kind() != TokenKind::PipeCase
                || !self.tokens[index].line_start()
                || self.line_indent_of_token(index) != arm_indent
            {
                break;
            }
            let arm_end = self
                .find_next_signal_reactive_arm_start(index + 1, end, arm_indent)
                .unwrap_or(end);
            arms.push(self.parse_signal_reactive_arm(index, arm_end));
            *cursor = arm_end;
        }
        arms
    }

    fn find_next_signal_reactive_arm_start(
        &self,
        from: usize,
        end: usize,
        arm_indent: usize,
    ) -> Option<usize> {
        let mut depth = 0usize;
        for index in from..end {
            let token = self.tokens[index];
            if token.kind().is_trivia() {
                continue;
            }
            if depth == 0
                && token.kind() == TokenKind::PipeCase
                && token.line_start()
                && self.line_indent_of_token(index) == arm_indent
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

    /// Parse a single `||> [source] pattern => body` arm.
    fn parse_signal_reactive_arm(&mut self, arm_start: usize, arm_end: usize) -> SignalReactiveArm {
        let mut cursor = arm_start + 1; // skip `||>`

        // Determine if this arm has a source prefix: `||> sourceName pattern => body`
        // vs plain `||> pattern => body` or `||> _ => body`.
        let source = self.try_parse_arm_source_prefix(&mut cursor, arm_end);

        let pattern = self
            .parse_pattern(
                &mut cursor,
                arm_end,
                PatternStop::signal_reactive_arm_context(),
            )
            .or_else(|| {
                self.diagnostics.push(
                    Diagnostic::error("signal reactive arm is missing its pattern")
                        .with_code(MISSING_REACTIVE_UPDATE_ARM_PATTERN)
                        .with_primary_label(
                            self.source_span_of_token(arm_start),
                            "expected a pattern after `||>`",
                        ),
                );
                None
            });

        let arrow_anchor = pattern
            .as_ref()
            .map(|p| p.span)
            .or_else(|| source.as_ref().map(|s| s.span))
            .unwrap_or_else(|| self.source_span_of_token(arm_start));
        let arrow_present = self
            .consume_kind(&mut cursor, arm_end, TokenKind::Arrow)
            .is_some();
        if !arrow_present {
            self.diagnostics.push(
                Diagnostic::error("signal reactive arm is missing `=>` before its body")
                    .with_code(MISSING_REACTIVE_UPDATE_ARM_ARROW)
                    .with_primary_label(arrow_anchor, "expected `=>` followed by the arm body"),
            );
        }

        let body = if arrow_present {
            self.parse_expr(&mut cursor, arm_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("signal reactive arm is missing its body expression")
                            .with_code(MISSING_REACTIVE_UPDATE_ARM_BODY)
                            .with_primary_label(arrow_anchor, "expected an expression after `=>`"),
                    );
                    None
                })
        } else {
            None
        };

        SignalReactiveArm {
            source,
            pattern,
            body,
            span: self.source_span_for_range(arm_start, arm_end),
        }
    }

    /// Try to parse a source signal name prefix in a reactive arm.
    /// In multi-source merges: `||> tick _ => ...` — `tick` is the source prefix.
    /// We detect this by checking if there's an identifier followed by a pattern and `=>`.
    /// If the identifier IS the pattern (e.g., `||> True => ...`), we don't consume it as source.
    fn try_parse_arm_source_prefix(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<Identifier> {
        let ident_idx = self.peek_nontrivia(*cursor, end)?;
        if self.tokens[ident_idx].kind() != TokenKind::Identifier {
            return None;
        }

        let text = self.tokens[ident_idx].text(self.source);
        // If this is `_`, it's a wildcard pattern, not a source prefix.
        if text == "_" {
            return None;
        }
        // If it starts with uppercase, it could be a constructor pattern.
        // Check: is there something after it before `=>`?
        let first_char = text.chars().next().unwrap_or('a');
        if first_char.is_uppercase() {
            // Uppercase: likely a constructor pattern like `Tick`, `Turn dir`, etc.
            // Only treat as source prefix if the text is lowercase.
            return None;
        }

        // lowercase identifier. Check if it's followed by another token (pattern) before `=>`.
        let next_idx = self.peek_nontrivia(ident_idx + 1, end)?;
        // If immediately followed by `=>`, this identifier IS the pattern, not a source prefix.
        if self.tokens[next_idx].kind() == TokenKind::Arrow {
            return None;
        }
        // It's a source prefix: lowercase ident followed by pattern tokens.
        let ident = Identifier {
            text: text.to_owned(),
            span: self.source_span_of_token(ident_idx),
        };
        *cursor = ident_idx + 1;
        Some(ident)
    }

    fn parse_class_body(&mut self, cursor: &mut usize, end: usize) -> Option<ClassBody> {
        // A class body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("class declaration is missing `{` after `=`")
                        .with_code(MISSING_CLASS_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the class body")
                        .with_help("expected `{` to open the class body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start class member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start()
                || !matches!(
                    self.tokens[first].kind(),
                    TokenKind::Identifier | TokenKind::LParen
                )
            {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        let mut with_decls = Vec::new();
        let mut require_decls = Vec::new();
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            // Detect the context-sensitive `with` and `require` soft-keywords.
            // They are treated as declarations only when NOT immediately followed by `:`,
            // which disambiguates them from method names (`with: A -> A`).
            if self.tokens[index].kind() == TokenKind::Identifier {
                let text = self.tokens[index].text(self.source);
                if text == "with" && self.peek_kind(index + 1, inner_end) != Some(TokenKind::Colon)
                {
                    if let Some(decl) = self.parse_class_with_decl(cursor, inner_end) {
                        with_decls.push(decl);
                        continue;
                    }
                } else if text == "require"
                    && self.peek_kind(index + 1, inner_end) != Some(TokenKind::Colon)
                    && let Some(decl) = self.parse_class_require_decl(cursor, inner_end) {
                        require_decls.push(decl);
                        continue;
                    }
            }
            let before = *cursor;
            let Some(member) = self.parse_class_member(cursor, inner_end) else {
                break;
            };
            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!with_decls.is_empty() || !require_decls.is_empty() || !members.is_empty()).then_some(
            ClassBody {
                with_decls,
                require_decls,
                members,
                span: self.source_span_for_range(body_start, *cursor),
            },
        )
    }

    fn parse_class_with_decl(&mut self, cursor: &mut usize, end: usize) -> Option<ClassWithDecl> {
        let start = *cursor;
        let with_index = self.peek_nontrivia(*cursor, end)?;
        // Consume the `with` soft-keyword.
        *cursor = with_index + 1;
        // Require the superclass type to start on the same line as `with`.
        let Some(type_index) = self.peek_nontrivia(*cursor, end) else {
            *cursor = start;
            return None;
        };
        if self.tokens[type_index].line_start() {
            *cursor = start;
            return None;
        }
        let Some(superclass) = self.parse_type_expr(cursor, end, TypeStop::default()) else {
            self.diagnostics.push(
                Diagnostic::error("`with` declaration is missing its superclass type")
                    .with_code(MISSING_CLASS_WITH_TYPE)
                    .with_primary_label(
                        self.source_span_of_token(with_index),
                        "expected a superclass type such as `Applicative M`",
                    ),
            );
            *cursor = start;
            return None;
        };
        Some(ClassWithDecl {
            superclass,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_class_require_decl(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ClassRequireDecl> {
        let start = *cursor;
        let require_index = self.peek_nontrivia(*cursor, end)?;
        // Consume the `require` soft-keyword.
        *cursor = require_index + 1;
        // Require the constraint type to start on the same line as `require`.
        let Some(type_index) = self.peek_nontrivia(*cursor, end) else {
            *cursor = start;
            return None;
        };
        if self.tokens[type_index].line_start() {
            *cursor = start;
            return None;
        }
        let Some(constraint) = self.parse_type_expr(cursor, end, TypeStop::default()) else {
            self.diagnostics.push(
                Diagnostic::error("`require` declaration is missing its constraint type")
                    .with_code(MISSING_CLASS_REQUIRE_TYPE)
                    .with_primary_label(
                        self.source_span_of_token(require_index),
                        "expected a constraint type such as `Eq K`",
                    ),
            );
            *cursor = start;
            return None;
        };
        Some(ClassRequireDecl {
            constraint,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_instance_body(&mut self, cursor: &mut usize, end: usize) -> Option<InstanceBody> {
        // An instance body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("instance declaration is missing `{` after `=`")
                        .with_code(MISSING_INSTANCE_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the instance body")
                        .with_help("expected `{` to open the instance body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start instance member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start() || !self.starts_instance_member(first) {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if !self.starts_instance_member(index) {
                break;
            }
            let before = *cursor;
            let Some(member) = self.parse_instance_member(cursor, inner_end, member_indent) else {
                break;
            };
            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!members.is_empty()).then_some(InstanceBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_domain_body(&mut self, cursor: &mut usize, end: usize) -> Option<DomainBody> {
        // A domain body may be written as `= { ... }` (brace syntax) or as an
        // indented block (indentation syntax, no `=` or `{`).
        let first = self.peek_nontrivia(*cursor, end)?;
        let has_equals = self.tokens[first].kind() == TokenKind::Equals;

        let (body_start, inner_end, brace_syntax) = if has_equals {
            let head_span = self.source_span_of_token(first);
            self.consume_kind(cursor, end, TokenKind::Equals);
            let lbrace_index = if let Some(idx) = self.consume_kind(cursor, end, TokenKind::LBrace)
            {
                idx
            } else {
                self.diagnostics.push(
                    Diagnostic::error("domain declaration is missing `{` after `=`")
                        .with_code(MISSING_DOMAIN_OPEN_BRACE)
                        .with_primary_label(head_span, "expected `{` to open the domain body")
                        .with_help("expected `{` to open the domain body"),
                );
                return None;
            };
            let inner_end = self.find_matching_brace(lbrace_index, end).unwrap_or(end);
            let body_start = *cursor;
            (body_start, inner_end, true)
        } else {
            // Indentation syntax: the first token must be a line-start domain member token
            // at non-zero indentation (otherwise there is no body).
            if !self.tokens[first].line_start() || !self.starts_domain_member(first) {
                return None;
            }
            let member_indent = self.line_indent_of_token(first);
            if member_indent == 0 {
                return None;
            }
            let body_start = *cursor;
            (body_start, end, false)
        };

        let mut members = Vec::new();
        let mut pending_annotation: Option<TypeExpr> = None;
        // When we see `name : TypeExpr` (colon annotation), we hold the member back here.
        // If the very next member has the same name and a body, we merge the annotation in.
        // Otherwise we flush the held member as-is.
        let mut pending_colon_member: Option<DomainMember> = None;

        // Determine member indent from the first member (if any).
        let member_indent = self
            .peek_nontrivia(*cursor, inner_end)
            .filter(|&i| self.tokens[i].line_start())
            .map(|i| self.line_indent_of_token(i))
            .unwrap_or(0);

        while let Some(index) = self.peek_nontrivia(*cursor, inner_end) {
            // In indented mode, stop at any line-start token not at the member indent level.
            if !brace_syntax
                && self.tokens[index].line_start()
                && self.line_indent_of_token(index) != member_indent
            {
                break;
            }
            if !self.starts_domain_member(index) {
                break;
            }

            // Handle `type TypeExpr` lines as annotations for the next member
            if self.tokens[index].kind() == TokenKind::TypeKw {
                if let Some(held) = pending_colon_member.take() {
                    members.push(held);
                }
                *cursor = index + 1;
                let type_end = self
                    .find_next_domain_member_start(*cursor, inner_end, member_indent)
                    .unwrap_or(inner_end);
                let annotation = self
                    .parse_type_expr(cursor, type_end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "domain member type annotation is missing its type after `type`",
                            )
                            .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                            .with_primary_label(
                                self.source_span_for_range(index, *cursor),
                                "expected a type such as `Int -> Duration`",
                            ),
                        );
                        None
                    });
                *cursor = type_end;
                pending_annotation = annotation;
                continue;
            }

            let before = *cursor;
            let Some(mut member) = self.parse_domain_member(cursor, inner_end, member_indent)
            else {
                break;
            };

            // Apply `type TypeExpr` annotation if pending.
            if let Some(annotation) = pending_annotation.take() {
                member.annotation = Some(annotation);
            }

            // Try to pair a pending `name : TypeExpr` annotation with this implementation.
            if member.annotation.is_none()
                && let Some(held) = pending_colon_member.take() {
                    let held_name = domain_member_surface_name_str(&held.name);
                    let this_name = domain_member_surface_name_str(&member.name);
                    if held_name == this_name {
                        // Merge: attach the annotation from the held member.
                        member.annotation = held.annotation;
                    } else {
                        // Different name — flush held first.
                        members.push(held);
                    }
                }

            // If this member is annotation-only (from `name : TypeExpr`), hold it.
            if member.annotation.is_some()
                && member.body.is_none()
                && member.parameters.is_empty()
                && pending_annotation.is_none()
            {
                if let Some(held) = pending_colon_member.take() {
                    members.push(held);
                }
                pending_colon_member = Some(member);
                if *cursor <= before {
                    break;
                }
                continue;
            }

            members.push(member);
            if *cursor <= before {
                break;
            }
        }

        // Flush any held colon-annotation member.
        if let Some(held) = pending_colon_member.take() {
            members.push(held);
        }

        if let Some(annotation) = pending_annotation {
            self.diagnostics.push(
                Diagnostic::error("domain member type annotation has no following member")
                    .with_code(MISSING_DOMAIN_MEMBER_NAME)
                    .with_primary_label(
                        annotation.span,
                        "expected a member binding after this type annotation",
                    ),
            );
        }

        // Consume closing `}` only in brace syntax.
        if brace_syntax {
            *cursor = inner_end + 1;
        }

        (!members.is_empty()).then_some(DomainBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_source_provider_contract_body(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SourceProviderContractBody> {
        let body_start = *cursor;
        let mut members = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if !self.tokens[index].line_start() {
                break;
            }
            let Some(member) = self.parse_source_provider_contract_member(cursor, end) else {
                break;
            };
            members.push(member);
        }

        (!members.is_empty()).then_some(SourceProviderContractBody {
            members,
            span: self.source_span_for_range(body_start, *cursor),
        })
    }

    fn parse_class_member(&mut self, cursor: &mut usize, end: usize) -> Option<ClassMember> {
        let start = *cursor;
        let name = self.parse_signature_member_name(cursor, end)?;
        let (constraints, annotation) =
            if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                let (constraints, annotation) = self.parse_constrained_type(cursor, end);
                let annotation = annotation.or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("class member is missing its type after `:`")
                            .with_code(MISSING_CLASS_MEMBER_TYPE)
                            .with_primary_label(
                                name.span(),
                                "expected a member type such as `A -> A -> Bool`",
                            ),
                    );
                    None
                });
                (constraints, annotation)
            } else {
                self.diagnostics.push(
                    Diagnostic::error("class member is missing `:` before its type")
                        .with_code(MISSING_CLASS_MEMBER_TYPE)
                        .with_primary_label(name.span(), "expected `:` followed by a member type"),
                );
                (Vec::new(), None)
            };

        Some(ClassMember {
            name,
            constraints,
            annotation,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_instance_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<InstanceMember> {
        let start = *cursor;
        let name = self.parse_signature_member_name(cursor, end)?;
        let mut parameters = Vec::new();

        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() {
                break;
            }
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    parameters.push(self.identifier_from_token(index));
                    *cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        let equals_span = if let Some(index) = self.consume_kind(cursor, end, TokenKind::Equals) {
            Some(self.source_span_of_token(index))
        } else {
            self.diagnostics.push(
                Diagnostic::error("instance member is missing `=` before its body")
                    .with_code(MISSING_INSTANCE_MEMBER_BODY)
                    .with_primary_label(name.span(), "expected `=` followed by an expression body"),
            );
            None
        };
        let member_end = self
            .find_next_instance_member_start(*cursor, end, member_indent)
            .unwrap_or(end);
        let body = if let Some(equals_span) = equals_span {
            self.parse_expr(cursor, member_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("instance member is missing its body after `=`")
                            .with_code(MISSING_INSTANCE_MEMBER_BODY)
                            .with_primary_label(
                                equals_span,
                                "expected an expression body for this instance member",
                            ),
                    );
                    None
                })
        } else {
            None
        };
        if body.is_some()
            && let Some(trailing_index) = self.next_significant_in_range(*cursor, member_end) {
                self.diagnostics.push(
                    Diagnostic::error("instance member body must contain exactly one expression")
                        .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                        .with_primary_label(
                            self.source_span_of_token(trailing_index),
                            "this token is outside the instance member body",
                        ),
                );
            }
        *cursor = member_end;
        Some(InstanceMember {
            name,
            parameters,
            body,
            span: self.source_span_for_range(start, member_end),
        })
    }

    fn parse_domain_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
        member_indent: usize,
    ) -> Option<DomainMember> {
        let start = *cursor;

        // Literal members keep the colon syntax: `literal ms : Int -> Duration`
        if self
            .consume_identifier_text(cursor, end, "literal")
            .is_some()
        {
            let Some(suffix) = self.parse_identifier(cursor, end) else {
                self.diagnostics.push(
                    Diagnostic::error("domain literal declaration is missing its suffix name")
                        .with_code(MISSING_DOMAIN_MEMBER_NAME)
                        .with_primary_label(
                            self.source_span_for_range(start, *cursor),
                            "expected a suffix name such as `ms` or `sec`",
                        ),
                );
                return None;
            };
            let annotation = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                self.parse_type_expr(cursor, end, TypeStop::default())
                    .or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error("domain literal is missing its type after `:`")
                                .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                                .with_primary_label(
                                    suffix.span,
                                    "expected a member type such as `Int -> Duration`",
                                ),
                        );
                        None
                    })
            } else {
                self.diagnostics.push(
                    Diagnostic::error("domain literal is missing `:` before its type")
                        .with_code(MISSING_DOMAIN_MEMBER_TYPE)
                        .with_primary_label(suffix.span, "expected `:` followed by a member type"),
                );
                None
            };
            return Some(DomainMember {
                name: DomainMemberName::Literal(suffix),
                annotation,
                parameters: Vec::new(),
                body: None,
                span: self.source_span_for_range(start, *cursor),
            });
        }

        // Signature members: `name params = body` (binding) or `name` (declaration-only)
        let name = self.parse_signature_member_name(cursor, end)?;

        // Inline colon annotation: `name : TypeExpr` — returns annotation-only member.
        // The implementation `name params = body` may follow on the next line with the same name.
        if let Some(colon_idx) = self.peek_nontrivia(*cursor, end)
            && self.tokens[colon_idx].kind() == TokenKind::Colon
                && !self.tokens[colon_idx].line_start()
            {
                *cursor = colon_idx + 1;
                let ann_end = self
                    .find_next_domain_member_start(*cursor, end, member_indent)
                    .unwrap_or(end);
                let annotation = self.parse_type_expr(cursor, ann_end, TypeStop::default());
                *cursor = ann_end;
                return Some(DomainMember {
                    name: DomainMemberName::Signature(name),
                    annotation,
                    parameters: Vec::new(),
                    body: None,
                    span: self.source_span_for_range(start, *cursor),
                });
            }

        let mut parameters = Vec::new();
        while let Some(index) = self.peek_nontrivia(*cursor, end) {
            if self.tokens[index].line_start() {
                break;
            }
            match self.tokens[index].kind() {
                TokenKind::Identifier => {
                    parameters.push(self.identifier_from_token(index));
                    *cursor = index + 1;
                }
                TokenKind::Equals => break,
                _ => break,
            }
        }

        if let Some(eq_index) = self.consume_kind(cursor, end, TokenKind::Equals) {
            let equals_span = self.source_span_of_token(eq_index);
            let member_end = self
                .find_next_domain_member_start(*cursor, end, member_indent)
                .unwrap_or(end);
            let body = self
                .parse_expr(cursor, member_end, ExprStop::default())
                .or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("domain member binding is missing its body after `=`")
                            .with_code(MISSING_DOMAIN_MEMBER_BODY)
                            .with_primary_label(
                                equals_span,
                                "expected an expression body for this domain member",
                            ),
                    );
                    None
                });
            if body.is_some()
                && let Some(trailing_index) = self.next_significant_in_range(*cursor, member_end) {
                    self.diagnostics.push(
                        Diagnostic::error("domain member body must contain exactly one expression")
                            .with_code(TRAILING_DECLARATION_BODY_TOKEN)
                            .with_primary_label(
                                self.source_span_of_token(trailing_index),
                                "this token is outside the domain member body",
                            ),
                    );
                }
            *cursor = member_end;
            return Some(DomainMember {
                name: DomainMemberName::Signature(name),
                annotation: None,
                parameters,
                body,
                span: self.source_span_for_range(start, member_end),
            });
        }

        // No `=` — declaration-only member (just a name, no body)
        if !parameters.is_empty() {
            self.diagnostics.push(
                Diagnostic::error("domain member binding is missing `=` before its body")
                    .with_code(MISSING_DOMAIN_MEMBER_BODY)
                    .with_primary_label(name.span(), "expected `=` followed by an expression body"),
            );
            return None;
        }

        Some(DomainMember {
            name: DomainMemberName::Signature(name),
            annotation: None,
            parameters: Vec::new(),
            body: None,
            span: self.source_span_for_range(start, *cursor),
        })
    }

    fn parse_source_provider_contract_member(
        &mut self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<SourceProviderContractMember> {
        let start = *cursor;
        let name = self.parse_identifier(cursor, end)?;
        match name.text.as_str() {
            "option" | "argument" | "operation" | "command" => {
                let schema_kind = match name.text.as_str() {
                    "option" => ("source option", "timeout", "Text", "OptionSchema"),
                    "argument" => ("source argument", "path", "Text", "ArgumentSchema"),
                    "operation" => (
                        "provider operation",
                        "read",
                        "Text -> Signal Text",
                        "OperationSchema",
                    ),
                    "command" => (
                        "provider command",
                        "delete",
                        "Text -> Task Text Unit",
                        "CommandSchema",
                    ),
                    _ => {
                        unreachable!("provider contract schema members stay within known keywords")
                    }
                };
                let schema_name = self.parse_identifier(cursor, end).or_else(|| {
                    self.diagnostics.push(
                        Diagnostic::error("provider contract schema member is missing its name")
                            .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_NAME)
                            .with_primary_label(
                                name.span,
                                format!(
                                    "expected a {} name such as `{}`",
                                    schema_kind.0, schema_kind.1
                                ),
                            ),
                    );
                    None
                });
                let annotation = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                    self.parse_type_expr(cursor, end, TypeStop::default())
                        .or_else(|| {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    "provider contract schema member is missing its type after `:`",
                                )
                                .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE)
                                .with_primary_label(
                                    schema_name.as_ref().map_or(name.span, |item| item.span),
                                    format!(
                                        "expected a {} type such as `{}`",
                                        schema_kind.0, schema_kind.2
                                    ),
                                ),
                            );
                            None
                        })
                } else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "provider contract schema member is missing `:` before its type",
                        )
                        .with_code(MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE)
                        .with_primary_label(
                            schema_name.as_ref().map_or(name.span, |item| item.span),
                            "expected `:` followed by a schema type",
                        ),
                    );
                    None
                };
                let member = SourceProviderContractSchemaMember {
                    name: schema_name,
                    annotation,
                    span: self.source_span_for_range(start, *cursor),
                };
                Some(match schema_kind.3 {
                    "OptionSchema" => SourceProviderContractMember::OptionSchema(member),
                    "ArgumentSchema" => SourceProviderContractMember::ArgumentSchema(member),
                    "OperationSchema" => SourceProviderContractMember::OperationSchema(member),
                    "CommandSchema" => SourceProviderContractMember::CommandSchema(member),
                    _ => unreachable!("provider contract schema kinds stay within known variants"),
                })
            }
            _ => {
                let value = if self.consume_kind(cursor, end, TokenKind::Colon).is_some() {
                    self.parse_identifier(cursor, end).or_else(|| {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "provider contract member is missing its value after `:`",
                            )
                            .with_code(MISSING_PROVIDER_CONTRACT_MEMBER_VALUE)
                            .with_primary_label(
                                name.span,
                                "expected a provider-contract value such as `providerTrigger`",
                            ),
                        );
                        None
                    })
                } else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "provider contract member is missing `:` before its value",
                        )
                        .with_code(MISSING_PROVIDER_CONTRACT_MEMBER_VALUE)
                        .with_primary_label(name.span, "expected `:` followed by a contract value"),
                    );
                    None
                };

                Some(SourceProviderContractMember::FieldValue(
                    SourceProviderContractFieldValue {
                        name: Some(name),
                        value,
                        span: self.source_span_for_range(start, *cursor),
                    },
                ))
            }
        }
    }

    fn parse_signature_member_name(
        &self,
        cursor: &mut usize,
        end: usize,
    ) -> Option<ClassMemberName> {
        if let Some(identifier) = self.parse_identifier(cursor, end) {
            return Some(ClassMemberName::Identifier(identifier));
        }

        let start = self.consume_kind(cursor, end, TokenKind::LParen)?;
        let operator_index = self.peek_nontrivia(*cursor, end)?;
        let operator = match self.tokens[operator_index].kind() {
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::Greater => OperatorName {
                text: self.tokens[operator_index].text(self.source).to_owned(),
                span: self.source_span_of_token(operator_index),
            },
            _ => return None,
        };
        *cursor = operator_index + 1;
        let _ = self.consume_kind(cursor, end, TokenKind::RParen)?;
        let span = self.source_span_for_range(start, *cursor);
        Some(ClassMemberName::Operator(OperatorName {
            text: operator.text,
            span,
        }))
    }

    fn parse_use_item(&mut self, base: ItemBase, keyword_index: usize, end: usize) -> UseItem {
        let mut cursor = keyword_index + 1;
        let path = self.parse_qualified_name(&mut cursor, end);
        if path.is_none() {
            self.diagnostics.push(
                Diagnostic::error("`use` declaration is missing its module path")
                    .with_code(MISSING_USE_PATH)
                    .with_primary_label(
                        self.source_span_of_token(keyword_index),
                        "expected a dotted module path such as `aivi.network`",
                    ),
            );
        }

        let mut imports = Vec::new();
        if self
            .consume_kind(&mut cursor, end, TokenKind::LParen)
            .is_some()
        {
            loop {
                if self
                    .consume_kind(&mut cursor, end, TokenKind::RParen)
                    .is_some()
                {
                    break;
                }
                let Some(import) = self.parse_use_import(&mut cursor, end) else {
                    break;
                };
                imports.push(import);
                let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
            }
        }

        UseItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            path,
            imports,
        }
    }

    fn parse_use_import(&mut self, cursor: &mut usize, end: usize) -> Option<UseImport> {
        let path = self.parse_qualified_name(cursor, end)?;
        let alias = match self.peek_nontrivia(*cursor, end) {
            Some(index)
                if self.tokens[index].kind() == TokenKind::Identifier
                    && self.is_identifier_text(index, "as") =>
            {
                *cursor = index + 1;
                match self.parse_identifier(cursor, end) {
                    Some(alias) => Some(alias),
                    None => {
                        self.diagnostics.push(
                            Diagnostic::error("`use` import alias is missing its local name")
                                .with_code(MISSING_USE_ALIAS)
                                .with_primary_label(
                                    self.source_span_of_token(index),
                                    "expected a local alias such as `request` after `as`",
                                ),
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        Some(UseImport { path, alias })
    }

    fn parse_export_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> ExportItem {
        let mut cursor = keyword_index + 1;
        let targets = self.parse_export_targets(&mut cursor, end, keyword_index);

        ExportItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            targets,
        }
    }

    fn parse_export_targets(
        &mut self,
        cursor: &mut usize,
        end: usize,
        keyword_index: usize,
    ) -> Vec<Identifier> {
        let Some(next_index) = self.peek_nontrivia(*cursor, end) else {
            self.diagnostics.push(self.missing_export_name_diagnostic(
                self.source_span_of_token(keyword_index),
                "expected an identifier after `export`",
            ));
            return Vec::new();
        };

        if self.tokens[next_index].kind() != TokenKind::LParen {
            let target = self.parse_identifier(cursor, end);
            if target.is_none() {
                self.diagnostics.push(self.missing_export_name_diagnostic(
                    self.source_span_of_token(keyword_index),
                    "expected an identifier after `export`",
                ));
            }
            return target.into_iter().collect();
        }

        *cursor = next_index + 1;
        let group_span = self.source_span_of_token(next_index);
        let mut targets = Vec::new();
        let mut emitted_missing_target = false;

        loop {
            if self.consume_kind(cursor, end, TokenKind::RParen).is_some() {
                break;
            }

            match self.parse_identifier(cursor, end) {
                Some(target) => targets.push(target),
                None => {
                    let span = self
                        .peek_nontrivia(*cursor, end)
                        .map(|index| self.source_span_of_token(index))
                        .unwrap_or(group_span);
                    let message = if targets.is_empty() {
                        "expected at least one identifier inside `export (...)`"
                    } else {
                        "expected an identifier after `,` inside `export (...)`"
                    };
                    self.diagnostics
                        .push(self.missing_export_name_diagnostic(span, message));
                    emitted_missing_target = true;
                    break;
                }
            }

            let _ = self.consume_kind(cursor, end, TokenKind::Comma);
        }

        if targets.is_empty() && !emitted_missing_target {
            self.diagnostics.push(self.missing_export_name_diagnostic(
                group_span,
                "expected at least one identifier inside `export (...)`",
            ));
        }

        targets
    }

    fn missing_export_name_diagnostic(&self, span: SourceSpan, label: &str) -> Diagnostic {
        Diagnostic::error("`export` declaration is missing the exported name")
            .with_code(MISSING_EXPORT_NAME)
            .with_primary_label(span, label)
    }

    fn parse_hoist_item(
        &mut self,
        base: ItemBase,
        keyword_index: usize,
        end: usize,
    ) -> crate::cst::HoistItem {
        let mut cursor = keyword_index + 1;

        // Optional kind filter list: `(func, value, signal, type, domain, class)`
        let kind_filters = if self
            .peek_nontrivia(cursor, end)
            .map(|i| self.tokens[i].kind() == TokenKind::LParen)
            .unwrap_or(false)
        {
            let lparen_idx = self.peek_nontrivia(cursor, end).unwrap();
            cursor = lparen_idx + 1;
            let mut filters = Vec::new();
            loop {
                if self
                    .consume_kind(&mut cursor, end, TokenKind::RParen)
                    .is_some()
                {
                    break;
                }
                match self.parse_identifier(&mut cursor, end) {
                    Some(ident) => {
                        filters.push(crate::cst::HoistKindFilter {
                            span: ident.span,
                            text: ident.text,
                        });
                    }
                    None => break,
                }
                let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
            }
            filters
        } else {
            Vec::new()
        };

        // Optional `hiding (name1, name2, ...)` clause
        let hiding = if self
            .peek_nontrivia(cursor, end)
            .map(|i| {
                self.tokens[i].kind() == TokenKind::Identifier
                    && self.is_identifier_text(i, "hiding")
            })
            .unwrap_or(false)
        {
            let hiding_idx = self.peek_nontrivia(cursor, end).unwrap();
            cursor = hiding_idx + 1;
            if self
                .peek_nontrivia(cursor, end)
                .map(|i| self.tokens[i].kind() == TokenKind::LParen)
                .unwrap_or(false)
            {
                let lparen_idx = self.peek_nontrivia(cursor, end).unwrap();
                cursor = lparen_idx + 1;
                let mut names = Vec::new();
                loop {
                    if self
                        .consume_kind(&mut cursor, end, TokenKind::RParen)
                        .is_some()
                    {
                        break;
                    }
                    match self.parse_identifier(&mut cursor, end) {
                        Some(ident) => names.push(ident),
                        None => break,
                    }
                    let _ = self.consume_kind(&mut cursor, end, TokenKind::Comma);
                }
                names
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        crate::cst::HoistItem {
            base,
            keyword_span: self.source_span_of_token(keyword_index),
            kind_filters,
            hiding,
        }
    }

    fn parse_error_item(&mut self, start: usize) -> Item {
        let end = self
            .find_next_item_start(start + 1)
            .unwrap_or(self.tokens.len());
        let base = self.make_base(start, end, Vec::new());
        self.diagnostics.push(
            Diagnostic::error("expected a top-level declaration")
                .with_code(UNEXPECTED_TOP_LEVEL_TOKEN)
                .with_primary_label(
                    self.source_span_of_token(start),
                    "expected `type`, `value`, `func`, `signal`, `class`, `use`, `export`, `hoist`, or `@decorator` here",
                ),
        );
        Item::Error(ErrorItem {
            base,
            message: "unexpected top-level token".to_owned(),
        })
    }

}
