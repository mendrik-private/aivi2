struct Lowerer<'a> {
    module: Module,
    diagnostics: Vec<Diagnostic>,
    resolver: &'a dyn crate::resolver::ImportResolver,
    next_lambda_id: usize,
}

#[derive(Clone, Copy)]
enum AmbientProjectionWork {
    Expr {
        expr: ExprId,
        ambient_allowed: bool,
    },
    Markup {
        node: MarkupNodeId,
        ambient_allowed: bool,
    },
    Control {
        node: ControlNodeId,
        ambient_allowed: bool,
    },
}

#[derive(Clone, Copy)]
struct NamedSite<T> {
    value: T,
    span: SourceSpan,
}

#[derive(Clone, Default)]
struct LambdaOwnerContext {
    type_parameters: Vec<TypeParameterId>,
    context: Vec<TypeId>,
    /// Bindings + annotation from the enclosing function, used to propagate
    /// types onto capture parameters so the typechecker can resolve operators
    /// like `==` inside hoisted lambdas.
    owner_parameters: Vec<BindingId>,
    owner_annotation: Option<TypeId>,
}

#[derive(Clone, Default)]
struct LambdaScopeStack {
    scopes: Vec<Vec<BindingId>>,
}

impl LambdaScopeStack {
    fn with_bindings(bindings: impl IntoIterator<Item = BindingId>) -> Self {
        let mut scope = Self::default();
        scope.push(bindings);
        scope
    }

    fn push(&mut self, bindings: impl IntoIterator<Item = BindingId>) {
        self.scopes.push(bindings.into_iter().collect());
    }

    fn contains(&self, binding: BindingId) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(&binding))
    }
}

#[derive(Default)]
struct Namespaces {
    term_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    ambient_term_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    domain_terms: HashMap<String, Vec<NamedSite<DomainMemberResolution>>>,
    class_terms: HashMap<String, Vec<NamedSite<crate::hir::ClassMemberResolution>>>,
    ambient_class_terms: HashMap<String, Vec<NamedSite<crate::hir::ClassMemberResolution>>>,
    type_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    ambient_type_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    any_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    provider_contracts: HashMap<String, Vec<NamedSite<ItemId>>>,
    literal_suffixes: HashMap<String, Vec<NamedSite<LiteralSuffixResolution>>>,
    ambient_literal_suffixes: HashMap<String, Vec<NamedSite<LiteralSuffixResolution>>>,
    term_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    type_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    /// Names made available project-wide by `hoist` declarations.  Consulted
    /// after explicit `use` imports but before class/builtin fallbacks.
    hoisted_term_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    hoisted_type_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    /// Module paths (dot-joined) that have already been registered via a local
    /// `hoist` declaration.  Prevents double-registration when the workspace
    /// scan returns the same module path.
    hoisted_module_paths: std::collections::HashSet<String>,
}

#[derive(Clone, Copy)]
enum LookupResult<T> {
    Unique(T),
    Ambiguous,
    Missing,
}

#[derive(Clone, Default)]
struct ResolveEnv {
    term_scopes: Vec<HashMap<String, BindingId>>,
    type_scopes: Vec<HashMap<String, TypeParameterId>>,
    implicit_type_parameters: Vec<TypeParameterId>,
    allow_implicit_type_parameters: bool,
    prefer_ambient_names: bool,
}

#[derive(Clone, Copy)]
enum MarkupPlacement {
    Renderable,
    EachEmpty,
    MatchCase,
}

enum LoweredMarkup {
    Renderable(MarkupNodeId),
    Empty(ControlNodeId),
    Case(ControlNodeId),
}

impl<'a> Lowerer<'a> {
    fn new(file: aivi_base::FileId, resolver: &'a dyn crate::resolver::ImportResolver) -> Self {
        Self {
            module: Module::new(file),
            diagnostics: Vec::new(),
            resolver,
            next_lambda_id: 0,
        }
    }

    fn from_module(module: Module, resolver: &'a dyn crate::resolver::ImportResolver) -> Self {
        Self {
            module,
            diagnostics: Vec::new(),
            resolver,
            next_lambda_id: 0,
        }
    }

    fn lower_ambient_prelude(&mut self) {
        let source = aivi_base::SourceFile::new(
            self.module.file(),
            "<aivi.prelude>",
            AMBIENT_PRELUDE_SOURCE,
        );
        let parsed = syn::parse_module(&source);
        debug_assert!(
            !parsed.has_errors(),
            "ambient prelude must parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        for item in &parsed.module.items {
            self.lower_ambient_item(item);
        }
    }

    fn lower_item(&mut self, item: &syn::Item) {
        self.lower_item_with_storage(item, false);
    }

    fn lower_ambient_item(&mut self, item: &syn::Item) {
        self.lower_item_with_storage(item, true);
    }

    fn lower_item_with_storage(&mut self, item: &syn::Item, ambient: bool) {
        if let syn::Item::Export(item) = item {
            for export in self.lower_export_items(item) {
                self.store_item(Item::Export(export), ambient);
            }
            return;
        }

        if let syn::Item::Type(item) = item {
            let (lowered, companions) = self.lower_type_item(item);
            self.store_item(Item::Type(lowered), ambient);
            for companion in companions {
                self.store_item(Item::Function(companion), ambient);
            }
            return;
        }

        if let syn::Item::From(item) = item {
            for lowered in self.lower_from_item(item) {
                self.store_item(lowered, ambient);
            }
            return;
        }

        let lowered = match item {
            syn::Item::Fun(item) => Some(Item::Function(self.lower_function_item(item))),
            syn::Item::Value(item) => Some(Item::Value(self.lower_value_item(item))),
            syn::Item::Signal(item) => Some(Item::Signal(self.lower_signal_item(item))),
            syn::Item::Class(item) => Some(Item::Class(self.lower_class_item(item))),
            syn::Item::Instance(item) => Some(Item::Instance(self.lower_instance_item(item))),
            syn::Item::Domain(item) => Some(Item::Domain(self.lower_domain_item(item))),
            syn::Item::SourceProviderContract(item) => Some(Item::SourceProviderContract(
                self.lower_source_provider_contract_item(item),
            )),
            syn::Item::Use(item) => Some(Item::Use(self.lower_use_item(item))),
            syn::Item::Hoist(item) => Some(Item::Hoist(self.lower_hoist_item(item))),
            syn::Item::Export(_) => {
                unreachable!("export items are handled before single-item lowering")
            }
            syn::Item::Type(_) => {
                unreachable!("type items are handled before single-item lowering")
            }
            syn::Item::From(_) => {
                unreachable!("from items are handled before single-item lowering")
            }
            syn::Item::Error(item) => {
                self.emit_error(
                    item.base.span,
                    "error recovery item cannot enter Milestone 2 HIR",
                    code("error-item"),
                );
                None
            }
        };

        if let Some(item) = lowered {
            self.store_item(item, ambient);
        }
    }

    fn store_item(&mut self, item: Item, ambient: bool) {
        if ambient {
            if self.module.push_ambient_item(item).is_err() {
                self.emit_arena_overflow("HIR ambient item arena");
            }
        } else if self.module.push_item(item).is_err() {
            self.emit_arena_overflow("HIR item arena");
        }
    }

    fn lower_type_item(&mut self, item: &syn::NamedItem) -> (TypeItem, Vec<FunctionItem>) {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Type, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "type declaration");
        let parameters = self.lower_type_parameters(&item.type_parameters);
        let mut companions = Vec::new();
        let body = match item.type_body() {
            Some(syn::TypeDeclBody::Alias(expr)) => TypeItemBody::Alias(self.lower_type_expr(expr)),
            Some(syn::TypeDeclBody::Sum(sum)) => {
                let variants = sum
                    .variants
                    .iter()
                    .map(|variant| TypeVariant {
                        span: variant.span,
                        name: self.required_name(
                            variant.name.as_ref(),
                            variant.span,
                            "type variant",
                        ),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| crate::hir::TypeVariantField {
                                label: field.label.as_ref().map(|l| l.text.as_str().into()),
                                ty: self.lower_type_expr(&field.ty),
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                companions = sum
                    .companions
                    .iter()
                    .map(|member| self.lower_type_companion_member(member, &parameters))
                    .collect();
                match crate::NonEmpty::from_vec(variants) {
                    Ok(variants) => TypeItemBody::Sum(variants),
                    Err(_) => {
                        self.emit_error(
                            item.base.span,
                            "sum type must contain at least one constructor",
                            code("empty-sum-type"),
                        );
                        TypeItemBody::Alias(self.placeholder_type(item.base.span))
                    }
                }
            }
            None => {
                self.emit_error(
                    item.base.span,
                    "type declaration is missing a body",
                    code("missing-type-body"),
                );
                TypeItemBody::Alias(self.placeholder_type(item.base.span))
            }
        };

        (
            TypeItem {
                header,
                name,
                parameters,
                body,
            },
            companions,
        )
    }

    fn lower_value_item(&mut self, item: &syn::NamedItem) -> ValueItem {
        if !item.type_parameters.is_empty() {
            let type_param_span = item
                .type_parameters
                .first()
                .unwrap()
                .span
                .join(item.type_parameters.last().unwrap().span)
                .unwrap_or(item.base.span);
            self.emit_warning(
                type_param_span,
                "generic value declarations are not yet supported and will be ignored",
                code("unsupported-generic-value"),
            );
        }
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Value, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "value declaration");
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));
        let body = item
            .expr_body()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "value declaration is missing a body",
                    code("missing-value-body"),
                );
                self.placeholder_expr(item.base.span)
            });

        ValueItem {
            header,
            name,
            annotation,
            body,
        }
    }

    fn lower_function_item(&mut self, item: &syn::NamedItem) -> FunctionItem {
        if !item.type_parameters.is_empty() {
            self.emit_warning(
                item.base.span,
                "generic function type parameters are not yet supported and will be ignored",
                code("unsupported-generic-function"),
            );
        }
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Function, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "function declaration");
        let parameters = item
            .parameters
            .iter()
            .map(|parameter| self.lower_function_parameter(parameter))
            .collect();
        let context = item
            .constraints
            .iter()
            .map(|constraint| self.lower_type_expr(constraint))
            .collect();
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));
        let body = item
            .expr_body()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "function declaration is missing a body",
                    code("missing-function-body"),
                );
                self.placeholder_expr(item.base.span)
            });

        FunctionItem {
            header,
            name,
            type_parameters: Vec::new(),
            context,
            parameters,
            annotation,
            body,
        }
    }

    fn lower_signal_item(&mut self, item: &syn::NamedItem) -> SignalItem {
        if !item.type_parameters.is_empty() {
            let type_param_span = item
                .type_parameters
                .first()
                .unwrap()
                .span
                .join(item.type_parameters.last().unwrap().span)
                .unwrap_or(item.base.span);
            self.emit_warning(
                type_param_span,
                "generic signal declarations are not yet supported and will be ignored",
                code("unsupported-generic-signal"),
            );
        }
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Signal, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "signal declaration");
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));

        let (body, reactive_updates) = if let Some(merge) = item.merge_body() {
            let (seed, updates) = self.lower_signal_merge_arms(merge, item.keyword_span);
            (seed, updates)
        } else {
            let body = item.expr_body().map(|expr| self.lower_expr(expr));
            (body, Vec::new())
        };

        SignalItem {
            header,
            name,
            annotation,
            body,
            reactive_updates,
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }
    }

    fn lower_from_item(&mut self, item: &syn::FromItem) -> Vec<Item> {
        item.entries
            .iter()
            .map(|entry| self.lower_from_entry_item(item, entry))
            .collect()
    }

    fn lower_from_entry_item(&mut self, from_item: &syn::FromItem, entry: &syn::FromEntry) -> Item {
        if entry.parameters.is_empty() {
            Item::Signal(self.lower_from_entry_signal(from_item, entry))
        } else {
            Item::Function(self.lower_from_entry_function(from_item, entry))
        }
    }

    fn lower_from_entry_signal(
        &mut self,
        from_item: &syn::FromItem,
        entry: &syn::FromEntry,
    ) -> SignalItem {
        let synthetic = syn::NamedItem {
            base: syn::ItemBase {
                span: entry.span,
                token_range: from_item.base.token_range,
                decorators: from_item.base.decorators.clone(),
                leading_comments: Vec::new(),
            },
            keyword_span: from_item.keyword_span,
            name: Some(entry.name.clone()),
            type_parameters: Vec::new(),
            constraints: entry.constraints.clone(),
            annotation: entry
                .annotation
                .as_ref()
                .map(|annotation| self.wrap_from_entry_annotation_result_in_signal(annotation)),
            function_form: syn::cst::FunctionSurfaceForm::Explicit,
            parameters: Vec::new(),
            body: Some(syn::NamedItemBody::Expr(self.synthesize_from_entry_body(
                from_item.source.as_ref(),
                entry.body.as_ref(),
                entry.span,
            ))),
        };
        self.lower_signal_item(&synthetic)
    }

    fn lower_from_entry_function(
        &mut self,
        from_item: &syn::FromItem,
        entry: &syn::FromEntry,
    ) -> FunctionItem {
        if entry.annotation.is_none() {
            self.emit_error(
                entry.span,
                format!(
                    "parameterized `from` entry `{}` is missing a preceding type annotation",
                    entry.name.text
                ),
                code("missing-from-entry-type"),
            );
        }
        let synthetic = syn::NamedItem {
            base: syn::ItemBase {
                span: entry.span,
                token_range: from_item.base.token_range,
                decorators: from_item.base.decorators.clone(),
                leading_comments: Vec::new(),
            },
            keyword_span: from_item.keyword_span,
            name: Some(entry.name.clone()),
            type_parameters: Vec::new(),
            constraints: entry.constraints.clone(),
            annotation: entry
                .annotation
                .as_ref()
                .map(|annotation| self.wrap_from_entry_annotation_result_in_signal(annotation)),
            function_form: syn::cst::FunctionSurfaceForm::Explicit,
            parameters: entry.parameters.clone(),
            body: Some(syn::NamedItemBody::Expr(self.synthesize_from_entry_body(
                from_item.source.as_ref(),
                entry.body.as_ref(),
                entry.span,
            ))),
        };
        self.lower_function_item(&synthetic)
    }

    fn synthesize_from_entry_body(
        &self,
        source: Option<&syn::Expr>,
        body: Option<&syn::Expr>,
        fallback_span: SourceSpan,
    ) -> syn::Expr {
        match (source, body) {
            (Some(source), Some(body)) => self.prepend_from_source(source, body),
            (Some(source), None) => source.clone(),
            (None, Some(body)) => body.clone(),
            (None, None) => syn::Expr {
                span: fallback_span,
                kind: syn::ExprKind::Name(syn::Identifier {
                    text: "invalid".to_owned(),
                    span: fallback_span,
                }),
            },
        }
    }

    fn wrap_from_entry_annotation_result_in_signal(
        &self,
        annotation: &syn::TypeExpr,
    ) -> syn::TypeExpr {
        match &annotation.kind {
            syn::TypeExprKind::Arrow { parameter, result } => syn::TypeExpr {
                span: annotation.span,
                kind: syn::TypeExprKind::Arrow {
                    parameter: parameter.clone(),
                    result: Box::new(self.wrap_from_entry_annotation_result_in_signal(result)),
                },
            },
            _ => syn::TypeExpr {
                span: annotation.span,
                kind: syn::TypeExprKind::Apply {
                    callee: Box::new(syn::TypeExpr {
                        span: annotation.span,
                        kind: syn::TypeExprKind::Name(syn::Identifier {
                            text: "Signal".to_owned(),
                            span: annotation.span,
                        }),
                    }),
                    arguments: vec![annotation.clone()],
                },
            },
        }
    }

    fn prepend_from_source(&self, source: &syn::Expr, body: &syn::Expr) -> syn::Expr {
        let span = source.span.join(body.span).unwrap_or(body.span);
        match &body.kind {
            syn::ExprKind::Pipe(pipe) => {
                let mut pipe = pipe.clone();
                let mut stages =
                    Vec::with_capacity(pipe.stages.len() + usize::from(pipe.head.is_some()));
                if let Some(head) = pipe.head.take() {
                    let head = *head;
                    stages.push(syn::PipeStage {
                        subject_memo: None,
                        result_memo: None,
                        span: head.span,
                        kind: syn::PipeStageKind::Transform { expr: head },
                    });
                }
                stages.extend(pipe.stages);
                pipe.head = Some(Box::new(source.clone()));
                pipe.stages = stages;
                pipe.span = span;
                syn::Expr {
                    span,
                    kind: syn::ExprKind::Pipe(pipe),
                }
            }
            _ => syn::Expr {
                span,
                kind: syn::ExprKind::Pipe(syn::PipeExpr {
                    head: Some(Box::new(source.clone())),
                    stages: vec![syn::PipeStage {
                        subject_memo: None,
                        result_memo: None,
                        span: body.span,
                        kind: syn::PipeStageKind::Transform { expr: body.clone() },
                    }],
                    span,
                }),
            },
        }
    }

    fn lower_signal_merge_arms(
        &mut self,
        merge: &syn::SignalMergeBody,
        _keyword_span: SourceSpan,
    ) -> (Option<ExprId>, Vec<ReactiveUpdateClause>) {
        let mut updates = Vec::new();
        let mut seed_body: Option<ExprId> = None;

        // Resolve source signals.
        let source_items: Vec<Option<ItemId>> = merge
            .sources
            .iter()
            .map(|source| self.resolve_merge_source(source))
            .collect();

        for arm in &merge.arms {
            let Some(pattern) = arm.pattern.as_ref() else {
                self.emit_error(
                    arm.span,
                    "signal reactive arm is missing its pattern",
                    code("merge-arm-missing-pattern"),
                );
                continue;
            };
            let Some(body) = arm.body.as_ref() else {
                self.emit_error(
                    arm.span,
                    "signal reactive arm is missing its body expression",
                    code("merge-arm-missing-body"),
                );
                continue;
            };

            // Determine if this is a default/wildcard arm.
            let is_default_arm =
                arm.source.is_none() && matches!(pattern.kind, syn::PatternKind::Wildcard);

            // Determine trigger source from the arm's source prefix.
            let trigger_source = if is_default_arm {
                // Default arm (wildcard): no specific trigger source.
                None
            } else if let Some(source_ident) = &arm.source {
                // Multi-source arm: find the source in the merge list.
                let pos = merge
                    .sources
                    .iter()
                    .position(|s| s.text == source_ident.text);
                match pos {
                    Some(idx) => source_items[idx],
                    None => {
                        self.emit_error(
                            source_ident.span,
                            format!(
                                "signal name `{}` does not appear in the merge source list",
                                source_ident.text
                            ),
                            code("merge-arm-unknown-source"),
                        );
                        None
                    }
                }
            } else if merge.sources.len() == 1 {
                // Single-source arm: the only source is the trigger.
                source_items[0]
            } else {
                None
            };

            // Build the source expression for pattern matching.
            let source_expr_ident = if is_default_arm {
                None
            } else {
                arm.source.as_ref().or_else(|| {
                    if merge.sources.len() == 1 {
                        Some(&merge.sources[0])
                    } else {
                        None
                    }
                })
            };

            if let Some(source_ident) = source_expr_ident {
                let source_expr =
                    self.lower_unresolved_name_expr(source_ident.text.as_str(), source_ident.span);
                let guard = self.make_pattern_match_bool_expr(source_expr, pattern, arm.span);
                let body_expr =
                    self.make_pattern_match_optional_expr(source_expr, pattern, body, arm.span);
                updates.push(ReactiveUpdateClause {
                    span: arm.span,
                    keyword_span: arm.span,
                    target_span: arm.span,
                    guard,
                    body: body_expr,
                    body_mode: ReactiveUpdateBodyMode::OptionalPayload,
                    trigger_source,
                });
            } else {
                // Default arm: becomes the signal's seed body (initial value).
                let body_expr = self.lower_expr(body);
                seed_body = Some(body_expr);
            }
        }

        (seed_body, updates)
    }

    fn resolve_merge_source(&mut self, source: &syn::Identifier) -> Option<ItemId> {
        let Some(source_item) = self.find_predeclared_named_item(source.text.as_str()) else {
            self.emit_error(
                source.span,
                format!(
                    "merge source `{}` must name a previously declared signal",
                    source.text
                ),
                code("merge-unknown-source"),
            );
            return None;
        };
        if !matches!(self.module.items()[source_item], Item::Signal(_)) {
            self.emit_error(
                source.span,
                format!(
                    "merge source `{}` must refer to a signal, not another kind of declaration",
                    source.text
                ),
                code("merge-source-not-signal"),
            );
            return None;
        }
        Some(source_item)
    }

    fn make_pattern_match_bool_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        span: SourceSpan,
    ) -> ExprId {
        let on_match = self.lower_unresolved_name_expr("True", span);
        let on_fallback = self.lower_unresolved_name_expr("False", span);
        self.make_pattern_match_pipe_expr(subject, pattern, on_match, on_fallback, span)
    }

    fn make_pattern_match_optional_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        body: &syn::Expr,
        span: SourceSpan,
    ) -> ExprId {
        let matched_body = self.lower_expr(body);
        let on_match = self.lower_constructor_apply_expr("Some", span, vec![matched_body]);
        let on_fallback = self.lower_unresolved_name_expr("None", span);
        self.make_pattern_match_pipe_expr(subject, pattern, on_match, on_fallback, span)
    }

    fn make_pattern_match_pipe_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        on_match: ExprId,
        on_fallback: ExprId,
        span: SourceSpan,
    ) -> ExprId {
        let match_stage = PipeStage {
            span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: self.lower_pattern(pattern),
                body: on_match,
            },
        };
        let fallback_stage = PipeStage {
            span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: self.alloc_pattern(Pattern {
                    span,
                    kind: PatternKind::Wildcard,
                }),
                body: on_fallback,
            },
        };
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head: subject,
                stages: NonEmpty::new(match_stage, vec![fallback_stage]),
                result_block_desugaring: false,
            }),
        })
    }

    fn find_predeclared_named_item(&self, name: &str) -> Option<ItemId> {
        self.module
            .root_items()
            .iter()
            .rev()
            .copied()
            .find(|item_id| match &self.module.items()[*item_id] {
                Item::Type(item) => item.name.text() == name,
                Item::Value(item) => item.name.text() == name,
                Item::Function(item) => item.name.text() == name,
                Item::Signal(item) => item.name.text() == name,
                Item::Class(item) => item.name.text() == name,
                Item::Domain(item) => item.name.text() == name,
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => false,
            })
    }

    fn lower_class_item(&mut self, item: &syn::NamedItem) -> ClassItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Class, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "class declaration");
        let mut parameters = self.lower_type_parameters(&item.type_parameters);
        if parameters.is_empty() {
            self.emit_error(
                item.base.span,
                "class declarations require at least one type parameter",
                code("missing-class-parameter"),
            );
            parameters.push(self.alloc_type_parameter(TypeParameter {
                span: item.base.span,
                name: self.make_name("A", item.base.span),
            }));
        }
        let parameters = crate::NonEmpty::from_vec(parameters)
            .expect("class fallback parameter list should be non-empty");
        let (superclasses, param_constraints, members) = item
            .class_body()
            .map(|body| {
                let superclasses: Vec<TypeId> = body
                    .with_decls
                    .iter()
                    .map(|w| self.lower_type_expr(&w.superclass))
                    .collect();
                let param_constraints: Vec<TypeId> = body
                    .require_decls
                    .iter()
                    .map(|r| self.lower_type_expr(&r.constraint))
                    .collect();
                let members = body
                    .members
                    .iter()
                    .map(|member| ClassMember {
                        span: member.span,
                        name: self.make_name(member.name.text(), member.name.span()),
                        type_parameters: Vec::new(),
                        context: member
                            .constraints
                            .iter()
                            .map(|constraint| self.lower_type_expr(constraint))
                            .collect(),
                        annotation: member
                            .annotation
                            .as_ref()
                            .map(|annotation| self.lower_type_expr(annotation))
                            .unwrap_or_else(|| {
                                self.emit_error(
                                    member.span,
                                    "class member is missing a type annotation",
                                    code("missing-class-member-type"),
                                );
                                self.placeholder_type(member.span)
                            }),
                    })
                    .collect();
                (superclasses, param_constraints, members)
            })
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "class declaration is missing a body",
                    code("missing-class-body"),
                );
                (Vec::new(), Vec::new(), Vec::new())
            });

        ClassItem {
            header,
            name,
            parameters,
            superclasses,
            param_constraints,
            members,
        }
    }

    fn lower_instance_item(&mut self, item: &syn::InstanceItem) -> InstanceItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Instance, item.base.span);
        let class = item
            .class
            .as_ref()
            .map(|class| TypeReference::unresolved(self.lower_qualified_name(class)))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing its class name",
                    code("missing-instance-class"),
                );
                TypeReference::unresolved(
                    self.make_path(&[self.make_name("invalid", item.base.span)]),
                )
            });
        let argument = item
            .target
            .as_ref()
            .map(|target| self.lower_type_expr(target))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing its target type",
                    code("missing-instance-target"),
                );
                self.placeholder_type(item.base.span)
            });
        let members = item
            .body
            .as_ref()
            .map(|body| {
                body.members
                    .iter()
                    .map(|member| InstanceMember {
                        span: member.span,
                        name: self.make_name(member.name.text(), member.name.span()),
                        parameters: member
                            .parameters
                            .iter()
                            .map(|parameter| self.lower_instance_parameter(parameter))
                            .collect(),
                        annotation: None,
                        body: member
                            .body
                            .as_ref()
                            .map(|body| self.lower_expr(body))
                            .unwrap_or_else(|| {
                                self.emit_error(
                                    member.span,
                                    "instance member is missing a body",
                                    code("missing-instance-member-body"),
                                );
                                self.placeholder_expr(member.span)
                            }),
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing a body",
                    code("missing-instance-body"),
                );
                Vec::new()
            });

        InstanceItem {
            header,
            class,
            arguments: crate::NonEmpty::new(argument, Vec::new()),
            type_parameters: Vec::new(),
            context: item
                .context
                .iter()
                .map(|constraint| self.lower_type_expr(constraint))
                .collect(),
            members,
        }
    }

    fn lower_domain_item(&mut self, item: &syn::DomainItem) -> DomainItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Domain, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "domain declaration");
        let parameters = self.lower_type_parameters(&item.type_parameters);
        let carrier = item
            .carrier
            .as_ref()
            .map(|carrier| self.lower_type_expr(carrier))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "domain declaration is missing a carrier type after `over`",
                    code("missing-domain-carrier"),
                );
                self.placeholder_type(item.base.span)
            });

        let mut members = Vec::new();
        if let Some(body) = &item.body {
            let mut seen_keys = HashMap::<String, SourceSpan>::new();
            for member in &body.members {
                let key = domain_member_surface_key(&member.name);
                if let Some(previous_span) = seen_keys.get(&key) {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "duplicate domain member `{}`",
                            domain_member_surface_name(&member.name)
                        ))
                        .with_code(code("duplicate-domain-member"))
                        .with_primary_label(
                            member.span,
                            "this domain member reuses an existing member name",
                        )
                        .with_secondary_label(*previous_span, "previous domain member here"),
                    );
                } else {
                    seen_keys.insert(key, member.span);
                }
                members.push(self.lower_domain_member(
                    member,
                    item.name.as_ref(),
                    &item.type_parameters,
                ));
            }
        }

        DomainItem {
            header,
            name,
            parameters,
            carrier,
            members,
        }
    }

    fn lower_type_companion_member(
        &mut self,
        member: &syn::TypeCompanionMember,
        owner_parameter_ids: &[TypeParameterId],
    ) -> FunctionItem {
        let header = self.lower_item_header(&[], ItemKind::Function, member.span);
        let name = self.make_name(&member.name.text, member.name.span);
        let annotation = member
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation))
            .unwrap_or_else(|| {
                self.emit_error(
                    member.span,
                    format!(
                        "type companion member `{}` is missing a type annotation",
                        member.name.text
                    ),
                    code("missing-type-companion-type"),
                );
                self.placeholder_type(member.span)
            });
        let parameters = member
            .parameters
            .iter()
            .map(|parameter| self.lower_function_parameter(parameter))
            .collect();

        let body = member
            .body
            .as_ref()
            .map(|body| self.lower_expr(body))
            .unwrap_or_else(|| {
                self.emit_error(
                    member.span,
                    format!(
                        "type companion member `{}` is missing a body",
                        member.name.text
                    ),
                    code("missing-type-companion-body"),
                );
                self.placeholder_expr(member.span)
            });

        FunctionItem {
            header,
            name,
            type_parameters: owner_parameter_ids.to_vec(),
            context: Vec::new(),
            parameters,
            annotation: Some(annotation),
            body,
        }
    }

    fn lower_domain_member(
        &mut self,
        member: &syn::DomainMember,
        domain_name: Option<&syn::Identifier>,
        domain_type_parameters: &[syn::Identifier],
    ) -> DomainMember {
        let (kind, name) = match &member.name {
            syn::DomainMemberName::Signature(signature) => match signature {
                syn::ClassMemberName::Identifier(identifier) => (
                    DomainMemberKind::Method,
                    self.make_name(&identifier.text, identifier.span),
                ),
                syn::ClassMemberName::Operator(operator) => (
                    DomainMemberKind::Operator,
                    self.make_name(&operator.text, operator.span),
                ),
            },
            syn::DomainMemberName::Literal(identifier) => (
                DomainMemberKind::Literal,
                self.make_name(&identifier.text, identifier.span),
            ),
        };

        let uses_self = member
            .body
            .as_ref()
            .is_some_and(|body| body.contains_self_reference());
        let has_explicit_self_parameter = member
            .parameters
            .first()
            .is_some_and(|parameter| parameter.text == "self");

        let annotation = member
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation))
            .unwrap_or_else(|| {
                self.emit_error(
                    member.span,
                    format!(
                        "domain member `{}` is missing a type annotation",
                        domain_member_surface_name(&member.name)
                    ),
                    code("missing-domain-member-type"),
                );
                self.placeholder_type(member.span)
            });

        // When `self` is used, prepend `DomainType ->` to the annotation.
        let annotation = if uses_self {
            if let Some(domain_id) = domain_name {
                let domain_type_id =
                    self.synthesise_owner_self_type(domain_id, domain_type_parameters);
                self.alloc_type(TypeNode {
                    span: member.span,
                    kind: TypeKind::Arrow {
                        parameter: domain_type_id,
                        result: annotation,
                    },
                })
            } else {
                annotation
            }
        } else {
            annotation
        };

        // Synthesise implicit `self` binding before explicit parameters.
        let mut parameters: Vec<FunctionParameter> = if uses_self && !has_explicit_self_parameter {
            let self_name = self.make_name("self", member.span);
            let self_binding = self.alloc_binding(Binding {
                span: member.span,
                name: self_name,
                kind: BindingKind::FunctionParameter,
            });
            vec![FunctionParameter {
                span: member.span,
                binding: self_binding,
                annotation: None,
            }]
        } else {
            Vec::new()
        };

        parameters.extend(
            member
                .parameters
                .iter()
                .map(|parameter| self.lower_instance_parameter(parameter)),
        );

        let body = member.body.as_ref().map(|body| self.lower_expr(body));

        DomainMember {
            span: member.span,
            kind,
            name,
            annotation,
            parameters,
            body,
        }
    }

    /// Construct an unresolved HIR type for the domain itself, applying type
    /// parameters when the domain is generic (e.g. `NonEmpty A`).
    fn synthesise_owner_self_type(
        &mut self,
        owner_name: &syn::Identifier,
        type_parameters: &[syn::Identifier],
    ) -> TypeId {
        let name_type = self.alloc_type(TypeNode {
            span: owner_name.span,
            kind: TypeKind::Name(TypeReference::unresolved(
                self.make_path(&[self.make_name(&owner_name.text, owner_name.span)]),
            )),
        });
        if type_parameters.is_empty() {
            return name_type;
        }
        let arguments: Vec<TypeId> = type_parameters
            .iter()
            .map(|param| {
                self.alloc_type(TypeNode {
                    span: param.span,
                    kind: TypeKind::Name(TypeReference::unresolved(
                        self.make_path(&[self.make_name(&param.text, param.span)]),
                    )),
                })
            })
            .collect();
        let arguments = NonEmpty::from_vec(arguments).expect("non-empty type parameter list");
        self.alloc_type(TypeNode {
            span: owner_name.span,
            kind: TypeKind::Apply {
                callee: name_type,
                arguments,
            },
        })
    }

    fn lower_source_provider_contract_item(
        &mut self,
        item: &syn::SourceProviderContractItem,
    ) -> SourceProviderContractItem {
        let header = self.lower_item_header(
            &item.base.decorators,
            ItemKind::SourceProviderContract,
            item.base.span,
        );
        let provider_path = item
            .provider
            .as_ref()
            .map(|provider| self.lower_qualified_name(provider));
        let provider = SourceProviderRef::from_path(provider_path.as_ref());
        match &provider {
            SourceProviderRef::Builtin(provider_ref) => {
                self.emit_error(
                    item.base.span,
                    format!(
                        "provider contract declarations cannot target built-in source provider `{}`",
                        provider_ref.key()
                    ),
                    code("builtin-source-provider-contract"),
                );
            }
            SourceProviderRef::InvalidShape(key) => {
                self.emit_error(
                    item.base.span,
                    format!(
                        "provider contract `{key}` must use a qualified provider key such as `custom.feed`"
                    ),
                    code("invalid-source-provider-contract-shape"),
                );
            }
            SourceProviderRef::Missing | SourceProviderRef::Custom(_) => {}
        }

        SourceProviderContractItem {
            header,
            provider,
            contract: self.lower_custom_source_contract(item.body.as_ref()),
        }
    }

    fn lower_custom_source_contract(
        &mut self,
        body: Option<&syn::SourceProviderContractBody>,
    ) -> crate::CustomSourceContractMetadata {
        let mut contract = crate::CustomSourceContractMetadata::default();
        let mut wakeup_span = None;
        let mut argument_spans = HashMap::new();
        let mut option_spans = HashMap::new();
        let mut capability_member_spans = HashMap::new();
        let Some(body) = body else {
            return contract;
        };

        for member in &body.members {
            match member {
                syn::SourceProviderContractMember::FieldValue(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    match name.text.as_str() {
                        "wakeup" => {
                            if let Some(previous_span) = wakeup_span {
                                self.diagnostics.push(
                                    Diagnostic::error(
                                        "provider contract field `wakeup` is duplicated",
                                    )
                                    .with_code(code("duplicate-source-provider-contract-field"))
                                    .with_primary_label(
                                        member.span,
                                        "this `wakeup` field repeats an earlier contract field",
                                    )
                                    .with_secondary_label(
                                        previous_span,
                                        "previous `wakeup` field declared here",
                                    ),
                                );
                                continue;
                            }
                            wakeup_span = Some(member.span);
                            let Some(value) = member.value.as_ref() else {
                                continue;
                            };
                            contract.recurrence_wakeup =
                                self.lower_custom_source_contract_wakeup(value);
                        }
                        _ => {
                            self.emit_error(
                                name.span,
                                format!("unknown provider contract field `{}`", name.text),
                                code("unknown-source-provider-contract-field"),
                            );
                        }
                    }
                }
                syn::SourceProviderContractMember::ArgumentSchema(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    if let Some(previous_span) =
                        argument_spans.insert(name.text.clone(), member.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "provider contract argument `{}` is duplicated",
                                name.text
                            ))
                            .with_code(code("duplicate-source-provider-contract-field"))
                            .with_primary_label(
                                member.span,
                                "this argument schema repeats an earlier declaration",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous argument schema declared here",
                            ),
                        );
                        continue;
                    }
                    let Some(annotation) = member.annotation.as_ref() else {
                        continue;
                    };
                    contract.arguments.push(crate::CustomSourceArgumentSchema {
                        span: member.span,
                        name: self.make_name(&name.text, name.span),
                        annotation: self.lower_type_expr(annotation),
                    });
                }
                syn::SourceProviderContractMember::OptionSchema(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    if let Some(previous_span) = option_spans.insert(name.text.clone(), member.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "provider contract option `{}` is duplicated",
                                name.text
                            ))
                            .with_code(code("duplicate-source-provider-contract-field"))
                            .with_primary_label(
                                member.span,
                                "this option schema repeats an earlier declaration",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous option schema declared here",
                            ),
                        );
                        continue;
                    }
                    let Some(annotation) = member.annotation.as_ref() else {
                        continue;
                    };
                    contract.options.push(crate::CustomSourceOptionSchema {
                        span: member.span,
                        name: self.make_name(&name.text, name.span),
                        annotation: self.lower_type_expr(annotation),
                    });
                }
                syn::SourceProviderContractMember::OperationSchema(member) => {
                    self.lower_custom_source_contract_capability_member(
                        member,
                        "operation",
                        &mut contract.operations,
                        &mut capability_member_spans,
                    );
                }
                syn::SourceProviderContractMember::CommandSchema(member) => {
                    self.lower_custom_source_contract_capability_member(
                        member,
                        "command",
                        &mut contract.commands,
                        &mut capability_member_spans,
                    );
                }
            }
        }

        contract
    }

    fn lower_custom_source_contract_capability_member(
        &mut self,
        member: &syn::SourceProviderContractSchemaMember,
        kind: &'static str,
        members: &mut Vec<crate::CustomSourceCapabilityMember>,
        seen: &mut HashMap<String, (SourceSpan, &'static str)>,
    ) {
        let Some(name) = member.name.as_ref() else {
            return;
        };
        if let Some((previous_span, previous_kind)) =
            seen.insert(name.text.clone(), (member.span, kind))
        {
            let detail = if previous_kind == kind {
                format!("this {kind} repeats an earlier declaration")
            } else {
                format!("this {kind} conflicts with an earlier {previous_kind} declaration")
            };
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "provider contract capability member `{}` is duplicated",
                    name.text
                ))
                .with_code(code("duplicate-source-provider-contract-field"))
                .with_primary_label(member.span, detail)
                .with_secondary_label(
                    previous_span,
                    format!("previous {previous_kind} declared here"),
                ),
            );
            return;
        }
        let Some(annotation) = member.annotation.as_ref() else {
            return;
        };
        members.push(crate::CustomSourceCapabilityMember {
            span: member.span,
            name: self.make_name(&name.text, name.span),
            annotation: self.lower_type_expr(annotation),
        });
    }

    fn lower_custom_source_contract_wakeup(
        &mut self,
        value: &syn::Identifier,
    ) -> Option<crate::CustomSourceRecurrenceWakeup> {
        match value.text.as_str() {
            "timer" => Some(crate::CustomSourceRecurrenceWakeup::Timer),
            "backoff" => Some(crate::CustomSourceRecurrenceWakeup::Backoff),
            "sourceEvent" => Some(crate::CustomSourceRecurrenceWakeup::SourceEvent),
            "providerTrigger" => Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            _ => {
                self.emit_error(
                    value.span,
                    format!("unknown provider contract wakeup `{}`", value.text),
                    code("unknown-source-provider-contract-wakeup"),
                );
                None
            }
        }
    }

    fn lower_use_item(&mut self, item: &syn::UseItem) -> UseItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Use, item.base.span);
        let module = item
            .path
            .as_ref()
            .map(|path| self.lower_qualified_name(path))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "use declaration is missing a module path",
                    code("missing-use-path"),
                );
                self.make_path(&[self.make_name("invalid", item.base.span)])
            });
        let module_name = path_text(&module);
        let module_segments = module.segments().iter().map(Name::text).collect::<Vec<_>>();
        let module_resolution = self.resolver.resolve(&module_segments);
        if let ImportModuleResolution::Cycle(cycle) = &module_resolution {
            // A module may import its own intrinsics by name (e.g. `aivi.core.bytes`
            // importing `length` from `aivi.core.bytes`).  Detect this as a direct
            // self-import where every requested name is a known intrinsic and suppress
            // the cycle error in that case.
            let is_direct_self_import = cycle.modules().iter().all(|m| m.as_ref() == module_name);
            let all_intrinsics = item.imports.iter().all(|import| {
                import
                    .path
                    .segments
                    .last()
                    .map(|s| known_import_metadata(&module_name, &s.text).is_some())
                    .unwrap_or(false)
            });
            // Suppress false-positive import-cycle errors that arise when stdlib modules
            // are compiled transitively during workspace-hoist registration. In that
            // context the module is compiled with an import stack inherited from app
            // code, so `module_name_for_file` may fall back to a file-system path string
            // (containing '/') instead of the logical dotted module name. When the cycle
            // path contains a path-encoded entry (detected by the presence of a '/'
            // separator), the cycle is an artefact of hoist-chain compilation order, not
            // a real circular import dependency.
            let is_hoist_induced_cycle = cycle.modules().iter().any(|m| m.contains('/'));
            if !(is_hoist_induced_cycle || is_direct_self_import && all_intrinsics) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "import cycle detected: {}",
                        cycle
                            .modules()
                            .iter()
                            .map(|module| module.as_ref())
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    ))
                    .with_code(code("import-cycle"))
                    .with_primary_label(
                        item.base.span,
                        "this `use` item closes a cycle in the workspace import graph",
                    ),
                );
            }
        }
        let mut imports = item
            .imports
            .iter()
            .map(|import| {
                let imported_name = import
                    .path
                    .segments
                    .last()
                    .map(|segment| self.make_name(&segment.text, segment.span))
                    .unwrap_or_else(|| self.make_name("invalid", item.base.span));
                let local_name = import
                    .alias
                    .as_ref()
                    .map(|alias| self.make_name(&alias.text, alias.span))
                    .unwrap_or_else(|| imported_name.clone());
                let (resolution, metadata, callable_type, deprecation) =
                    self.resolve_import_binding(&module_name, &imported_name, &module_resolution);
                self.alloc_import(ImportBinding {
                    span: import.path.span,
                    source_module: Some(module_name.clone().into()),
                    imported_name: imported_name.clone(),
                    local_name,
                    resolution,
                    metadata,
                    callable_type,
                    deprecation,
                })
            })
            .collect::<Vec<_>>();
        // Auto-register imported instance member bindings so cross-module instance
        // resolution can find them without the user explicitly importing by name.
        if let ImportModuleResolution::Resolved(exports) = &module_resolution {
            for instance_decl in &exports.instances {
                for member in &instance_decl.members {
                    let synthetic_name = format!(
                        "__instance_{}_{}_{}_{}",
                        instance_decl.class_name,
                        member.name,
                        instance_decl.subject,
                        import_value_type_label(&member.ty),
                    );
                    let name = self.make_name(&synthetic_name, item.base.span);
                    imports.push(self.alloc_import(ImportBinding {
                        span: item.base.span,
                        source_module: Some(module_name.clone().into()),
                        imported_name: self.make_name(&member.name, item.base.span),
                        local_name: name,
                        resolution: ImportBindingResolution::Resolved,
                        metadata: ImportBindingMetadata::InstanceMember {
                            class_name: instance_decl.class_name.clone(),
                            member_name: member.name.clone(),
                            subject: instance_decl.subject.clone(),
                            ty: member.ty.clone(),
                        },
                        callable_type: None,
                        deprecation: None,
                    }));
                }
            }
        }
        if imports.is_empty() {
            self.emit_error(
                item.base.span,
                "use declaration must import at least one member",
                code("empty-use-imports"),
            );
            imports.push(self.alloc_import(ImportBinding {
                span: item.base.span,
                source_module: Some(module_name.into()),
                imported_name: self.make_name("invalid", item.base.span),
                local_name: self.make_name("invalid", item.base.span),
                resolution: ImportBindingResolution::UnknownModule,
                metadata: ImportBindingMetadata::Unknown,
                callable_type: None,
                deprecation: None,
            }));
        }
        let imports =
            crate::NonEmpty::from_vec(imports).expect("fallback import list should be non-empty");

        UseItem {
            header,
            module,
            imports,
        }
    }

    fn lower_export_items(&mut self, item: &syn::ExportItem) -> Vec<ExportItem> {
        if item.targets.is_empty() {
            return vec![self.lower_export_target(item, None)];
        }

        item.targets
            .iter()
            .map(|target| self.lower_export_target(item, Some(target)))
            .collect()
    }

    fn lower_export_target(
        &mut self,
        item: &syn::ExportItem,
        target: Option<&syn::Identifier>,
    ) -> ExportItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Export, item.base.span);
        let target_name = self.required_name(target, item.base.span, "export declaration");
        let target = self.make_path(&[target_name]);
        ExportItem {
            header,
            target,
            resolution: ResolutionState::Unresolved,
        }
    }

    fn lower_hoist_item(&mut self, item: &syn::HoistItem) -> HoistItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Hoist, item.base.span);

        let kind_filters = item
            .kind_filters
            .iter()
            .filter_map(|f| match f.text.as_str() {
                "func" => Some(HoistKindFilter::Func),
                "value" => Some(HoistKindFilter::Value),
                "signal" => Some(HoistKindFilter::Signal),
                "type" => Some(HoistKindFilter::Type),
                "domain" => Some(HoistKindFilter::Domain),
                "class" => Some(HoistKindFilter::Class),
                other => {
                    self.emit_error(
                        f.span,
                        format!("unknown hoist kind filter `{other}`; expected one of: func, value, signal, type, domain, class"),
                        code("unknown-hoist-kind-filter"),
                    );
                    None
                }
            })
            .collect();

        let hiding = item
            .hiding
            .iter()
            .map(|ident| self.make_name(&ident.text, ident.span))
            .collect();

        HoistItem {
            header,
            kind_filters,
            hiding,
        }
    }

    fn resolve_import_binding(
        &self,
        module_name: &str,
        imported_name: &Name,
        module_resolution: &ImportModuleResolution,
    ) -> (
        ImportBindingResolution,
        ImportBindingMetadata,
        Option<ImportValueType>,
        Option<crate::DeprecationNotice>,
    ) {
        match module_resolution {
            ImportModuleResolution::Resolved(exports) => match exports.find(imported_name.text()) {
                Some(exported) => {
                    // Ambient values provide full polymorphic type inference through the
                    // prelude item system. Always prefer them over exported callable_type,
                    // which only carries the portable ImportValueType representation.
                    if let Some(metadata) = known_import_metadata(module_name, imported_name.text())
                        && matches!(
                            metadata,
                            ImportBindingMetadata::AmbientValue { .. }
                                | ImportBindingMetadata::AmbientType
                        ) {
                            return (ImportBindingResolution::Resolved, metadata, None, None);
                        }
                    // For non-ambient exports, fall back to known_import_metadata when the
                    // export is opaque (e.g. stdlib modules that can't resolve their own
                    // self-imports), OR when the compiler explicitly marks the function as an
                    // IntrinsicValue but the stdlib exports it as a regular Value (e.g.
                    // aivi.text.lower is a thin wrapper around the toLower intrinsic and exports
                    // as Value, but the compiler can lower it as a direct intrinsic call).
                    let export_is_plain_value =
                        matches!(exported.metadata, ImportBindingMetadata::Value { .. });
                    if matches!(exported.metadata, ImportBindingMetadata::OpaqueValue)
                        && let Some(metadata) =
                            known_import_metadata(module_name, imported_name.text())
                        {
                            return (ImportBindingResolution::Resolved, metadata, None, None);
                        }
                    if export_is_plain_value
                        && let Some(metadata) =
                            known_import_metadata(module_name, imported_name.text())
                            && matches!(metadata, ImportBindingMetadata::IntrinsicValue { .. }) {
                                return (ImportBindingResolution::Resolved, metadata, None, None);
                            }
                    (
                        ImportBindingResolution::Resolved,
                        exported.metadata.clone(),
                        exported.callable_type.clone(),
                        exported.deprecation.clone(),
                    )
                }
                // Fall back to compiler-known intrinsics when the stdlib file exists but
                // does not re-export the builtin function (e.g. aivi.stdio.stdoutWrite).
                None => match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None => (
                        ImportBindingResolution::MissingExport,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                },
            },
            ImportModuleResolution::Missing => {
                match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None if is_known_module(module_name) => (
                        ImportBindingResolution::MissingExport,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                    None => (
                        ImportBindingResolution::UnknownModule,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                }
            }
            ImportModuleResolution::Cycle(_) => {
                // For direct self-imports, intrinsics are still accessible by name.
                match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None => (
                        ImportBindingResolution::Cycle,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                }
            }
        }
    }

    fn lower_item_header(
        &mut self,
        decorators: &[syn::Decorator],
        target: ItemKind,
        span: SourceSpan,
    ) -> ItemHeader {
        let lowered_decorators = decorators
            .iter()
            .map(|decorator| self.lower_decorator(decorator, target))
            .collect();
        self.validate_recurrence_wakeup_decorator_set(decorators, target);
        ItemHeader {
            span,
            decorators: lowered_decorators,
        }
    }

    fn lower_decorator(&mut self, decorator: &syn::Decorator, target: ItemKind) -> DecoratorId {
        let name = self.lower_qualified_name(&decorator.name);
        let payload = if is_source_decorator(&name) {
            if target != ItemKind::Signal {
                self.emit_error(
                    decorator.span,
                    "`@source` is only valid on `sig` declarations in Milestone 2",
                    code("invalid-source-target"),
                );
            }
            match &decorator.payload {
                syn::DecoratorPayload::Source(source) => {
                    let source = SourceDecorator {
                        provider: source
                            .provider
                            .as_ref()
                            .map(|provider| self.lower_qualified_name(provider)),
                        arguments: source
                            .arguments
                            .iter()
                            .map(|expr| self.lower_expr(expr))
                            .collect(),
                        options: source
                            .options
                            .as_ref()
                            .map(|record| self.lower_record_expr_as_expr(record)),
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
                syn::DecoratorPayload::Arguments(arguments) => {
                    let source = SourceDecorator {
                        provider: None,
                        arguments: arguments
                            .arguments
                            .iter()
                            .map(|expr| self.lower_expr(expr))
                            .collect(),
                        options: arguments
                            .options
                            .as_ref()
                            .map(|record| self.lower_record_expr_as_expr(record)),
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
                syn::DecoratorPayload::Bare => {
                    let source = SourceDecorator {
                        provider: None,
                        arguments: Vec::new(),
                        options: None,
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
            }
        } else if let Some(kind) = recurrence_wakeup_decorator_kind(&name) {
            if !matches!(
                target,
                ItemKind::Value | ItemKind::Function | ItemKind::Signal
            ) {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` is only valid on `value`, `func`, or `signal` declarations in Milestone 2",
                        path_text(&name)
                    ),
                    code("invalid-recurrence-wakeup-target"),
                );
            }
            self.lower_recurrence_wakeup_decorator_payload(decorator, kind)
        } else if is_test_decorator(&name) {
            if target != ItemKind::Value {
                self.emit_error(
                    decorator.span,
                    "`@test` is only valid on top-level `val` declarations",
                    code("invalid-test-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if !call.arguments.is_empty() || call.options.is_some() {
                self.emit_error(
                    decorator.span,
                    "`@test` does not accept arguments or `with { ... }` options",
                    code("invalid-test-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Test(TestDecorator)
            }
        } else if is_debug_decorator(&name) {
            if !matches!(
                target,
                ItemKind::Value | ItemKind::Function | ItemKind::Signal
            ) {
                self.emit_error(
                    decorator.span,
                    "`@debug` is only valid on top-level `value`, `func`, or `signal` declarations",
                    code("invalid-debug-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if !call.arguments.is_empty() || call.options.is_some() {
                self.emit_error(
                    decorator.span,
                    "`@debug` does not accept arguments or `with { ... }` options",
                    code("invalid-debug-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Debug(DebugDecorator)
            }
        } else if is_deprecated_decorator(&name) {
            if !matches!(
                target,
                ItemKind::Type
                    | ItemKind::Value
                    | ItemKind::Function
                    | ItemKind::Signal
                    | ItemKind::Class
                    | ItemKind::Domain
            ) {
                self.emit_error(
                    decorator.span,
                    "`@deprecated` is only valid on top-level named type, value, function, signal, class, or domain declarations",
                    code("invalid-deprecated-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if call.arguments.len() > 1 {
                self.emit_error(
                    decorator.span,
                    "`@deprecated` accepts at most one positional text message",
                    code("invalid-deprecated-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Deprecated(DeprecatedDecorator {
                    message: call.arguments.first().copied(),
                    options: call.options,
                })
            }
        } else if is_mock_decorator(&name) {
            if target != ItemKind::Value {
                self.emit_error(
                    decorator.span,
                    "`@mock` is only valid on top-level `val` declarations",
                    code("invalid-mock-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            let mock_arguments = if call.options.is_some() {
                None
            } else if call.arguments.len() == 2 {
                Some((call.arguments[0], call.arguments[1]))
            } else if call.arguments.len() == 1 {
                match &self.module.exprs()[call.arguments[0]].kind {
                    ExprKind::Tuple(arguments) if arguments.len() == 2 => {
                        Some((*arguments.first(), *arguments.second()))
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some((target, replacement)) = mock_arguments {
                DecoratorPayload::Mock(MockDecorator {
                    target,
                    replacement,
                })
            } else {
                self.emit_error(
                    decorator.span,
                    "`@mock` must carry exactly two positional references and no `with { ... }` options",
                    code("invalid-mock-decorator"),
                );
                DecoratorPayload::Call(call)
            }
        } else {
            self.emit_error(
                decorator.span,
                format!("unknown decorator `@{}`", path_text(&name)),
                code("unknown-decorator"),
            );
            match &decorator.payload {
                syn::DecoratorPayload::Bare => DecoratorPayload::Bare,
                syn::DecoratorPayload::Arguments(_) | syn::DecoratorPayload::Source(_) => {
                    DecoratorPayload::Call(
                        self.lower_call_like_decorator_payload(&decorator.payload),
                    )
                }
            }
        };

        self.alloc_decorator(Decorator {
            span: decorator.span,
            name,
            payload,
        })
    }

    fn lower_recurrence_wakeup_decorator_payload(
        &mut self,
        decorator: &syn::Decorator,
        kind: RecurrenceWakeupDecoratorKind,
    ) -> DecoratorPayload {
        let call = self.lower_call_like_decorator_payload(&decorator.payload);
        if call.options.is_some() || call.arguments.len() != 1 {
            self.emit_error(
                decorator.span,
                format!(
                    "`@{}` must carry exactly one positional witness expression and no `with {{ ... }}` options",
                    decorator.name.as_dotted()
                ),
                code("invalid-recurrence-wakeup-decorator"),
            );
            return DecoratorPayload::Call(call);
        }
        DecoratorPayload::RecurrenceWakeup(RecurrenceWakeupDecorator {
            kind,
            witness: call.arguments[0],
        })
    }

    fn lower_call_like_decorator_payload(
        &mut self,
        payload: &syn::DecoratorPayload,
    ) -> DecoratorCall {
        match payload {
            syn::DecoratorPayload::Bare => DecoratorCall {
                arguments: Vec::new(),
                options: None,
            },
            syn::DecoratorPayload::Arguments(arguments) => DecoratorCall {
                arguments: arguments
                    .arguments
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect(),
                options: arguments
                    .options
                    .as_ref()
                    .map(|record| self.lower_record_expr_as_expr(record)),
            },
            syn::DecoratorPayload::Source(source) => DecoratorCall {
                arguments: source
                    .arguments
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect(),
                options: source
                    .options
                    .as_ref()
                    .map(|record| self.lower_record_expr_as_expr(record)),
            },
        }
    }

    fn validate_recurrence_wakeup_decorator_set(
        &mut self,
        decorators: &[syn::Decorator],
        target: ItemKind,
    ) {
        let source = decorators
            .iter()
            .find(|decorator| decorator.name.as_dotted() == "source");
        let mut first_recurrence: Option<&syn::Decorator> = None;
        for decorator in decorators {
            if !is_recurrence_wakeup_decorator_name(&decorator.name) {
                continue;
            }
            if let Some(first) = first_recurrence {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` cannot be combined with `@{}`; current non-source recurrence proofs accept at most one wakeup witness per declaration",
                        decorator.name.as_dotted(),
                        first.name.as_dotted()
                    ),
                    code("duplicate-recurrence-wakeup-decorator"),
                );
            } else {
                first_recurrence = Some(decorator);
            }
            if target == ItemKind::Signal && source.is_some() {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` is only valid on non-`@source` recurrence declarations in the current wakeup slice",
                        decorator.name.as_dotted()
                    ),
                    code("invalid-source-recurrence-wakeup"),
                );
            }
        }
    }

    fn validate_source_decorator_shape(&mut self, span: SourceSpan, source: &SourceDecorator) {
        let provider = match source.provider.as_ref() {
            Some(provider) => match SourceProviderRef::from_path(Some(provider)) {
                SourceProviderRef::Builtin(provider_ref) => Some(provider_ref),
                SourceProviderRef::Custom(_) => None,
                SourceProviderRef::InvalidShape(_) => {
                    if !crate::capability_handle_elaboration::is_builtin_source_capability_family_path(provider)
                    {
                        self.emit_error(
                            provider.span(),
                            "source decorators must name a provider variant such as `timer.every`",
                            code("invalid-source-provider"),
                        );
                    }
                    None
                }
                SourceProviderRef::Missing => unreachable!(
                    "classifying an explicit source provider should never yield Missing"
                ),
            },
            None => {
                self.emit_error(
                    span,
                    "source decorators must name a provider variant such as `timer.every`",
                    code("missing-source-provider"),
                );
                None
            }
        };

        let Some(options) = source.options else {
            return;
        };
        let ExprKind::Record(record) = &self.module.exprs()[options].kind else {
            self.emit_error(
                span,
                "`@source ... with` options must lower to a closed record literal",
                code("invalid-source-options"),
            );
            return;
        };

        let mut seen = HashMap::new();
        for field in &record.fields {
            if let Some(previous_span) = seen.insert(field.label.text().to_owned(), field.span) {
                self.diagnostics.push(
                    Diagnostic::error(format!("duplicate source option `{}`", field.label.text()))
                        .with_code(code("duplicate-source-option"))
                        .with_primary_label(field.span, "this option label is repeated")
                        .with_secondary_label(previous_span, "previous source option here"),
                );
            }
            if let Some(provider) = provider {
                let contract = provider.contract();
                if contract.option(field.label.text()).is_none() {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "unknown source option `{}` for `{}`",
                            field.label.text(),
                            provider.key()
                        ))
                        .with_code(code("unknown-source-option"))
                        .with_primary_label(
                            field.span,
                            "this option is not supported for the selected source provider",
                        ),
                    );
                }
            }
        }
    }

    fn lower_function_parameter(&mut self, parameter: &syn::FunctionParam) -> FunctionParameter {
        let name = self.required_name(
            parameter.name.as_ref(),
            parameter.span,
            "function parameter",
        );
        let binding = self.alloc_binding(Binding {
            span: parameter.span,
            name: name.clone(),
            kind: BindingKind::FunctionParameter,
        });
        FunctionParameter {
            span: parameter.span,
            binding,
            annotation: parameter
                .annotation
                .as_ref()
                .map(|annotation| self.lower_type_expr(annotation)),
        }
    }

    fn lower_instance_parameter(&mut self, parameter: &syn::Identifier) -> FunctionParameter {
        let name = self.make_name(&parameter.text, parameter.span);
        let binding = self.alloc_binding(Binding {
            span: parameter.span,
            name,
            kind: BindingKind::FunctionParameter,
        });
        FunctionParameter {
            span: parameter.span,
            binding,
            annotation: None,
        }
    }

    fn lower_type_parameters(&mut self, parameters: &[syn::Identifier]) -> Vec<TypeParameterId> {
        parameters
            .iter()
            .map(|parameter| {
                self.alloc_type_parameter(TypeParameter {
                    span: parameter.span,
                    name: self.make_name(&parameter.text, parameter.span),
                })
            })
            .collect()
    }

    fn lower_expr(&mut self, expr: &syn::Expr) -> ExprId {
        match &expr.kind {
            syn::ExprKind::Group(inner) => self.lower_expr(inner),
            syn::ExprKind::Name(name) => {
                let reference = TermReference::unresolved(
                    self.make_path(&[self.make_name(&name.text, name.span)]),
                );
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Name(reference),
                })
            }
            syn::ExprKind::Integer(integer) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Integer(IntegerLiteral {
                    raw: integer.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Float(float) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Float(FloatLiteral {
                    raw: float.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Decimal(decimal) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Decimal(DecimalLiteral {
                    raw: decimal.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::BigInt(bigint) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::BigInt(BigIntLiteral {
                    raw: bigint.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::SuffixedInteger(literal) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::SuffixedInteger(SuffixedIntegerLiteral {
                    raw: literal.literal.raw.clone().into_boxed_str(),
                    suffix: self.make_name(&literal.suffix.text, literal.suffix.span),
                    resolution: ResolutionState::Unresolved,
                }),
            }),
            syn::ExprKind::Text(text) => {
                let text = self.lower_text_literal(text);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Text(text),
                })
            }
            syn::ExprKind::Regex(regex) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Regex(RegexLiteral {
                    raw: regex.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "tuple expressions require at least two elements",
                            code("short-tuple-expr"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_expr(expr.span),
                            self.placeholder_expr(expr.span),
                            Vec::new(),
                        )
                    }
                };
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Tuple(elements),
                })
            }
            syn::ExprKind::List(elements) => {
                if let [
                    syn::Expr {
                        kind: syn::ExprKind::Range { start, end },
                        ..
                    },
                ] = elements.as_slice()
                {
                    return self.lower_integer_range_expr(expr.span, start, end);
                }
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr(element))
                    .collect();
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::List(elements),
                })
            }
            syn::ExprKind::Map(map) => {
                let map = self.lower_map_expr(map);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Map(map),
                })
            }
            syn::ExprKind::Set(elements) => {
                let mut seen_elements =
                    Vec::<(&syn::Expr, SourceSpan)>::with_capacity(elements.len());
                let mut lowered_elements = Vec::with_capacity(elements.len());
                for element in elements {
                    if let Some((_, previous_span)) = seen_elements
                        .iter()
                        .find(|(previous, _)| surface_exprs_equal(previous, element))
                    {
                        self.diagnostics.push(
                            Diagnostic::warning(
                                "duplicate set element is redundant and will be ignored",
                            )
                            .with_code(code("duplicate-set-element"))
                            .with_primary_label(
                                element.span,
                                "this element duplicates an earlier set entry",
                            )
                            .with_secondary_label(
                                *previous_span,
                                "previous equivalent set element here",
                            )
                            .with_note(
                                "set literals are canonicalized during HIR lowering to one structurally equal entry",
                            ),
                        );
                        continue;
                    }
                    seen_elements.push((element, element.span));
                    lowered_elements.push(self.lower_expr(element));
                }
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Set(lowered_elements),
                })
            }
            syn::ExprKind::Lambda(lambda) => {
                let parameters = lambda
                    .parameters
                    .iter()
                    .map(|parameter| self.lower_function_parameter(parameter))
                    .collect();
                let body = self.lower_expr(&lambda.body);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Lambda(crate::hir::LambdaExpr {
                        parameters,
                        body,
                        surface_form: match lambda.surface_form {
                            syn::cst::LambdaSurfaceForm::Explicit => {
                                crate::hir::LambdaSurfaceForm::Explicit
                            }
                            syn::cst::LambdaSurfaceForm::SubjectShorthand => {
                                crate::hir::LambdaSurfaceForm::SubjectShorthand
                            }
                        },
                    }),
                })
            }
            syn::ExprKind::Record(record) => {
                // Detect record projection: { field: . } or { a.b.c: . }
                // When a field value is SubjectPlaceholder, this is a projection
                // from the ambient subject, not record construction.
                if let Some(proj_field) = record.fields.iter().find(|f| {
                    matches!(
                        f.value.as_ref().map(|v| &v.kind),
                        Some(syn::ExprKind::SubjectPlaceholder)
                    )
                }) {
                    let mut names =
                        vec![self.make_name(&proj_field.label.text, proj_field.label.span)];
                    for seg in &proj_field.label_path {
                        names.push(self.make_name(&seg.text, seg.span));
                    }
                    let path = self.make_path(&names);
                    self.alloc_expr(Expr {
                        span: expr.span,
                        kind: ExprKind::Projection {
                            base: ProjectionBase::Ambient,
                            path,
                        },
                    })
                } else {
                    let record = self.lower_record_expr(record);
                    self.alloc_expr(Expr {
                        span: expr.span,
                        kind: ExprKind::Record(record),
                    })
                }
            }
            syn::ExprKind::SubjectPlaceholder => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::AmbientSubject,
            }),
            syn::ExprKind::AmbientProjection(path) => {
                let path = self.lower_projection_path(path);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        path,
                    },
                })
            }
            syn::ExprKind::Range { start, end } => {
                self.lower_integer_range_expr(expr.span, start, end)
            }
            syn::ExprKind::Projection { base, path } => {
                let base = self.lower_expr(base);
                let path = self.lower_projection_path(path);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        path,
                    },
                })
            }
            syn::ExprKind::Apply { callee, arguments } => {
                let callee = self.lower_expr(callee);
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_expr(argument))
                    .collect::<Vec<_>>();
                let arguments = match crate::NonEmpty::from_vec(arguments) {
                    Ok(arguments) => arguments,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "applications require at least one argument",
                            code("empty-apply-args"),
                        );
                        crate::NonEmpty::new(self.placeholder_expr(expr.span), Vec::new())
                    }
                };
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Apply { callee, arguments },
                })
            }
            syn::ExprKind::Unary {
                operator,
                expr: inner,
            } => {
                let inner = self.lower_expr(inner);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Unary {
                        operator: lower_unary_operator(*operator),
                        expr: inner,
                    },
                })
            }
            syn::ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.lower_expr(left);
                let right = self.lower_expr(right);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Binary {
                        left,
                        operator: lower_binary_operator(*operator),
                        right,
                    },
                })
            }
            syn::ExprKind::ResultBlock(block) => self.lower_result_block_expr(block),
            syn::ExprKind::PatchApply { target, patch } => {
                let target = self.lower_expr(target);
                let patch = self.lower_patch_block(patch);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::PatchApply { target, patch },
                })
            }
            syn::ExprKind::PatchLiteral(patch) => {
                let patch = self.lower_patch_block(patch);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::PatchLiteral(patch),
                })
            }
            syn::ExprKind::Pipe(pipe) => self.lower_pipe_expr(pipe),
            syn::ExprKind::Markup(markup) => {
                let markup = self.lower_markup_node(markup, MarkupPlacement::Renderable);
                let span = markup.span(self);
                let markup = self.renderable_markup(markup, span, "top-level markup");
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Markup(markup),
                })
            }
            syn::ExprKind::OperatorSection(op) => self.lower_operator_section(*op, expr.span),
        }
    }

    fn lower_operator_section(
        &mut self,
        op: syn::BinaryOperator,
        span: aivi_base::SourceSpan,
    ) -> ExprId {
        let ambient_name = match op {
            syn::BinaryOperator::Equals => Some("__aivi_binary_eq"),
            syn::BinaryOperator::NotEquals => Some("__aivi_binary_neq"),
            _ => None,
        };
        if let Some(name) = ambient_name {
            let reference =
                TermReference::unresolved(self.make_path(&[self.make_name(name, span)]));
            self.alloc_expr(Expr {
                span,
                kind: ExprKind::Name(reference),
            })
        } else {
            self.diagnostics.push(
                aivi_base::Diagnostic::error("operator section not supported for this operator")
                    .with_code(code("unsupported-operator-section"))
                    .with_primary_label(
                        span,
                        "no built-in function backing for this operator section",
                    ),
            );
            let reference = TermReference::unresolved(
                self.make_path(&[self.make_name("__aivi_binary_eq", span)]),
            );
            self.alloc_expr(Expr {
                span,
                kind: ExprKind::Name(reference),
            })
        }
    }

    fn lower_patch_block(&mut self, patch: &syn::PatchBlock) -> PatchBlock {
        PatchBlock {
            entries: patch
                .entries
                .iter()
                .map(|entry| PatchEntry {
                    span: entry.span,
                    selector: self.lower_patch_selector(&entry.selector),
                    instruction: self.lower_patch_instruction(&entry.instruction),
                })
                .collect(),
        }
    }

    fn lower_patch_selector(&mut self, selector: &syn::PatchSelector) -> PatchSelector {
        PatchSelector {
            span: selector.span,
            segments: selector
                .segments
                .iter()
                .map(|segment| match segment {
                    syn::PatchSelectorSegment::Named { name, dotted, span } => {
                        PatchSelectorSegment::Named {
                            name: self.make_name(&name.text, name.span),
                            dotted: *dotted,
                            span: *span,
                        }
                    }
                    syn::PatchSelectorSegment::BracketTraverse { span } => {
                        PatchSelectorSegment::BracketTraverse { span: *span }
                    }
                    syn::PatchSelectorSegment::BracketExpr { expr, span } => {
                        PatchSelectorSegment::BracketExpr {
                            expr: self.lower_expr(expr),
                            span: *span,
                        }
                    }
                })
                .collect(),
        }
    }

    fn lower_patch_instruction(&mut self, instruction: &syn::PatchInstruction) -> PatchInstruction {
        PatchInstruction {
            span: instruction.span,
            kind: match &instruction.kind {
                syn::PatchInstructionKind::Replace(expr) => {
                    PatchInstructionKind::Replace(self.lower_expr(expr))
                }
                syn::PatchInstructionKind::Store(expr) => {
                    PatchInstructionKind::Store(self.lower_expr(expr))
                }
                syn::PatchInstructionKind::Remove => PatchInstructionKind::Remove,
            },
        }
    }

    fn lower_integer_range_expr(
        &mut self,
        span: SourceSpan,
        start: &syn::Expr,
        end: &syn::Expr,
    ) -> ExprId {
        let Some(start) = self.parse_compile_time_range_bound(start) else {
            self.emit_error(
                span,
                "range bounds must be plain `Int` literals in this surface revision",
                code("invalid-range-bounds"),
            );
            return self.placeholder_expr(span);
        };
        let Some(end) = self.parse_compile_time_range_bound(end) else {
            self.emit_error(
                span,
                "range bounds must be plain `Int` literals in this surface revision",
                code("invalid-range-bounds"),
            );
            return self.placeholder_expr(span);
        };

        let element_count = start.abs_diff(end).saturating_add(1);
        if element_count > MAX_COMPILE_TIME_RANGE_ELEMENTS {
            self.emit_error(
                span,
                format!(
                    "range expands to {element_count} elements, which exceeds the compile-time limit of {MAX_COMPILE_TIME_RANGE_ELEMENTS}"
                ),
                code("range-too-large"),
            );
            return self.placeholder_expr(span);
        }

        let step = if start <= end { 1 } else { -1 };
        let mut current = start;
        let mut elements = Vec::with_capacity(element_count as usize);
        loop {
            elements.push(self.alloc_expr(Expr {
                span,
                kind: ExprKind::Integer(IntegerLiteral {
                    raw: current.to_string().into_boxed_str(),
                }),
            }));
            if current == end {
                break;
            }
            current += step;
        }

        self.alloc_expr(Expr {
            span,
            kind: ExprKind::List(elements),
        })
    }

    fn parse_compile_time_range_bound(&self, expr: &syn::Expr) -> Option<i64> {
        let syn::ExprKind::Integer(integer) = &expr.kind else {
            return None;
        };
        integer.raw.parse::<i64>().ok()
    }

    fn lower_result_block_expr(&mut self, block: &syn::ResultBlockExpr) -> ExprId {
        let Some(mut current) = self.lower_result_block_tail(block) else {
            self.emit_error(
                block.span,
                "result blocks must produce a final success value",
                code("empty-result-block"),
            );
            return self.placeholder_expr(block.span);
        };
        for binding in block.bindings.iter().rev() {
            let source = self.lower_expr(&binding.expr);
            current = self.lower_result_binding(binding, source, current);
        }
        current
    }

    fn lower_result_block_tail(&mut self, block: &syn::ResultBlockExpr) -> Option<ExprId> {
        let tail = match block.tail.as_deref() {
            Some(expr) => self.lower_expr(expr),
            None => {
                let binding = block.bindings.last()?;
                self.lower_unresolved_name_expr(&binding.name.text, binding.name.span)
            }
        };
        Some(self.lower_constructor_apply_expr("Ok", block.span, vec![tail]))
    }

    fn lower_result_binding(
        &mut self,
        binding: &syn::ResultBinding,
        source: ExprId,
        ok_body: ExprId,
    ) -> ExprId {
        let ok_binding_name = self.make_name(&binding.name.text, binding.name.span);
        let ok_binding = self.alloc_binding(Binding {
            span: binding.name.span,
            name: ok_binding_name.clone(),
            kind: BindingKind::Pattern,
        });
        let ok_argument = self.alloc_pattern(Pattern {
            span: binding.name.span,
            kind: PatternKind::Binding(BindingPattern {
                binding: ok_binding,
                name: ok_binding_name,
            }),
        });
        let ok_pattern = self.alloc_pattern(Pattern {
            span: binding.span,
            kind: PatternKind::Constructor {
                callee: self.make_unresolved_term_reference("Ok", binding.name.span),
                arguments: vec![ok_argument],
            },
        });

        let error_name = format!("__resultBlockErr{}", self.module.bindings().len());
        let error_span = binding.expr.span;
        let error_binding_name = self.make_name(&error_name, error_span);
        let error_binding = self.alloc_binding(Binding {
            span: error_span,
            name: error_binding_name.clone(),
            kind: BindingKind::Pattern,
        });
        let error_argument = self.alloc_pattern(Pattern {
            span: error_span,
            kind: PatternKind::Binding(BindingPattern {
                binding: error_binding,
                name: error_binding_name,
            }),
        });
        let err_pattern = self.alloc_pattern(Pattern {
            span: binding.span,
            kind: PatternKind::Constructor {
                callee: self.make_unresolved_term_reference("Err", binding.expr.span),
                arguments: vec![error_argument],
            },
        });
        let err_value = self.lower_unresolved_name_expr(&error_name, error_span);
        let err_body = self.lower_constructor_apply_expr("Err", binding.expr.span, vec![err_value]);

        let ok_stage = PipeStage {
            span: binding.span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: ok_pattern,
                body: ok_body,
            },
        };
        let err_stage = PipeStage {
            span: binding.span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: err_pattern,
                body: err_body,
            },
        };
        self.alloc_expr(Expr {
            span: binding.span,
            kind: ExprKind::Pipe(PipeExpr {
                head: source,
                stages: crate::NonEmpty::new(ok_stage, vec![err_stage]),
                result_block_desugaring: true,
            }),
        })
    }

    fn lower_constructor_apply_expr(
        &mut self,
        constructor: &str,
        span: SourceSpan,
        arguments: Vec<ExprId>,
    ) -> ExprId {
        let callee = self.lower_unresolved_name_expr(constructor, span);
        let arguments = crate::NonEmpty::from_vec(arguments)
            .expect("result block constructors always receive at least one argument");
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Apply { callee, arguments },
        })
    }

    fn lower_unresolved_name_expr(&mut self, name: &str, span: SourceSpan) -> ExprId {
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(self.make_unresolved_term_reference(name, span)),
        })
    }

    fn make_unresolved_term_reference(&self, name: &str, span: SourceSpan) -> TermReference {
        TermReference::unresolved(self.make_path(&[self.make_name(name, span)]))
    }

    fn lower_text_literal(&mut self, text: &syn::TextLiteral) -> TextLiteral {
        TextLiteral {
            segments: text
                .segments
                .iter()
                .map(|segment| match segment {
                    syn::TextSegment::Text(fragment) => TextSegment::Text(TextFragment {
                        raw: fragment.raw.clone().into_boxed_str(),
                        span: fragment.span,
                    }),
                    syn::TextSegment::Interpolation(interpolation) => {
                        TextSegment::Interpolation(TextInterpolation {
                            span: interpolation.span,
                            expr: self.lower_expr(&interpolation.expr),
                        })
                    }
                })
                .collect(),
        }
    }

    fn lower_pipe_expr(&mut self, pipe: &syn::PipeExpr) -> ExprId {
        self.validate_pipe_stages(&pipe.stages);
        let mut current = pipe.head.as_ref().map(|head| self.lower_expr(head));
        let mut ordinary = Vec::new();
        let mut index = 0;
        while index < pipe.stages.len() {
            match &pipe.stages[index].kind {
                syn::PipeStageKind::Apply { .. } => {
                    self.flush_pipe_segment(&mut current, &mut ordinary, pipe.span);
                    let cluster_expr =
                        self.lower_cluster_segment(current.take(), &pipe.stages, &mut index);
                    current = Some(cluster_expr);
                }
                syn::PipeStageKind::ClusterFinalizer { expr } => {
                    self.emit_error(
                        pipe.stages[index].span,
                        "cluster finalizer appeared without an active `&|>` region",
                        code("orphan-cluster-finalizer"),
                    );
                    ordinary.push(PipeStage {
                        span: pipe.stages[index].span,
                        subject_memo: None,
                        result_memo: None,
                        kind: PipeStageKind::Transform {
                            expr: self.lower_expr(expr),
                        },
                    });
                    index += 1;
                }
                _ => {
                    ordinary.push(self.lower_pipe_stage(&pipe.stages[index]));
                    index += 1;
                }
            }
        }
        self.flush_pipe_segment(&mut current, &mut ordinary, pipe.span);
        current.unwrap_or_else(|| {
            self.emit_error(
                pipe.span,
                "pipe expression is missing a head expression",
                code("missing-pipe-head"),
            );
            self.placeholder_expr(pipe.span)
        })
    }

    fn flush_pipe_segment(
        &mut self,
        current: &mut Option<ExprId>,
        ordinary: &mut Vec<PipeStage>,
        span: SourceSpan,
    ) {
        if ordinary.is_empty() {
            return;
        }
        let head = current.take().unwrap_or_else(|| {
            self.emit_error(
                span,
                "pipe stage sequence is missing a head expression",
                code("missing-pipe-head"),
            );
            self.placeholder_expr(span)
        });
        let mut stages = std::mem::take(ordinary);
        self.normalize_grouped_pipe_memos(&mut stages);
        let stages =
            crate::NonEmpty::from_vec(stages).expect("flush only runs for non-empty stage buffers");
        let expr = self.alloc_expr(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head,
                stages,
                result_block_desugaring: false,
            }),
        });
        *current = Some(expr);
    }

    fn lower_cluster_segment(
        &mut self,
        head: Option<ExprId>,
        stages: &[syn::PipeStage],
        index: &mut usize,
    ) -> ExprId {
        let presentation = if head.is_some() {
            ClusterPresentation::ExpressionHeaded
        } else {
            ClusterPresentation::Leading
        };
        let mut members = head.into_iter().collect::<Vec<_>>();
        let mut cluster_span = members
            .first()
            .and_then(|expr| self.module.exprs().get(*expr))
            .map(|expr| expr.span)
            .unwrap_or(stages[*index].span);
        while *index < stages.len() {
            match &stages[*index].kind {
                syn::PipeStageKind::Apply { expr } => {
                    let lowered = self.lower_expr(expr);
                    cluster_span = cluster_span
                        .join(self.module.exprs()[lowered].span)
                        .unwrap_or(cluster_span);
                    members.push(lowered);
                    *index += 1;
                }
                _ => break,
            }
        }

        let finalizer = if *index < stages.len() {
            if let syn::PipeStageKind::ClusterFinalizer { expr } = &stages[*index].kind {
                let lowered = self.lower_expr(expr);
                cluster_span = cluster_span
                    .join(self.module.exprs()[lowered].span)
                    .unwrap_or(cluster_span);
                *index += 1;
                ClusterFinalizer::Explicit(lowered)
            } else {
                self.emit_error(
                    stages[*index].span,
                    "unfinished `&|>` cluster cannot continue with this pipe stage",
                    code("illegal-unfinished-cluster"),
                );
                ClusterFinalizer::ImplicitTuple
            }
        } else {
            ClusterFinalizer::ImplicitTuple
        };

        if members.len() < 2 {
            self.emit_error(
                cluster_span,
                "`&|>` clusters require at least two members",
                code("short-cluster"),
            );
            members.push(self.placeholder_expr(cluster_span));
        }
        let members =
            AtLeastTwo::from_vec(members).expect("cluster fallback should guarantee two members");
        let cluster = self.alloc_cluster(ApplicativeCluster {
            span: cluster_span,
            presentation,
            members,
            finalizer,
        });
        self.alloc_expr(Expr {
            span: cluster_span,
            kind: ExprKind::Cluster(cluster),
        })
    }

    fn lower_pipe_stage(&mut self, stage: &syn::PipeStage) -> PipeStage {
        let subject_memo = stage
            .subject_memo
            .as_ref()
            .map(|memo| self.lower_pipe_memo_binding(memo, BindingKind::PipeSubjectMemo));
        let result_memo = stage
            .result_memo
            .as_ref()
            .map(|memo| self.lower_pipe_memo_binding(memo, BindingKind::PipeResultMemo));
        let kind = match &stage.kind {
            syn::PipeStageKind::Transform { expr } => PipeStageKind::Transform {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Gate { expr } => PipeStageKind::Gate {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Case(arm) => PipeStageKind::Case {
                pattern: self.lower_pattern(&arm.pattern),
                body: self.lower_expr(&arm.body),
            },
            syn::PipeStageKind::Map { expr } => PipeStageKind::Map {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Apply { expr } => PipeStageKind::Apply {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::ClusterFinalizer { expr } => PipeStageKind::Transform {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::RecurStart { expr } => PipeStageKind::RecurStart {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::RecurStep { expr } => PipeStageKind::RecurStep {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Tap { expr } => PipeStageKind::Tap {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::FanIn { expr } => PipeStageKind::FanIn {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Truthy { expr } => PipeStageKind::Truthy {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Falsy { expr } => PipeStageKind::Falsy {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Validate { expr } => PipeStageKind::Validate {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Previous { expr } => PipeStageKind::Previous {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Accumulate { seed, step } => PipeStageKind::Accumulate {
                seed: self.lower_expr(seed),
                step: self.lower_expr(step),
            },
            syn::PipeStageKind::Diff { expr } => PipeStageKind::Diff {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Delay { duration } => PipeStageKind::Delay {
                duration: self.lower_expr(duration),
            },
            syn::PipeStageKind::Burst { every, count } => PipeStageKind::Burst {
                every: self.lower_expr(every),
                count: self.lower_expr(count),
            },
        };
        PipeStage {
            span: stage.span,
            subject_memo,
            result_memo,
            kind,
        }
    }

    fn lower_pipe_memo_binding(&mut self, memo: &syn::Identifier, kind: BindingKind) -> BindingId {
        self.alloc_binding(Binding {
            span: memo.span,
            name: self.make_name(&memo.text, memo.span),
            kind,
        })
    }

    fn normalize_grouped_pipe_memos(&mut self, stages: &mut [PipeStage]) {
        let mut index = 0usize;
        while index < stages.len() {
            match stages[index].kind {
                PipeStageKind::Case { .. } => {
                    let start = index;
                    while index < stages.len()
                        && matches!(stages[index].kind, PipeStageKind::Case { .. })
                    {
                        index += 1;
                    }
                    self.promote_grouped_pipe_memos(&mut stages[start..index]);
                }
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    if index + 1 < stages.len()
                        && matches!(
                            (&stages[index].kind, &stages[index + 1].kind),
                            (PipeStageKind::Truthy { .. }, PipeStageKind::Falsy { .. })
                                | (PipeStageKind::Falsy { .. }, PipeStageKind::Truthy { .. })
                        )
                    {
                        self.promote_grouped_pipe_memos(&mut stages[index..index + 2]);
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                _ => index += 1,
            }
        }
    }

    fn promote_grouped_pipe_memos(&mut self, stages: &mut [PipeStage]) {
        let Some((first, rest)) = stages.split_first_mut() else {
            return;
        };
        let subject_memo = std::iter::once(first.subject_memo)
            .chain(rest.iter().map(|stage| stage.subject_memo))
            .find_map(|binding| binding);
        let result_memo = std::iter::once(first.result_memo)
            .chain(rest.iter().map(|stage| stage.result_memo))
            .find_map(|binding| binding);
        first.subject_memo = subject_memo;
        first.result_memo = result_memo;
        for stage in rest {
            stage.subject_memo = None;
            stage.result_memo = None;
        }
    }

    fn validate_pipe_stages(&mut self, stages: &[syn::PipeStage]) {
        self.validate_pipe_branch_and_join_stages(stages);
        self.validate_pipe_recurrence_stages(stages);
    }

    fn validate_pipe_branch_and_join_stages(&mut self, stages: &[syn::PipeStage]) {
        let mut index = 0;
        while index < stages.len() {
            match &stages[index].kind {
                syn::PipeStageKind::Truthy { .. } | syn::PipeStageKind::Falsy { .. } => {
                    let run_start = index;
                    let mut truthy = 0usize;
                    let mut falsy = 0usize;
                    while index < stages.len() {
                        match &stages[index].kind {
                            syn::PipeStageKind::Truthy { .. } => {
                                truthy += 1;
                                index += 1;
                            }
                            syn::PipeStageKind::Falsy { .. } => {
                                falsy += 1;
                                index += 1;
                            }
                            _ => break,
                        }
                    }
                    if index - run_start != 2 || truthy != 1 || falsy != 1 {
                        let mut diagnostic = Diagnostic::error(
                            "`T|>` and `F|>` must appear as one adjacent pair within a pipe spine",
                        )
                        .with_code(code("invalid-truthy-falsy-pair"))
                        .with_primary_label(
                            stages[run_start].span,
                            "this truthy/falsy shorthand run is not a single adjacent pair",
                        );
                        for stage in stages[run_start + 1..index].iter().take(2) {
                            diagnostic = diagnostic.with_secondary_label(
                                stage.span,
                                "paired truthy/falsy stage involved here",
                            );
                        }
                        self.diagnostics.push(diagnostic);
                    }
                }
                syn::PipeStageKind::FanIn { .. } => {
                    let mut scan = index;
                    while scan > 0
                        && matches!(&stages[scan - 1].kind, syn::PipeStageKind::Gate { .. })
                    {
                        scan -= 1;
                    }
                    if scan == 0
                        || !matches!(&stages[scan - 1].kind, syn::PipeStageKind::Map { .. })
                    {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "`<|*` must close a `*|>` fan-out body with only `?|>` filters in between",
                            )
                                .with_code(code("orphan-fan-in"))
                                .with_primary_label(
                                    stages[index].span,
                                    "place `<|*` after the `*|>` stage it joins, allowing only `?|>` filters between them",
                                ),
                        );
                    }
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn validate_pipe_recurrence_stages(&mut self, stages: &[syn::PipeStage]) {
        #[derive(Clone, Copy)]
        enum RecurrenceState {
            Outside,
            AwaitingStep { start: usize },
            InSuffix { start: usize },
            AfterSuffix { start: usize },
        }

        let mut state = RecurrenceState::Outside;
        let mut index = 0usize;
        while index < stages.len() {
            match state {
                RecurrenceState::Outside => match &stages[index].kind {
                    syn::PipeStageKind::RecurStart { .. } => {
                        state = RecurrenceState::AwaitingStep { start: index };
                        index += 1;
                    }
                    syn::PipeStageKind::RecurStep { .. } => {
                        self.emit_orphan_recur_step(stages[index].span);
                        index += 1;
                    }
                    _ => index += 1,
                },
                RecurrenceState::AwaitingStep { start } => match &stages[index].kind {
                    syn::PipeStageKind::Gate { .. } => {
                        index += 1;
                    }
                    syn::PipeStageKind::RecurStep { .. } => {
                        state = RecurrenceState::InSuffix { start };
                        index += 1;
                    }
                    _ => {
                        self.emit_unfinished_recurrence(
                            stages[start].span,
                            Some(stages[index].span),
                        );
                        state = RecurrenceState::AfterSuffix { start };
                        index += 1;
                    }
                },
                RecurrenceState::InSuffix { start } => match &stages[index].kind {
                    syn::PipeStageKind::RecurStep { .. } => index += 1,
                    _ => {
                        self.emit_illegal_recurrence_continuation(
                            stages[start].span,
                            stages[index].span,
                            "this stage appears after a recurrent pipe suffix",
                        );
                        state = RecurrenceState::AfterSuffix { start };
                        index += 1;
                    }
                },
                RecurrenceState::AfterSuffix { start } => {
                    if matches!(
                        &stages[index].kind,
                        syn::PipeStageKind::RecurStart { .. }
                            | syn::PipeStageKind::RecurStep { .. }
                    ) {
                        self.emit_illegal_recurrence_continuation(
                            stages[start].span,
                            stages[index].span,
                            "this recurrent stage appears after the recurrent suffix has already ended",
                        );
                    }
                    index += 1;
                }
            }
        }

        if let RecurrenceState::AwaitingStep { start } = state {
            self.emit_unfinished_recurrence(stages[start].span, None);
        }
    }

    fn emit_orphan_recur_step(&mut self, span: SourceSpan) {
        self.diagnostics.push(
            Diagnostic::error("`<|@` must appear inside a recurrent pipe suffix started by `@|>`")
                .with_code(code("orphan-recur-step"))
                .with_primary_label(
                    span,
                    "add `@|>` before this recurrence step or remove `<|@`",
                )
                .with_note(
                    "the current structural recurrence form is a trailing suffix shaped `@|> init (?|> gate)* <|@ step (<|@ step)*`",
                ),
        );
    }

    fn emit_unfinished_recurrence(
        &mut self,
        start_span: SourceSpan,
        continuation_span: Option<SourceSpan>,
    ) {
        let mut diagnostic = Diagnostic::error(
            "`@|>` must be followed by zero or more `?|>` guards and one or more `<|@` stages",
        )
            .with_code(code("unfinished-recurrence"))
            .with_primary_label(
                start_span,
                "this recurrent suffix never receives a recurrence step",
            )
            .with_note(
                "the current structural recurrence form is a trailing suffix shaped `@|> init (?|> gate)* <|@ step (<|@ step)*`",
            );
        if let Some(span) = continuation_span {
            diagnostic = diagnostic.with_secondary_label(
                span,
                "a recurrent suffix may only use `?|>` guards before its first `<|@` step",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn emit_illegal_recurrence_continuation(
        &mut self,
        start_span: SourceSpan,
        span: SourceSpan,
        label: &'static str,
    ) {
        self.diagnostics.push(
            Diagnostic::error(
                "a recurrent pipe suffix may only use `?|>` guards before its first `<|@`, then only `<|@` stages, and must reach pipe end",
            )
            .with_code(code("illegal-recurrence-continuation"))
            .with_primary_label(span, label)
            .with_secondary_label(start_span, "recurrent suffix started here")
            .with_note(
                "keep recurrence as one trailing `@|> ... <|@ ...` suffix so the scheduler-node handoff stays explicit",
            ),
        );
    }

    fn lower_record_expr(&mut self, record: &syn::RecordExpr) -> RecordExpr {
        let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(record.fields.len());
        RecordExpr {
            fields: record
                .fields
                .iter()
                .map(|field| {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate record field `{}`",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let value = field
                        .value
                        .as_ref()
                        .map(|value| self.lower_expr(value))
                        .unwrap_or_else(|| {
                            self.alloc_expr(Expr {
                                span: field.label.span,
                                kind: ExprKind::Name(TermReference::unresolved(self.make_path(&[
                                    self.make_name(&field.label.text, field.label.span),
                                ]))),
                            })
                        });
                    RecordExprField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        value,
                        surface: if field.value.is_some() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    }
                })
                .collect(),
        }
    }

    fn lower_map_expr(&mut self, map: &syn::MapExpr) -> MapExpr {
        let mut seen_keys = Vec::<(&syn::Expr, SourceSpan)>::with_capacity(map.entries.len());
        let mut entries = Vec::with_capacity(map.entries.len());
        for entry in &map.entries {
            // This slice keeps duplicate-key checking purely structural so later typed equality
            // semantics can widen it without rewriting the literal surface.
            if let Some((_, previous_span)) = seen_keys
                .iter()
                .find(|(previous_key, _)| surface_exprs_equal(previous_key, &entry.key))
            {
                self.diagnostics.push(
                    Diagnostic::error("duplicate map key")
                        .with_code(code("duplicate-map-key"))
                        .with_primary_label(entry.key.span, "this map key is repeated")
                        .with_secondary_label(*previous_span, "previous map key here"),
                );
            }
            seen_keys.push((&entry.key, entry.key.span));
            let key = self.lower_expr(&entry.key);
            let value = self.lower_expr(&entry.value);
            entries.push(MapExprEntry {
                span: entry.span,
                key,
                value,
            });
        }
        MapExpr { entries }
    }

    fn lower_record_expr_as_expr(&mut self, record: &syn::RecordExpr) -> ExprId {
        let span = record.span;
        let record = self.lower_record_expr(record);
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Record(record),
        })
    }

    fn lower_pattern(&mut self, pattern: &syn::Pattern) -> PatternId {
        if let syn::PatternKind::Group(inner) = &pattern.kind {
            return self.lower_pattern(inner);
        }
        let kind = match &pattern.kind {
            syn::PatternKind::Wildcard => PatternKind::Wildcard,
            syn::PatternKind::Name(name) => self.lower_name_pattern(name),
            syn::PatternKind::Integer(integer) => PatternKind::Integer(IntegerLiteral {
                raw: integer.raw.clone().into_boxed_str(),
            }),
            syn::PatternKind::Text(text) => {
                if text.has_interpolation() {
                    self.emit_error(
                        text.span,
                        "pattern text literals cannot contain interpolation",
                        code("interpolated-pattern-text"),
                    );
                }
                PatternKind::Text(self.lower_text_literal(text))
            }
            syn::PatternKind::Group(_) => unreachable!("group patterns are handled above"),
            syn::PatternKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_pattern(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            pattern.span,
                            "tuple patterns require at least two elements",
                            code("short-tuple-pattern"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_pattern(pattern.span),
                            self.placeholder_pattern(pattern.span),
                            Vec::new(),
                        )
                    }
                };
                PatternKind::Tuple(elements)
            }
            syn::PatternKind::List { elements, rest } => PatternKind::List {
                elements: elements
                    .iter()
                    .map(|element| self.lower_pattern(element))
                    .collect(),
                rest: rest.as_deref().map(|rest| self.lower_pattern(rest)),
            },
            syn::PatternKind::Record(fields) => {
                let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(fields.len());
                let mut lowered_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record pattern",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record pattern entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }

                    // Build the leaf pattern (innermost binding).
                    let leaf_pat = field
                        .pattern
                        .as_ref()
                        .map(|pattern| self.lower_pattern(pattern))
                        .unwrap_or_else(|| {
                            // Shorthand: bind the leaf segment name.
                            let leaf_ident = field.label_path.last().unwrap_or(&field.label);
                            let binding_name = self.make_name(&leaf_ident.text, leaf_ident.span);
                            let binding = self.alloc_binding(Binding {
                                span: leaf_ident.span,
                                name: binding_name.clone(),
                                kind: BindingKind::Pattern,
                            });
                            self.alloc_pattern(Pattern {
                                span: leaf_ident.span,
                                kind: PatternKind::Binding(BindingPattern {
                                    binding,
                                    name: binding_name,
                                }),
                            })
                        });

                    // Wrap in nested record patterns for dotted paths:
                    // { a.b.c } → { a: { b: { c } } }
                    let pat = if field.label_path.is_empty() {
                        leaf_pat
                    } else {
                        // Build from inside out: start with leaf_pat, wrap in
                        // record patterns for each path segment (reversed).
                        let mut current = leaf_pat;
                        for seg in field.label_path.iter().rev() {
                            let seg_name = self.make_name(&seg.text, seg.span);
                            let inner_field = RecordPatternField {
                                span: seg.span,
                                label: seg_name,
                                pattern: current,
                                surface: RecordFieldSurface::Explicit,
                            };
                            current = self.alloc_pattern(Pattern {
                                span: seg.span,
                                kind: PatternKind::Record(vec![inner_field]),
                            });
                        }
                        current
                    };

                    lowered_fields.push(RecordPatternField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        pattern: pat,
                        surface: if field.pattern.is_some() || !field.label_path.is_empty() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    });
                }
                PatternKind::Record(lowered_fields)
            }
            syn::PatternKind::Apply { callee, arguments } => PatternKind::Constructor {
                callee: self.pattern_callee_from_pattern(callee, pattern.span),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_pattern(argument))
                    .collect(),
            },
        };
        self.alloc_pattern(Pattern {
            span: pattern.span,
            kind,
        })
    }

    fn lower_expr_pattern(&mut self, expr: &syn::Expr) -> PatternId {
        if let syn::ExprKind::Group(inner) = &expr.kind {
            return self.lower_expr_pattern(inner);
        }
        let kind = match &expr.kind {
            syn::ExprKind::Name(name) => self.lower_name_pattern(name),
            syn::ExprKind::Integer(integer) => PatternKind::Integer(IntegerLiteral {
                raw: integer.raw.clone().into_boxed_str(),
            }),
            syn::ExprKind::Text(text) => {
                if text.has_interpolation() {
                    self.emit_error(
                        text.span,
                        "pattern text literals cannot contain interpolation",
                        code("interpolated-pattern-text"),
                    );
                }
                PatternKind::Text(self.lower_text_literal(text))
            }
            syn::ExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr_pattern(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "tuple patterns require at least two elements",
                            code("short-tuple-pattern"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_pattern(expr.span),
                            self.placeholder_pattern(expr.span),
                            Vec::new(),
                        )
                    }
                };
                PatternKind::Tuple(elements)
            }
            syn::ExprKind::List(elements) => PatternKind::List {
                elements: elements
                    .iter()
                    .map(|element| self.lower_expr_pattern(element))
                    .collect(),
                rest: None,
            },
            syn::ExprKind::Record(record) => {
                let mut seen_fields =
                    HashMap::<String, SourceSpan>::with_capacity(record.fields.len());
                let mut lowered_fields = Vec::with_capacity(record.fields.len());
                for field in &record.fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record pattern",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record pattern entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let pat = field
                        .value
                        .as_ref()
                        .map(|value| self.lower_expr_pattern(value))
                        .unwrap_or_else(|| {
                            let binding_name = self.make_name(&field.label.text, field.label.span);
                            let binding = self.alloc_binding(Binding {
                                span: field.label.span,
                                name: binding_name.clone(),
                                kind: BindingKind::Pattern,
                            });
                            self.alloc_pattern(Pattern {
                                span: field.label.span,
                                kind: PatternKind::Binding(BindingPattern {
                                    binding,
                                    name: binding_name,
                                }),
                            })
                        });
                    lowered_fields.push(RecordPatternField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        pattern: pat,
                        surface: if field.value.is_some() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    });
                }
                PatternKind::Record(lowered_fields)
            }
            syn::ExprKind::Apply { callee, arguments } => PatternKind::Constructor {
                callee: self.pattern_callee_from_expr(callee, expr.span),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_expr_pattern(argument))
                    .collect(),
            },
            syn::ExprKind::Group(_) => unreachable!("group expressions are handled above"),
            _ => {
                self.emit_error(
                    expr.span,
                    "markup `pattern={...}` expressions must stay within the pattern subset",
                    code("invalid-pattern-expr"),
                );
                PatternKind::Wildcard
            }
        };
        self.alloc_pattern(Pattern {
            span: expr.span,
            kind,
        })
    }

    fn lower_name_pattern(&mut self, name: &syn::Identifier) -> PatternKind {
        if name.is_uppercase_initial() {
            PatternKind::UnresolvedName(TermReference::unresolved(
                self.make_path(&[self.make_name(&name.text, name.span)]),
            ))
        } else {
            let binding_name = self.make_name(&name.text, name.span);
            let binding = self.alloc_binding(Binding {
                span: name.span,
                name: binding_name.clone(),
                kind: BindingKind::Pattern,
            });
            PatternKind::Binding(BindingPattern {
                binding,
                name: binding_name,
            })
        }
    }

    fn pattern_callee_from_pattern(
        &mut self,
        callee: &syn::Pattern,
        span: SourceSpan,
    ) -> TermReference {
        match &callee.kind {
            syn::PatternKind::Name(name) => {
                TermReference::unresolved(self.make_path(&[self.make_name(&name.text, name.span)]))
            }
            syn::PatternKind::Group(inner) => self.pattern_callee_from_pattern(inner, span),
            _ => {
                self.emit_error(
                    span,
                    "pattern constructor heads must be names",
                    code("invalid-pattern-callee"),
                );
                TermReference::unresolved(self.make_path(&[self.make_name("invalid", span)]))
            }
        }
    }

    fn pattern_callee_from_expr(&mut self, callee: &syn::Expr, span: SourceSpan) -> TermReference {
        match &callee.kind {
            syn::ExprKind::Name(name) => {
                TermReference::unresolved(self.make_path(&[self.make_name(&name.text, name.span)]))
            }
            syn::ExprKind::Group(inner) => self.pattern_callee_from_expr(inner, span),
            _ => {
                self.emit_error(
                    span,
                    "pattern constructor heads must be names",
                    code("invalid-pattern-callee"),
                );
                TermReference::unresolved(self.make_path(&[self.make_name("invalid", span)]))
            }
        }
    }

    fn lower_type_expr(&mut self, ty: &syn::TypeExpr) -> TypeId {
        match &ty.kind {
            syn::TypeExprKind::Group(inner) => self.lower_type_expr(inner),
            syn::TypeExprKind::Name(name) => {
                let path = self.make_path(&[self.make_name(&name.text, name.span)]);
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Name(TypeReference {
                        path,
                        resolution: ResolutionState::Unresolved,
                    }),
                })
            }
            syn::TypeExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_type_expr(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            ty.span,
                            "tuple types require at least two elements",
                            code("short-tuple-type"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_type(ty.span),
                            self.placeholder_type(ty.span),
                            Vec::new(),
                        )
                    }
                };
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Tuple(elements),
                })
            }
            syn::TypeExprKind::Record(fields) => {
                let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(fields.len());
                let mut lowered_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record type",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record type entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let field_ty = field
                        .ty
                        .as_ref()
                        .map(|field_ty| self.lower_type_expr(field_ty))
                        .unwrap_or_else(|| {
                            self.emit_error(
                                field.span,
                                "record type field is missing a type",
                                code("missing-record-field-type"),
                            );
                            self.placeholder_type(field.span)
                        });
                    lowered_fields.push(TypeField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        ty: field_ty,
                    });
                }
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(lowered_fields),
                })
            }
            syn::TypeExprKind::Arrow { parameter, result } => {
                let parameter = self.lower_type_expr(parameter);
                let result = self.lower_type_expr(result);
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow { parameter, result },
                })
            }
            syn::TypeExprKind::Apply { callee, arguments } => {
                if let Some(transform) = self.lower_record_row_transform(ty, callee, arguments) {
                    return transform;
                }
                let callee = self.lower_type_expr(callee);
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_type_expr(argument))
                    .collect::<Vec<_>>();
                let arguments = match crate::NonEmpty::from_vec(arguments) {
                    Ok(arguments) => arguments,
                    Err(_) => {
                        self.emit_error(
                            ty.span,
                            "type application requires at least one argument",
                            code("empty-type-args"),
                        );
                        crate::NonEmpty::new(self.placeholder_type(ty.span), Vec::new())
                    }
                };
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Apply { callee, arguments },
                })
            }
        }
    }

    fn lower_record_row_transform(
        &mut self,
        ty: &syn::TypeExpr,
        callee: &syn::TypeExpr,
        arguments: &[syn::TypeExpr],
    ) -> Option<TypeId> {
        let syn::TypeExprKind::Name(name) = &callee.kind else {
            return None;
        };
        let transform = match name.text.as_str() {
            "Pick" => Some("Pick"),
            "Omit" => Some("Omit"),
            "Optional" => Some("Optional"),
            "Required" => Some("Required"),
            "Defaulted" => Some("Defaulted"),
            "Rename" => Some("Rename"),
            _ => None,
        }?;
        if arguments.len() != 2 {
            self.emit_error(
                ty.span,
                format!("record row transform `{transform}` expects exactly two arguments"),
                code("invalid-record-row-transform"),
            );
            return Some(self.placeholder_type(ty.span));
        }
        let source = self.lower_type_expr(&arguments[1]);
        let transform = match transform {
            "Pick" => RecordRowTransform::Pick(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Pick",
            )),
            "Omit" => RecordRowTransform::Omit(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Omit",
            )),
            "Optional" => RecordRowTransform::Optional(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Optional",
            )),
            "Required" => RecordRowTransform::Required(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Required",
            )),
            "Defaulted" => RecordRowTransform::Defaulted(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Defaulted",
            )),
            "Rename" => {
                RecordRowTransform::Rename(self.lower_record_row_renames(ty.span, &arguments[0]))
            }
            _ => unreachable!("checked transform names should stay exhaustive"),
        };
        Some(self.alloc_type(TypeNode {
            span: ty.span,
            kind: TypeKind::RecordTransform { transform, source },
        }))
    }

    fn lower_record_row_labels(
        &mut self,
        transform_span: SourceSpan,
        labels: &syn::TypeExpr,
        transform_name: &str,
    ) -> Vec<Name> {
        match &labels.kind {
            syn::TypeExprKind::Group(inner) => {
                self.lower_record_row_labels(transform_span, inner, transform_name)
            }
            syn::TypeExprKind::Name(name) => vec![self.make_name(&name.text, name.span)],
            syn::TypeExprKind::Tuple(elements) => elements
                .iter()
                .flat_map(|element| {
                    self.lower_record_row_labels(transform_span, element, transform_name)
                })
                .collect(),
            _ => {
                self.emit_error(
                    labels.span,
                    format!(
                        "record row transform `{transform_name}` expects a tuple of field labels"
                    ),
                    code("invalid-record-row-transform"),
                );
                vec![self.make_name("invalid", transform_span)]
            }
        }
    }

    fn lower_record_row_renames(
        &mut self,
        transform_span: SourceSpan,
        mapping: &syn::TypeExpr,
    ) -> Vec<RecordRowRename> {
        let syn::TypeExprKind::Record(fields) = &mapping.kind else {
            self.emit_error(
                mapping.span,
                "record row transform `Rename` expects a record-shaped mapping",
                code("invalid-record-row-transform"),
            );
            return vec![RecordRowRename {
                span: transform_span,
                from: self.make_name("invalid", transform_span),
                to: self.make_name("invalid", transform_span),
            }];
        };
        let mut renames = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(target) = field.ty.as_ref() else {
                self.emit_error(
                    field.span,
                    "rename mappings must use `old: new` field pairs",
                    code("invalid-record-row-transform"),
                );
                continue;
            };
            let target = match &target.kind {
                syn::TypeExprKind::Group(inner) => inner.as_ref(),
                _ => target,
            };
            let syn::TypeExprKind::Name(target_name) = &target.kind else {
                self.emit_error(
                    target.span,
                    "rename mapping values must be field labels",
                    code("invalid-record-row-transform"),
                );
                continue;
            };
            renames.push(RecordRowRename {
                span: field.span,
                from: self.make_name(&field.label.text, field.label.span),
                to: self.make_name(&target_name.text, target_name.span),
            });
        }
        if renames.is_empty() {
            renames.push(RecordRowRename {
                span: transform_span,
                from: self.make_name("invalid", transform_span),
                to: self.make_name("invalid", transform_span),
            });
        }
        renames
    }

    fn lower_markup_node(
        &mut self,
        node: &syn::MarkupNode,
        placement: MarkupPlacement,
    ) -> LoweredMarkup {
        let tag_name = match node.name.segments.as_slice() {
            [segment] => Some(segment.text.as_str()),
            _ => None,
        };
        match tag_name {
            Some("show") => {
                let control = ControlNode::Show(self.lower_show_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("each") => {
                let control = ControlNode::Each(self.lower_each_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("match") => {
                let control = ControlNode::Match(self.lower_match_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("fragment") => {
                let control = ControlNode::Fragment(self.lower_fragment_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("with") => {
                let control = ControlNode::With(self.lower_with_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("empty") => {
                let control = ControlNode::Empty(self.lower_empty_control(node));
                let control = self.alloc_control(control);
                match placement {
                    MarkupPlacement::EachEmpty => LoweredMarkup::Empty(control),
                    _ => LoweredMarkup::Renderable(self.invalid_branch_control(
                        control,
                        node.span,
                        "`<empty>` is only valid directly under `<each>`",
                    )),
                }
            }
            Some("case") => {
                let control = ControlNode::Case(self.lower_case_control(node));
                let control = self.alloc_control(control);
                match placement {
                    MarkupPlacement::MatchCase => LoweredMarkup::Case(control),
                    _ => LoweredMarkup::Renderable(self.invalid_branch_control(
                        control,
                        node.span,
                        "`<case>` is only valid directly under `<match>`",
                    )),
                }
            }
            _ => LoweredMarkup::Renderable(self.lower_markup_element(node)),
        }
    }

    fn lower_markup_element(&mut self, node: &syn::MarkupNode) -> MarkupNodeId {
        let attributes = node
            .attributes
            .iter()
            .map(|attribute| MarkupAttribute {
                span: attribute.span,
                name: self.make_name(&attribute.name.text, attribute.name.span),
                value: match &attribute.value {
                    Some(syn::MarkupAttributeValue::Text(text)) => {
                        MarkupAttributeValue::Text(self.lower_text_literal(text))
                    }
                    Some(syn::MarkupAttributeValue::Expr(expr)) => {
                        MarkupAttributeValue::Expr(self.lower_expr(expr))
                    }
                    Some(syn::MarkupAttributeValue::Pattern(_)) => {
                        self.emit_error(
                            attribute.span,
                            "only `<case pattern={...}>` accepts pattern-valued markup attributes",
                            code("invalid-markup-pattern-attr"),
                        );
                        MarkupAttributeValue::Expr(self.placeholder_expr(attribute.span))
                    }
                    None => MarkupAttributeValue::ImplicitTrue,
                },
            })
            .collect();
        let children = node
            .children
            .iter()
            .map(|child| {
                let lowered = self.lower_markup_node(child, MarkupPlacement::Renderable);
                self.renderable_markup(lowered, child.span, "ordinary markup element")
            })
            .collect();
        let name_segments = node
            .name
            .segments
            .iter()
            .map(|segment| self.make_name(&segment.text, segment.span))
            .collect::<Vec<_>>();
        let name = self.make_path(&name_segments);
        let close_name = node.close_name.as_ref().map(|close_name| {
            self.make_path(
                &close_name
                    .segments
                    .iter()
                    .map(|segment| self.make_name(&segment.text, segment.span))
                    .collect::<Vec<_>>(),
            )
        });
        self.alloc_markup_node(MarkupNode {
            span: node.span,
            kind: MarkupNodeKind::Element(MarkupElement {
                name,
                attributes,
                children,
                close_name,
                self_closing: node.self_closing,
            }),
        })
    }

    fn lower_show_control(&mut self, node: &syn::MarkupNode) -> ShowControl {
        ShowControl {
            span: node.span,
            when: self.required_markup_expr_attr(node, "when"),
            keep_mounted: self.optional_markup_expr_attr(node, "keepMounted"),
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<show>`",
            ),
        }
    }

    fn lower_each_control(&mut self, node: &syn::MarkupNode) -> EachControl {
        let binding = self.required_markup_binder_attr(node, "as", BindingKind::MarkupEach);
        let mut children = Vec::new();
        let mut empty = None;
        for child in &node.children {
            match self.lower_markup_node(child, MarkupPlacement::EachEmpty) {
                LoweredMarkup::Renderable(node_id) => children.push(node_id),
                LoweredMarkup::Empty(control_id) => {
                    if empty.is_some() {
                        self.emit_error(
                            child.span,
                            "`<each>` may contain at most one `<empty>` branch",
                            code("duplicate-empty-branch"),
                        );
                    } else {
                        empty = Some(control_id);
                    }
                }
                LoweredMarkup::Case(control_id) => {
                    children.push(self.invalid_branch_control(
                        control_id,
                        child.span,
                        "`<case>` is only valid directly under `<match>`",
                    ));
                }
            }
        }
        EachControl {
            span: node.span,
            collection: self.required_markup_expr_attr(node, "of"),
            binding,
            key: self.optional_markup_expr_attr(node, "key"),
            children,
            empty,
        }
    }

    fn lower_match_control(&mut self, node: &syn::MarkupNode) -> MatchControl {
        let mut cases = Vec::new();
        for child in &node.children {
            match self.lower_markup_node(child, MarkupPlacement::MatchCase) {
                LoweredMarkup::Case(control_id) => cases.push(control_id),
                LoweredMarkup::Renderable(_) | LoweredMarkup::Empty(_) => {
                    self.emit_error(
                        child.span,
                        "`<match>` children must be `<case>` branches",
                        code("invalid-match-child"),
                    );
                }
            }
        }
        if cases.is_empty() {
            self.emit_error(
                node.span,
                "`<match>` requires at least one `<case>` branch",
                code("missing-match-case"),
            );
            let wildcard = self.alloc_pattern(Pattern {
                span: node.span,
                kind: PatternKind::Wildcard,
            });
            cases.push(self.alloc_control(ControlNode::Case(CaseControl {
                span: node.span,
                pattern: wildcard,
                children: Vec::new(),
            })));
        }
        MatchControl {
            span: node.span,
            scrutinee: self.required_markup_expr_attr(node, "on"),
            cases: crate::NonEmpty::from_vec(cases)
                .expect("match fallback should provide one case"),
        }
    }

    fn lower_fragment_control(&mut self, node: &syn::MarkupNode) -> FragmentControl {
        FragmentControl {
            span: node.span,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<fragment>`",
            ),
        }
    }

    fn lower_with_control(&mut self, node: &syn::MarkupNode) -> WithControl {
        WithControl {
            span: node.span,
            value: self.required_markup_expr_attr(node, "value"),
            binding: self.required_markup_binder_attr(node, "as", BindingKind::MarkupWith),
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<with>`",
            ),
        }
    }

    fn lower_empty_control(&mut self, node: &syn::MarkupNode) -> EmptyControl {
        EmptyControl {
            span: node.span,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<empty>`",
            ),
        }
    }

    fn lower_case_control(&mut self, node: &syn::MarkupNode) -> CaseControl {
        let pattern = self
            .required_markup_pattern_attr(node, "pattern")
            .unwrap_or_else(|| self.placeholder_pattern(node.span));
        CaseControl {
            span: node.span,
            pattern,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<case>`",
            ),
        }
    }

    fn lower_renderable_children(
        &mut self,
        children: &[syn::MarkupNode],
        placement: MarkupPlacement,
        parent: &str,
    ) -> Vec<MarkupNodeId> {
        children
            .iter()
            .map(|child| {
                let lowered = self.lower_markup_node(child, placement);
                self.renderable_markup(lowered, child.span, parent)
            })
            .collect()
    }

    fn renderable_markup(
        &mut self,
        lowered: LoweredMarkup,
        span: SourceSpan,
        parent: &str,
    ) -> MarkupNodeId {
        match lowered {
            LoweredMarkup::Renderable(id) => id,
            LoweredMarkup::Empty(control_id) => self.invalid_branch_control(
                control_id,
                span,
                format!("`<empty>` cannot render directly under {parent}"),
            ),
            LoweredMarkup::Case(control_id) => self.invalid_branch_control(
                control_id,
                span,
                format!("`<case>` cannot render directly under {parent}"),
            ),
        }
    }

    fn invalid_branch_control(
        &mut self,
        control_id: ControlNodeId,
        span: SourceSpan,
        message: impl Into<String>,
    ) -> MarkupNodeId {
        self.emit_error(span, message, code("misplaced-control-branch"));
        let children = match self
            .module
            .control_nodes()
            .get(control_id)
            .expect("misplaced control branch should exist")
            .clone()
        {
            ControlNode::Empty(node) => node.children,
            ControlNode::Case(node) => node.children,
            _ => Vec::new(),
        };
        let control = self.alloc_control(ControlNode::Fragment(FragmentControl { span, children }));
        self.alloc_markup_node(MarkupNode {
            span,
            kind: MarkupNodeKind::Control(control),
        })
    }

    fn required_markup_expr_attr(&mut self, node: &syn::MarkupNode, name: &str) -> ExprId {
        self.required_markup_attr(node, name)
            .as_ref()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| self.placeholder_expr(node.span))
    }

    fn optional_markup_expr_attr(&mut self, node: &syn::MarkupNode, name: &str) -> Option<ExprId> {
        self.find_markup_attr(node, name)
            .map(|attribute| match &attribute.value {
                Some(syn::MarkupAttributeValue::Expr(expr)) => self.lower_expr(expr),
                Some(syn::MarkupAttributeValue::Text(_)) => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    self.placeholder_expr(attribute.span)
                }
                Some(syn::MarkupAttributeValue::Pattern(_)) => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    self.placeholder_expr(attribute.span)
                }
                None => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    self.placeholder_expr(attribute.span)
                }
            })
    }

    fn required_markup_binder_attr(
        &mut self,
        node: &syn::MarkupNode,
        name: &str,
        kind: BindingKind,
    ) -> BindingId {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return self.alloc_binding(Binding {
                span: node.span,
                name: self.make_name("invalid", node.span),
                kind,
            });
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Expr(expr)) => match &expr.kind {
                syn::ExprKind::Name(identifier) => self.alloc_binding(Binding {
                    span: identifier.span,
                    name: self.make_name(&identifier.text, identifier.span),
                    kind,
                }),
                syn::ExprKind::Group(inner) => match &inner.kind {
                    syn::ExprKind::Name(identifier) => self.alloc_binding(Binding {
                        span: identifier.span,
                        name: self.make_name(&identifier.text, identifier.span),
                        kind,
                    }),
                    _ => {
                        self.emit_error(
                            attribute.span,
                            format!("attribute `{name}` expects a binder name"),
                            code("invalid-binder-attr"),
                        );
                        self.alloc_binding(Binding {
                            span: attribute.span,
                            name: self.make_name("invalid", attribute.span),
                            kind,
                        })
                    }
                },
                _ => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects a binder name"),
                        code("invalid-binder-attr"),
                    );
                    self.alloc_binding(Binding {
                        span: attribute.span,
                        name: self.make_name("invalid", attribute.span),
                        kind,
                    })
                }
            },
            Some(syn::MarkupAttributeValue::Pattern(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a binder name"),
                    code("invalid-binder-attr"),
                );
                self.alloc_binding(Binding {
                    span: attribute.span,
                    name: self.make_name("invalid", attribute.span),
                    kind,
                })
            }
            _ => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a binder name"),
                    code("invalid-binder-attr"),
                );
                self.alloc_binding(Binding {
                    span: attribute.span,
                    name: self.make_name("invalid", attribute.span),
                    kind,
                })
            }
        }
    }

    fn required_markup_attr<'node>(
        &mut self,
        node: &'node syn::MarkupNode,
        name: &str,
    ) -> Option<&'node syn::Expr> {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return None;
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Expr(expr)) => Some(expr),
            Some(syn::MarkupAttributeValue::Text(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
            Some(syn::MarkupAttributeValue::Pattern(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
            None => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
        }
    }

    fn required_markup_pattern_attr(
        &mut self,
        node: &syn::MarkupNode,
        name: &str,
    ) -> Option<PatternId> {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return None;
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Pattern(pattern)) => Some(self.lower_pattern(pattern)),
            Some(syn::MarkupAttributeValue::Expr(expr)) => Some(self.lower_expr_pattern(expr)),
            Some(syn::MarkupAttributeValue::Text(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a pattern"),
                    code("invalid-control-attr"),
                );
                None
            }
            None => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a pattern"),
                    code("invalid-control-attr"),
                );
                None
            }
        }
    }

    fn find_markup_attr<'node>(
        &self,
        node: &'node syn::MarkupNode,
        name: &str,
    ) -> Option<&'node syn::MarkupAttribute> {
        node.attributes
            .iter()
            .find(|attribute| attribute.name.text == name)
    }

    fn lower_projection_path(&mut self, path: &syn::ProjectionPath) -> NamePath {
        let names = path
            .fields
            .iter()
            .map(|field| self.make_name(&field.text, field.span))
            .collect::<Vec<_>>();
        self.make_path(&names)
    }

    fn lower_qualified_name(&mut self, name: &syn::QualifiedName) -> NamePath {
        let segments = name
            .segments
            .iter()
            .map(|segment| self.make_name(&segment.text, segment.span))
            .collect::<Vec<_>>();
        self.make_path(&segments)
    }

    fn required_name(
        &mut self,
        name: Option<&syn::Identifier>,
        span: SourceSpan,
        subject: &str,
    ) -> Name {
        match name {
            Some(name) => self.make_name(&name.text, name.span),
            None => {
                self.emit_error(
                    span,
                    format!("{subject} is missing a name"),
                    code("missing-name"),
                );
                self.make_name("invalid", span)
            }
        }
    }

    fn build_namespaces(&mut self) -> Namespaces {
        let mut namespaces = Namespaces::default();
        let root_ids = self.module.root_items().to_vec();
        for item_id in root_ids {
            let item = self.module.items()[item_id].clone();
            match item {
                Item::Type(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            insert_named(
                                &mut namespaces.term_items,
                                variant.name.text(),
                                item_id,
                                variant.span,
                                &mut self.diagnostics,
                                code("duplicate-constructor-name"),
                                "constructor",
                            );
                        }
                    }
                }
                Item::Value(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Function(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Signal(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Class(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        insert_site(
                            &mut namespaces.class_terms,
                            member.name.text(),
                            crate::hir::ClassMemberResolution {
                                class: item_id,
                                member_index,
                            },
                            member.span,
                        );
                    }
                }
                Item::Domain(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        if member.kind == DomainMemberKind::Literal
                            && member.name.text().chars().count() >= 2
                        {
                            insert_site(
                                &mut namespaces.literal_suffixes,
                                member.name.text(),
                                LiteralSuffixResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                            continue;
                        }
                        if member.kind == DomainMemberKind::Method {
                            insert_site(
                                &mut namespaces.domain_terms,
                                member.name.text(),
                                DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                        }
                    }
                }
                Item::SourceProviderContract(item) => {
                    if let Some(key) = item.provider.custom_key() {
                        insert_named(
                            &mut namespaces.provider_contracts,
                            key,
                            item_id,
                            item.header.span,
                            &mut self.diagnostics,
                            code("duplicate-source-provider-contract"),
                            "provider contract",
                        );
                    }
                }
                Item::Use(item) => self.register_use_item(&item, &mut namespaces),
                Item::Hoist(item) => self.register_hoist_item(&item, &mut namespaces),
                Item::Export(_) | Item::Instance(_) => {}
            }
        }
        for item_id in self.module.ambient_items().to_vec() {
            let item = self.module.items()[item_id].clone();
            match item {
                Item::Type(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            insert_site(
                                &mut namespaces.ambient_term_items,
                                variant.name.text(),
                                item_id,
                                variant.span,
                            );
                        }
                    }
                }
                Item::Class(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        insert_site(
                            &mut namespaces.ambient_class_terms,
                            member.name.text(),
                            crate::hir::ClassMemberResolution {
                                class: item_id,
                                member_index,
                            },
                            member.span,
                        );
                    }
                }
                Item::Value(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Function(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Signal(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Domain(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        if member.kind == DomainMemberKind::Literal
                            && member.name.text().chars().count() >= 2
                        {
                            insert_site(
                                &mut namespaces.ambient_literal_suffixes,
                                member.name.text(),
                                LiteralSuffixResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                            continue;
                        }
                        if member.kind == DomainMemberKind::Method {
                            insert_site(
                                &mut namespaces.domain_terms,
                                member.name.text(),
                                DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                        }
                    }
                }
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => {}
            }
        }
        self.register_workspace_hoists(&mut namespaces);
        namespaces
    }

    fn register_use_item(&mut self, item: &UseItem, namespaces: &mut Namespaces) {
        let module_name = path_text(&item.module);
        for import_id in item.imports.iter() {
            let import = self.module.imports()[*import_id].clone();
            match import.resolution {
                ImportBindingResolution::Resolved => {}
                ImportBindingResolution::UnknownModule => {
                    self.diagnostics.push(
                        Diagnostic::error(format!("unknown import module `{module_name}`"))
                            .with_code(code("unknown-import-module"))
                            .with_primary_label(
                                import.span,
                                "this workspace does not contain the imported module",
                            )
                            .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
                ImportBindingResolution::MissingExport => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "module `{module_name}` does not export `{}`",
                            import.imported_name.text()
                        ))
                        .with_code(code("unknown-imported-name"))
                        .with_primary_label(
                            import.span,
                            "this imported name is not exported by the target module",
                        )
                        .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
                ImportBindingResolution::Cycle => continue,
            }

            match import.metadata.clone() {
                ImportBindingMetadata::Value { .. }
                | ImportBindingMetadata::IntrinsicValue { .. }
                | ImportBindingMetadata::OpaqueValue
                | ImportBindingMetadata::AmbientValue { .. }
                | ImportBindingMetadata::BuiltinTerm(_) => insert_site(
                    &mut namespaces.term_imports,
                    import.local_name.text(),
                    *import_id,
                    import.span,
                ),
                ImportBindingMetadata::TypeConstructor { .. }
                | ImportBindingMetadata::BuiltinType(_)
                | ImportBindingMetadata::AmbientType => insert_site(
                    &mut namespaces.type_imports,
                    import.local_name.text(),
                    *import_id,
                    import.span,
                ),
                ImportBindingMetadata::Domain {
                    literal_suffixes, ..
                } => {
                    insert_site(
                        &mut namespaces.type_imports,
                        import.local_name.text(),
                        *import_id,
                        import.span,
                    );
                    if !literal_suffixes.is_empty() {
                        self.register_imported_domain_literal_suffixes(
                            &import.local_name,
                            import.span,
                            &literal_suffixes,
                            &mut namespaces.literal_suffixes,
                        );
                    }
                }
                ImportBindingMetadata::Bundle(_) | ImportBindingMetadata::InstanceMember { .. } => {
                }
                ImportBindingMetadata::Unknown => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "import `{}` from `{module_name}` resolved without compiler-known metadata",
                            import.imported_name.text()
                        ))
                            .with_code(code("invalid-import-resolution"))
                            .with_primary_label(
                                import.span,
                                "resolved imports must carry explicit metadata before name registration",
                            )
                            .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
            }
        }
    }

    fn register_hoist_item(&mut self, _item: &HoistItem, _namespaces: &mut Namespaces) {
        // Self-hoist declarations are propagated by the workspace scanner in aivi-query.
        // Nothing to do here — the current module's own names are already in scope.
    }

    /// Register all workspace-level hoist items from other modules.
    ///
    /// Called after `build_namespaces()` processes local `Item::Hoist`
    /// declarations.  Module paths already registered locally are skipped to
    /// prevent double-registration.  Missing modules are silently ignored (no
    /// diagnostic) since they may belong to a different workspace scope.
    fn register_workspace_hoists(&mut self, namespaces: &mut Namespaces) {
        use crate::resolver::RawHoistItem;
        use aivi_base::Span;

        let workspace_hoists: Vec<RawHoistItem> = self.resolver.workspace_hoist_items();
        if workspace_hoists.is_empty() {
            return;
        }

        // Skip hoists that would resolve to the current module — a module
        // declaring `hoist libs.foo` inside `libs/foo.aivi` itself is a
        // self-hoist used as a "publish myself" pattern. Injecting the module's
        // own names into its own namespace would create a compilation cycle, and
        // is unnecessary since the names are already in scope.
        let self_path = self.resolver.current_module_path();

        let synthetic_span = SourceSpan::new(self.module.file(), Span::from(0..0));

        let debug_hoist = std::env::var("AIVI_DEBUG_HOIST").is_ok();
        if debug_hoist {
            eprintln!(
                "[hoist] module={:?} found {} workspace hoists",
                self_path,
                workspace_hoists.len()
            );
        }
        for raw in workspace_hoists {
            let module_key = raw.module_path.join(".");
            if Some(&module_key) == self_path.as_ref() {
                if debug_hoist {
                    eprintln!("[hoist]   skip self: {module_key}");
                }
                continue; // self-hoist: skip for this module, still propagates to others
            }
            if namespaces.hoisted_module_paths.contains(&module_key) {
                if debug_hoist {
                    eprintln!("[hoist]   skip dup: {module_key}");
                }
                continue;
            }
            namespaces.hoisted_module_paths.insert(module_key.clone());

            let module_segments: Vec<&str> = raw.module_path.iter().map(String::as_str).collect();
            let module_resolution = self.resolver.resolve_for_hoist(&module_segments);
            let crate::resolver::ImportModuleResolution::Resolved(ref exports) = module_resolution
            else {
                if debug_hoist {
                    eprintln!("[hoist]   MISS: {module_key} (not resolved)");
                }
                continue; // silent — workspace hoists may reference optional modules
            };

            if debug_hoist {
                let names: Vec<&str> = exports.names.iter().map(|n| n.name.as_str()).collect();
                eprintln!(
                    "[hoist]   OK: {module_key} -> {} exports: {:?}",
                    names.len(),
                    &names[..names.len().min(10)]
                );
            }
            self.register_hoist_exports(
                &module_key,
                exports,
                &raw.kind_filters,
                &raw.hiding,
                synthetic_span,
                namespaces,
            );
        }
    }

    /// Core of hoist registration: given a resolved module's `ExportedNames`,
    /// apply kind filters and hiding, then insert synthetic imports into the
    /// hoisted namespace maps.  Shared by both local `Item::Hoist` and
    /// workspace hoist propagation.
    fn register_hoist_exports(
        &mut self,
        module_name: &str,
        exports: &crate::exports::ExportedNames,
        kind_filters: &[HoistKindFilter],
        hiding: &[impl AsRef<str>],
        span: SourceSpan,
        namespaces: &mut Namespaces,
    ) {
        use crate::exports::ExportedNameKind;
        use crate::resolver::ImportModuleResolution;

        let wrapped = ImportModuleResolution::Resolved(exports.clone());

        for exported in exports.names.iter() {
            if !kind_filters.is_empty() {
                let kind_matches = kind_filters.iter().any(|f| matches!(
                    (f, &exported.kind),
                    (HoistKindFilter::Func, ExportedNameKind::Function)
                    | (HoistKindFilter::Value, ExportedNameKind::Value)
                    | (HoistKindFilter::Signal, ExportedNameKind::Signal)
                    | (HoistKindFilter::Type, ExportedNameKind::Type)
                    | (HoistKindFilter::Domain, ExportedNameKind::Domain)
                    | (HoistKindFilter::Class, ExportedNameKind::Class)
                ));
                if !kind_matches {
                    continue;
                }
            }

            if hiding.iter().any(|h| h.as_ref() == exported.name) {
                continue;
            }

            let imported_name = self.make_name(&exported.name, span);
            let (resolution, metadata, callable_type, deprecation) =
                self.resolve_import_binding(module_name, &imported_name, &wrapped);

            if !matches!(resolution, ImportBindingResolution::Resolved) {
                continue;
            }

            let import_id = self.alloc_import(ImportBinding {
                span,
                source_module: Some(module_name.into()),
                imported_name: imported_name.clone(),
                local_name: imported_name.clone(),
                resolution,
                metadata: metadata.clone(),
                callable_type,
                deprecation,
            });

            match &metadata {
                ImportBindingMetadata::TypeConstructor { .. }
                | ImportBindingMetadata::BuiltinType(_)
                | ImportBindingMetadata::AmbientType => insert_site(
                    &mut namespaces.hoisted_type_imports,
                    imported_name.text(),
                    import_id,
                    span,
                ),
                ImportBindingMetadata::Domain {
                    literal_suffixes, ..
                } => {
                    let suffixes = literal_suffixes.clone();
                    insert_site(
                        &mut namespaces.hoisted_type_imports,
                        imported_name.text(),
                        import_id,
                        span,
                    );
                    if !suffixes.is_empty() {
                        self.register_imported_domain_literal_suffixes(
                            &imported_name,
                            span,
                            &suffixes,
                            &mut namespaces.literal_suffixes,
                        );
                    }
                }
                ImportBindingMetadata::Bundle(_) | ImportBindingMetadata::InstanceMember { .. } => {
                }
                _ => insert_site(
                    &mut namespaces.hoisted_term_imports,
                    imported_name.text(),
                    import_id,
                    span,
                ),
            }
        }

        // Auto-register instance members for class dispatch.
        for instance_decl in &exports.instances {
            for member in &instance_decl.members {
                let synthetic_name = format!(
                    "__instance_{}_{}_{}_{}",
                    instance_decl.class_name,
                    member.name,
                    instance_decl.subject,
                    import_value_type_label(&member.ty),
                );
                let name = self.make_name(&synthetic_name, span);
                self.alloc_import(ImportBinding {
                    span,
                    source_module: Some(module_name.into()),
                    imported_name: self.make_name(&member.name, span),
                    local_name: name,
                    resolution: ImportBindingResolution::Resolved,
                    metadata: ImportBindingMetadata::InstanceMember {
                        class_name: instance_decl.class_name.clone(),
                        member_name: member.name.clone(),
                        subject: instance_decl.subject.clone(),
                        ty: member.ty.clone(),
                    },
                    callable_type: None,
                    deprecation: None,
                });
            }
        }
    }

    /// Synthesise a minimal `Item::Domain` stub in the current module so that
    /// `LiteralSuffixResolution.domain` has a valid `ItemId` to point at.
    /// The stub is allocated but NOT appended to `root_items`, so it is invisible
    /// to name resolution and exports — it exists only to satisfy look-up code
    /// that accesses `module.items()[resolution.domain]` at the type-checking
    /// and validation layer.
    fn register_imported_domain_literal_suffixes(
        &mut self,
        domain_name: &Name,
        span: SourceSpan,
        literal_suffixes: &[ImportedDomainLiteralSuffix],
        target: &mut HashMap<String, Vec<NamedSite<LiteralSuffixResolution>>>,
    ) {
        // Allocate a placeholder carrier type — resolved to Unit so validation
        // does not report spurious "unresolved-name" errors on synthetic stubs.
        let stub_path = self.make_path(&[self.make_name("Unit", span)]);
        let stub_type_kind = || {
            TypeKind::Name(TypeReference {
                path: stub_path.clone(),
                resolution: ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Unit)),
            })
        };

        let carrier = self.alloc_type(TypeNode {
            span,
            kind: stub_type_kind(),
        });

        // Build stub members — one per literal suffix, in member_index order.
        // We need indices stable across the allocated slice, so compute the
        // max member_index first and pre-fill with placeholder members at
        // non-suffix positions.
        let max_index = literal_suffixes
            .iter()
            .map(|s| s.member_index)
            .max()
            .unwrap_or(0);
        let mut members: Vec<Option<DomainMember>> = vec![None; max_index + 1];
        for suffix in literal_suffixes {
            let annotation = self.alloc_type(TypeNode {
                span,
                kind: stub_type_kind(),
            });
            members[suffix.member_index] = Some(DomainMember {
                span,
                kind: DomainMemberKind::Literal,
                name: self.make_name(&suffix.name, span),
                annotation,
                parameters: Vec::new(),
                body: None,
            });
        }
        // Fill gaps with placeholder members so member indices are consistent.
        let members: Vec<DomainMember> = members
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                m.unwrap_or_else(|| {
                    let annotation = self.alloc_type(TypeNode {
                        span,
                        kind: stub_type_kind(),
                    });
                    DomainMember {
                        span,
                        kind: DomainMemberKind::Literal,
                        name: self.make_name(&format!("__stub_{i}"), span),
                        annotation,
                        parameters: Vec::new(),
                        body: None,
                    }
                })
            })
            .collect();

        let domain_stub = Item::Domain(DomainItem {
            header: ItemHeader {
                span,
                decorators: Vec::new(),
            },
            name: domain_name.clone(),
            parameters: Vec::new(),
            carrier,
            members: members.clone(),
        });

        let domain_item_id = self.module.alloc_item(domain_stub).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR item arena (imported domain stub)");
            std::process::exit(1);
        });

        // Register each literal suffix entry.
        for (member_index, member) in members.iter().enumerate() {
            if member.kind != DomainMemberKind::Literal {
                continue;
            }
            if member.name.text().starts_with("__stub_") {
                continue;
            }
            insert_site(
                target,
                member.name.text(),
                LiteralSuffixResolution {
                    domain: domain_item_id,
                    member_index,
                },
                span,
            );
        }
    }

    fn resolve_module(&mut self, namespaces: &Namespaces) {
        for item_id in self.module.root_items().to_vec() {
            self.resolve_item(item_id, namespaces, false);
        }
        for item_id in self.module.ambient_items().to_vec() {
            self.resolve_item(item_id, namespaces, true);
        }
    }

    fn hoist_lambdas(&mut self) {
        for item_id in self.module.root_items().to_vec() {
            self.hoist_lambdas_in_item(item_id);
        }
        for item_id in self.module.ambient_items().to_vec() {
            self.hoist_lambdas_in_item(item_id);
        }
    }

    fn hoist_lambdas_in_item(&mut self, item_id: ItemId) {
        let item = self.module.items()[item_id].clone();
        let hoisted = match item {
            Item::Type(item) => {
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &LambdaOwnerContext::default());
                }
                Item::Type(item)
            }
            Item::Value(mut item) => {
                let owner = LambdaOwnerContext::default();
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &owner);
                }
                item.body = self.hoist_expr(item.body, &owner);
                Item::Value(item)
            }
            Item::Function(mut item) => {
                let owner = LambdaOwnerContext {
                    type_parameters: item.type_parameters.clone(),
                    context: item.context.clone(),
                    owner_parameters: item
                        .parameters
                        .iter()
                        .map(|p| p.binding)
                        .collect(),
                    owner_annotation: item.annotation,
                };
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &owner);
                }
                item.body = self.hoist_expr(item.body, &owner);
                Item::Function(item)
            }
            Item::Signal(mut item) => {
                let owner = LambdaOwnerContext::default();
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &owner);
                }
                if let Some(body) = item.body {
                    item.body = Some(self.hoist_expr(body, &owner));
                }
                for update in &mut item.reactive_updates {
                    update.guard = self.hoist_expr(update.guard, &owner);
                    update.body = self.hoist_expr(update.body, &owner);
                }
                Item::Signal(item)
            }
            Item::Class(item) => {
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &LambdaOwnerContext::default());
                }
                Item::Class(item)
            }
            Item::Domain(mut item) => {
                let owner = LambdaOwnerContext {
                    type_parameters: item.parameters.clone(),
                    context: Vec::new(),
                    ..Default::default()
                };
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &owner);
                }
                for member in &mut item.members {
                    if let Some(body) = member.body {
                        member.body = Some(self.hoist_expr(body, &owner));
                    }
                }
                Item::Domain(item)
            }
            Item::SourceProviderContract(item) => {
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &LambdaOwnerContext::default());
                }
                Item::SourceProviderContract(item)
            }
            Item::Instance(mut item) => {
                let owner = LambdaOwnerContext {
                    type_parameters: item.type_parameters.clone(),
                    context: item.context.clone(),
                    ..Default::default()
                };
                for decorator_id in &item.header.decorators {
                    self.hoist_lambda_decorator(*decorator_id, &owner);
                }
                for member in &mut item.members {
                    member.body = self.hoist_expr(member.body, &owner);
                }
                Item::Instance(item)
            }
            Item::Use(item) => Item::Use(item),
            Item::Export(item) => Item::Export(item),
            Item::Hoist(item) => Item::Hoist(item),
        };
        *self
            .module
            .arenas
            .items
            .get_mut(item_id)
            .expect("item id should remain valid during lambda hoisting") = hoisted;
    }

    fn hoist_lambda_decorator(&mut self, decorator_id: DecoratorId, owner: &LambdaOwnerContext) {
        let decorator = self.module.decorators()[decorator_id].clone();
        let payload = match decorator.payload {
            DecoratorPayload::Bare => DecoratorPayload::Bare,
            DecoratorPayload::Call(mut call) => {
                call.arguments = call
                    .arguments
                    .into_iter()
                    .map(|argument| self.hoist_expr(argument, owner))
                    .collect();
                call.options = call.options.map(|options| self.hoist_expr(options, owner));
                DecoratorPayload::Call(call)
            }
            DecoratorPayload::RecurrenceWakeup(mut wakeup) => {
                wakeup.witness = self.hoist_expr(wakeup.witness, owner);
                DecoratorPayload::RecurrenceWakeup(wakeup)
            }
            DecoratorPayload::Source(mut source) => {
                source.arguments = source
                    .arguments
                    .into_iter()
                    .map(|argument| self.hoist_expr(argument, owner))
                    .collect();
                source.options = source.options.map(|options| self.hoist_expr(options, owner));
                DecoratorPayload::Source(source)
            }
            DecoratorPayload::Test(test) => DecoratorPayload::Test(test),
            DecoratorPayload::Debug(debug) => DecoratorPayload::Debug(debug),
            DecoratorPayload::Deprecated(mut deprecated) => {
                deprecated.message = deprecated
                    .message
                    .map(|message| self.hoist_expr(message, owner));
                deprecated.options = deprecated
                    .options
                    .map(|options| self.hoist_expr(options, owner));
                DecoratorPayload::Deprecated(deprecated)
            }
            DecoratorPayload::Mock(mut mock) => {
                mock.target = self.hoist_expr(mock.target, owner);
                mock.replacement = self.hoist_expr(mock.replacement, owner);
                DecoratorPayload::Mock(mock)
            }
        };
        *self
            .module
            .arenas
            .decorators
            .get_mut(decorator_id)
            .expect("decorator id should remain valid during lambda hoisting") = Decorator {
            span: decorator.span,
            name: decorator.name,
            payload,
        };
    }

    fn hoist_expr(&mut self, expr_id: ExprId, owner: &LambdaOwnerContext) -> ExprId {
        let expr = self.module.exprs()[expr_id].clone();
        let kind = match expr.kind {
            ExprKind::Name(_)
            | ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::AmbientSubject
            | ExprKind::Regex(_) => return expr_id,
            ExprKind::Text(mut text) => {
                self.hoist_text_literal(&mut text, owner);
                ExprKind::Text(text)
            }
            ExprKind::Tuple(elements) => {
                let lowered = elements
                    .into_vec()
                    .into_iter()
                    .map(|element| self.hoist_expr(element, owner))
                    .collect::<Vec<_>>();
                ExprKind::Tuple(AtLeastTwo::from_vec(lowered).expect("tuple arity should stay valid"))
            }
            ExprKind::List(elements) => ExprKind::List(
                elements
                    .into_iter()
                    .map(|element| self.hoist_expr(element, owner))
                    .collect(),
            ),
            ExprKind::Map(mut map) => {
                for entry in &mut map.entries {
                    entry.key = self.hoist_expr(entry.key, owner);
                    entry.value = self.hoist_expr(entry.value, owner);
                }
                ExprKind::Map(map)
            }
            ExprKind::Set(elements) => ExprKind::Set(
                elements
                    .into_iter()
                    .map(|element| self.hoist_expr(element, owner))
                    .collect(),
            ),
            ExprKind::Lambda(mut lambda) => {
                lambda.body = self.hoist_expr(lambda.body, owner);
                return self.hoist_lambda_expr(expr.span, &lambda, owner, None);
            }
            ExprKind::Record(mut record) => {
                for field in &mut record.fields {
                    field.value = self.hoist_expr(field.value, owner);
                }
                ExprKind::Record(record)
            }
            ExprKind::Projection { base, path } => ExprKind::Projection {
                base: match base {
                    ProjectionBase::Ambient => ProjectionBase::Ambient,
                    ProjectionBase::Expr(base) => ProjectionBase::Expr(self.hoist_expr(base, owner)),
                },
                path,
            },
            ExprKind::Apply { callee, arguments } => {
                let callee = self.hoist_expr(callee, owner);
                // Before hoisting arguments, try to resolve the callee's type
                // annotation so that lambda arguments can derive their parameter
                // types from the callee's expected argument types.
                let callee_annotation = self.callee_type_annotation(callee);
                let arguments = arguments
                    .into_vec()
                    .into_iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        let arg_expr = &self.module.exprs()[argument];
                        if matches!(arg_expr.kind, ExprKind::Lambda(_)) {
                            let expected =
                                callee_annotation.and_then(|ty| self.arrow_at_position(ty, index));
                            self.hoist_expr_with_lambda_hint(argument, owner, expected)
                        } else {
                            self.hoist_expr(argument, owner)
                        }
                    })
                    .collect::<Vec<_>>();
                ExprKind::Apply {
                    callee,
                    arguments: NonEmpty::from_vec(arguments)
                        .expect("applications should keep at least one argument"),
                }
            }
            ExprKind::Unary { operator, expr } => ExprKind::Unary {
                operator,
                expr: self.hoist_expr(expr, owner),
            },
            ExprKind::Binary {
                left,
                operator,
                right,
            } => ExprKind::Binary {
                left: self.hoist_expr(left, owner),
                operator,
                right: self.hoist_expr(right, owner),
            },
            ExprKind::PatchApply { target, mut patch } => {
                let target = self.hoist_expr(target, owner);
                self.hoist_patch_block(&mut patch, owner);
                ExprKind::PatchApply { target, patch }
            }
            ExprKind::PatchLiteral(mut patch) => {
                self.hoist_patch_block(&mut patch, owner);
                ExprKind::PatchLiteral(patch)
            }
            ExprKind::Pipe(mut pipe) => {
                pipe.head = self.hoist_expr(pipe.head, owner);
                let stages = pipe
                    .stages
                    .into_vec()
                    .into_iter()
                    .map(|stage| self.hoist_pipe_stage(stage, owner))
                    .collect::<Vec<_>>();
                pipe.stages =
                    NonEmpty::from_vec(stages).expect("pipes should keep at least one stage");
                ExprKind::Pipe(pipe)
            }
            ExprKind::Cluster(cluster_id) => {
                self.hoist_cluster(cluster_id, owner);
                ExprKind::Cluster(cluster_id)
            }
            ExprKind::Markup(node_id) => {
                self.hoist_markup_node(node_id, owner);
                ExprKind::Markup(node_id)
            }
        };
        *self
            .module
            .arenas
            .exprs
            .get_mut(expr_id)
            .expect("expr id should remain valid during lambda hoisting") = Expr {
            span: expr.span,
            kind,
        };
        expr_id
    }

    /// Like `hoist_expr` but when the top-level expression is a lambda, pass
    /// `lambda_hint` as the expected type so the hoisted function gets an annotation.
    fn hoist_expr_with_lambda_hint(
        &mut self,
        expr_id: ExprId,
        owner: &LambdaOwnerContext,
        lambda_hint: Option<TypeId>,
    ) -> ExprId {
        let expr = self.module.exprs()[expr_id].clone();
        if let ExprKind::Lambda(mut lambda) = expr.kind {
            lambda.body = self.hoist_expr(lambda.body, owner);
            return self.hoist_lambda_expr(expr.span, &lambda, owner, lambda_hint);
        }
        self.hoist_expr(expr_id, owner)
    }

    fn hoist_text_literal(&mut self, text: &mut TextLiteral, owner: &LambdaOwnerContext) {
        for segment in &mut text.segments {
            if let TextSegment::Interpolation(interpolation) = segment {
                interpolation.expr = self.hoist_expr(interpolation.expr, owner);
            }
        }
    }

    fn hoist_patch_block(&mut self, patch: &mut PatchBlock, owner: &LambdaOwnerContext) {
        for entry in &mut patch.entries {
            for segment in &mut entry.selector.segments {
                if let PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                    *expr = self.hoist_expr(*expr, owner);
                }
            }
            match &mut entry.instruction.kind {
                PatchInstructionKind::Replace(expr) | PatchInstructionKind::Store(expr) => {
                    *expr = self.hoist_expr(*expr, owner);
                }
                PatchInstructionKind::Remove => {}
            }
        }
    }

    fn hoist_pipe_stage(&mut self, mut stage: PipeStage, owner: &LambdaOwnerContext) -> PipeStage {
        stage.kind = match stage.kind {
            PipeStageKind::Transform { expr } => PipeStageKind::Transform {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Gate { expr } => PipeStageKind::Gate {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Case { pattern, body } => {
                self.hoist_pattern(pattern, owner);
                PipeStageKind::Case {
                    pattern,
                    body: self.hoist_expr(body, owner),
                }
            }
            PipeStageKind::Map { expr } => PipeStageKind::Map {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Apply { expr } => PipeStageKind::Apply {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Tap { expr } => PipeStageKind::Tap {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::FanIn { expr } => PipeStageKind::FanIn {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Truthy { expr } => PipeStageKind::Truthy {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Falsy { expr } => PipeStageKind::Falsy {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::RecurStart { expr } => PipeStageKind::RecurStart {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::RecurStep { expr } => PipeStageKind::RecurStep {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Validate { expr } => PipeStageKind::Validate {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Previous { expr } => PipeStageKind::Previous {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Accumulate { seed, step } => PipeStageKind::Accumulate {
                seed: self.hoist_expr(seed, owner),
                step: self.hoist_expr(step, owner),
            },
            PipeStageKind::Diff { expr } => PipeStageKind::Diff {
                expr: self.hoist_expr(expr, owner),
            },
            PipeStageKind::Delay { duration } => PipeStageKind::Delay {
                duration: self.hoist_expr(duration, owner),
            },
            PipeStageKind::Burst { every, count } => PipeStageKind::Burst {
                every: self.hoist_expr(every, owner),
                count: self.hoist_expr(count, owner),
            },
        };
        stage
    }

    fn hoist_cluster(&mut self, cluster_id: crate::ClusterId, owner: &LambdaOwnerContext) {
        let mut cluster = self.module.clusters()[cluster_id].clone();
        let members = cluster
            .members
            .into_vec()
            .into_iter()
            .map(|member| self.hoist_expr(member, owner))
            .collect::<Vec<_>>();
        cluster.members =
            AtLeastTwo::from_vec(members).expect("clusters should keep at least two members");
        cluster.finalizer = match cluster.finalizer {
            ClusterFinalizer::Explicit(expr) => {
                ClusterFinalizer::Explicit(self.hoist_expr(expr, owner))
            }
            ClusterFinalizer::ImplicitTuple => ClusterFinalizer::ImplicitTuple,
        };
        *self
            .module
            .arenas
            .clusters
            .get_mut(cluster_id)
            .expect("cluster id should remain valid during lambda hoisting") = cluster;
    }

    fn hoist_markup_node(&mut self, node_id: MarkupNodeId, owner: &LambdaOwnerContext) {
        let node = self.module.markup_nodes()[node_id].clone();
        let kind = match node.kind {
            MarkupNodeKind::Element(mut element) => {
                for attribute in &mut element.attributes {
                    match &mut attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            *expr = self.hoist_expr(*expr, owner);
                        }
                        MarkupAttributeValue::Text(text) => self.hoist_text_literal(text, owner),
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in &element.children {
                    self.hoist_markup_node(*child, owner);
                }
                MarkupNodeKind::Element(element)
            }
            MarkupNodeKind::Control(control_id) => {
                self.hoist_control_node(control_id, owner);
                MarkupNodeKind::Control(control_id)
            }
        };
        *self
            .module
            .arenas
            .markup_nodes
            .get_mut(node_id)
            .expect("markup node id should remain valid during lambda hoisting") = MarkupNode {
            span: node.span,
            kind,
        };
    }

    fn hoist_control_node(&mut self, control_id: ControlNodeId, owner: &LambdaOwnerContext) {
        let control = self.module.control_nodes()[control_id].clone();
        let lowered = match control {
            ControlNode::Show(mut node) => {
                node.when = self.hoist_expr(node.when, owner);
                if let Some(expr) = node.keep_mounted {
                    node.keep_mounted = Some(self.hoist_expr(expr, owner));
                }
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                ControlNode::Show(node)
            }
            ControlNode::Each(mut node) => {
                node.collection = self.hoist_expr(node.collection, owner);
                if let Some(key) = node.key {
                    node.key = Some(self.hoist_expr(key, owner));
                }
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                if let Some(empty) = node.empty {
                    self.hoist_control_node(empty, owner);
                }
                ControlNode::Each(node)
            }
            ControlNode::Match(mut node) => {
                node.scrutinee = self.hoist_expr(node.scrutinee, owner);
                for case in node.cases.iter() {
                    self.hoist_control_node(*case, owner);
                }
                ControlNode::Match(node)
            }
            ControlNode::Empty(node) => {
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                ControlNode::Empty(node)
            }
            ControlNode::Case(node) => {
                self.hoist_pattern(node.pattern, owner);
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                ControlNode::Case(node)
            }
            ControlNode::Fragment(node) => {
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                ControlNode::Fragment(node)
            }
            ControlNode::With(mut node) => {
                node.value = self.hoist_expr(node.value, owner);
                for child in &node.children {
                    self.hoist_markup_node(*child, owner);
                }
                ControlNode::With(node)
            }
        };
        *self
            .module
            .arenas
            .control_nodes
            .get_mut(control_id)
            .expect("control node id should remain valid during lambda hoisting") = lowered;
    }

    fn hoist_pattern(&mut self, pattern_id: PatternId, owner: &LambdaOwnerContext) {
        let pattern = self.module.patterns()[pattern_id].clone();
        let kind = match pattern.kind {
            PatternKind::Wildcard
            | PatternKind::Binding(_)
            | PatternKind::Integer(_)
            | PatternKind::UnresolvedName(_) => return,
            PatternKind::Text(mut text) => {
                self.hoist_text_literal(&mut text, owner);
                PatternKind::Text(text)
            }
            PatternKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.hoist_pattern(*element, owner);
                }
                PatternKind::Tuple(elements)
            }
            PatternKind::List { elements, rest } => {
                for element in &elements {
                    self.hoist_pattern(*element, owner);
                }
                if let Some(rest) = rest {
                    self.hoist_pattern(rest, owner);
                }
                PatternKind::List { elements, rest }
            }
            PatternKind::Record(fields) => {
                for field in &fields {
                    self.hoist_pattern(field.pattern, owner);
                }
                PatternKind::Record(fields)
            }
            PatternKind::Constructor { callee, arguments } => {
                for argument in &arguments {
                    self.hoist_pattern(*argument, owner);
                }
                PatternKind::Constructor { callee, arguments }
            }
        };
        *self
            .module
            .arenas
            .patterns
            .get_mut(pattern_id)
            .expect("pattern id should remain valid during lambda hoisting") = Pattern {
            span: pattern.span,
            kind,
        };
    }

    fn hoist_lambda_expr(
        &mut self,
        span: SourceSpan,
        lambda: &crate::hir::LambdaExpr,
        owner: &LambdaOwnerContext,
        // Expected type for the entire lambda (e.g. `Int -> Bool`),
        // derived from the callee's annotation at the call site.
        callee_hint: Option<TypeId>,
    ) -> ExprId {
        if matches!(
            lambda.surface_form,
            crate::hir::LambdaSurfaceForm::SubjectShorthand
        )
            && let Some(parameter) = lambda.parameters.first() {
                self.rewrite_subject_shorthand_expr(lambda.body, parameter.binding, false);
            }

        let lambda_bindings = lambda
            .parameters
            .iter()
            .map(|parameter| parameter.binding)
            .collect::<Vec<_>>();
        let captures = self.collect_lambda_captures(lambda.body, &lambda_bindings);

        let capture_parameters = captures
            .iter()
            .map(|binding| {
                let mut param = self.synthetic_capture_parameter(*binding);
                // Propagate the type annotation from the enclosing function's
                // arrow signature so the typechecker can resolve operators (e.g.
                // `==`) inside the hoisted lambda body.
                if let Some(ty) = self.owner_annotation_for_capture(*binding, owner) {
                    param.annotation = Some(ty);
                }
                param
            })
            .collect::<Vec<_>>();
        let capture_map = captures
            .iter()
            .zip(capture_parameters.iter())
            .map(|(outer, parameter)| (*outer, parameter.binding))
            .collect::<HashMap<_, _>>();
        if !capture_map.is_empty() {
            self.rewrite_captured_bindings_expr(lambda.body, &capture_map);
        }

        let mut parameters = capture_parameters;
        // Propagate parameter types from the callee's expected argument type
        // so the typechecker can resolve operators inside the hoisted body.
        // Skip polymorphic (type-parameter-containing) types — they would
        // conflict with the concrete types inferred later.
        let mut lambda_params = lambda.parameters.clone();
        if let Some(hint) = callee_hint {
            for (index, param) in lambda_params.iter_mut().enumerate() {
                if param.annotation.is_none()
                    && let Some(ty) = self.arrow_at_position(hint, index)
                        && !self.type_contains_type_params(ty) {
                            param.annotation = Some(ty);
                        }
            }
        }
        parameters.extend(lambda_params);

        let item_name = self.synthetic_lambda_name(span);
        let item_id = self.push_root_item(Item::Function(FunctionItem {
            header: ItemHeader {
                span,
                decorators: Vec::new(),
            },
            name: item_name.clone(),
            type_parameters: owner.type_parameters.clone(),
            context: owner.context.clone(),
            parameters,
            annotation: None,
            body: lambda.body,
        }));

        let callee = self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(self.resolved_item_reference(item_name.text(), span, item_id)),
        });
        if captures.is_empty() {
            callee
        } else {
            let arguments = captures
                .into_iter()
                .map(|binding| self.synthetic_local_expr(binding, span))
                .collect::<Vec<_>>();
            self.alloc_expr(Expr {
                span,
                kind: ExprKind::Apply {
                    callee,
                    arguments: NonEmpty::from_vec(arguments)
                        .expect("captured lambda application should keep arguments"),
                },
            })
        }
    }

    fn synthetic_lambda_name(&mut self, span: SourceSpan) -> Name {
        let name = format!("__aivi_lambda_{}", self.next_lambda_id);
        self.next_lambda_id += 1;
        Name::new(name, span).expect("synthetic lambda names should be valid")
    }

    fn push_root_item(&mut self, item: Item) -> ItemId {
        self.module.push_item(item).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR item arena");
            std::process::exit(1);
        })
    }

    fn synthetic_capture_parameter(&mut self, outer_binding: BindingId) -> FunctionParameter {
        let outer = self.module.bindings()[outer_binding].clone();
        let binding = self.alloc_binding(Binding {
            span: outer.span,
            name: outer.name.clone(),
            kind: BindingKind::FunctionParameter,
        });
        FunctionParameter {
            span: outer.span,
            binding,
            annotation: None,
        }
    }

    /// Extract the type of a captured binding from the enclosing function's
    /// arrow annotation. Walks the `Arrow { parameter, result }` chain to
    /// the position that matches the capture's binding in the owner's
    /// parameter list.
    fn owner_annotation_for_capture(
        &self,
        capture: BindingId,
        owner: &LambdaOwnerContext,
    ) -> Option<TypeId> {
        let annotation = owner.owner_annotation?;
        let position = owner
            .owner_parameters
            .iter()
            .position(|b| *b == capture)?;
        self.arrow_at_position(annotation, position)
    }

    /// Walk an Arrow type chain and return the parameter type at `position`.
    /// Position 0 is the first arrow's `parameter`, position 1 is the next
    /// arrow's `parameter` (from the first arrow's `result`), etc.
    fn arrow_at_position(&self, mut ty: TypeId, position: usize) -> Option<TypeId> {
        for _ in 0..position {
            match &self.module.types()[ty].kind {
                TypeKind::Arrow { result, .. } => ty = *result,
                _ => return None,
            }
        }
        match &self.module.types()[ty].kind {
            TypeKind::Arrow { parameter, .. } => Some(*parameter),
            _ => None,
        }
    }

    /// Check whether a HIR type tree rooted at `ty` references any type
    /// parameters. Used to avoid propagating polymorphic annotations from
    /// callee signatures to hoisted lambda parameters.
    fn type_contains_type_params(&self, ty: TypeId) -> bool {
        match &self.module.types()[ty].kind {
            TypeKind::Name(reference) => matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            ),
            TypeKind::Arrow { parameter, result } => {
                self.type_contains_type_params(*parameter)
                    || self.type_contains_type_params(*result)
            }
            TypeKind::Tuple(members) => members
                .iter()
                .any(|m| self.type_contains_type_params(*m)),
            TypeKind::Record(fields) => fields
                .iter()
                .any(|f| self.type_contains_type_params(f.ty)),
            TypeKind::Apply { callee, arguments } => {
                self.type_contains_type_params(*callee)
                    || arguments
                        .iter()
                        .any(|a| self.type_contains_type_params(*a))
            }
            TypeKind::RecordTransform { source, .. } => self.type_contains_type_params(*source),
        }
    }

    /// Try to resolve the type annotation of a callee expression. When the
    /// callee is a resolved item reference to a function (possibly imported),
    /// returns the function's type annotation (the full Arrow chain). For
    /// stdlib imports that resolve to `Import { target }`, follows the target
    /// chain to find the underlying function annotation.
    fn callee_type_annotation(&self, callee: ExprId) -> Option<TypeId> {
        let expr = &self.module.exprs()[callee];
        let ExprKind::Name(reference) = &expr.kind else {
            return None;
        };
        let ResolutionState::Resolved(resolution) = &reference.resolution else {
            return None;
        };
        match resolution {
            TermResolution::Item(item_id) => self.item_type_annotation(*item_id),
            _ => None,
        }
    }

    /// Resolve a same-module item's type annotation.
    fn item_type_annotation(&self, item_id: ItemId) -> Option<TypeId> {
        match &self.module.items()[item_id] {
            Item::Function(function) => function.annotation,
            Item::Value(value) => value.annotation,
            _ => None,
        }
    }

    fn resolved_item_reference(
        &self,
        text: &str,
        span: SourceSpan,
        item_id: ItemId,
    ) -> TermReference {
        let name = Name::new(text, span).expect("synthetic item reference names should be valid");
        let path = NamePath::from_vec(vec![name]).expect("single-segment item path should be valid");
        TermReference::resolved(path, TermResolution::Item(item_id))
    }

    fn synthetic_local_expr(&mut self, binding: BindingId, span: SourceSpan) -> ExprId {
        let binding_name = self.module.bindings()[binding].name.text().to_owned();
        let name = Name::new(binding_name, span).expect("synthetic local names should be valid");
        let path =
            NamePath::from_vec(vec![name]).expect("single-segment local path should be valid");
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(path, TermResolution::Local(binding))),
        })
    }

    fn collect_lambda_captures(&self, body: ExprId, lambda_bindings: &[BindingId]) -> Vec<BindingId> {
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        let scope = LambdaScopeStack::with_bindings(lambda_bindings.iter().copied());
        self.collect_captures_expr(body, &scope, &mut seen, &mut ordered);
        ordered
    }

    fn collect_captures_expr(
        &self,
        expr_id: ExprId,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        let expr = self.module.exprs()[expr_id].clone();
        match expr.kind {
            ExprKind::Name(reference) => {
                if let ResolutionState::Resolved(TermResolution::Local(binding)) = reference.resolution
                    && !scope.contains(binding) && seen.insert(binding) {
                        ordered.push(binding);
                    }
            }
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::AmbientSubject
            | ExprKind::Regex(_) => {}
            ExprKind::Text(text) => {
                for segment in text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        self.collect_captures_expr(interpolation.expr, scope, seen, ordered);
                    }
                }
            }
            ExprKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.collect_captures_expr(*element, scope, seen, ordered);
                }
            }
            ExprKind::List(elements) | ExprKind::Set(elements) => {
                for element in elements {
                    self.collect_captures_expr(element, scope, seen, ordered);
                }
            }
            ExprKind::Map(map) => {
                for entry in map.entries {
                    self.collect_captures_expr(entry.key, scope, seen, ordered);
                    self.collect_captures_expr(entry.value, scope, seen, ordered);
                }
            }
            ExprKind::Lambda(_) => {}
            ExprKind::Record(record) => {
                for field in record.fields {
                    self.collect_captures_expr(field.value, scope, seen, ordered);
                }
            }
            ExprKind::Projection { base, .. } => {
                if let ProjectionBase::Expr(base) = base {
                    self.collect_captures_expr(base, scope, seen, ordered);
                }
            }
            ExprKind::Apply { callee, arguments } => {
                self.collect_captures_expr(callee, scope, seen, ordered);
                for argument in arguments.iter() {
                    self.collect_captures_expr(*argument, scope, seen, ordered);
                }
            }
            ExprKind::Unary { expr, .. } => {
                self.collect_captures_expr(expr, scope, seen, ordered);
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_captures_expr(left, scope, seen, ordered);
                self.collect_captures_expr(right, scope, seen, ordered);
            }
            ExprKind::PatchApply { target, patch } => {
                self.collect_captures_expr(target, scope, seen, ordered);
                self.collect_captures_patch(&patch, scope, seen, ordered);
            }
            ExprKind::PatchLiteral(patch) => {
                self.collect_captures_patch(&patch, scope, seen, ordered);
            }
            ExprKind::Pipe(pipe) => self.collect_captures_pipe(&pipe, scope, seen, ordered),
            ExprKind::Cluster(cluster_id) => {
                let cluster = self.module.clusters()[cluster_id].clone();
                for member in cluster.members.iter() {
                    self.collect_captures_expr(*member, scope, seen, ordered);
                }
                if let ClusterFinalizer::Explicit(expr) = cluster.finalizer {
                    self.collect_captures_expr(expr, scope, seen, ordered);
                }
            }
            ExprKind::Markup(node_id) => {
                self.collect_captures_markup(node_id, scope, seen, ordered);
            }
        }
    }

    fn collect_captures_patch(
        &self,
        patch: &PatchBlock,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        for entry in &patch.entries {
            for segment in &entry.selector.segments {
                if let PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                    self.collect_captures_expr(*expr, scope, seen, ordered);
                }
            }
            match entry.instruction.kind {
                PatchInstructionKind::Replace(expr) | PatchInstructionKind::Store(expr) => {
                    self.collect_captures_expr(expr, scope, seen, ordered);
                }
                PatchInstructionKind::Remove => {}
            }
        }
    }

    fn collect_captures_pipe(
        &self,
        pipe: &PipeExpr,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        self.collect_captures_expr(pipe.head, scope, seen, ordered);
        let stages = pipe.stages.iter().collect::<Vec<_>>();
        let mut pipe_scope = scope.clone();
        let mut index = 0usize;
        while index < stages.len() {
            let stage = stages[index];
            match &stage.kind {
                PipeStageKind::Case { .. } => {
                    let mut case_scope = pipe_scope.clone();
                    if let Some(binding) = stage.subject_memo {
                        case_scope.push([binding]);
                    }
                    while index < stages.len() {
                        let PipeStageKind::Case { pattern, body } = &stages[index].kind else {
                            break;
                        };
                        self.collect_captures_pattern(*pattern, &case_scope, seen, ordered);
                        let mut branch_scope = case_scope.clone();
                        branch_scope.push(self.pattern_bindings(*pattern));
                        self.collect_captures_expr(*body, &branch_scope, seen, ordered);
                        index += 1;
                    }
                }
                PipeStageKind::Transform { expr }
                | PipeStageKind::Gate { expr }
                | PipeStageKind::Map { expr }
                | PipeStageKind::Apply { expr }
                | PipeStageKind::Tap { expr }
                | PipeStageKind::FanIn { expr }
                | PipeStageKind::Truthy { expr }
                | PipeStageKind::Falsy { expr }
                | PipeStageKind::RecurStart { expr }
                | PipeStageKind::RecurStep { expr }
                | PipeStageKind::Validate { expr }
                | PipeStageKind::Previous { expr }
                | PipeStageKind::Diff { expr }
                | PipeStageKind::Delay { duration: expr } => {
                    let mut stage_scope = pipe_scope.clone();
                    if let Some(binding) = stage.subject_memo {
                        stage_scope.push([binding]);
                    }
                    self.collect_captures_expr(*expr, &stage_scope, seen, ordered);
                    index += 1;
                }
                PipeStageKind::Accumulate { seed, step } => {
                    let mut stage_scope = pipe_scope.clone();
                    if let Some(binding) = stage.subject_memo {
                        stage_scope.push([binding]);
                    }
                    self.collect_captures_expr(*seed, &stage_scope, seen, ordered);
                    self.collect_captures_expr(*step, &stage_scope, seen, ordered);
                    index += 1;
                }
                PipeStageKind::Burst { every, count } => {
                    let mut stage_scope = pipe_scope.clone();
                    if let Some(binding) = stage.subject_memo {
                        stage_scope.push([binding]);
                    }
                    self.collect_captures_expr(*every, &stage_scope, seen, ordered);
                    self.collect_captures_expr(*count, &stage_scope, seen, ordered);
                    index += 1;
                }
            }
            if let Some(binding) = stage.subject_memo {
                pipe_scope.push([binding]);
            }
            if let Some(binding) = stage.result_memo {
                pipe_scope.push([binding]);
            }
        }
    }

    fn collect_captures_markup(
        &self,
        node_id: MarkupNodeId,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        let node = self.module.markup_nodes()[node_id].clone();
        match node.kind {
            MarkupNodeKind::Element(element) => {
                for attribute in &element.attributes {
                    match &attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            self.collect_captures_expr(*expr, scope, seen, ordered);
                        }
                        MarkupAttributeValue::Text(text) => {
                            for segment in &text.segments {
                                if let TextSegment::Interpolation(interpolation) = segment {
                                    self.collect_captures_expr(
                                        interpolation.expr,
                                        scope,
                                        seen,
                                        ordered,
                                    );
                                }
                            }
                        }
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in element.children {
                    self.collect_captures_markup(child, scope, seen, ordered);
                }
            }
            MarkupNodeKind::Control(control_id) => {
                self.collect_captures_control(control_id, scope, seen, ordered);
            }
        }
    }

    fn collect_captures_control(
        &self,
        control_id: ControlNodeId,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        let control = self.module.control_nodes()[control_id].clone();
        match control {
            ControlNode::Show(node) => {
                self.collect_captures_expr(node.when, scope, seen, ordered);
                if let Some(expr) = node.keep_mounted {
                    self.collect_captures_expr(expr, scope, seen, ordered);
                }
                for child in node.children {
                    self.collect_captures_markup(child, scope, seen, ordered);
                }
            }
            ControlNode::Each(node) => {
                self.collect_captures_expr(node.collection, scope, seen, ordered);
                let mut child_scope = scope.clone();
                child_scope.push([node.binding]);
                if let Some(key) = node.key {
                    self.collect_captures_expr(key, &child_scope, seen, ordered);
                }
                for child in node.children {
                    self.collect_captures_markup(child, &child_scope, seen, ordered);
                }
                if let Some(empty) = node.empty {
                    self.collect_captures_control(empty, scope, seen, ordered);
                }
            }
            ControlNode::Match(node) => {
                self.collect_captures_expr(node.scrutinee, scope, seen, ordered);
                for case in node.cases.iter() {
                    self.collect_captures_control(*case, scope, seen, ordered);
                }
            }
            ControlNode::Empty(node) => {
                for child in node.children {
                    self.collect_captures_markup(child, scope, seen, ordered);
                }
            }
            ControlNode::Fragment(node) => {
                for child in node.children {
                    self.collect_captures_markup(child, scope, seen, ordered);
                }
            }
            ControlNode::Case(node) => {
                self.collect_captures_pattern(node.pattern, scope, seen, ordered);
                let mut child_scope = scope.clone();
                child_scope.push(self.pattern_bindings(node.pattern));
                for child in node.children {
                    self.collect_captures_markup(child, &child_scope, seen, ordered);
                }
            }
            ControlNode::With(node) => {
                self.collect_captures_expr(node.value, scope, seen, ordered);
                let mut child_scope = scope.clone();
                child_scope.push([node.binding]);
                for child in node.children {
                    self.collect_captures_markup(child, &child_scope, seen, ordered);
                }
            }
        }
    }

    fn collect_captures_pattern(
        &self,
        pattern_id: PatternId,
        scope: &LambdaScopeStack,
        seen: &mut HashSet<BindingId>,
        ordered: &mut Vec<BindingId>,
    ) {
        let pattern = self.module.patterns()[pattern_id].clone();
        match pattern.kind {
            PatternKind::Text(text) => {
                for segment in text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        self.collect_captures_expr(interpolation.expr, scope, seen, ordered);
                    }
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.collect_captures_pattern(*element, scope, seen, ordered);
                }
            }
            PatternKind::List { elements, rest } => {
                for element in elements {
                    self.collect_captures_pattern(element, scope, seen, ordered);
                }
                if let Some(rest) = rest {
                    self.collect_captures_pattern(rest, scope, seen, ordered);
                }
            }
            PatternKind::Record(fields) => {
                for field in fields {
                    self.collect_captures_pattern(field.pattern, scope, seen, ordered);
                }
            }
            PatternKind::Constructor { arguments, .. } => {
                for argument in arguments {
                    self.collect_captures_pattern(argument, scope, seen, ordered);
                }
            }
            PatternKind::Wildcard
            | PatternKind::Binding(_)
            | PatternKind::Integer(_)
            | PatternKind::UnresolvedName(_) => {}
        }
    }

    fn pattern_bindings(&self, pattern_id: PatternId) -> Vec<BindingId> {
        let pattern = self.module.patterns()[pattern_id].clone();
        match pattern.kind {
            PatternKind::Binding(binding) => vec![binding.binding],
            PatternKind::Tuple(elements) => elements
                .iter()
                .flat_map(|element| self.pattern_bindings(*element))
                .collect(),
            PatternKind::List { elements, rest } => {
                let mut bindings = elements
                    .into_iter()
                    .flat_map(|element| self.pattern_bindings(element))
                    .collect::<Vec<_>>();
                if let Some(rest) = rest {
                    bindings.extend(self.pattern_bindings(rest));
                }
                bindings
            }
            PatternKind::Record(fields) => fields
                .into_iter()
                .flat_map(|field| self.pattern_bindings(field.pattern))
                .collect(),
            PatternKind::Constructor { arguments, .. } => arguments
                .into_iter()
                .flat_map(|argument| self.pattern_bindings(argument))
                .collect(),
            PatternKind::Wildcard
            | PatternKind::Integer(_)
            | PatternKind::Text(_)
            | PatternKind::UnresolvedName(_) => Vec::new(),
        }
    }

    fn rewrite_captured_bindings_expr(
        &mut self,
        expr_id: ExprId,
        captures: &HashMap<BindingId, BindingId>,
    ) {
        let expr = self.module.exprs()[expr_id].clone();
        let kind = match expr.kind {
            ExprKind::Name(mut reference) => {
                if let ResolutionState::Resolved(TermResolution::Local(binding)) = reference.resolution
                    && let Some(mapped) = captures.get(&binding) {
                        let text = self.module.bindings()[*mapped].name.text().to_owned();
                        reference = self.resolved_local_reference(&text, expr.span, *mapped);
                    }
                ExprKind::Name(reference)
            }
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::AmbientSubject
            | ExprKind::Regex(_) => return,
            ExprKind::Text(mut text) => {
                for segment in &mut text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        self.rewrite_captured_bindings_expr(interpolation.expr, captures);
                    }
                }
                ExprKind::Text(text)
            }
            ExprKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.rewrite_captured_bindings_expr(*element, captures);
                }
                ExprKind::Tuple(elements)
            }
            ExprKind::List(elements) => {
                for element in &elements {
                    self.rewrite_captured_bindings_expr(*element, captures);
                }
                ExprKind::List(elements)
            }
            ExprKind::Map(map) => {
                for entry in &map.entries {
                    self.rewrite_captured_bindings_expr(entry.key, captures);
                    self.rewrite_captured_bindings_expr(entry.value, captures);
                }
                ExprKind::Map(map)
            }
            ExprKind::Set(elements) => {
                for element in &elements {
                    self.rewrite_captured_bindings_expr(*element, captures);
                }
                ExprKind::Set(elements)
            }
            ExprKind::Lambda(_) => return,
            ExprKind::Record(record) => {
                for field in &record.fields {
                    self.rewrite_captured_bindings_expr(field.value, captures);
                }
                ExprKind::Record(record)
            }
            ExprKind::Projection { base, path } => {
                if let ProjectionBase::Expr(base) = base {
                    self.rewrite_captured_bindings_expr(base, captures);
                    ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        path,
                    }
                } else {
                    ExprKind::Projection { base, path }
                }
            }
            ExprKind::Apply { callee, arguments } => {
                self.rewrite_captured_bindings_expr(callee, captures);
                for argument in arguments.iter() {
                    self.rewrite_captured_bindings_expr(*argument, captures);
                }
                ExprKind::Apply { callee, arguments }
            }
            ExprKind::Unary { operator, expr } => {
                self.rewrite_captured_bindings_expr(expr, captures);
                ExprKind::Unary { operator, expr }
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.rewrite_captured_bindings_expr(left, captures);
                self.rewrite_captured_bindings_expr(right, captures);
                ExprKind::Binary {
                    left,
                    operator,
                    right,
                }
            }
            ExprKind::PatchApply { target, mut patch } => {
                self.rewrite_captured_bindings_expr(target, captures);
                self.rewrite_captured_bindings_patch(&mut patch, captures);
                ExprKind::PatchApply { target, patch }
            }
            ExprKind::PatchLiteral(mut patch) => {
                self.rewrite_captured_bindings_patch(&mut patch, captures);
                ExprKind::PatchLiteral(patch)
            }
            ExprKind::Pipe(pipe) => {
                self.rewrite_captured_bindings_expr(pipe.head, captures);
                for stage in pipe.stages.iter() {
                    self.rewrite_captured_bindings_stage(stage, captures);
                }
                ExprKind::Pipe(pipe)
            }
            ExprKind::Cluster(cluster_id) => {
                let cluster = self.module.clusters()[cluster_id].clone();
                for member in cluster.members.iter() {
                    self.rewrite_captured_bindings_expr(*member, captures);
                }
                if let ClusterFinalizer::Explicit(expr) = cluster.finalizer {
                    self.rewrite_captured_bindings_expr(expr, captures);
                }
                ExprKind::Cluster(cluster_id)
            }
            ExprKind::Markup(node_id) => {
                self.rewrite_captured_bindings_markup(node_id, captures);
                ExprKind::Markup(node_id)
            }
        };
        *self
            .module
            .arenas
            .exprs
            .get_mut(expr_id)
            .expect("expr id should remain valid during capture rewrite") = Expr {
            span: expr.span,
            kind,
        };
    }

    fn rewrite_captured_bindings_patch(
        &mut self,
        patch: &mut PatchBlock,
        captures: &HashMap<BindingId, BindingId>,
    ) {
        for entry in &mut patch.entries {
            for segment in &mut entry.selector.segments {
                if let PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                    self.rewrite_captured_bindings_expr(*expr, captures);
                }
            }
            match entry.instruction.kind {
                PatchInstructionKind::Replace(expr) | PatchInstructionKind::Store(expr) => {
                    self.rewrite_captured_bindings_expr(expr, captures);
                }
                PatchInstructionKind::Remove => {}
            }
        }
    }

    fn rewrite_captured_bindings_stage(
        &mut self,
        stage: &PipeStage,
        captures: &HashMap<BindingId, BindingId>,
    ) {
        match &stage.kind {
            PipeStageKind::Transform { expr }
            | PipeStageKind::Gate { expr }
            | PipeStageKind::Map { expr }
            | PipeStageKind::Apply { expr }
            | PipeStageKind::Tap { expr }
            | PipeStageKind::FanIn { expr }
            | PipeStageKind::Truthy { expr }
            | PipeStageKind::Falsy { expr }
            | PipeStageKind::RecurStart { expr }
            | PipeStageKind::RecurStep { expr }
            | PipeStageKind::Validate { expr }
            | PipeStageKind::Previous { expr }
            | PipeStageKind::Diff { expr }
            | PipeStageKind::Delay { duration: expr } => {
                self.rewrite_captured_bindings_expr(*expr, captures);
            }
            PipeStageKind::Case { body, .. } => {
                self.rewrite_captured_bindings_expr(*body, captures);
            }
            PipeStageKind::Accumulate { seed, step } => {
                self.rewrite_captured_bindings_expr(*seed, captures);
                self.rewrite_captured_bindings_expr(*step, captures);
            }
            PipeStageKind::Burst { every, count } => {
                self.rewrite_captured_bindings_expr(*every, captures);
                self.rewrite_captured_bindings_expr(*count, captures);
            }
        }
    }

    fn rewrite_captured_bindings_markup(
        &mut self,
        node_id: MarkupNodeId,
        captures: &HashMap<BindingId, BindingId>,
    ) {
        let node = self.module.markup_nodes()[node_id].clone();
        match node.kind {
            MarkupNodeKind::Element(element) => {
                for attribute in &element.attributes {
                    match &attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            self.rewrite_captured_bindings_expr(*expr, captures);
                        }
                        MarkupAttributeValue::Text(text) => {
                            for segment in &text.segments {
                                if let TextSegment::Interpolation(interpolation) = segment {
                                    self.rewrite_captured_bindings_expr(
                                        interpolation.expr,
                                        captures,
                                    );
                                }
                            }
                        }
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in element.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
            MarkupNodeKind::Control(control_id) => {
                self.rewrite_captured_bindings_control(control_id, captures);
            }
        }
    }

    fn rewrite_captured_bindings_control(
        &mut self,
        control_id: ControlNodeId,
        captures: &HashMap<BindingId, BindingId>,
    ) {
        let control = self.module.control_nodes()[control_id].clone();
        match control {
            ControlNode::Show(node) => {
                self.rewrite_captured_bindings_expr(node.when, captures);
                if let Some(expr) = node.keep_mounted {
                    self.rewrite_captured_bindings_expr(expr, captures);
                }
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
            ControlNode::Each(node) => {
                self.rewrite_captured_bindings_expr(node.collection, captures);
                if let Some(key) = node.key {
                    self.rewrite_captured_bindings_expr(key, captures);
                }
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
                if let Some(empty) = node.empty {
                    self.rewrite_captured_bindings_control(empty, captures);
                }
            }
            ControlNode::Match(node) => {
                self.rewrite_captured_bindings_expr(node.scrutinee, captures);
                for case in node.cases.iter() {
                    self.rewrite_captured_bindings_control(*case, captures);
                }
            }
            ControlNode::Empty(node) => {
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
            ControlNode::Fragment(node) => {
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
            ControlNode::Case(node) => {
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
            ControlNode::With(node) => {
                self.rewrite_captured_bindings_expr(node.value, captures);
                for child in node.children {
                    self.rewrite_captured_bindings_markup(child, captures);
                }
            }
        }
    }

    fn rewrite_subject_shorthand_expr(
        &mut self,
        expr_id: ExprId,
        binding: BindingId,
        ambient_allowed: bool,
    ) {
        let expr = self.module.exprs()[expr_id].clone();
        let kind = match expr.kind {
            ExprKind::AmbientSubject if !ambient_allowed => {
                ExprKind::Name(self.resolved_local_reference(
                    self.module.bindings()[binding].name.text(),
                    expr.span,
                    binding,
                ))
            }
            ExprKind::Name(_)
            | ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::Regex(_)
            | ExprKind::AmbientSubject => return,
            ExprKind::Text(text) => {
                for segment in &text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        self.rewrite_subject_shorthand_expr(
                            interpolation.expr,
                            binding,
                            ambient_allowed,
                        );
                    }
                }
                ExprKind::Text(text)
            }
            ExprKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.rewrite_subject_shorthand_expr(*element, binding, ambient_allowed);
                }
                ExprKind::Tuple(elements)
            }
            ExprKind::List(elements) => {
                for element in &elements {
                    self.rewrite_subject_shorthand_expr(*element, binding, ambient_allowed);
                }
                ExprKind::List(elements)
            }
            ExprKind::Map(map) => {
                for entry in &map.entries {
                    self.rewrite_subject_shorthand_expr(entry.key, binding, ambient_allowed);
                    self.rewrite_subject_shorthand_expr(entry.value, binding, ambient_allowed);
                }
                ExprKind::Map(map)
            }
            ExprKind::Set(elements) => {
                for element in &elements {
                    self.rewrite_subject_shorthand_expr(*element, binding, ambient_allowed);
                }
                ExprKind::Set(elements)
            }
            ExprKind::Lambda(_) => return,
            ExprKind::Record(record) => {
                for field in &record.fields {
                    self.rewrite_subject_shorthand_expr(field.value, binding, ambient_allowed);
                }
                ExprKind::Record(record)
            }
            ExprKind::Projection { base, path } => match base {
                ProjectionBase::Ambient if !ambient_allowed => ExprKind::Projection {
                    base: ProjectionBase::Expr(self.synthetic_local_expr(binding, expr.span)),
                    path,
                },
                ProjectionBase::Ambient => ExprKind::Projection {
                    base: ProjectionBase::Ambient,
                    path,
                },
                ProjectionBase::Expr(base) => {
                    self.rewrite_subject_shorthand_expr(base, binding, ambient_allowed);
                    ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        path,
                    }
                }
            },
            ExprKind::Apply { callee, arguments } => {
                self.rewrite_subject_shorthand_expr(callee, binding, ambient_allowed);
                for argument in arguments.iter() {
                    self.rewrite_subject_shorthand_expr(*argument, binding, ambient_allowed);
                }
                ExprKind::Apply { callee, arguments }
            }
            ExprKind::Unary { operator, expr } => {
                self.rewrite_subject_shorthand_expr(expr, binding, ambient_allowed);
                ExprKind::Unary { operator, expr }
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.rewrite_subject_shorthand_expr(left, binding, ambient_allowed);
                self.rewrite_subject_shorthand_expr(right, binding, ambient_allowed);
                ExprKind::Binary {
                    left,
                    operator,
                    right,
                }
            }
            ExprKind::PatchApply { target, patch } => {
                self.rewrite_subject_shorthand_expr(target, binding, ambient_allowed);
                self.rewrite_subject_shorthand_patch(&patch, binding, ambient_allowed);
                ExprKind::PatchApply { target, patch }
            }
            ExprKind::PatchLiteral(patch) => {
                self.rewrite_subject_shorthand_patch(&patch, binding, ambient_allowed);
                ExprKind::PatchLiteral(patch)
            }
            ExprKind::Pipe(pipe) => {
                self.rewrite_subject_shorthand_expr(pipe.head, binding, ambient_allowed);
                for stage in pipe.stages.iter() {
                    self.rewrite_subject_shorthand_stage(stage, binding);
                }
                ExprKind::Pipe(pipe)
            }
            ExprKind::Cluster(cluster_id) => {
                let cluster = self.module.clusters()[cluster_id].clone();
                for member in cluster.members.iter() {
                    self.rewrite_subject_shorthand_expr(*member, binding, ambient_allowed);
                }
                if let ClusterFinalizer::Explicit(expr) = cluster.finalizer {
                    self.rewrite_subject_shorthand_expr(expr, binding, ambient_allowed);
                }
                ExprKind::Cluster(cluster_id)
            }
            ExprKind::Markup(node_id) => {
                self.rewrite_subject_shorthand_markup(node_id, binding, ambient_allowed);
                ExprKind::Markup(node_id)
            }
        };
        *self
            .module
            .arenas
            .exprs
            .get_mut(expr_id)
            .expect("expr id should remain valid during subject shorthand rewrite") = Expr {
            span: expr.span,
            kind,
        };
    }

    fn rewrite_subject_shorthand_patch(
        &mut self,
        patch: &PatchBlock,
        binding: BindingId,
        ambient_allowed: bool,
    ) {
        for entry in &patch.entries {
            for segment in &entry.selector.segments {
                if let PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                    self.rewrite_subject_shorthand_expr(*expr, binding, ambient_allowed);
                }
            }
            match entry.instruction.kind {
                PatchInstructionKind::Replace(expr) | PatchInstructionKind::Store(expr) => {
                    self.rewrite_subject_shorthand_expr(expr, binding, ambient_allowed);
                }
                PatchInstructionKind::Remove => {}
            }
        }
    }

    fn rewrite_subject_shorthand_stage(&mut self, stage: &PipeStage, binding: BindingId) {
        match &stage.kind {
            PipeStageKind::Transform { expr }
            | PipeStageKind::Gate { expr }
            | PipeStageKind::Map { expr }
            | PipeStageKind::Apply { expr }
            | PipeStageKind::Tap { expr }
            | PipeStageKind::FanIn { expr }
            | PipeStageKind::Truthy { expr }
            | PipeStageKind::Falsy { expr }
            | PipeStageKind::RecurStart { expr }
            | PipeStageKind::RecurStep { expr }
            | PipeStageKind::Validate { expr }
            | PipeStageKind::Previous { expr }
            | PipeStageKind::Diff { expr }
            | PipeStageKind::Delay { duration: expr } => {
                self.rewrite_subject_shorthand_expr(*expr, binding, true);
            }
            PipeStageKind::Case { body, .. } => {
                self.rewrite_subject_shorthand_expr(*body, binding, true);
            }
            PipeStageKind::Accumulate { seed, step } => {
                self.rewrite_subject_shorthand_expr(*seed, binding, true);
                self.rewrite_subject_shorthand_expr(*step, binding, true);
            }
            PipeStageKind::Burst { every, count } => {
                self.rewrite_subject_shorthand_expr(*every, binding, true);
                self.rewrite_subject_shorthand_expr(*count, binding, true);
            }
        }
    }

    fn rewrite_subject_shorthand_markup(
        &mut self,
        node_id: MarkupNodeId,
        binding: BindingId,
        ambient_allowed: bool,
    ) {
        let node = self.module.markup_nodes()[node_id].clone();
        match node.kind {
            MarkupNodeKind::Element(element) => {
                for attribute in &element.attributes {
                    match &attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            self.rewrite_subject_shorthand_expr(*expr, binding, ambient_allowed);
                        }
                        MarkupAttributeValue::Text(text) => {
                            for segment in &text.segments {
                                if let TextSegment::Interpolation(interpolation) = segment {
                                    self.rewrite_subject_shorthand_expr(
                                        interpolation.expr,
                                        binding,
                                        ambient_allowed,
                                    );
                                }
                            }
                        }
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in element.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
            MarkupNodeKind::Control(control_id) => {
                self.rewrite_subject_shorthand_control(control_id, binding, ambient_allowed);
            }
        }
    }

    fn rewrite_subject_shorthand_control(
        &mut self,
        control_id: ControlNodeId,
        binding: BindingId,
        ambient_allowed: bool,
    ) {
        let control = self.module.control_nodes()[control_id].clone();
        match control {
            ControlNode::Show(node) => {
                self.rewrite_subject_shorthand_expr(node.when, binding, ambient_allowed);
                if let Some(expr) = node.keep_mounted {
                    self.rewrite_subject_shorthand_expr(expr, binding, ambient_allowed);
                }
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
            ControlNode::Each(node) => {
                self.rewrite_subject_shorthand_expr(node.collection, binding, ambient_allowed);
                if let Some(key) = node.key {
                    self.rewrite_subject_shorthand_expr(key, binding, ambient_allowed);
                }
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
                if let Some(empty) = node.empty {
                    self.rewrite_subject_shorthand_control(empty, binding, ambient_allowed);
                }
            }
            ControlNode::Match(node) => {
                self.rewrite_subject_shorthand_expr(node.scrutinee, binding, ambient_allowed);
                for case in node.cases.iter() {
                    self.rewrite_subject_shorthand_control(*case, binding, ambient_allowed);
                }
            }
            ControlNode::Empty(node) => {
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
            ControlNode::Fragment(node) => {
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
            ControlNode::Case(node) => {
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
            ControlNode::With(node) => {
                self.rewrite_subject_shorthand_expr(node.value, binding, ambient_allowed);
                for child in node.children {
                    self.rewrite_subject_shorthand_markup(child, binding, ambient_allowed);
                }
            }
        }
    }

    fn resolved_local_reference(&self, text: &str, span: SourceSpan, binding: BindingId) -> TermReference {
        let name = Name::new(text, span).expect("synthetic local reference names should be valid");
        let path =
            NamePath::from_vec(vec![name]).expect("single-segment local path should be valid");
        TermReference::resolved(path, TermResolution::Local(binding))
    }

    fn normalize_function_signature_annotations(&mut self) {
        let function_ids = self
            .module
            .items()
            .iter()
            .filter_map(|(item_id, item)| matches!(item, Item::Function(_)).then_some(item_id))
            .collect::<Vec<_>>();
        for item_id in function_ids {
            self.normalize_function_signature_annotation(item_id);
        }
    }

    fn normalize_function_signature_annotation(&mut self, item_id: ItemId) {
        let Some((arity, context, annotation, already_split, span)) =
            (match &self.module.items()[item_id] {
                Item::Function(item) => Some((
                    item.parameters.len(),
                    item.context.clone(),
                    item.annotation,
                    item.parameters
                        .iter()
                        .any(|parameter| parameter.annotation.is_some()),
                    item.header.span,
                )),
                _ => None,
            })
        else {
            return;
        };

        let Some(annotation) = annotation else {
            return;
        };
        if arity == 0 || already_split {
            return;
        }

        let Some((constraint_count, parameter_annotations, result_annotation)) =
            self.split_normalized_function_signature_annotation(&context, annotation, arity)
        else {
            self.diagnostics.push(
                Diagnostic::error(
                    "function annotations with parameters must describe the full function signature",
                )
                .with_code(code("invalid-function-signature-annotation"))
                .with_primary_label(
                    span,
                    "expected one parameter type per function parameter before the result type",
                )
                .with_note("use a standalone `type ...` line or a compatible alias such as `type MyFunc = A -> B -> C`"),
            );
            return;
        };

        let Some(Item::Function(function)) = self.module.arenas.items.get_mut(item_id) else {
            return;
        };
        function.context.truncate(constraint_count);
        for (parameter, annotation) in function
            .parameters
            .iter_mut()
            .zip(parameter_annotations.into_iter())
        {
            parameter.annotation = Some(annotation);
        }
        function.annotation = Some(result_annotation);
    }

    fn split_normalized_function_signature_annotation(
        &mut self,
        context: &[TypeId],
        annotation: TypeId,
        arity: usize,
    ) -> Option<(usize, Vec<TypeId>, TypeId)> {
        let maximum_context_parameters = context.len().min(arity);
        for trailing_parameter_count in 0..=maximum_context_parameters {
            let constraint_count = context.len() - trailing_parameter_count;
            let constraints = &context[..constraint_count];
            let leading_parameters = &context[constraint_count..];
            if constraints
                .iter()
                .copied()
                .any(|type_id| !self.is_class_constraint_type(type_id))
            {
                continue;
            }
            let remaining_arity = arity - trailing_parameter_count;
            let mut item_stack = Vec::new();
            let Some((mut parameter_annotations, result_annotation)) = self
                .split_function_signature_annotation(
                    annotation,
                    remaining_arity,
                    &HashMap::new(),
                    &mut item_stack,
                )
            else {
                continue;
            };
            let mut normalized_parameters = leading_parameters.to_vec();
            normalized_parameters.append(&mut parameter_annotations);
            if normalized_parameters.len() == arity {
                return Some((constraint_count, normalized_parameters, result_annotation));
            }
        }
        None
    }

    fn split_function_signature_annotation(
        &mut self,
        type_id: TypeId,
        arity: usize,
        substitutions: &HashMap<TypeParameterId, TypeId>,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<(Vec<TypeId>, TypeId)> {
        if arity == 0 {
            return Some((
                Vec::new(),
                self.instantiate_signature_type(type_id, substitutions)?,
            ));
        }

        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Arrow { parameter, result } => {
                let parameter = self.instantiate_signature_type(parameter, substitutions)?;
                let (mut parameters, result) = self.split_function_signature_annotation(
                    result,
                    arity - 1,
                    substitutions,
                    item_stack,
                )?;
                parameters.insert(0, parameter);
                Some((parameters, result))
            }
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(alias_item_id)) => {
                    let alias_item_id = *alias_item_id;
                    if item_stack.contains(&alias_item_id) {
                        return None;
                    }
                    let Item::Type(item) = &self.module.items()[alias_item_id] else {
                        return None;
                    };
                    if !item.parameters.is_empty() {
                        return None;
                    }
                    let TypeItemBody::Alias(alias) = &item.body else {
                        return None;
                    };
                    let alias = *alias;
                    item_stack.push(alias_item_id);
                    let split = self.split_function_signature_annotation(
                        alias,
                        arity,
                        substitutions,
                        item_stack,
                    );
                    let popped = item_stack.pop();
                    debug_assert_eq!(popped, Some(alias_item_id));
                    split
                }
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let substituted = substitutions.get(parameter).copied()?;
                    self.split_function_signature_annotation(
                        substituted,
                        arity,
                        &HashMap::new(),
                        item_stack,
                    )
                }
                _ => None,
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
                    return None;
                };
                let ResolutionState::Resolved(TypeResolution::Item(alias_item_id)) =
                    reference.resolution.as_ref()
                else {
                    return None;
                };
                let alias_item_id = *alias_item_id;
                if item_stack.contains(&alias_item_id) {
                    return None;
                }
                let (parameters, alias) = match &self.module.items()[alias_item_id] {
                    Item::Type(item) => {
                        let TypeItemBody::Alias(alias) = &item.body else {
                            return None;
                        };
                        (item.parameters.clone(), *alias)
                    }
                    _ => return None,
                };
                if parameters.len() != arguments.len() {
                    return None;
                }
                let mut nested_substitutions = HashMap::with_capacity(parameters.len());
                for (parameter, argument) in parameters.iter().zip(arguments.iter()) {
                    nested_substitutions.insert(
                        *parameter,
                        self.instantiate_signature_type(*argument, substitutions)?,
                    );
                }
                item_stack.push(alias_item_id);
                let split = self.split_function_signature_annotation(
                    alias,
                    arity,
                    &nested_substitutions,
                    item_stack,
                );
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(alias_item_id));
                split
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::RecordTransform { .. } => None,
        }
    }

    fn is_class_constraint_type(&self, type_id: TypeId) -> bool {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    matches!(self.module.items()[*item_id], Item::Class(_))
                }
                _ => false,
            },
            TypeKind::Apply { callee, .. } => self.is_class_constraint_type(*callee),
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => false,
        }
    }

    fn instantiate_signature_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, TypeId>,
    ) -> Option<TypeId> {
        if substitutions.is_empty() {
            return Some(type_id);
        }

        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    substitutions.get(parameter).copied().or(Some(type_id))
                }
                _ => Some(type_id),
            },
            TypeKind::Tuple(elements) => {
                let mut changed = false;
                let mut instantiated = Vec::with_capacity(elements.len());
                for element in elements.iter().copied() {
                    let instantiated_element =
                        self.instantiate_signature_type(element, substitutions)?;
                    changed |= instantiated_element != element;
                    instantiated.push(instantiated_element);
                }
                if !changed {
                    return Some(type_id);
                }
                Some(
                    self.alloc_type(TypeNode {
                        span: ty.span,
                        kind: TypeKind::Tuple(
                            AtLeastTwo::from_vec(instantiated)
                                .expect("tuple instantiation preserves arity"),
                        ),
                    }),
                )
            }
            TypeKind::Record(fields) => {
                let mut changed = false;
                let mut instantiated = Vec::with_capacity(fields.len());
                for field in fields {
                    let field_ty = self.instantiate_signature_type(field.ty, substitutions)?;
                    changed |= field_ty != field.ty;
                    instantiated.push(TypeField {
                        span: field.span,
                        label: field.label,
                        ty: field_ty,
                    });
                }
                if !changed {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(instantiated),
                }))
            }
            TypeKind::RecordTransform { transform, source } => {
                let instantiated_source = self.instantiate_signature_type(source, substitutions)?;
                if instantiated_source == source {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::RecordTransform {
                        transform,
                        source: instantiated_source,
                    },
                }))
            }
            TypeKind::Arrow { parameter, result } => {
                let instantiated_parameter =
                    self.instantiate_signature_type(parameter, substitutions)?;
                let instantiated_result = self.instantiate_signature_type(result, substitutions)?;
                if instantiated_parameter == parameter && instantiated_result == result {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow {
                        parameter: instantiated_parameter,
                        result: instantiated_result,
                    },
                }))
            }
            TypeKind::Apply { callee, arguments } => {
                let instantiated_callee = self.instantiate_signature_type(callee, substitutions)?;
                let mut changed = instantiated_callee != callee;
                let mut instantiated_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter().copied() {
                    let argument_ty = self.instantiate_signature_type(argument, substitutions)?;
                    changed |= argument_ty != argument;
                    instantiated_arguments.push(argument_ty);
                }
                if !changed {
                    return Some(type_id);
                }
                Some(self.alloc_type(
                    TypeNode {
                        span: ty.span,
                        kind:
                            TypeKind::Apply {
                                callee: instantiated_callee,
                                arguments:
                                    NonEmpty::from_vec(instantiated_arguments).expect(
                                        "type applications preserve non-empty argument lists",
                                    ),
                            },
                    },
                ))
            }
        }
    }

    fn validate_cluster_normalization(&mut self) {
        let cluster_ids = self
            .module
            .clusters()
            .iter()
            .map(|(cluster_id, _)| cluster_id)
            .collect::<Vec<_>>();
        for cluster_id in cluster_ids {
            self.validate_cluster_ambient_projections(cluster_id);
        }
    }

    fn validate_cluster_ambient_projections(&mut self, cluster_id: crate::ClusterId) {
        let cluster = self.module.clusters()[cluster_id].clone();
        let spine = cluster.normalized_spine();
        for member in spine.apply_arguments() {
            if let Some(span) = self.find_free_ambient_projection(member) {
                self.emit_illegal_cluster_ambient_projection(span, cluster.span);
            }
        }
        if let ApplicativeSpineHead::Expr(finalizer) = spine.pure_head()
            && let Some(span) = self.find_free_ambient_projection(finalizer) {
                self.emit_illegal_cluster_ambient_projection(span, cluster.span);
            }
    }

    fn emit_illegal_cluster_ambient_projection(
        &mut self,
        span: SourceSpan,
        cluster_span: SourceSpan,
    ) {
        self.diagnostics.push(
            Diagnostic::error(
                "ambient-subject projections such as `.field` are illegal inside `&|>` clusters unless a nested expression provides its own subject",
            )
            .with_code(code("illegal-cluster-ambient-projection"))
            .with_primary_label(
                span,
                "this projection has no ambient subject inside the applicative cluster",
            )
            .with_secondary_label(
                cluster_span,
                "cluster members normalize independently before the finalizer runs",
            )
            .with_note(
                "use an explicit base such as `value.field` or a nested pipe with its own head",
            ),
        );
    }

    fn find_free_ambient_projection(&self, root: ExprId) -> Option<SourceSpan> {
        let mut work = vec![AmbientProjectionWork::Expr {
            expr: root,
            ambient_allowed: false,
        }];
        while let Some(node) = work.pop() {
            match node {
                AmbientProjectionWork::Expr {
                    expr,
                    ambient_allowed,
                } => match &self.module.exprs()[expr].kind {
                    ExprKind::Name(_)
                    | ExprKind::Integer(_)
                    | ExprKind::Float(_)
                    | ExprKind::Decimal(_)
                    | ExprKind::BigInt(_)
                    | ExprKind::SuffixedInteger(_)
                    | ExprKind::AmbientSubject
                    | ExprKind::Regex(_) => {}
                    ExprKind::Lambda(lambda) => {
                        work.push(AmbientProjectionWork::Expr {
                            expr: lambda.body,
                            ambient_allowed: matches!(
                                lambda.surface_form,
                                crate::hir::LambdaSurfaceForm::SubjectShorthand
                            ),
                        });
                    }
                    ExprKind::Text(text) => {
                        for segment in text.segments.iter().rev() {
                            if let TextSegment::Interpolation(interpolation) = segment {
                                work.push(AmbientProjectionWork::Expr {
                                    expr: interpolation.expr,
                                    ambient_allowed,
                                });
                            }
                        }
                    }
                    ExprKind::Tuple(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::List(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Map(map) => {
                        for entry in map.entries.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: entry.value,
                                ambient_allowed,
                            });
                            work.push(AmbientProjectionWork::Expr {
                                expr: entry.key,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Set(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Record(record) => {
                        for field in record.fields.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: field.value,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        ..
                    } if !ambient_allowed => return Some(self.module.exprs()[expr].span),
                    ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        ..
                    } => {}
                    ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        ..
                    } => work.push(AmbientProjectionWork::Expr {
                        expr: *base,
                        ambient_allowed,
                    }),
                    ExprKind::Apply { callee, arguments } => {
                        for argument in arguments.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *argument,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: *callee,
                            ambient_allowed,
                        });
                    }
                    ExprKind::Unary { expr, .. } => work.push(AmbientProjectionWork::Expr {
                        expr: *expr,
                        ambient_allowed,
                    }),
                    ExprKind::Binary { left, right, .. } => {
                        work.push(AmbientProjectionWork::Expr {
                            expr: *right,
                            ambient_allowed,
                        });
                        work.push(AmbientProjectionWork::Expr {
                            expr: *left,
                            ambient_allowed,
                        });
                    }
                    ExprKind::PatchApply { target, patch } => {
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr,
                                        ambient_allowed,
                                    });
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                            }
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: *target,
                            ambient_allowed,
                        });
                    }
                    ExprKind::PatchLiteral(patch) => {
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr,
                                        ambient_allowed,
                                    });
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                            }
                        }
                    }
                    ExprKind::Pipe(pipe) => {
                        for stage in pipe.stages.iter().rev() {
                            match &stage.kind {
                                PipeStageKind::Transform { expr }
                                | PipeStageKind::Gate { expr }
                                | PipeStageKind::Map { expr }
                                | PipeStageKind::Apply { expr }
                                | PipeStageKind::Tap { expr }
                                | PipeStageKind::FanIn { expr }
                                | PipeStageKind::Truthy { expr }
                                | PipeStageKind::Falsy { expr }
                                | PipeStageKind::RecurStart { expr }
                                | PipeStageKind::RecurStep { expr }
                                | PipeStageKind::Validate { expr }
                                | PipeStageKind::Previous { expr }
                                | PipeStageKind::Diff { expr }
                                | PipeStageKind::Delay { duration: expr } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed: true,
                                    });
                                }
                                PipeStageKind::Accumulate { seed, step } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *step,
                                        ambient_allowed: true,
                                    });
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *seed,
                                        ambient_allowed: true,
                                    });
                                }
                                PipeStageKind::Burst { every, count } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *every,
                                        ambient_allowed: true,
                                    });
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *count,
                                        ambient_allowed: true,
                                    });
                                }
                                PipeStageKind::Case { body, .. } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *body,
                                        ambient_allowed: true,
                                    });
                                }
                            }
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: pipe.head,
                            ambient_allowed,
                        });
                    }
                    ExprKind::Cluster(_) => {}
                    ExprKind::Markup(node) => work.push(AmbientProjectionWork::Markup {
                        node: *node,
                        ambient_allowed,
                    }),
                },
                AmbientProjectionWork::Markup {
                    node,
                    ambient_allowed,
                } => match &self.module.markup_nodes()[node].kind {
                    MarkupNodeKind::Element(element) => {
                        for child in element.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        for attribute in element.attributes.iter().rev() {
                            match &attribute.value {
                                MarkupAttributeValue::ImplicitTrue => {}
                                MarkupAttributeValue::Expr(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                                MarkupAttributeValue::Text(text) => {
                                    for segment in text.segments.iter().rev() {
                                        if let TextSegment::Interpolation(interpolation) = segment {
                                            work.push(AmbientProjectionWork::Expr {
                                                expr: interpolation.expr,
                                                ambient_allowed,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    MarkupNodeKind::Control(control) => work.push(AmbientProjectionWork::Control {
                        node: *control,
                        ambient_allowed,
                    }),
                },
                AmbientProjectionWork::Control {
                    node,
                    ambient_allowed,
                } => match &self.module.control_nodes()[node] {
                    ControlNode::Show(show) => {
                        for child in show.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        if let Some(keep_mounted) = show.keep_mounted {
                            work.push(AmbientProjectionWork::Expr {
                                expr: keep_mounted,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: show.when,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Each(each) => {
                        if let Some(empty) = each.empty {
                            work.push(AmbientProjectionWork::Control {
                                node: empty,
                                ambient_allowed,
                            });
                        }
                        for child in each.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        if let Some(key) = each.key {
                            work.push(AmbientProjectionWork::Expr {
                                expr: key,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: each.collection,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Empty(empty) => {
                        for child in empty.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::Match(match_node) => {
                        for case in match_node.cases.iter().rev() {
                            work.push(AmbientProjectionWork::Control {
                                node: *case,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: match_node.scrutinee,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Case(case) => {
                        for child in case.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::Fragment(fragment) => {
                        for child in fragment.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::With(with_node) => {
                        for child in with_node.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: with_node.value,
                            ambient_allowed,
                        });
                    }
                },
            }
        }
        None
    }

    fn resolve_item(
        &mut self,
        item_id: ItemId,
        namespaces: &Namespaces,
        prefer_ambient_names: bool,
    ) {
        let item = self.module.items()[item_id].clone();
        for decorator in item.decorators() {
            self.resolve_decorator(*decorator, namespaces, prefer_ambient_names);
        }
        let resolved = match item {
            Item::Type(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                match &item.body {
                    TypeItemBody::Alias(alias) => self.resolve_type(*alias, namespaces, &mut env),
                    TypeItemBody::Sum(variants) => {
                        for variant in variants.iter() {
                            for field in &variant.fields {
                                self.resolve_type(field.ty, namespaces, &mut env);
                            }
                        }
                    }
                }
                Item::Type(item)
            }
            Item::Value(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                self.resolve_expr(item.body, namespaces, &env);
                Item::Value(item)
            }
            Item::Function(mut item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                if !item.type_parameters.is_empty() {
                    env.push_type_scope(
                        self.type_parameter_scope(item.type_parameters.iter().copied()),
                    );
                }
                env.enable_implicit_type_parameters();
                for constraint in &item.context {
                    self.resolve_type(*constraint, namespaces, &mut env);
                }
                env.push_term_scope(
                    self.binding_scope(item.parameters.iter().map(|parameter| parameter.binding)),
                );
                for parameter in &item.parameters {
                    if let Some(annotation) = parameter.annotation {
                        self.resolve_type(annotation, namespaces, &mut env);
                    }
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                self.resolve_expr(item.body, namespaces, &env);
                let mut type_parameters = item.type_parameters.clone();
                for parameter in env.implicit_type_parameters() {
                    if !type_parameters.contains(&parameter) {
                        type_parameters.push(parameter);
                    }
                }
                item.type_parameters = type_parameters;
                Item::Function(item)
            }
            Item::Signal(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                if let Some(body) = item.body {
                    self.resolve_expr(body, namespaces, &env);
                }
                for update in &item.reactive_updates {
                    self.resolve_expr(update.guard, namespaces, &env);
                    self.resolve_expr(update.body, namespaces, &env);
                }
                Item::Signal(item)
            }
            Item::Class(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                for superclass in &item.superclasses {
                    self.resolve_type(*superclass, namespaces, &mut env);
                }
                for constraint in &item.param_constraints {
                    self.resolve_type(*constraint, namespaces, &mut env);
                }
                let mut item = item;
                for member in &mut item.members {
                    let mut member_env = env.clone();
                    member_env.enable_implicit_type_parameters();
                    for constraint in &member.context {
                        self.resolve_type(*constraint, namespaces, &mut member_env);
                    }
                    self.resolve_type(member.annotation, namespaces, &mut member_env);
                    member.type_parameters = member_env.implicit_type_parameters();
                }
                Item::Class(item)
            }
            Item::Domain(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                self.resolve_type(item.carrier, namespaces, &mut env);
                if self.type_contains_item_reference(item.carrier, item_id) {
                    self.emit_error(
                        item.header.span,
                        format!(
                            "domain `{}` cannot use itself in its carrier type",
                            item.name.text()
                        ),
                        code("recursive-domain-carrier"),
                    );
                }
                for member in &item.members {
                    self.resolve_type(member.annotation, namespaces, &mut env);
                    if let Some(body) = member.body {
                        let mut member_env = env.clone();
                        member_env.push_term_scope(self.binding_scope(
                            member.parameters.iter().map(|parameter| parameter.binding),
                        ));
                        self.resolve_expr(body, namespaces, &member_env);
                    }
                }
                Item::Domain(item)
            }
            Item::SourceProviderContract(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                for argument in &item.contract.arguments {
                    self.resolve_type(argument.annotation, namespaces, &mut env);
                }
                for option in &item.contract.options {
                    self.resolve_type(option.annotation, namespaces, &mut env);
                }
                for operation in &item.contract.operations {
                    self.resolve_type(operation.annotation, namespaces, &mut env);
                }
                for command in &item.contract.commands {
                    self.resolve_type(command.annotation, namespaces, &mut env);
                }
                Item::SourceProviderContract(item)
            }
            Item::Use(item) => Item::Use(item),
            Item::Export(mut item) => {
                item.resolution = self.resolve_export_target(&item.target, namespaces);
                Item::Export(item)
            }
            Item::Hoist(item) => Item::Hoist(item),
            Item::Instance(mut item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.enable_implicit_type_parameters();
                self.resolve_type_reference(&mut item.class, namespaces, &mut env);
                for argument in item.arguments.iter() {
                    self.resolve_type(*argument, namespaces, &mut env);
                }
                for context in &item.context {
                    self.resolve_type(*context, namespaces, &mut env);
                }
                for member in &item.members {
                    if let Some(annotation) = member.annotation {
                        self.resolve_type(annotation, namespaces, &mut env);
                    }
                    let mut member_env = env.clone();
                    member_env.push_term_scope(self.binding_scope(
                        member.parameters.iter().map(|parameter| parameter.binding),
                    ));
                    self.resolve_expr(member.body, namespaces, &member_env);
                }
                item.type_parameters = env.implicit_type_parameters();
                Item::Instance(item)
            }
        };
        *self
            .module
            .arenas
            .items
            .get_mut(item_id)
            .expect("resolved item id should still exist") = resolved;
    }

    fn resolve_decorator(
        &mut self,
        decorator_id: DecoratorId,
        namespaces: &Namespaces,
        prefer_ambient_names: bool,
    ) {
        let decorator = self.module.decorators()[decorator_id].clone();
        let mut env = ResolveEnv::default();
        if prefer_ambient_names {
            env.set_prefer_ambient_names();
        }
        match &decorator.payload {
            DecoratorPayload::Bare => {}
            DecoratorPayload::Call(call) => {
                for argument in &call.arguments {
                    self.resolve_expr(*argument, namespaces, &env);
                }
                if let Some(options) = call.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::RecurrenceWakeup(wakeup) => {
                self.resolve_expr(wakeup.witness, namespaces, &env);
            }
            DecoratorPayload::Source(source) => {
                for argument in &source.arguments {
                    self.resolve_expr(*argument, namespaces, &env);
                }
                if let Some(options) = source.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::Test(_) | DecoratorPayload::Debug(_) => {}
            DecoratorPayload::Deprecated(deprecated) => {
                if let Some(message) = deprecated.message {
                    self.resolve_expr(message, namespaces, &env);
                }
                if let Some(options) = deprecated.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::Mock(mock) => {
                self.resolve_expr(mock.target, namespaces, &env);
                self.resolve_expr(mock.replacement, namespaces, &env);
            }
        }
        *self
            .module
            .arenas
            .decorators
            .get_mut(decorator_id)
            .expect("resolved decorator id should still exist") = decorator;
    }

    fn resolve_expr(&mut self, expr_id: ExprId, namespaces: &Namespaces, env: &ResolveEnv) {
        let expr = self.module.exprs()[expr_id].clone();
        let resolved = match expr.kind {
            ExprKind::Name(mut reference) => {
                self.resolve_term_reference(&mut reference, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Name(reference),
                }
            }
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::AmbientSubject
            | ExprKind::Regex(_) => expr,
            ExprKind::Text(text) => {
                self.resolve_text_literal(&text, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Text(text),
                }
            }
            ExprKind::SuffixedInteger(mut literal) => {
                literal.resolution = self.resolve_literal_suffix(&literal.suffix, namespaces);
                Expr {
                    span: expr.span,
                    kind: ExprKind::SuffixedInteger(literal),
                }
            }
            ExprKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Tuple(elements),
                }
            }
            ExprKind::List(elements) => {
                for element in &elements {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::List(elements),
                }
            }
            ExprKind::Map(map) => {
                for entry in &map.entries {
                    self.resolve_expr(entry.key, namespaces, env);
                    self.resolve_expr(entry.value, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Map(map),
                }
            }
            ExprKind::Set(elements) => {
                for element in &elements {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Set(elements),
                }
            }
            ExprKind::Lambda(lambda) => {
                let mut lambda_env = env.clone();
                lambda_env.push_term_scope(
                    self.binding_scope(lambda.parameters.iter().map(|parameter| parameter.binding)),
                );
                for parameter in &lambda.parameters {
                    if let Some(annotation) = parameter.annotation {
                        self.resolve_type(annotation, namespaces, &mut lambda_env);
                    }
                }
                self.resolve_expr(lambda.body, namespaces, &lambda_env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Lambda(lambda),
                }
            }
            ExprKind::Record(record) => {
                for field in &record.fields {
                    self.resolve_expr(field.value, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Record(record),
                }
            }
            ExprKind::Projection { base, path } => {
                if let ProjectionBase::Expr(base) = base {
                    self.resolve_expr(base, namespaces, env);
                    Expr {
                        span: expr.span,
                        kind: ExprKind::Projection {
                            base: ProjectionBase::Expr(base),
                            path,
                        },
                    }
                } else {
                    Expr {
                        span: expr.span,
                        kind: ExprKind::Projection { base, path },
                    }
                }
            }
            ExprKind::Apply { callee, arguments } => {
                self.resolve_expr(callee, namespaces, env);
                for argument in arguments.iter() {
                    self.resolve_expr(*argument, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Apply { callee, arguments },
                }
            }
            ExprKind::Unary {
                operator,
                expr: inner,
            } => {
                self.resolve_expr(inner, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Unary {
                        operator,
                        expr: inner,
                    },
                }
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.resolve_expr(left, namespaces, env);
                self.resolve_expr(right, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Binary {
                        left,
                        operator,
                        right,
                    },
                }
            }
            ExprKind::PatchApply { target, patch } => {
                self.resolve_expr(target, namespaces, env);
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            self.resolve_expr(*expr, namespaces, env);
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            self.resolve_expr(expr, namespaces, env);
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::PatchApply { target, patch },
                }
            }
            ExprKind::PatchLiteral(patch) => {
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            self.resolve_expr(*expr, namespaces, env);
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            self.resolve_expr(expr, namespaces, env);
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::PatchLiteral(patch),
                }
            }
            ExprKind::Pipe(pipe) => {
                self.resolve_expr(pipe.head, namespaces, env);
                let mut pipe_env = env.clone();
                let stages = pipe.stages.iter().collect::<Vec<_>>();
                let mut stage_index = 0usize;
                while stage_index < stages.len() {
                    let stage = stages[stage_index];
                    match &stage.kind {
                        PipeStageKind::Case { .. } => {
                            let case_start = stage_index;
                            while stage_index < stages.len()
                                && matches!(stages[stage_index].kind, PipeStageKind::Case { .. })
                            {
                                stage_index += 1;
                            }
                            let first_stage = stages[case_start];
                            let mut case_env = pipe_env.clone();
                            if let Some(binding) = first_stage.subject_memo {
                                case_env.push_term_scope(self.binding_scope([binding]));
                            }
                            for case_stage in &stages[case_start..stage_index] {
                                let PipeStageKind::Case { pattern, body } = &case_stage.kind else {
                                    continue;
                                };
                                let bindings =
                                    self.resolve_pattern(*pattern, namespaces, &case_env);
                                let mut branch_env = case_env.clone();
                                branch_env.push_term_scope(self.binding_scope(bindings));
                                self.resolve_expr(*body, namespaces, &branch_env);
                            }
                            if let Some(binding) = first_stage.subject_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            if let Some(binding) = first_stage.result_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                        }
                        PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                            if let Some(pair) = crate::typecheck_context::truthy_falsy_pair_stages(
                                &stages,
                                stage_index,
                            ) {
                                let first_stage = stages[stage_index];
                                let mut pair_env = pipe_env.clone();
                                if let Some(binding) = first_stage.subject_memo {
                                    pair_env.push_term_scope(self.binding_scope([binding]));
                                }
                                self.resolve_expr(pair.truthy_expr, namespaces, &pair_env);
                                self.resolve_expr(pair.falsy_expr, namespaces, &pair_env);
                                if let Some(binding) = first_stage.subject_memo {
                                    pipe_env.push_term_scope(self.binding_scope([binding]));
                                }
                                if let Some(binding) = first_stage.result_memo {
                                    pipe_env.push_term_scope(self.binding_scope([binding]));
                                }
                                stage_index = pair.next_index;
                            } else {
                                let mut stage_env = pipe_env.clone();
                                if let Some(binding) = stage.subject_memo {
                                    stage_env.push_term_scope(self.binding_scope([binding]));
                                }
                                let (PipeStageKind::Truthy { expr }
                                | PipeStageKind::Falsy { expr }) = &stage.kind
                                else {
                                    unreachable!();
                                };
                                self.resolve_expr(*expr, namespaces, &stage_env);
                                if let Some(binding) = stage.subject_memo {
                                    pipe_env.push_term_scope(self.binding_scope([binding]));
                                }
                                if let Some(binding) = stage.result_memo {
                                    pipe_env.push_term_scope(self.binding_scope([binding]));
                                }
                                stage_index += 1;
                            }
                        }
                        PipeStageKind::Transform { expr }
                        | PipeStageKind::Gate { expr }
                        | PipeStageKind::Map { expr }
                        | PipeStageKind::Apply { expr }
                        | PipeStageKind::Tap { expr }
                        | PipeStageKind::FanIn { expr }
                        | PipeStageKind::RecurStart { expr }
                        | PipeStageKind::RecurStep { expr }
                        | PipeStageKind::Validate { expr }
                        | PipeStageKind::Previous { expr }
                        | PipeStageKind::Diff { expr }
                        | PipeStageKind::Delay { duration: expr } => {
                            let mut stage_env = pipe_env.clone();
                            if let Some(binding) = stage.subject_memo {
                                stage_env.push_term_scope(self.binding_scope([binding]));
                            }
                            self.resolve_expr(*expr, namespaces, &stage_env);
                            if let Some(binding) = stage.subject_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            if let Some(binding) = stage.result_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            stage_index += 1;
                        }
                        PipeStageKind::Accumulate { seed, step } => {
                            let mut stage_env = pipe_env.clone();
                            if let Some(binding) = stage.subject_memo {
                                stage_env.push_term_scope(self.binding_scope([binding]));
                            }
                            self.resolve_expr(*seed, namespaces, &stage_env);
                            self.resolve_expr(*step, namespaces, &stage_env);
                            if let Some(binding) = stage.subject_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            if let Some(binding) = stage.result_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            stage_index += 1;
                        }
                        PipeStageKind::Burst { every, count } => {
                            let mut stage_env = pipe_env.clone();
                            if let Some(binding) = stage.subject_memo {
                                stage_env.push_term_scope(self.binding_scope([binding]));
                            }
                            self.resolve_expr(*every, namespaces, &stage_env);
                            self.resolve_expr(*count, namespaces, &stage_env);
                            if let Some(binding) = stage.subject_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            if let Some(binding) = stage.result_memo {
                                pipe_env.push_term_scope(self.binding_scope([binding]));
                            }
                            stage_index += 1;
                        }
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Pipe(pipe),
                }
            }
            ExprKind::Cluster(cluster_id) => {
                self.resolve_cluster(cluster_id, namespaces, env);
                expr
            }
            ExprKind::Markup(node_id) => {
                self.resolve_markup_node(node_id, namespaces, env);
                expr
            }
        };
        *self
            .module
            .arenas
            .exprs
            .get_mut(expr_id)
            .expect("resolved expr id should still exist") = resolved;
    }

    fn resolve_cluster(
        &mut self,
        cluster_id: crate::ClusterId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let cluster = self.module.clusters()[cluster_id].clone();
        let spine = cluster.normalized_spine();
        for member in spine.apply_arguments() {
            self.resolve_expr(member, namespaces, env);
        }
        if let ApplicativeSpineHead::Expr(expr) = spine.pure_head() {
            self.resolve_expr(expr, namespaces, env);
        }
    }

    fn resolve_markup_node(
        &mut self,
        node_id: MarkupNodeId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let node = self.module.markup_nodes()[node_id].clone();
        match node.kind {
            MarkupNodeKind::Element(element) => {
                for attribute in &element.attributes {
                    match &attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            self.resolve_expr(*expr, namespaces, env)
                        }
                        MarkupAttributeValue::Text(text) => {
                            self.resolve_text_literal(text, namespaces, env)
                        }
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in &element.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            MarkupNodeKind::Control(control_id) => {
                self.resolve_control_node(control_id, namespaces, env)
            }
        }
    }

    fn resolve_control_node(
        &mut self,
        control_id: ControlNodeId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let control = self.module.control_nodes()[control_id].clone();
        match control {
            ControlNode::Show(node) => {
                self.resolve_expr(node.when, namespaces, env);
                if let Some(expr) = node.keep_mounted {
                    self.resolve_expr(expr, namespaces, env);
                }
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::Each(node) => {
                self.resolve_expr(node.collection, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope([node.binding]));
                if let Some(key) = node.key {
                    self.resolve_expr(key, namespaces, &child_env);
                }
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
                if let Some(empty) = node.empty {
                    self.resolve_control_node(empty, namespaces, env);
                }
            }
            ControlNode::Match(node) => {
                self.resolve_expr(node.scrutinee, namespaces, env);
                for case in node.cases.iter() {
                    self.resolve_control_node(*case, namespaces, env);
                }
            }
            ControlNode::Empty(node) => {
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::Case(node) => {
                let bindings = self.resolve_pattern(node.pattern, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope(bindings));
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
            }
            ControlNode::Fragment(node) => {
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::With(node) => {
                self.resolve_expr(node.value, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope([node.binding]));
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
            }
        }
    }

    fn resolve_pattern(
        &mut self,
        pattern_id: PatternId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) -> Vec<BindingId> {
        let pattern = self.module.patterns()[pattern_id].clone();
        let mut bindings = Vec::new();
        let resolved = match pattern.kind {
            PatternKind::Wildcard | PatternKind::Integer(_) => pattern,
            PatternKind::Text(text) => {
                self.resolve_text_literal(&text, namespaces, env);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Text(text),
                }
            }
            PatternKind::Binding(binding) => {
                bindings.push(binding.binding);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Binding(binding),
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements.iter() {
                    bindings.extend(self.resolve_pattern(*element, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Tuple(elements),
                }
            }
            PatternKind::List { elements, rest } => {
                for element in &elements {
                    bindings.extend(self.resolve_pattern(*element, namespaces, env));
                }
                if let Some(rest) = rest {
                    bindings.extend(self.resolve_pattern(rest, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::List { elements, rest },
                }
            }
            PatternKind::Record(fields) => {
                for field in &fields {
                    bindings.extend(self.resolve_pattern(field.pattern, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Record(fields),
                }
            }
            PatternKind::Constructor {
                mut callee,
                arguments,
            } => {
                self.resolve_term_reference(&mut callee, namespaces, env);
                for argument in &arguments {
                    bindings.extend(self.resolve_pattern(*argument, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Constructor { callee, arguments },
                }
            }
            PatternKind::UnresolvedName(mut reference) => {
                self.resolve_term_reference(&mut reference, namespaces, env);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::UnresolvedName(reference),
                }
            }
        };
        *self
            .module
            .arenas
            .patterns
            .get_mut(pattern_id)
            .expect("resolved pattern id should still exist") = resolved;
        bindings
    }

    fn resolve_text_literal(
        &mut self,
        text: &TextLiteral,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        for segment in &text.segments {
            if let TextSegment::Interpolation(interpolation) = segment {
                self.resolve_expr(interpolation.expr, namespaces, env);
            }
        }
    }

    fn resolve_type(&mut self, type_id: TypeId, namespaces: &Namespaces, env: &mut ResolveEnv) {
        let ty = self.module.types()[type_id].clone();
        let resolved = match ty.kind {
            TypeKind::Name(mut reference) => {
                self.resolve_type_reference(&mut reference, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Name(reference),
                }
            }
            TypeKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.resolve_type(*element, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Tuple(elements),
                }
            }
            TypeKind::Record(fields) => {
                for field in &fields {
                    self.resolve_type(field.ty, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(fields),
                }
            }
            TypeKind::RecordTransform { transform, source } => {
                self.resolve_type(source, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::RecordTransform { transform, source },
                }
            }
            TypeKind::Arrow { parameter, result } => {
                self.resolve_type(parameter, namespaces, env);
                self.resolve_type(result, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow { parameter, result },
                }
            }
            TypeKind::Apply { callee, arguments } => {
                self.resolve_type(callee, namespaces, env);
                for argument in arguments.iter() {
                    self.resolve_type(*argument, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Apply { callee, arguments },
                }
            }
        };
        *self
            .module
            .arenas
            .types
            .get_mut(type_id)
            .expect("resolved type id should still exist") = resolved;
    }

    fn resolve_term_reference(
        &mut self,
        reference: &mut TermReference,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        if reference.path.segments().len() != 1 {
            self.emit_error(
                reference.span(),
                format!(
                    "ordinary term reference `{}` is not supported in Milestone 2",
                    path_text(&reference.path)
                ),
                code("unsupported-qualified-term-ref"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        let name = reference.path.segments().first().text();
        if let Some(binding) = env.lookup_term(name) {
            reference.resolution = ResolutionState::Resolved(TermResolution::Local(binding));
            return;
        }
        if env.prefer_ambient_names() {
            match lookup_item(&namespaces.ambient_term_items, name) {
                LookupResult::Unique(item) => {
                    reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
                    return;
                }
                LookupResult::Ambiguous => {
                    self.emit_error(
                        reference.span(),
                        format!("ambient term `{name}` is ambiguous"),
                        code("ambiguous-term-name"),
                    );
                    reference.resolution = ResolutionState::Unresolved;
                    return;
                }
                LookupResult::Missing => {}
            }
            match lookup_item(&namespaces.ambient_class_terms, name) {
                LookupResult::Unique(resolution) => {
                    reference.resolution =
                        ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                    return;
                }
                LookupResult::Ambiguous => {
                    if let Some(candidates) = namespaces.ambient_class_terms.get(name)
                        && let Ok(candidates) = crate::NonEmpty::from_vec(
                            candidates
                                .iter()
                                .map(|site| site.value)
                                .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                        )
                    {
                        reference.resolution = ResolutionState::Resolved(
                            TermResolution::AmbiguousClassMembers(candidates),
                        );
                        return;
                    }
                }
                LookupResult::Missing => {}
            }
            if let Some(builtin) = builtin_term(name) {
                reference.resolution = ResolutionState::Resolved(TermResolution::Builtin(builtin));
                return;
            }
        }
        let term_lookup = lookup_item(&namespaces.term_items, name);
        let ambient_term_lookup = lookup_item(&namespaces.ambient_term_items, name);
        let domain_candidates = namespaces
            .domain_terms
            .get(name)
            .map(|candidates| {
                candidates
                    .iter()
                    .map(|candidate| candidate.value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if matches!(term_lookup, LookupResult::Ambiguous) {
            self.emit_error(
                reference.span(),
                format!("term `{name}` is ambiguous in this module"),
                code("ambiguous-term-name"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        if let LookupResult::Unique(item) = term_lookup {
            if !domain_candidates.is_empty() {
                self.emit_error(
                    reference.span(),
                    format!("term `{name}` is ambiguous in this module"),
                    code("ambiguous-term-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
            return;
        }
        if let LookupResult::Ambiguous = ambient_term_lookup {
            self.emit_error(
                reference.span(),
                format!("ambient term `{name}` is ambiguous"),
                code("ambiguous-term-name"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        if let LookupResult::Unique(item) = ambient_term_lookup {
            reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
            return;
        }
        let import_lookup = lookup_item(&namespaces.term_imports, name);
        if !domain_candidates.is_empty() {
            if !matches!(import_lookup, LookupResult::Missing) {
                self.emit_error(
                    reference.span(),
                    format!("term `{name}` is ambiguous in this module"),
                    code("ambiguous-term-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            if domain_candidates.len() == 1 {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::DomainMember(domain_candidates[0]));
                return;
            }
            if let Ok(candidates) = crate::NonEmpty::from_vec(domain_candidates) {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates));
                return;
            }
        }
        match import_lookup {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match &import_binding.metadata {
                    ImportBindingMetadata::BuiltinTerm(builtin) => {
                        ResolutionState::Resolved(TermResolution::Builtin(*builtin))
                    }
                    ImportBindingMetadata::IntrinsicValue { value, .. } => {
                        ResolutionState::Resolved(TermResolution::IntrinsicValue(*value))
                    }
                    ImportBindingMetadata::AmbientValue { name } => {
                        match lookup_item(&namespaces.ambient_term_items, name) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TermResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TermResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TermResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("imported term `{name}` is ambiguous"),
                    code("ambiguous-import-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        // Hoisted names (from `hoist` declarations) are consulted after explicit
        // `use` imports.  Unique → resolved as a normal import.  Multiple
        // candidates (same name from different hoisted modules) → deferred to
        // type-directed disambiguation at the type-checking layer.
        match lookup_item(&namespaces.hoisted_term_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match &import_binding.metadata {
                    ImportBindingMetadata::BuiltinTerm(builtin) => {
                        ResolutionState::Resolved(TermResolution::Builtin(*builtin))
                    }
                    ImportBindingMetadata::IntrinsicValue { value, .. } => {
                        ResolutionState::Resolved(TermResolution::IntrinsicValue(*value))
                    }
                    ImportBindingMetadata::AmbientValue { name } => {
                        match lookup_item(&namespaces.ambient_term_items, name) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TermResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TermResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TermResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.hoisted_term_imports.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates
                            .iter()
                            .map(|site| site.value)
                            .collect::<Vec<ImportId>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousHoistedImports(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.class_terms, name) {
            LookupResult::Unique(resolution) => {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.class_terms.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates
                            .iter()
                            .map(|site| site.value)
                            .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousClassMembers(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.ambient_class_terms, name) {
            LookupResult::Unique(resolution) => {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.ambient_class_terms.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates
                            .iter()
                            .map(|site| site.value)
                            .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousClassMembers(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        if let Some(builtin) = builtin_term(name) {
            reference.resolution = ResolutionState::Resolved(TermResolution::Builtin(builtin));
            return;
        }
        {
            let mut candidates: Vec<&str> = Vec::new();
            candidates.extend(
                env.term_scopes
                    .iter()
                    .flat_map(|scope| scope.keys().map(|k| k.as_str())),
            );
            candidates.extend(namespaces.term_items.keys().map(|k| k.as_str()));
            candidates.extend(namespaces.ambient_term_items.keys().map(|k| k.as_str()));
            candidates.extend(namespaces.term_imports.keys().map(|k| k.as_str()));
            let mut diag = Diagnostic::error(format!("unknown term `{name}`"))
                .with_code(code("unresolved-term-name"))
                .with_primary_label(reference.span(), "reported during Milestone 2 HIR lowering");
            if let Some(suggestion) = closest_name(name, &candidates) {
                diag = diag.with_help(format!("did you mean `{suggestion}`?"));
            }
            self.diagnostics.push(diag);
        }
        reference.resolution = ResolutionState::Unresolved;
    }

    fn resolve_type_reference(
        &mut self,
        reference: &mut TypeReference,
        namespaces: &Namespaces,
        env: &mut ResolveEnv,
    ) {
        if reference.path.segments().len() != 1 {
            self.emit_error(
                reference.span(),
                format!(
                    "ordinary type reference `{}` is not supported in Milestone 2",
                    path_text(&reference.path)
                ),
                code("unsupported-qualified-type-ref"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        let name = reference.path.segments().first().text();
        if let Some(parameter) = env.lookup_type(name) {
            reference.resolution =
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter));
            return;
        }
        if env.prefer_ambient_names() {
            match lookup_item(&namespaces.ambient_type_items, name) {
                LookupResult::Unique(item) => {
                    reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                    return;
                }
                LookupResult::Ambiguous => {
                    self.emit_error(
                        reference.span(),
                        format!("ambient type `{name}` is ambiguous"),
                        code("ambiguous-type-name"),
                    );
                    reference.resolution = ResolutionState::Unresolved;
                    return;
                }
                LookupResult::Missing => {}
            }
            if let Some(builtin) = builtin_type(name) {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Builtin(builtin));
                return;
            }
        }
        match lookup_item(&namespaces.type_items, name) {
            LookupResult::Unique(item) => {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("type `{name}` is ambiguous in this module"),
                    code("ambiguous-type-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.ambient_type_items, name) {
            LookupResult::Unique(item) => {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("ambient type `{name}` is ambiguous"),
                    code("ambiguous-type-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.type_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match import_binding.metadata {
                    ImportBindingMetadata::BuiltinType(builtin) => {
                        ResolutionState::Resolved(TypeResolution::Builtin(builtin))
                    }
                    ImportBindingMetadata::AmbientType => {
                        match lookup_item(
                            &namespaces.ambient_type_items,
                            import_binding.imported_name.text(),
                        ) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TypeResolution::Item(item))
                            }
                            LookupResult::Ambiguous => {
                                self.emit_error(
                                    reference.span(),
                                    format!(
                                        "ambient type `{}` is ambiguous",
                                        import_binding.imported_name.text()
                                    ),
                                    code("ambiguous-type-name"),
                                );
                                ResolutionState::Unresolved
                            }
                            LookupResult::Missing => {
                                self.emit_error(
                                    reference.span(),
                                    format!(
                                        "import `{}` resolved without an ambient type target",
                                        import_binding.imported_name.text()
                                    ),
                                    code("invalid-import-resolution"),
                                );
                                ResolutionState::Unresolved
                            }
                        }
                    }
                    _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("imported type `{name}` is ambiguous"),
                    code("ambiguous-import-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        // Hoisted type imports (from `hoist` declarations): consulted after
        // explicit `use` type imports but before builtins.
        match lookup_item(&namespaces.hoisted_type_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match import_binding.metadata {
                    ImportBindingMetadata::BuiltinType(builtin) => {
                        ResolutionState::Resolved(TypeResolution::Builtin(builtin))
                    }
                    ImportBindingMetadata::AmbientType => {
                        match lookup_item(
                            &namespaces.ambient_type_items,
                            import_binding.imported_name.text(),
                        ) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TypeResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                // Multiple hoisted modules export the same type name.  Report an
                // error and suggest narrowing with `hiding`.
                if let Some(candidates) = namespaces.hoisted_type_imports.get(name) {
                    let modules = candidates
                        .iter()
                        .filter_map(|site| {
                            self.module
                                .imports()
                                .get(site.value)
                                .map(|b| b.imported_name.text().to_owned())
                        })
                        .collect::<Vec<_>>()
                        .join("`, `");
                    self.emit_error(
                        reference.span(),
                        format!(
                            "hoisted type `{name}` is ambiguous — it is exported by multiple \
                             hoisted modules (`{modules}`); add a `hiding` clause to the relevant \
                             `hoist` declarations to resolve the conflict",
                        ),
                        code("ambiguous-hoisted-type"),
                    );
                }
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        if let Some(builtin) = builtin_type(name) {
            reference.resolution = ResolutionState::Resolved(TypeResolution::Builtin(builtin));
            return;
        }
        if is_implicit_type_parameter_candidate(name) && env.allow_implicit_type_parameters() {
            let parameter = env.bind_implicit_type_parameter(name, reference.span(), self);
            reference.resolution =
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter));
            return;
        }
        self.emit_error(
            reference.span(),
            format!("unknown type `{name}`"),
            code("unresolved-type-name"),
        );
        reference.resolution = ResolutionState::Unresolved;
    }

    fn resolve_export_target(
        &mut self,
        target: &NamePath,
        namespaces: &Namespaces,
    ) -> ResolutionState<ExportResolution> {
        if let Some(resolution) =
            self.resolve_export_item_target(target, &namespaces.term_items, &namespaces.type_items)
        {
            return resolution;
        }
        if let Some(resolution) = self.resolve_export_item_target(
            target,
            &namespaces.ambient_term_items,
            &namespaces.ambient_type_items,
        ) {
            return resolution;
        }

        let name = target.segments().first().text();

        // Allow re-exporting imported names (e.g. intrinsics forwarded through
        // a module's own `use` declaration).
        if let Some(resolution) = self.resolve_export_import_target(
            target,
            &namespaces.term_imports,
            &namespaces.type_imports,
        ) {
            return resolution;
        }

        if let Some(builtin) = builtin_term(name) {
            return ResolutionState::Resolved(ExportResolution::BuiltinTerm(builtin));
        }
        if let Some(builtin) = builtin_type(name) {
            return ResolutionState::Resolved(ExportResolution::BuiltinType(builtin));
        }

        self.emit_error(
            target.span(),
            format!("cannot export unknown item `{}`", path_text(target)),
            code("unknown-export-target"),
        );
        ResolutionState::Unresolved
    }

    fn resolve_export_import_target(
        &mut self,
        target: &NamePath,
        term_imports: &HashMap<String, Vec<NamedSite<ImportId>>>,
        type_imports: &HashMap<String, Vec<NamedSite<ImportId>>>,
    ) -> Option<ResolutionState<ExportResolution>> {
        let name = target.segments().first().text();
        let mut candidates: Vec<ImportId> = Vec::new();

        match lookup_item(term_imports, name) {
            LookupResult::Unique(import_id) => candidates.push(import_id),
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }
        match lookup_item(type_imports, name) {
            LookupResult::Unique(import_id) => {
                if !candidates.contains(&import_id) {
                    candidates.push(import_id);
                }
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match candidates.as_slice() {
            [import_id] => Some(ResolutionState::Resolved(ExportResolution::Import(
                *import_id,
            ))),
            [] => None,
            _ => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                Some(ResolutionState::Unresolved)
            }
        }
    }

    fn resolve_export_item_target(
        &mut self,
        target: &NamePath,
        term_items: &HashMap<String, Vec<NamedSite<ItemId>>>,
        type_items: &HashMap<String, Vec<NamedSite<ItemId>>>,
    ) -> Option<ResolutionState<ExportResolution>> {
        let name = target.segments().first().text();
        let mut candidates = Vec::new();

        match lookup_item(term_items, name) {
            LookupResult::Unique(item) => candidates.push(item),
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match lookup_item(type_items, name) {
            LookupResult::Unique(item) => {
                if !candidates.contains(&item) {
                    candidates.push(item);
                }
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match candidates.as_slice() {
            [item] => Some(ResolutionState::Resolved(ExportResolution::Item(*item))),
            [] => None,
            _ => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                Some(ResolutionState::Unresolved)
            }
        }
    }

    fn resolve_literal_suffix(
        &mut self,
        suffix: &Name,
        namespaces: &Namespaces,
    ) -> ResolutionState<LiteralSuffixResolution> {
        if suffix.text().chars().count() < 2 {
            self.emit_error(
                suffix.span(),
                format!(
                    "literal suffix `{}` is too short; domain literal suffixes must be at least two characters long",
                    suffix.text()
                ),
                code("literal-suffix-too-short"),
            );
            return ResolutionState::Unresolved;
        }
        let local_root_candidates: Vec<_> = namespaces
            .literal_suffixes
            .get(suffix.text())
            .into_iter()
            .flatten()
            .copied()
            .filter(|candidate| self.module.root_items().contains(&candidate.value.domain))
            .collect();
        if !local_root_candidates.is_empty() {
            return self.finish_literal_suffix_resolution(suffix, &local_root_candidates);
        }

        let imported_candidates: Vec<_> = namespaces
            .literal_suffixes
            .get(suffix.text())
            .into_iter()
            .flatten()
            .copied()
            .filter(|candidate| !self.module.root_items().contains(&candidate.value.domain))
            .collect();
        if !imported_candidates.is_empty() {
            return self.finish_literal_suffix_resolution(suffix, &imported_candidates);
        }

        let Some(candidates) = namespaces.ambient_literal_suffixes.get(suffix.text()) else {
            self.emit_error(
                suffix.span(),
                format!("unknown literal suffix `{}`", suffix.text()),
                code("unknown-literal-suffix"),
            );
            return ResolutionState::Unresolved;
        };

        self.finish_literal_suffix_resolution(suffix, candidates)
    }

    fn finish_literal_suffix_resolution(
        &mut self,
        suffix: &Name,
        candidates: &[NamedSite<LiteralSuffixResolution>],
    ) -> ResolutionState<LiteralSuffixResolution> {
        if candidates.len() > 1 {
            let mut diagnostic =
                Diagnostic::error(format!("literal suffix `{}` is ambiguous", suffix.text()))
                    .with_code(code("ambiguous-literal-suffix"))
                    .with_primary_label(
                        suffix.span(),
                        "this suffixed literal matches multiple domain literal declarations",
                    );
            for candidate in candidates.iter().take(3) {
                diagnostic = diagnostic
                    .with_secondary_label(candidate.span, "matching literal suffix declared here");
            }
            self.diagnostics.push(diagnostic);
            return ResolutionState::Unresolved;
        }

        ResolutionState::Resolved(candidates[0].value)
    }

    fn type_contains_item_reference(&self, root: TypeId, target: ItemId) -> bool {
        let mut stack = vec![root];
        while let Some(type_id) = stack.pop() {
            let ty = &self.module.types()[type_id];
            match &ty.kind {
                TypeKind::Name(reference) => {
                    if matches!(
                        reference.resolution,
                        ResolutionState::Resolved(TypeResolution::Item(item_id)) if item_id == target
                    ) {
                        return true;
                    }
                }
                TypeKind::Tuple(elements) => {
                    stack.extend(elements.iter().copied());
                }
                TypeKind::Record(fields) => {
                    stack.extend(fields.iter().map(|field| field.ty));
                }
                TypeKind::RecordTransform { source, .. } => {
                    stack.push(*source);
                }
                TypeKind::Arrow { parameter, result } => {
                    stack.push(*parameter);
                    stack.push(*result);
                }
                TypeKind::Apply { callee, arguments } => {
                    stack.push(*callee);
                    stack.extend(arguments.iter().copied());
                }
            }
        }
        false
    }

    fn binding_scope<I>(&self, bindings: I) -> HashMap<String, BindingId>
    where
        I: IntoIterator<Item = BindingId>,
    {
        bindings
            .into_iter()
            .map(|binding| {
                let binding_name = self.module.bindings()[binding].name.text().to_owned();
                (binding_name, binding)
            })
            .collect()
    }

    fn type_parameter_scope<I>(&self, parameters: I) -> HashMap<String, TypeParameterId>
    where
        I: IntoIterator<Item = TypeParameterId>,
    {
        parameters
            .into_iter()
            .map(|parameter| {
                let parameter_name = self.module.type_parameters()[parameter]
                    .name
                    .text()
                    .to_owned();
                (parameter_name, parameter)
            })
            .collect()
    }

    fn placeholder_expr(&mut self, span: SourceSpan) -> ExprId {
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::unresolved(
                self.make_path(&[self.make_name("invalid", span)]),
            )),
        })
    }

    fn placeholder_type(&mut self, span: SourceSpan) -> TypeId {
        self.alloc_type(TypeNode {
            span,
            kind: TypeKind::Name(TypeReference {
                path: self.make_path(&[self.make_name("Unit", span)]),
                resolution: ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Unit)),
            }),
        })
    }

    fn placeholder_pattern(&mut self, span: SourceSpan) -> PatternId {
        self.alloc_pattern(Pattern {
            span,
            kind: PatternKind::Wildcard,
        })
    }

    fn make_name(&self, text: &str, span: SourceSpan) -> Name {
        Name::new(text.to_owned(), span).expect("non-empty lowered names should always be valid")
    }

    fn make_path(&self, names: &[Name]) -> NamePath {
        NamePath::from_vec(names.to_vec())
            .expect("non-empty same-file paths should always be valid")
    }

    fn emit_error(
        &mut self,
        span: SourceSpan,
        message: impl Into<String>,
        error_code: DiagnosticCode,
    ) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(error_code)
                .with_primary_label(span, "reported during Milestone 2 HIR lowering"),
        );
    }

    fn emit_warning(
        &mut self,
        span: SourceSpan,
        message: impl Into<String>,
        warning_code: DiagnosticCode,
    ) {
        self.diagnostics.push(
            Diagnostic::warning(message)
                .with_code(warning_code)
                .with_primary_label(span, "reported during Milestone 2 HIR lowering"),
        );
    }

    fn emit_arena_overflow(&mut self, arena_name: &str) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "program too large: arena capacity exceeded ({})",
                arena_name
            ))
            .with_code(code("arena-overflow")),
        );
    }

    fn alloc_expr(&mut self, expr: Expr) -> ExprId {
        self.module.alloc_expr(expr).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR expr arena");
            std::process::exit(1);
        })
    }

    fn alloc_pattern(&mut self, pattern: Pattern) -> PatternId {
        self.module.alloc_pattern(pattern).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR pattern arena");
            std::process::exit(1);
        })
    }

    fn alloc_type(&mut self, ty: TypeNode) -> TypeId {
        self.module.alloc_type(ty).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR type arena");
            std::process::exit(1);
        })
    }

    fn alloc_decorator(&mut self, decorator: Decorator) -> DecoratorId {
        self.module.alloc_decorator(decorator).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR decorator arena");
            std::process::exit(1);
        })
    }

    fn alloc_markup_node(&mut self, node: MarkupNode) -> MarkupNodeId {
        self.module.alloc_markup_node(node).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR markup arena");
            std::process::exit(1);
        })
    }

    fn alloc_control(&mut self, control: ControlNode) -> ControlNodeId {
        self.module.alloc_control_node(control).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR control arena");
            std::process::exit(1);
        })
    }

    fn wrap_control(&mut self, control: ControlNode) -> MarkupNodeId {
        let span = control.span();
        let control_id = self.alloc_control(control);
        self.alloc_markup_node(MarkupNode {
            span,
            kind: MarkupNodeKind::Control(control_id),
        })
    }

    fn alloc_cluster(&mut self, cluster: ApplicativeCluster) -> crate::ClusterId {
        self.module.alloc_cluster(cluster).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR cluster arena");
            std::process::exit(1);
        })
    }

    fn alloc_binding(&mut self, binding: Binding) -> BindingId {
        self.module.alloc_binding(binding).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR binding arena");
            std::process::exit(1);
        })
    }

    fn alloc_type_parameter(&mut self, parameter: TypeParameter) -> TypeParameterId {
        self.module
            .alloc_type_parameter(parameter)
            .unwrap_or_else(|_| {
                self.emit_arena_overflow("HIR type parameter arena");
                std::process::exit(1);
            })
    }

    fn alloc_import(&mut self, import: ImportBinding) -> ImportId {
        self.module.alloc_import(import).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR import arena");
            std::process::exit(1);
        })
    }
}
