struct TypeChecker<'a> {
    module: &'a Module,
    typing: GateTypeContext<'a>,
    diagnostics: Vec<Diagnostic>,
    option_default_in_scope: bool,
    imported_default_values: Vec<ImportedDefaultValue>,
    default_record_elisions: Vec<DefaultRecordElision>,
    pending_eq_constraints: Vec<PendingEqConstraint>,
    /// Eq-like constraints available in the current checking scope after expanding
    /// any in-scope class evidence through `with` / `require`.
    eq_constrained_parameters: HashSet<TypeParameterId>,
    in_scope_class_constraints: Vec<ClassConstraintBinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOperatorExpectation {
    BoolOperands,
    MatchingNumericOperands,
    CommonTypeOperands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResultBlockCaseRun {
    ok_pattern: PatternId,
    ok_body: ExprId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ResultBindingShape {
    Result {
        error: Option<GateType>,
        value: Option<GateType>,
    },
    NonResult {
        actual: GateType,
    },
    Unknown,
}

pub(crate) fn collect_contextual_function_signature_evidence(
    module: &Module,
    _function_ids: &[ItemId],
    typing: GateTypeContext<'_>,
    function_set: &HashSet<ItemId>,
) -> Vec<FunctionSignatureEvidence> {
    let mut checker = TypeChecker::with_typing(module, typing);
    checker.run();
    checker
        .typing
        .take_function_signature_evidence()
        .into_iter()
        .filter(|evidence| function_set.contains(&evidence.item_id))
        .collect()
}

impl<'a> TypeChecker<'a> {
    fn new(module: &'a Module) -> Self {
        let (option_default_in_scope, imported_default_values) =
            Self::collect_default_imports(module);
        Self {
            module,
            typing: GateTypeContext::new(module),
            diagnostics: Vec::new(),
            option_default_in_scope,
            imported_default_values,
            default_record_elisions: Vec::new(),
            pending_eq_constraints: Vec::new(),
            eq_constrained_parameters: HashSet::new(),
            in_scope_class_constraints: Vec::new(),
        }
    }

    fn with_typing(module: &'a Module, typing: GateTypeContext<'a>) -> Self {
        let (option_default_in_scope, imported_default_values) =
            Self::collect_default_imports(module);
        Self {
            module,
            typing,
            diagnostics: Vec::new(),
            option_default_in_scope,
            imported_default_values,
            default_record_elisions: Vec::new(),
            pending_eq_constraints: Vec::new(),
            eq_constrained_parameters: HashSet::new(),
            in_scope_class_constraints: Vec::new(),
        }
    }

    fn run(&mut self) {
        let items = self
            .module
            .items()
            .iter()
            .map(|(item_id, item)| (item_id, item.clone()))
            .collect::<Vec<_>>();
        for (item_id, item) in items {
            match item {
                Item::Value(item) => self.check_value_item(&item),
                Item::Function(item) => self.check_function_item(item_id, &item),
                Item::Signal(item) => self.check_signal_item(&item),
                Item::Instance(item) => self.check_instance_item(&item),
                Item::Domain(item) => self.check_domain_item(item_id, &item),
                Item::Type(_)
                | Item::Class(_)
                | Item::SourceProviderContract(_)
                | Item::Use(_)
                | Item::Export(_)
                | Item::Hoist(_) => {}
            }
        }
        self.solve_pending_eq_constraints();
    }

    fn collect_default_imports(module: &Module) -> (bool, Vec<ImportedDefaultValue>) {
        let mut option_default_in_scope = false;
        let mut imported_default_values = Vec::new();
        for item_id in module.root_items().iter().copied() {
            let Some(Item::Use(use_item)) = module.items().get(item_id) else {
                continue;
            };
            let is_defaults_module = {
                let segments = use_item.module.segments();
                let mut seg_iter = segments.iter();
                seg_iter.next().is_some_and(|s| s.text() == "aivi")
                    && seg_iter.next().is_some_and(|s| s.text() == "defaults")
                    && seg_iter.next().is_none()
            };
            for import_id in use_item.imports.iter().copied() {
                let import = &module.imports()[import_id];
                match import.imported_name.text() {
                    "Option"
                        if matches!(
                            &import.metadata,
                            ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
                        ) || (is_defaults_module
                            && matches!(
                                &import.metadata,
                                ImportBindingMetadata::BuiltinType(BuiltinType::Option)
                            )) =>
                    {
                        option_default_in_scope = true;
                    }
                    "defaultText"
                        if Self::import_binding_has_primitive_type(import, BuiltinType::Text) =>
                    {
                        imported_default_values.push(ImportedDefaultValue {
                            builtin: BuiltinType::Text,
                            import: import_id,
                        });
                    }
                    "defaultInt"
                        if Self::import_binding_has_primitive_type(import, BuiltinType::Int) =>
                    {
                        imported_default_values.push(ImportedDefaultValue {
                            builtin: BuiltinType::Int,
                            import: import_id,
                        });
                    }
                    "defaultBool"
                        if Self::import_binding_has_primitive_type(import, BuiltinType::Bool) =>
                    {
                        imported_default_values.push(ImportedDefaultValue {
                            builtin: BuiltinType::Bool,
                            import: import_id,
                        });
                    }
                    _ => {}
                }
            }
        }
        (option_default_in_scope, imported_default_values)
    }

    fn import_binding_has_primitive_type(
        import: &crate::ImportBinding,
        builtin: BuiltinType,
    ) -> bool {
        matches!(
            &import.metadata,
            ImportBindingMetadata::Value {
                ty: crate::ImportValueType::Primitive(found),
            } if *found == builtin
        )
    }

    fn check_value_item(&mut self, item: &ValueItem) {
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        self.check_expr(
            item.body,
            &GateExprEnv::default(),
            expected.as_ref(),
            &mut Vec::new(),
        );
    }

    fn check_function_item(&mut self, item_id: ItemId, item: &FunctionItem) {
        let context = self.constraint_bindings(&item.context, &PolyTypeBindings::new());
        self.with_class_constraint_scope(context, |this| {
            let inferred_signature = supports_same_module_function_inference(item)
                .then(|| this.typing.item_value_type(item_id))
                .flatten();
            let inferred_parts = inferred_signature
                .as_ref()
                .and_then(|ty| this.expected_function_signature(ty, item.parameters.len()));
            let mut env = GateExprEnv::default();
            for (index, parameter) in item.parameters.iter().enumerate() {
                let Some(parameter_ty) = parameter
                    .annotation
                    .and_then(|annotation| this.typing.lower_open_annotation(annotation))
                    .or_else(|| {
                        inferred_parts
                            .as_ref()
                            .and_then(|(parameter_types, _)| parameter_types.get(index).cloned())
                    })
                else {
                    continue;
                };
                env.locals.insert(parameter.binding, parameter_ty);
            }
            let expected = item
                .annotation
                .and_then(|annotation| this.typing.lower_open_annotation(annotation))
                .or_else(|| inferred_parts.as_ref().map(|(_, result)| result.clone()));
            this.check_expr(item.body, &env, expected.as_ref(), &mut Vec::new());
        });
    }

    fn check_signal_item(&mut self, item: &SignalItem) {
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        if let Some(body) = item.body {
            match expected.as_ref() {
                Some(annotation @ GateType::Signal(payload)) => {
                    let checkpoint = self.diagnostics.len();
                    if self.check_expr(
                        body,
                        &GateExprEnv::default(),
                        Some(annotation),
                        &mut Vec::new(),
                    ) {
                        self.check_signal_reactive_updates(item, Some(payload.as_ref()));
                        return;
                    }
                    self.diagnostics.truncate(checkpoint);
                    self.check_expr(
                        body,
                        &GateExprEnv::default(),
                        Some(payload.as_ref()),
                        &mut Vec::new(),
                    );
                    self.check_signal_reactive_updates(item, Some(payload.as_ref()));
                }
                Some(annotation) => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "signal bodies require a `Signal A` annotation, found `{annotation}`"
                        ))
                        .with_code(code("invalid-signal-annotation"))
                        .with_primary_label(
                            self.module.exprs()[body].span,
                            "this signal body is checked against the payload type of `Signal A`",
                        ),
                    );
                    self.check_signal_reactive_updates(item, None);
                }
                None => {
                    self.check_inferred_expr(body, &GateExprEnv::default(), None);
                    let inferred_payload = self.inferred_expr_type(body, &GateExprEnv::default());
                    self.check_signal_reactive_updates(item, inferred_payload.as_ref());
                }
            }
            return;
        }
        self.check_signal_reactive_updates(item, signal_annotation_payload(expected.as_ref()));
    }

    fn check_signal_reactive_updates(
        &mut self,
        item: &SignalItem,
        expected_payload: Option<&GateType>,
    ) {
        if item.reactive_updates.is_empty() {
            return;
        }
        let Some(expected_payload) = expected_payload else {
            if let Some(annotation) = item
                .annotation
                .and_then(|annotation| self.typing.lower_annotation(annotation))
                && !annotation.is_signal()
            {
                let update = &item.reactive_updates[0];
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "reactive update targets require a `Signal A` annotation, found `{annotation}`"
                    ))
                    .with_code(code("invalid-reactive-update-target-type"))
                    .with_primary_label(
                        self.module.exprs()[update.body].span,
                        "this reactive update body is checked against the payload type of `Signal A`",
                    ),
                );
            } else {
                let update = &item.reactive_updates[0];
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "reactive update target `{}` needs a known payload type",
                        item.name.text()
                    ))
                    .with_code(code("missing-reactive-update-target-type"))
                    .with_primary_label(
                        self.module.exprs()[update.body].span,
                        "declare the target as `Signal A` or give it an initial body before updating it",
                    ),
                );
            }
            return;
        };
        let bool_ty = GateType::Primitive(BuiltinType::Bool);
        for update in &item.reactive_updates {
            let expected_body = match update.body_mode {
                ReactiveUpdateBodyMode::Payload => expected_payload.clone(),
                ReactiveUpdateBodyMode::OptionalPayload => {
                    GateType::Option(Box::new(expected_payload.clone()))
                }
            };
            let signal_bool_ty = GateType::Signal(Box::new(bool_ty.clone()));
            let signal_body_ty = GateType::Signal(Box::new(expected_body.clone()));
            let guard_expected = if update.body_mode == ReactiveUpdateBodyMode::OptionalPayload
                && expression_matches(
                    self.module,
                    update.guard,
                    &GateExprEnv::default(),
                    &signal_bool_ty,
                ) {
                &signal_bool_ty
            } else {
                &bool_ty
            };
            let body_expected = if update.body_mode == ReactiveUpdateBodyMode::OptionalPayload
                && expression_matches(
                    self.module,
                    update.body,
                    &GateExprEnv::default(),
                    &signal_body_ty,
                ) {
                &signal_body_ty
            } else {
                &expected_body
            };
            self.check_expr(
                update.guard,
                &GateExprEnv::default(),
                Some(guard_expected),
                &mut Vec::new(),
            );
            self.check_expr(
                update.body,
                &GateExprEnv::default(),
                Some(body_expected),
                &mut Vec::new(),
            );
        }
    }

    fn check_instance_item(&mut self, item: &InstanceItem) {
        let Some(class_item_id) = self.instance_class_item_id(item) else {
            return;
        };
        let Some(argument_bindings) = self.instance_argument_bindings(class_item_id, item) else {
            return;
        };
        let expected_members = match &self.module.items()[class_item_id] {
            Item::Class(class_item) => class_item
                .members
                .iter()
                .map(|member| (member.name.text().to_owned(), member.annotation))
                .collect::<HashMap<_, _>>(),
            _ => return,
        };
        let instance_context = self.constraint_bindings(&item.context, &PolyTypeBindings::new());
        let class_requirement_seeds =
            self.class_requirement_bindings(class_item_id, &argument_bindings);
        let class_requirements = self.expand_class_constraint_bindings(class_requirement_seeds);
        self.with_class_constraint_scope(instance_context.clone(), |this| {
            for requirement in &class_requirements {
                if let Err(reason) = this.require_class_binding(requirement) {
                    this.emit_missing_instance_requirement(
                        item.header.span,
                        class_item_id,
                        requirement,
                        &reason,
                    );
                }
            }
        });
        let mut body_constraints = instance_context;
        body_constraints.extend(class_requirements);
        self.with_class_constraint_scope(body_constraints, |this| {
            for member in &item.members {
                let Some(annotation) = expected_members.get(member.name.text()).copied() else {
                    continue;
                };
                let Some(expected) = this
                    .typing
                    .instantiate_poly_hir_type(annotation, &argument_bindings)
                else {
                    continue;
                };
                this.check_instance_member(member, &expected);
            }
        });
    }

    fn check_instance_member(&mut self, member: &InstanceMember, expected: &GateType) {
        let mut env = GateExprEnv::default();
        let mut current = expected.clone();
        for parameter in &member.parameters {
            let GateType::Arrow {
                parameter: parameter_ty,
                result,
            } = current
            else {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "instance member `{}` takes more parameters than its class signature allows",
                        member.name.text()
                    ))
                    .with_code(code("instance-member-arity-mismatch"))
                    .with_primary_label(
                        member.span,
                        "remove parameters or widen the class member signature",
                    ),
                );
                return;
            };
            env.locals
                .insert(parameter.binding, parameter_ty.as_ref().clone());
            current = *result;
        }
        self.check_expr(member.body, &env, Some(&current), &mut Vec::new());
    }

    fn check_domain_item(&mut self, owner: ItemId, item: &crate::DomainItem) {
        let Some(carrier) = self.typing.lower_open_annotation(item.carrier) else {
            return;
        };
        for member in &item.members {
            let Some(body) = member.body else {
                continue;
            };
            let Some(surface) = self.typing.lower_open_annotation(member.annotation) else {
                continue;
            };
            let expected = rewrite_domain_carrier_view(&surface, owner, &item.parameters, &carrier);
            self.check_domain_member(member, body, &expected);
        }
    }

    fn check_domain_member(&mut self, member: &DomainMember, body: ExprId, expected: &GateType) {
        let mut env = GateExprEnv::default();
        let mut current = expected.clone();
        for parameter in &member.parameters {
            let GateType::Arrow {
                parameter: parameter_ty,
                result,
            } = current
            else {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "domain member `{}` takes more parameters than its declaration allows",
                        member.name.text()
                    ))
                    .with_code(code("domain-member-arity-mismatch"))
                    .with_primary_label(
                        member.span,
                        "remove parameters or widen the domain member declaration",
                    ),
                );
                return;
            };
            env.locals
                .insert(parameter.binding, parameter_ty.as_ref().clone());
            current = *result;
        }
        self.check_expr(body, &env, Some(&current), &mut Vec::new());
    }

    fn with_class_constraint_scope<T>(
        &mut self,
        seeds: Vec<ClassConstraintBinding>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let expanded = self.expand_class_constraint_bindings(seeds);
        let eq_context = self.eq_constrained_parameters_from_bindings(&expanded);
        let prev_constraints =
            std::mem::replace(&mut self.in_scope_class_constraints, expanded.clone());
        let prev_eq_context = std::mem::replace(&mut self.eq_constrained_parameters, eq_context);
        let result = f(self);
        self.eq_constrained_parameters = prev_eq_context;
        self.in_scope_class_constraints = prev_constraints;
        result
    }

    fn constraint_bindings(
        &mut self,
        constraints: &[TypeId],
        bindings: &PolyTypeBindings,
    ) -> Vec<ClassConstraintBinding> {
        constraints
            .iter()
            .filter_map(|constraint| {
                self.typing
                    .open_class_constraint_binding(*constraint, bindings)
            })
            .collect()
    }

    fn class_requirement_bindings(
        &mut self,
        class_item_id: ItemId,
        bindings: &PolyTypeBindings,
    ) -> Vec<ClassConstraintBinding> {
        let Item::Class(class_item) = &self.module.items()[class_item_id] else {
            return Vec::new();
        };
        class_item
            .superclasses
            .iter()
            .chain(class_item.param_constraints.iter())
            .filter_map(|constraint| {
                self.typing
                    .open_class_constraint_binding(*constraint, bindings)
            })
            .collect()
    }

    fn expand_class_constraint_bindings(
        &mut self,
        seeds: Vec<ClassConstraintBinding>,
    ) -> Vec<ClassConstraintBinding> {
        let mut expanded = Vec::new();
        let mut pending = seeds;
        while let Some(binding) = pending.pop() {
            if expanded.contains(&binding) {
                continue;
            }
            for implied in self.implied_class_constraints(&binding) {
                if !expanded.contains(&implied) && !pending.contains(&implied) {
                    pending.push(implied);
                }
            }
            expanded.push(binding);
        }
        expanded
    }

    fn implied_class_constraints(
        &mut self,
        binding: &ClassConstraintBinding,
    ) -> Vec<ClassConstraintBinding> {
        let Item::Class(class_item) = &self.module.items()[binding.class_item] else {
            return Vec::new();
        };
        let substitutions =
            std::iter::once((*class_item.parameters.first(), binding.subject.clone()))
                .collect::<PolyTypeBindings>();
        class_item
            .superclasses
            .iter()
            .chain(class_item.param_constraints.iter())
            .filter_map(|constraint| {
                self.typing
                    .open_class_constraint_binding(*constraint, &substitutions)
            })
            .collect()
    }

    fn eq_constrained_parameters_from_bindings(
        &self,
        bindings: &[ClassConstraintBinding],
    ) -> HashSet<TypeParameterId> {
        bindings
            .iter()
            .filter(|binding| {
                matches!(
                    self.class_name(binding.class_item),
                    Some("Eq") | Some("Setoid")
                )
            })
            .filter_map(|binding| match &binding.subject {
                TypeBinding::Type(GateType::TypeParameter { parameter, .. }) => Some(*parameter),
                _ => None,
            })
            .collect()
    }

    fn check_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        match self.module.exprs()[expr_id].kind.clone() {
            ExprKind::PatchApply { target, patch } => {
                return self.check_patch_apply_expr(
                    expr_id,
                    target,
                    &patch,
                    env,
                    expected,
                    value_stack,
                );
            }
            ExprKind::PatchLiteral(patch) => {
                return match expected {
                    Some(expected) => {
                        self.check_patch_literal_expr(expr_id, &patch, env, expected, value_stack)
                    }
                    None => self.check_patch_block_children(&patch, env, value_stack),
                };
            }
            _ => {}
        }

        if let Some(result) = self.check_operator_expr(expr_id, env, expected, value_stack) {
            return result;
        }

        if let Some(result) = self.check_result_block_pipe_expr(expr_id, env, expected, value_stack)
        {
            return result;
        }

        if let Some(expected) = expected
            && let Some(result) =
                self.check_expected_special_case(expr_id, env, expected, value_stack)
            {
                return result;
            }

        self.check_inferred_expr(expr_id, env, expected)
    }

    fn check_expr_with_ambient(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        ambient: &GateType,
    ) -> bool {
        let info = self.typing.infer_expr(expr_id, env, Some(ambient));
        self.emit_expr_issues(&info.issues);
        self.handle_constraints(&info.constraints);

        match (expected, info.ty.as_ref()) {
            (Some(expected), Some(actual)) if actual.same_shape(expected) => true,
            (Some(expected), Some(actual)) => {
                self.emit_type_mismatch(self.module.exprs()[expr_id].span, expected, actual);
                false
            }
            (Some(_), None) => false,
            (None, Some(_)) | (None, None) => true,
        }
    }

    fn expr_matches_with_ambient(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: &GateType,
        ambient: &GateType,
    ) -> bool {
        let info = self.typing.infer_expr(expr_id, env, Some(ambient));
        self.enqueue_eq_constraints(&info.constraints);
        info.issues.is_empty()
            && info
                .ty
                .as_ref()
                .is_some_and(|actual| actual.same_shape(expected))
    }

    fn check_inferred_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
    ) -> bool {
        let info = self.typing.infer_expr(expr_id, env, None);
        self.emit_expr_issues(&info.issues);
        self.handle_constraints(&info.constraints);

        match (expected, info.ty.as_ref()) {
            (Some(expected), Some(actual)) if actual.same_shape(expected) => true,
            (Some(expected), Some(actual)) => {
                self.emit_type_mismatch(self.module.exprs()[expr_id].span, expected, actual);
                false
            }
            (Some(_), None) => false,
            (None, Some(_)) | (None, None) => true,
        }
    }

    fn check_operator_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let kind = self.module.exprs()[expr_id].kind.clone();
        match kind {
            ExprKind::Unary { operator, expr } => {
                Some(self.check_unary_expr(expr_id, operator, expr, env, expected, value_stack))
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => Some(self.check_binary_expr(
                expr_id,
                left,
                operator,
                right,
                env,
                expected,
                value_stack,
            )),
            _ => None,
        }
    }

    fn check_unary_expr(
        &mut self,
        expr_id: ExprId,
        operator: UnaryOperator,
        expr: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let bool_ty = GateType::Primitive(BuiltinType::Bool);
        let actual = self.inferred_expr_type(expr, env);
        let checkpoint = self.diagnostics.len();
        let operand_ok = self.check_expr(expr, env, Some(&bool_ty), value_stack);
        if !operand_ok {
            if self.diagnostics.len() == checkpoint {
                self.emit_invalid_unary_operator(
                    self.module.exprs()[expr_id].span,
                    operator,
                    actual.as_ref(),
                );
            }
            return false;
        }
        self.check_result_type(expr_id, expected, &bool_ty)
    }

    fn check_binary_expr(
        &mut self,
        expr_id: ExprId,
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        match operator {
            BinaryOperator::And | BinaryOperator::Or => self.check_bool_binary_expr(
                expr_id,
                left,
                operator,
                right,
                env,
                expected,
                value_stack,
            ),
            BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
            | BinaryOperator::GreaterThan
            | BinaryOperator::LessThan
            | BinaryOperator::GreaterThanOrEqual
            | BinaryOperator::LessThanOrEqual => self.check_numeric_binary_expr(
                expr_id,
                left,
                operator,
                right,
                env,
                expected,
                value_stack,
            ),
            BinaryOperator::Equals | BinaryOperator::NotEquals => self.check_equality_binary_expr(
                expr_id,
                left,
                operator,
                right,
                env,
                expected,
                value_stack,
            ),
        }
    }

    fn check_bool_binary_expr(
        &mut self,
        expr_id: ExprId,
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let bool_ty = GateType::Primitive(BuiltinType::Bool);
        let left_actual = self.inferred_expr_type(left, env);
        let right_actual = self.inferred_expr_type(right, env);
        let checkpoint = self.diagnostics.len();
        let left_ok = self.check_expr(left, env, Some(&bool_ty), value_stack);
        let right_ok = self.check_expr(right, env, Some(&bool_ty), value_stack);
        if !left_ok || !right_ok {
            if self.diagnostics.len() == checkpoint {
                self.emit_invalid_binary_operator(
                    self.module.exprs()[expr_id].span,
                    operator,
                    left_actual.as_ref(),
                    right_actual.as_ref(),
                    BinaryOperatorExpectation::BoolOperands,
                );
            }
            return false;
        }
        self.check_result_type(expr_id, expected, &bool_ty)
    }

    fn check_numeric_binary_expr(
        &mut self,
        expr_id: ExprId,
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let left_actual = self.inferred_expr_type(left, env);
        let right_actual = self.inferred_expr_type(right, env);
        if let (Some(left_actual), Some(right_actual)) =
            (left_actual.as_ref(), right_actual.as_ref())
            && let Some(domain_operator) = select_domain_binary_operator(
                self.module,
                &mut self.typing,
                operator,
                left_actual,
                right_actual,
            )
            .unwrap_or(None)
            {
                let checkpoint = self.diagnostics.len();
                let left_ok = self.check_expr(left, env, Some(left_actual), value_stack);
                let right_ok = self.check_expr(right, env, Some(right_actual), value_stack);
                if !left_ok || !right_ok {
                    if self.diagnostics.len() == checkpoint {
                        self.emit_invalid_binary_operator(
                            self.module.exprs()[expr_id].span,
                            operator,
                            Some(left_actual),
                            Some(right_actual),
                            BinaryOperatorExpectation::MatchingNumericOperands,
                        );
                    }
                    return false;
                }
                return self.check_result_type(expr_id, expected, &domain_operator.result_type);
            }
        let Some(operand_ty) = self.select_numeric_operand_type(
            operator,
            left_actual.as_ref(),
            right_actual.as_ref(),
            expected,
        ) else {
            if matches!(
                operator,
                BinaryOperator::GreaterThan
                    | BinaryOperator::LessThan
                    | BinaryOperator::GreaterThanOrEqual
                    | BinaryOperator::LessThanOrEqual
            ) && let (Some(left_actual), Some(right_actual)) =
                (left_actual.as_ref(), right_actual.as_ref())
                && left_actual.same_shape(right_actual)
                && self.require_class_named("Ord", left_actual).is_ok()
            {
                let checkpoint = self.diagnostics.len();
                let left_ok = self.check_expr(left, env, Some(left_actual), value_stack);
                let right_ok = self.check_expr(right, env, Some(right_actual), value_stack);
                if !left_ok || !right_ok {
                    if self.diagnostics.len() == checkpoint {
                        self.emit_invalid_binary_operator(
                            self.module.exprs()[expr_id].span,
                            operator,
                            Some(left_actual),
                            Some(right_actual),
                            BinaryOperatorExpectation::MatchingNumericOperands,
                        );
                    }
                    return false;
                }
                return self.check_result_type(
                    expr_id,
                    expected,
                    &GateType::Primitive(BuiltinType::Bool),
                );
            }
            let checkpoint = self.diagnostics.len();
            self.check_expr(left, env, None, value_stack);
            self.check_expr(right, env, None, value_stack);
            if self.diagnostics.len() == checkpoint {
                self.emit_invalid_binary_operator(
                    self.module.exprs()[expr_id].span,
                    operator,
                    left_actual.as_ref(),
                    right_actual.as_ref(),
                    BinaryOperatorExpectation::MatchingNumericOperands,
                );
            }
            return false;
        };

        let checkpoint = self.diagnostics.len();
        let left_ok = self.check_expr(left, env, Some(&operand_ty), value_stack);
        let right_ok = self.check_expr(right, env, Some(&operand_ty), value_stack);
        if !left_ok || !right_ok {
            if self.diagnostics.len() == checkpoint {
                self.emit_invalid_binary_operator(
                    self.module.exprs()[expr_id].span,
                    operator,
                    left_actual.as_ref(),
                    right_actual.as_ref(),
                    BinaryOperatorExpectation::MatchingNumericOperands,
                );
            }
            return false;
        }

        let result_ty = match operator {
            BinaryOperator::GreaterThan
            | BinaryOperator::LessThan
            | BinaryOperator::GreaterThanOrEqual
            | BinaryOperator::LessThanOrEqual => GateType::Primitive(BuiltinType::Bool),
            BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo => operand_ty,
            _ => unreachable!("numeric binary operator helper only handles numeric operators"),
        };
        self.check_result_type(expr_id, expected, &result_ty)
    }

    fn check_equality_binary_expr(
        &mut self,
        expr_id: ExprId,
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let left_actual = self.inferred_expr_type(left, env);
        let right_actual = self.inferred_expr_type(right, env);
        let Some(operand_ty) = left_actual.clone().or_else(|| right_actual.clone()) else {
            let checkpoint = self.diagnostics.len();
            self.check_expr(left, env, None, value_stack);
            self.check_expr(right, env, None, value_stack);
            if self.diagnostics.len() == checkpoint {
                self.emit_invalid_binary_operator(
                    self.module.exprs()[expr_id].span,
                    operator,
                    left_actual.as_ref(),
                    right_actual.as_ref(),
                    BinaryOperatorExpectation::CommonTypeOperands,
                );
            }
            return false;
        };

        let checkpoint = self.diagnostics.len();
        let left_ok = self.check_expr(left, env, Some(&operand_ty), value_stack);
        let right_ok = self.check_expr(right, env, Some(&operand_ty), value_stack);
        if (!left_ok || !right_ok)
            && self.diagnostics.len() > checkpoint {
                // At least one side emitted a concrete type error; propagate it.
                return false;
            }
            // No diagnostics were added: one or both operand types are unresolvable
            // (e.g. an imported generic function whose return type cannot be inferred
            // at the gate level). Treat as valid rather than emitting a spurious
            // invalid-binary-operator diagnostic.

        self.handle_constraints(&[TypeConstraint::eq(
            self.module.exprs()[expr_id].span,
            operand_ty,
        )]);
        let bool_ty = GateType::Primitive(BuiltinType::Bool);
        self.check_result_type(expr_id, expected, &bool_ty)
    }

    fn inferred_expr_type(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        let info = self.typing.infer_expr(expr_id, env, None);
        self.enqueue_eq_constraints(&info.constraints);
        info.ty
    }

    fn inferred_expr_shape(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        let info = self.typing.infer_expr(expr_id, env, None);
        self.enqueue_eq_constraints(&info.constraints);
        info.ty.clone().or_else(|| info.actual_gate_type())
    }

    fn select_numeric_operand_type(
        &self,
        operator: BinaryOperator,
        left: Option<&GateType>,
        right: Option<&GateType>,
        expected: Option<&GateType>,
    ) -> Option<GateType> {
        left.filter(|ty| is_numeric_gate_type(ty))
            .cloned()
            .or_else(|| right.filter(|ty| is_numeric_gate_type(ty)).cloned())
            .or_else(|| {
                matches!(
                    operator,
                    BinaryOperator::Add
                        | BinaryOperator::Subtract
                        | BinaryOperator::Multiply
                        | BinaryOperator::Divide
                        | BinaryOperator::Modulo
                )
                .then(|| expected.filter(|ty| is_numeric_gate_type(ty)).cloned())
                .flatten()
            })
    }

    fn check_result_type(
        &mut self,
        expr_id: ExprId,
        expected: Option<&GateType>,
        actual: &GateType,
    ) -> bool {
        match expected {
            Some(expected) if !actual.same_shape(expected) => {
                self.emit_type_mismatch(self.module.exprs()[expr_id].span, expected, actual);
                false
            }
            _ => true,
        }
    }

    fn check_result_block_pipe_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let ExprKind::Pipe(pipe) = self.module.exprs()[expr_id].kind.clone() else {
            return None;
        };
        pipe.result_block_desugaring.then(|| {
            self.check_result_block_pipe_with_error(
                expr_id,
                &pipe,
                env,
                expected,
                Self::result_block_expected_error(expected).cloned(),
                value_stack,
            )
        })
    }

    fn check_result_block_expr_with_error(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        expected_error: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let ExprKind::Pipe(pipe) = self.module.exprs()[expr_id].kind.clone() else {
            return self.check_expr(expr_id, env, expected, value_stack);
        };
        if !pipe.result_block_desugaring {
            return self.check_expr(expr_id, env, expected, value_stack);
        }
        self.check_result_block_pipe_with_error(
            expr_id,
            &pipe,
            env,
            expected,
            expected_error.cloned(),
            value_stack,
        )
    }

    fn check_result_block_pipe_with_error(
        &mut self,
        expr_id: ExprId,
        pipe: &PipeExpr,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        inherited_error: Option<GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let Some(case_run) = self.result_block_case_run(pipe) else {
            return self.check_inferred_expr(expr_id, env, expected);
        };

        let expected_error = match (
            inherited_error,
            Self::result_block_expected_error(expected).cloned(),
        ) {
            (Some(left), Some(right)) if left.same_shape(&right) => Some(left),
            (Some(left), Some(_)) => Some(left),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };

        let mut ok = self.check_result_block_expr_with_error(
            pipe.head,
            env,
            None,
            expected_error.as_ref(),
            value_stack,
        );

        let propagated_error =
            match self.result_block_binding_shape(pipe.head, env, expected_error.as_ref()) {
                ResultBindingShape::Result { error, value } => {
                    if let (Some(expected_error), Some(actual_error)) =
                        (expected_error.as_ref(), error.as_ref())
                        && !actual_error.same_shape(expected_error)
                    {
                        self.emit_result_block_error_mismatch(
                            self.module.exprs()[pipe.head].span,
                            expected_error,
                            actual_error,
                        );
                        ok = false;
                    }

                    let mut ok_env = env.clone();
                    if let Some(binding) = self.result_block_ok_binding(case_run.ok_pattern)
                        && let Some(value_ty) = value
                    {
                        ok_env.locals.insert(binding, value_ty);
                    }

                    let propagated_error = error.or(expected_error);
                    ok &= self.check_result_block_expr_with_error(
                        case_run.ok_body,
                        &ok_env,
                        expected,
                        propagated_error.as_ref(),
                        value_stack,
                    );
                    propagated_error
                }
                ResultBindingShape::NonResult { actual } => {
                    self.emit_result_block_binding_not_result(
                        self.module.exprs()[pipe.head].span,
                        &actual,
                    );
                    ok = false;
                    expected_error
                }
                ResultBindingShape::Unknown => expected_error,
            };

        let _ = propagated_error;
        ok & self.check_inferred_expr(expr_id, env, expected)
    }

    fn result_block_case_run(&self, pipe: &PipeExpr) -> Option<ResultBlockCaseRun> {
        let mut ok = None;
        let mut err = false;
        for stage in pipe.stages.iter() {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                return None;
            };
            match self.pattern_builtin_constructor(*pattern) {
                Some(BuiltinTerm::Ok) if ok.is_none() => {
                    ok = Some(ResultBlockCaseRun {
                        ok_pattern: *pattern,
                        ok_body: *body,
                    });
                }
                Some(BuiltinTerm::Err) if !err => {
                    err = true;
                }
                _ => return None,
            }
        }
        ok.filter(|_| err)
    }

    fn result_block_binding_shape(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected_error: Option<&GateType>,
    ) -> ResultBindingShape {
        if let ExprKind::Pipe(pipe) = &self.module.exprs()[expr_id].kind
            && pipe.result_block_desugaring
        {
            return ResultBindingShape::Result {
                error: expected_error.cloned(),
                value: self.result_block_value_shape(expr_id, env),
            };
        }

        if let Some(actual) = self.inferred_expr_shape(expr_id, env) {
            return match actual {
                GateType::Result { error, value } => ResultBindingShape::Result {
                    error: Some(*error),
                    value: Some(*value),
                },
                actual => ResultBindingShape::NonResult { actual },
            };
        }

        if let Some(argument) = self.builtin_constructor_argument(expr_id, BuiltinTerm::Ok) {
            return ResultBindingShape::Result {
                error: expected_error.cloned(),
                value: self.inferred_expr_shape(argument, env),
            };
        }

        if let Some(argument) = self.builtin_constructor_argument(expr_id, BuiltinTerm::Err) {
            return ResultBindingShape::Result {
                error: self
                    .inferred_expr_shape(argument, env)
                    .or_else(|| expected_error.cloned()),
                value: None,
            };
        }

        ResultBindingShape::Unknown
    }

    fn result_block_value_shape(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        match self.inferred_expr_shape(expr_id, env) {
            Some(GateType::Result { value, .. }) => Some(*value),
            _ => self
                .builtin_constructor_argument(expr_id, BuiltinTerm::Ok)
                .and_then(|argument| self.inferred_expr_shape(argument, env)),
        }
    }

    fn builtin_constructor_argument(
        &self,
        expr_id: ExprId,
        builtin: BuiltinTerm,
    ) -> Option<ExprId> {
        let ExprKind::Apply { callee, arguments } = &self.module.exprs()[expr_id].kind else {
            return None;
        };
        if arguments.len() != 1 {
            return None;
        }
        let ExprKind::Name(reference) = &self.module.exprs()[*callee].kind else {
            return None;
        };
        matches!(
            reference.resolution.as_ref(),
            ResolutionState::Resolved(TermResolution::Builtin(found)) if *found == builtin
        )
        .then_some(*arguments.first())
    }

    fn result_block_expected_error(expected: Option<&GateType>) -> Option<&GateType> {
        match expected {
            Some(GateType::Result { error, .. }) => Some(error.as_ref()),
            _ => None,
        }
    }

    fn result_block_ok_binding(&self, pattern_id: PatternId) -> Option<BindingId> {
        let PatternKind::Constructor { arguments, .. } = &self.module.patterns()[pattern_id].kind
        else {
            return None;
        };
        let argument = *arguments.first()?;
        let PatternKind::Binding(binding) = &self.module.patterns()[argument].kind else {
            return None;
        };
        Some(binding.binding)
    }

    fn pattern_builtin_constructor(&self, pattern_id: PatternId) -> Option<BuiltinTerm> {
        let PatternKind::Constructor { callee, .. } = &self.module.patterns()[pattern_id].kind
        else {
            return None;
        };
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            callee.resolution.as_ref()
        else {
            return None;
        };
        Some(*builtin)
    }

    fn emit_result_block_binding_not_result(&mut self, span: SourceSpan, actual: &GateType) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "result block bindings must produce `Result E A`, found `{actual}`"
            ))
            .with_code(code("result-block-binding-not-result"))
            .with_primary_label(span, "this `<-` binding expression is not a `Result` value"),
        );
    }

    fn emit_result_block_error_mismatch(
        &mut self,
        span: SourceSpan,
        expected: &GateType,
        actual: &GateType,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "result block bindings must share one error type, found `{actual}` where `{expected}` was required"
            ))
            .with_code(code("result-block-error-mismatch"))
            .with_primary_label(
                span,
                "this binding's `Err` carrier does not match the surrounding result block",
            ),
        );
    }

    fn check_expected_special_case(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let kind = self.module.exprs()[expr_id].kind.clone();
        match kind {
            ExprKind::Name(reference) => self
                .check_builtin_constructor_name(&reference, expected)
                .or_else(|| self.check_domain_member_name(&reference, expected))
                .or_else(|| self.check_class_member_name(&reference, expected))
                .or_else(|| self.check_signal_payload_name(expr_id, env, expected))
                .or_else(|| {
                    self.check_unannotated_value_name(&reference, env, expected, value_stack)
                })
                .or_else(|| {
                    self.check_unannotated_function_name(&reference, expected, value_stack)
                }),
            ExprKind::Apply { callee, arguments } => {
                let callee_kind = self.module.exprs()[callee].kind.clone();
                if let ExprKind::Name(reference) = callee_kind {
                    if let Some(result) = self.check_builtin_constructor_apply(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                    if let Some(result) = self.check_domain_member_apply(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                    if let Some(result) = self.check_class_member_apply(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                    if let Some(result) = self.check_function_apply_with_context(
                        &reference,
                        &arguments,
                        env,
                        expected,
                        value_stack,
                    ) {
                        return Some(result);
                    }
                }
                self.check_expected_apply(expr_id, callee, &arguments, env, expected, value_stack)
            }
            ExprKind::Record(record) => match expected {
                GateType::Record(fields) => {
                    let checkpoint = self.diagnostics.len();
                    let mut constraints = Vec::new();
                    let ok = self.check_record_expr(
                        self.module.exprs()[expr_id].span,
                        &record,
                        fields,
                        env,
                        value_stack,
                        &mut constraints,
                    );
                    let solved = self.handle_constraints(&constraints);
                    let no_new_diagnostics = self.diagnostics.len() == checkpoint;
                    if ok && no_new_diagnostics && !solved.default_record_fields.is_empty() {
                        self.default_record_elisions.push(DefaultRecordElision {
                            record_expr: expr_id,
                            fields: solved.default_record_fields,
                        });
                    }
                    Some(ok && no_new_diagnostics)
                }
                GateType::OpaqueImport { import, .. } => {
                    // Imported record type: look up fields from the import binding metadata
                    // and validate each record field against its expected type.
                    let field_types: Option<HashMap<String, GateType>> = {
                        let binding = &self.module.imports()[*import];
                        if let ImportBindingMetadata::TypeConstructor {
                            fields: Some(record_fields),
                            ..
                        } = &binding.metadata
                        {
                            Some(
                                record_fields
                                    .iter()
                                    .map(|f| {
                                        (
                                            f.name.to_string(),
                                            self.typing.lower_import_value_type(&f.ty),
                                        )
                                    })
                                    .collect(),
                            )
                        } else {
                            None
                        }
                    };
                    let field_types = field_types?;
                    let checkpoint = self.diagnostics.len();
                    let ok = record.fields.iter().all(|field| {
                        if let Some(expected_ty) = field_types.get(field.label.text()) {
                            self.check_expr(field.value, env, Some(expected_ty), value_stack)
                        } else {
                            false
                        }
                    });
                    let no_new_diagnostics = self.diagnostics.len() == checkpoint;
                    Some(ok && no_new_diagnostics)
                }
                _ => None,
            },
            ExprKind::Tuple(elements) => match expected {
                GateType::Tuple(expected_elements) => Some(self.check_tuple_expr(
                    self.module.exprs()[expr_id].span,
                    &elements,
                    expected_elements,
                    env,
                    value_stack,
                )),
                _ => None,
            },
            ExprKind::List(elements) => match expected {
                GateType::List(element) => Some(self.check_homogeneous_collection_expr(
                    &elements,
                    element.as_ref(),
                    env,
                    value_stack,
                )),
                _ => None,
            },
            ExprKind::Map(map) => match expected {
                GateType::Map { key, value } => {
                    Some(self.check_map_expr(&map, key.as_ref(), value.as_ref(), env, value_stack))
                }
                _ => None,
            },
            ExprKind::Set(elements) => match expected {
                GateType::Set(element) => Some(self.check_homogeneous_collection_expr(
                    &elements,
                    element.as_ref(),
                    env,
                    value_stack,
                )),
                _ => None,
            },
            ExprKind::Projection { base, path } => {
                self.check_projection_expr(expr_id, &base, &path, env, expected, value_stack)
            }
            ExprKind::PatchApply { target, patch } => Some(self.check_patch_apply_expr(
                expr_id,
                target,
                &patch,
                env,
                Some(expected),
                value_stack,
            )),
            ExprKind::PatchLiteral(patch) => {
                Some(self.check_patch_literal_expr(expr_id, &patch, env, expected, value_stack))
            }
            ExprKind::Lambda(_) => None,
            ExprKind::AmbientSubject => None,
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::Text(_)
            | ExprKind::Regex(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Pipe(_)
            | ExprKind::Cluster(_)
            | ExprKind::Markup(_) => None,
        }
    }

    fn check_unannotated_value_name(
        &mut self,
        reference: &TermReference,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let (body, annotated) = match &self.module.items()[*item_id] {
            Item::Value(item) => (item.body, item.annotation.is_some()),
            _ => return None,
        };
        if annotated || value_stack.contains(item_id) {
            return None;
        }
        value_stack.push(*item_id);
        let result = self.check_expr(body, env, Some(expected), value_stack);
        let popped = value_stack.pop();
        debug_assert_eq!(popped, Some(*item_id));
        Some(result)
    }

    fn check_unannotated_function_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let (parameters, result_annotation, body, inferable) = match &self.module.items()[*item_id]
        {
            Item::Function(item) => (
                item.parameters.clone(),
                item.annotation,
                item.body,
                supports_same_module_function_inference(item),
            ),
            _ => return None,
        };
        if !inferable
            || (result_annotation.is_some()
                && parameters
                    .iter()
                    .all(|parameter| parameter.annotation.is_some()))
            || value_stack.contains(item_id)
        {
            return None;
        }
        let (parameter_types, result_expected) =
            self.expected_function_signature(expected, parameters.len())?;
        let mut env = GateExprEnv::default();
        for (parameter, expected_parameter_ty) in parameters.iter().zip(parameter_types.iter()) {
            if let Some(annotation) = parameter.annotation {
                let parameter_ty = self.typing.lower_annotation(annotation)?;
                if !parameter_ty.same_shape(expected_parameter_ty) {
                    self.emit_type_mismatch(reference.span(), expected_parameter_ty, &parameter_ty);
                    return Some(false);
                }
                env.locals.insert(parameter.binding, parameter_ty);
            } else {
                env.locals
                    .insert(parameter.binding, expected_parameter_ty.clone());
            }
        }
        if let Some(annotation) = result_annotation {
            let result_ty = self.typing.lower_open_annotation(annotation)?;
            if !result_ty.same_shape(&result_expected) {
                self.emit_type_mismatch(reference.span(), &result_expected, &result_ty);
                return Some(false);
            }
        }
        self.record_function_signature_evidence(*item_id, &parameter_types, &result_expected);
        value_stack.push(*item_id);
        let result = self.check_expr(body, &env, Some(&result_expected), value_stack);
        let popped = value_stack.pop();
        debug_assert_eq!(popped, Some(*item_id));
        Some(result)
    }

    fn check_signal_payload_name(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: &GateType,
    ) -> Option<bool> {
        let info = self.typing.infer_expr(expr_id, env, None);
        if !info.issues.is_empty() {
            return None;
        }
        let actual = info.actual_gate_type().or(info.ty)?;
        signal_name_payload_type(self.module, expr_id, &actual)
            .is_some_and(|payload| payload.same_shape(expected))
            .then_some(true)
    }

    fn expected_function_signature(
        &self,
        expected: &GateType,
        arity: usize,
    ) -> Option<(Vec<GateType>, GateType)> {
        let mut current = expected;
        let mut parameter_types = Vec::with_capacity(arity);
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameter_types.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some((parameter_types, current.clone()))
    }

    fn contextual_same_module_function_signature(
        &mut self,
        function: &FunctionItem,
        argument_types: &[GateType],
        expected: &GateType,
    ) -> Option<(Vec<GateType>, GateType)> {
        if !supports_same_module_function_inference(function) {
            return None;
        }
        if function.annotation.is_some()
            && function
                .parameters
                .iter()
                .all(|parameter| parameter.annotation.is_some())
        {
            return None;
        }
        let remaining_arity = function
            .parameters
            .len()
            .checked_sub(argument_types.len())?;
        let (remaining_parameter_types, result_type) =
            self.expected_function_signature(expected, remaining_arity)?;
        let mut parameter_types = argument_types.to_vec();
        parameter_types.extend(remaining_parameter_types);
        if parameter_types.len() != function.parameters.len() {
            return None;
        }
        for (parameter, parameter_ty) in function.parameters.iter().zip(parameter_types.iter()) {
            if let Some(annotation) = parameter.annotation {
                let annotation_ty = self.typing.lower_open_annotation(annotation)?;
                if !annotation_ty.same_shape(parameter_ty) {
                    return None;
                }
            }
        }
        if let Some(annotation) = function.annotation {
            let annotation_ty = self.typing.lower_open_annotation(annotation)?;
            if !annotation_ty.same_shape(&result_type) {
                return None;
            }
        }
        Some((parameter_types, result_type))
    }

    fn record_function_signature_evidence(
        &mut self,
        item_id: ItemId,
        parameter_types: &[GateType],
        result_type: &GateType,
    ) {
        if result_type.has_type_params()
            || parameter_types
                .iter()
                .any(|parameter| parameter.has_type_params())
        {
            return;
        }
        self.typing
            .record_function_signature_evidence(FunctionSignatureEvidence {
                item_id,
                parameter_types: parameter_types.to_vec(),
                result_type: result_type.clone(),
            });
    }

    /// When an argument expression is a direct reference to a same-module
    /// function eligible for inference (e.g. a hoisted lambda helper), record
    /// `FunctionSignatureEvidence` so the iterative inference loop can
    /// determine the function's parameter types from the call-site context.
    fn record_argument_function_evidence(&mut self, argument: ExprId, expected: &GateType) {
        let GateType::Arrow { .. } = expected else {
            return;
        };
        let ExprKind::Name(reference) = &self.module.exprs()[argument].kind else {
            return;
        };
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return;
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return;
        };
        if !supports_same_module_function_inference(function) {
            return;
        }
        let Some((parameter_types, result_type)) =
            self.expected_function_signature(expected, function.parameters.len())
        else {
            return;
        };
        self.record_function_signature_evidence(*item_id, &parameter_types, &result_type);
    }

    fn check_builtin_constructor_name(
        &self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        match (builtin, expected) {
            (BuiltinTerm::None, GateType::Option(_)) => Some(true),
            _ => None,
        }
    }

    fn check_builtin_constructor_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        if arguments.len() != 1 {
            return None;
        }
        let argument = *arguments.first();
        match (builtin, expected) {
            (BuiltinTerm::Some, GateType::Option(payload)) => {
                Some(self.check_expr(argument, env, Some(payload.as_ref()), value_stack))
            }
            (BuiltinTerm::Ok, GateType::Result { value, .. }) => {
                Some(self.check_expr(argument, env, Some(value.as_ref()), value_stack))
            }
            (BuiltinTerm::Err, GateType::Result { error, .. }) => {
                Some(self.check_expr(argument, env, Some(error.as_ref()), value_stack))
            }
            (BuiltinTerm::Valid, GateType::Validation { value, .. }) => {
                Some(self.check_expr(argument, env, Some(value.as_ref()), value_stack))
            }
            (BuiltinTerm::Invalid, GateType::Validation { error, .. }) => {
                Some(self.check_expr(argument, env, Some(error.as_ref()), value_stack))
            }
            _ => None,
        }
    }

    fn check_domain_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<bool> {
        let labels = self.typing.domain_member_candidate_labels(reference)?;
        match self.typing.select_domain_member_name(reference, expected)? {
            DomainMemberSelection::Unique(_) => Some(true),
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_domain_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let labels = self.typing.domain_member_candidate_labels(reference)?;
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let info = self.typing.infer_expr(*argument, env, None);
            self.emit_expr_issues(&info.issues);
            self.handle_constraints(&info.constraints);
            let argument_ty = info.ty.clone().or_else(|| info.actual_gate_type())?;
            argument_types.push(argument_ty);
        }
        match self
            .typing
            .select_domain_member_call(reference, &argument_types, Some(expected))?
        {
            DomainMemberSelection::Unique(matched) => {
                for (argument, parameter) in arguments.iter().zip(matched.parameters.iter()) {
                    if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                        return Some(false);
                    }
                }
                Some(true)
            }
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_domain_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_class_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<bool> {
        let labels = self.typing.class_member_candidate_labels(reference)?;
        match self
            .typing
            .select_class_member_call(reference, &[], Some(expected))?
        {
            DomainMemberSelection::Unique(matched) => {
                if let Err(reason) = self.solve_class_constraint_bindings(
                    reference.span(),
                    &matched.evidence,
                    &matched.constraints,
                ) {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "this reference requires `{}`",
                            self.class_constraint_binding_label(&matched.evidence)
                        ))
                        .with_code(code("missing-class-instance"))
                        .with_primary_label(
                            reference.span(),
                            format!(
                                "`{}` is not currently available here",
                                self.class_constraint_binding_label(&matched.evidence)
                            ),
                        )
                        .with_note(reason),
                    );
                    return Some(false);
                }
                Some(true)
            }
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_class_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_class_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_class_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let labels = self.typing.class_member_candidate_labels(reference)?;
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let info = self.typing.infer_expr(*argument, env, None);
            self.emit_expr_issues(&info.issues);
            self.handle_constraints(&info.constraints);
            let argument_ty = info.ty.clone().or_else(|| info.actual_gate_type())?;
            argument_types.push(argument_ty);
        }
        match self
            .typing
            .select_class_member_call(reference, &argument_types, Some(expected))?
        {
            DomainMemberSelection::Unique(matched) => {
                if let Err(reason) = self.solve_class_constraint_bindings(
                    reference.span(),
                    &matched.evidence,
                    &matched.constraints,
                ) {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "this call requires `{}` evidence",
                            self.class_constraint_binding_label(&matched.evidence)
                        ))
                        .with_code(code("missing-class-instance"))
                        .with_primary_label(
                            reference.span(),
                            format!(
                                "`{}` is not currently available here",
                                self.class_constraint_binding_label(&matched.evidence)
                            ),
                        )
                        .with_note(reason),
                    );
                    return Some(false);
                }
                for (argument, parameter) in arguments.iter().zip(matched.parameters.iter()) {
                    if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                        return Some(false);
                    }
                }
                Some(true)
            }
            DomainMemberSelection::Ambiguous => {
                self.emit_ambiguous_class_member(reference.span(), reference, &labels);
                Some(false)
            }
            DomainMemberSelection::NoMatch => (labels.len() > 1).then(|| {
                self.emit_ambiguous_class_member(reference.span(), reference, &labels);
                false
            }),
        }
    }

    fn check_function_apply_with_context(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let crate::ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return None;
        };
        let function = function.clone();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let info = self.typing.infer_expr(*argument, env, None);
            self.emit_expr_issues(&info.issues);
            self.handle_constraints(&info.constraints);
            let argument_ty = info.ty?;
            argument_types.push(argument_ty);
        }
        if let Some((parameter_types, result_type)) =
            self.contextual_same_module_function_signature(&function, &argument_types, expected)
        {
            self.record_function_signature_evidence(*item_id, &parameter_types, &result_type);
            for (argument, parameter) in arguments.iter().zip(parameter_types.iter()) {
                if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                    return Some(false);
                }
            }
            return Some(true);
        }
        let (matched_parameters, constraints) =
            self.match_function_constraints(&function, arguments, &argument_types, expected)?;
        for constraint in &constraints {
            if let Err(reason) = self.require_class_binding(constraint) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "this call requires `{}`",
                        self.class_constraint_binding_label(constraint)
                    ))
                    .with_code(code("missing-class-instance"))
                    .with_primary_label(
                        reference.span(),
                        format!(
                            "`{}` is not currently available here",
                            self.class_constraint_binding_label(constraint)
                        ),
                    )
                    .with_note(reason),
                );
                return Some(false);
            }
        }
        for (argument, parameter) in arguments.iter().zip(matched_parameters.iter()) {
            if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                return Some(false);
            }
        }
        Some(true)
    }

    fn check_expected_apply(
        &mut self,
        expr_id: ExprId,
        callee: ExprId,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let checkpoint = self.diagnostics.len();
        let callee_info = self.typing.infer_expr(callee, env, None);
        self.emit_expr_issues(&callee_info.issues);
        self.handle_constraints(&callee_info.constraints);
        if self.diagnostics.len() != checkpoint {
            return Some(false);
        }
        let callee_ty = callee_info
            .ty
            .clone()
            .or_else(|| callee_info.actual_gate_type());

        let parameter_types = match callee_ty {
            Some(callee_ty) => {
                let (parameter_types, result_ty) =
                    self.expected_function_signature(&callee_ty, arguments.len())?;
                let has_poly = result_ty.has_type_params()
                    || parameter_types.iter().any(|p| p.has_type_params());
                if result_ty.same_shape(expected) && !has_poly {
                    parameter_types
                } else if has_poly {
                    // The callee has a polymorphic signature (e.g. imported with TypeVariables).
                    // Collect bindings from:
                    //   1. result type vs expected type
                    //   2. each argument's inferred type vs parameter type
                    // Then substitute all bindings into parameter types.
                    let mut bindings = HashMap::new();
                    expected.unify_type_params(&result_ty, &mut bindings);
                    for (argument, param) in arguments.iter().zip(parameter_types.iter()) {
                        if param.has_type_params() {
                            let arg_info = self.typing.infer_expr(*argument, env, None);
                            if let Some(arg_ty) = arg_info.ty.as_ref() {
                                arg_ty.unify_type_params(param, &mut bindings);
                            }
                        }
                    }
                    if !bindings.is_empty() {
                        let resolved: Vec<GateType> = parameter_types
                            .iter()
                            .map(|p| p.substitute_type_parameters(&bindings))
                            .collect();
                        let resolved_result = result_ty.substitute_type_parameters(&bindings);
                        if !resolved_result.same_shape(expected) {
                            self.emit_type_mismatch(
                                self.module.exprs()[expr_id].span,
                                expected,
                                &resolved_result,
                            );
                            return Some(false);
                        }
                        resolved
                    } else if result_ty.same_shape(expected) {
                        // All type params are in parameters only (e.g. `length : List A -> Int`).
                        // No bindings could be collected — fall back to checking arguments
                        // directly (the check_expr below will handle type param matching).
                        parameter_types
                    } else {
                        self.emit_type_mismatch(
                            self.module.exprs()[expr_id].span,
                            expected,
                            &result_ty,
                        );
                        return Some(false);
                    }
                } else {
                    self.emit_type_mismatch(
                        self.module.exprs()[expr_id].span,
                        expected,
                        &result_ty,
                    );
                    return Some(false);
                }
            }
            None => {
                let parameter_types =
                    self.fallback_apply_parameter_types(callee, arguments, env)?;
                let callee_expected = self.arrow_type(&parameter_types, expected);
                let checkpoint = self.diagnostics.len();
                if !self.check_expr(callee, env, Some(&callee_expected), value_stack) {
                    return (self.diagnostics.len() != checkpoint).then_some(false);
                }
                parameter_types
            }
        };

        // Record signature evidence for same-module inference-target functions
        // used as arguments. This enables hoisted lambda helpers to have their
        // parameter types inferred from the callee's resolved Arrow type.
        for (argument, parameter) in arguments.iter().zip(parameter_types.iter()) {
            self.record_argument_function_evidence(*argument, parameter);
        }

        for (argument, parameter) in arguments.iter().zip(parameter_types.iter()) {
            if !self.check_expr(*argument, env, Some(parameter), value_stack) {
                return Some(false);
            }
        }

        Some(true)
    }

    fn check_record_expr(
        &mut self,
        expr_span: SourceSpan,
        record: &RecordExpr,
        expected_fields: &[GateRecordField],
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
        constraints: &mut Vec<TypeConstraint>,
    ) -> bool {
        let expected = expected_fields
            .iter()
            .map(|field| (field.name.as_str(), &field.ty))
            .collect::<HashMap<_, _>>();
        let available_field_names: Vec<String> =
            expected_fields.iter().map(|f| f.name.clone()).collect();
        let mut seen = HashMap::<String, SourceSpan>::new();
        let mut ok = true;

        for field in &record.fields {
            let label = field.label.text();
            let Some(expected_ty) = expected.get(label) else {
                let mut diag = Diagnostic::error(format!(
                    "record literal provides unexpected field `{label}`"
                ))
                .with_code(code("unexpected-record-field"))
                .with_primary_label(
                    field.span,
                    "this field is not part of the expected closed record type",
                );
                if !available_field_names.is_empty() {
                    diag = diag.with_note(format!(
                        "available fields: {}",
                        available_field_names.join(", ")
                    ));
                }
                self.diagnostics.push(diag);
                ok = false;
                continue;
            };
            if let Some(previous_span) = seen.insert(label.to_owned(), field.span) {
                self.diagnostics.push(
                    Diagnostic::error(format!("duplicate record field `{label}`"))
                        .with_code(code("duplicate-record-field"))
                        .with_primary_label(
                            field.span,
                            "this field repeats an earlier record entry",
                        )
                        .with_secondary_label(
                            previous_span,
                            "previous field with the same label here",
                        ),
                );
                ok = false;
            }
            ok &= self.check_expr(field.value, env, Some(*expected_ty), value_stack);
        }

        for field in expected_fields {
            if seen.contains_key(&field.name) {
                continue;
            }
            constraints.push(TypeConstraint::default_record_field(
                expr_span,
                field.name.clone(),
                field.ty.clone(),
                available_field_names.clone(),
            ));
        }

        ok
    }

    fn check_tuple_expr(
        &mut self,
        expr_span: SourceSpan,
        elements: &crate::AtLeastTwo<ExprId>,
        expected_elements: &[GateType],
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let mut ok = true;
        for (index, element) in elements.iter().enumerate() {
            match expected_elements.get(index) {
                Some(expected) => {
                    ok &= self.check_expected_expr(*element, env, expected, value_stack);
                }
                None => {
                    ok &= self.check_expr(*element, env, None, value_stack);
                }
            }
        }

        if elements.len() != expected_elements.len() {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "expected `{}` but found a {}-element tuple",
                    GateType::Tuple(expected_elements.to_vec()),
                    elements.len()
                ))
                .with_code(code("type-mismatch"))
                .with_primary_label(expr_span, "this tuple has the wrong arity"),
            );
            return false;
        }

        ok
    }

    fn check_homogeneous_collection_expr(
        &mut self,
        elements: &[ExprId],
        expected_element: &GateType,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let mut ok = true;
        for element in elements {
            ok &= self.check_expected_expr(*element, env, expected_element, value_stack);
        }
        ok
    }

    fn check_map_expr(
        &mut self,
        map: &MapExpr,
        expected_key: &GateType,
        expected_value: &GateType,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let mut ok = true;
        for entry in &map.entries {
            ok &= self.check_expected_expr(entry.key, env, expected_key, value_stack);
            ok &= self.check_expected_expr(entry.value, env, expected_value, value_stack);
        }
        ok
    }

    fn check_projection_expr(
        &mut self,
        expr_id: ExprId,
        base: &ProjectionBase,
        path: &NamePath,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> Option<bool> {
        let ProjectionBase::Expr(base_expr) = base else {
            return None;
        };

        let checkpoint = self.diagnostics.len();
        let base_info = self.typing.infer_expr(*base_expr, env, None);
        self.emit_expr_issues(&base_info.issues);
        self.handle_constraints(&base_info.constraints);
        if self.diagnostics.len() != checkpoint {
            return Some(false);
        }

        let subject = base_info
            .ty
            .clone()
            .or_else(|| base_info.actual_gate_type())
            .or_else(|| self.infer_apply_result_type(*base_expr, env));
        if self.diagnostics.len() != checkpoint {
            return Some(false);
        }
        let subject = subject?;

        if !self.check_expected_expr(*base_expr, env, &subject, value_stack) {
            return Some(false);
        }

        match self.typing.project_type(&subject, path) {
            Ok(actual) => Some(self.check_result_type(expr_id, Some(expected), &actual)),
            Err(issue) => {
                self.emit_expr_issues(&[issue]);
                Some(false)
            }
        }
    }

    fn check_expected_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let checkpoint = self.diagnostics.len();
        let ok = self.check_expr(expr_id, env, Some(expected), value_stack);
        if !ok && self.diagnostics.len() == checkpoint {
            let actual = self.inferred_expr_shape(expr_id, env);
            self.emit_type_mismatch_or_unresolved(
                self.module.exprs()[expr_id].span,
                expected,
                actual.as_ref(),
            );
        }
        ok
    }

    fn infer_apply_result_type(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        let ExprKind::Apply { callee, arguments } = self.module.exprs()[expr_id].kind.clone()
        else {
            return None;
        };

        let callee_info = self.typing.infer_expr(callee, env, None);
        self.emit_expr_issues(&callee_info.issues);
        self.handle_constraints(&callee_info.constraints);

        let mut current = callee_info
            .ty
            .clone()
            .or_else(|| callee_info.actual_gate_type())?;
        for _ in arguments.iter() {
            let GateType::Arrow { result, .. } = current else {
                return None;
            };
            current = *result;
        }
        Some(current)
    }

    fn check_patch_apply_expr(
        &mut self,
        expr_id: ExprId,
        target: ExprId,
        patch: &crate::PatchBlock,
        env: &GateExprEnv,
        expected: Option<&GateType>,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let removed = Self::collect_patch_removed_fields(patch);

        // When there are removals, the result type differs from the input type.
        // Infer the target type from the expression, not from expected.
        let target_ty = if removed.is_empty() {
            expected
                .cloned()
                .or_else(|| self.inferred_expr_shape(target, env))
        } else {
            self.inferred_expr_shape(target, env)
                .or_else(|| expected.cloned())
        };

        let target_ok = match target_ty.as_ref() {
            Some(subject) => self.check_expr(target, env, Some(subject), value_stack),
            None => self.check_expr(target, env, None, value_stack),
        };
        let patch_ok = match target_ty.as_ref() {
            Some(subject) => self.check_patch_block(patch, subject, env, value_stack),
            None => self.check_patch_block_children(patch, env, value_stack),
        };
        // Compute the actual result type after field removal.
        let actual_result = match target_ty.as_ref() {
            Some(ty) if !removed.is_empty() => Some(Self::omit_fields_from_type(ty, &removed)),
            _ => target_ty.clone(),
        };
        let result_ok = match (expected, actual_result.as_ref()) {
            (Some(expected), Some(actual)) => {
                self.check_result_type(expr_id, Some(expected), actual)
            }
            _ => true,
        };
        target_ok && patch_ok && result_ok
    }

    /// Collect top-level field names targeted by Remove instructions.
    fn collect_patch_removed_fields(patch: &crate::PatchBlock) -> Vec<String> {
        let mut removed = Vec::new();
        for entry in &patch.entries {
            if !matches!(entry.instruction.kind, crate::PatchInstructionKind::Remove) {
                continue;
            }
            // Only handle top-level removals: selector has exactly one Named segment.
            if entry.selector.segments.len() == 1
                && let crate::PatchSelectorSegment::Named { name, .. } = &entry.selector.segments[0]
                {
                    removed.push(name.text().to_string());
                }
        }
        removed
    }

    /// Produce a record type with the named fields removed.  If the type is not
    /// a record or none of the named fields match, return the type unchanged.
    fn omit_fields_from_type(ty: &GateType, removed: &[String]) -> GateType {
        match ty {
            GateType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .filter(|f| !removed.iter().any(|r| r == &f.name))
                    .cloned()
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn check_patch_literal_expr(
        &mut self,
        expr_id: ExprId,
        patch: &crate::PatchBlock,
        env: &GateExprEnv,
        expected: &GateType,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        let GateType::Arrow { parameter, result } = expected else {
            self.diagnostics.push(
                Diagnostic::error(format!("expected `{expected}` but found a patch literal"))
                    .with_code(code("type-mismatch"))
                    .with_primary_label(
                        self.module.exprs()[expr_id].span,
                        "patch literals currently require an expected function type",
                    ),
            );
            let _ = self.check_patch_block_children(patch, env, value_stack);
            return false;
        };
        if !parameter.same_shape(result) {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "patch literals require a same-shape function type, found `{expected}`"
                ))
                .with_code(code("invalid-patch-literal-type"))
                .with_primary_label(
                    self.module.exprs()[expr_id].span,
                    "use a function type whose parameter and result have the same shape",
                ),
            );
            let _ = self.check_patch_block_children(patch, env, value_stack);
            return false;
        }
        self.check_patch_block(patch, parameter, env, value_stack)
    }

    fn check_patch_block(
        &mut self,
        patch: &crate::PatchBlock,
        subject: &GateType,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        patch
            .entries
            .iter()
            .all(|entry| self.check_patch_entry(entry, subject, env, value_stack))
    }

    fn check_patch_block_children(
        &mut self,
        patch: &crate::PatchBlock,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        patch.entries.iter().all(|entry| {
            let selector_ok = entry.selector.segments.iter().all(|segment| match segment {
                crate::PatchSelectorSegment::BracketExpr { expr, .. } => {
                    self.check_expr(*expr, env, None, value_stack)
                }
                crate::PatchSelectorSegment::Named { .. }
                | crate::PatchSelectorSegment::BracketTraverse { .. } => true,
            });
            let instruction_ok = match entry.instruction.kind {
                crate::PatchInstructionKind::Replace(expr)
                | crate::PatchInstructionKind::Store(expr) => {
                    self.check_expr(expr, env, None, value_stack)
                }
                crate::PatchInstructionKind::Remove => true,
            };
            selector_ok && instruction_ok
        })
    }

    fn check_patch_entry(
        &mut self,
        entry: &crate::PatchEntry,
        subject: &GateType,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        self.check_patch_selector_segments(
            &entry.selector.segments,
            0,
            subject,
            &entry.instruction,
            env,
            value_stack,
        )
    }

    fn check_patch_selector_segments(
        &mut self,
        segments: &[crate::PatchSelectorSegment],
        index: usize,
        current: &GateType,
        instruction: &crate::PatchInstruction,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        if index == segments.len() {
            return self.check_patch_instruction(instruction, current, env, value_stack);
        }
        match &segments[index] {
            crate::PatchSelectorSegment::Named { name, dotted, span } => {
                if let GateType::Record(fields) = current {
                    let Some(field) = fields.iter().find(|field| field.name == name.text()) else {
                        self.emit_unknown_patch_field(*span, name.text(), *dotted, current);
                        return false;
                    };
                    return self.check_patch_selector_segments(
                        segments,
                        index + 1,
                        &field.ty,
                        instruction,
                        env,
                        value_stack,
                    );
                }
                // Handle `<|` patching of imported record types (OpaqueImport with TypeConstructor fields).
                if let GateType::OpaqueImport { import, .. } = current {
                    let binding = &self.module.imports()[*import];
                    if let crate::ImportBindingMetadata::TypeConstructor {
                        fields: Some(record_fields),
                        ..
                    } = &binding.metadata
                    {
                        if let Some(field) = record_fields
                            .iter()
                            .find(|f| f.name.as_ref() == name.text())
                        {
                            let field_ty = self.typing.lower_import_value_type(&field.ty);
                            return self.check_patch_selector_segments(
                                segments,
                                index + 1,
                                &field_ty,
                                instruction,
                                env,
                                value_stack,
                            );
                        }
                        self.emit_unknown_patch_field(*span, name.text(), *dotted, current);
                        return false;
                    }
                }
                if *dotted {
                    self.emit_invalid_patch_selector(
                        *span,
                        "field selector",
                        current,
                        "record values support `<field>` and `.<field>` patch selectors",
                    );
                    return false;
                }
                let Some(next) = self.patch_constructor_focus(current, name.text(), *span) else {
                    return false;
                };
                self.check_patch_selector_segments(
                    segments,
                    index + 1,
                    &next,
                    instruction,
                    env,
                    value_stack,
                )
            }
            crate::PatchSelectorSegment::BracketTraverse { span } => {
                let next = match current {
                    GateType::List(element) => Some(element.as_ref().clone()),
                    GateType::Map { value, .. } => Some(value.as_ref().clone()),
                    _ => None,
                };
                let Some(next) = next else {
                    self.emit_invalid_patch_selector(
                        *span,
                        "traversal selector",
                        current,
                        "`[*]` patch traversal is currently supported for `List` and `Map` values only",
                    );
                    return false;
                };
                self.check_patch_selector_segments(
                    segments,
                    index + 1,
                    &next,
                    instruction,
                    env,
                    value_stack,
                )
            }
            crate::PatchSelectorSegment::BracketExpr { expr, span } => match current {
                GateType::List(element) => {
                    let bool_ty = GateType::Primitive(BuiltinType::Bool);
                    let predicate_ok =
                        self.check_expr_with_ambient(*expr, env, Some(&bool_ty), element);
                    let rest_ok = self.check_patch_selector_segments(
                        segments,
                        index + 1,
                        element,
                        instruction,
                        env,
                        value_stack,
                    );
                    predicate_ok && rest_ok
                }
                GateType::Map { key, value } => {
                    let entry_ty = patch_map_entry_type(key, value);
                    let bool_ty = GateType::Primitive(BuiltinType::Bool);
                    if self.expr_matches_with_ambient(*expr, env, &bool_ty, &entry_ty) {
                        let predicate_ok =
                            self.check_expr_with_ambient(*expr, env, Some(&bool_ty), &entry_ty);
                        let rest_ok = self.check_patch_selector_segments(
                            segments,
                            index + 1,
                            value,
                            instruction,
                            env,
                            value_stack,
                        );
                        predicate_ok && rest_ok
                    } else {
                        let key_ok = self.check_expr(*expr, env, Some(key), value_stack);
                        let rest_ok = self.check_patch_selector_segments(
                            segments,
                            index + 1,
                            value,
                            instruction,
                            env,
                            value_stack,
                        );
                        key_ok && rest_ok
                    }
                }
                _ => {
                    self.emit_invalid_patch_selector(
                        *span,
                        "bracket selector",
                        current,
                        "predicate and keyed patch selectors are currently supported on `List` and `Map` only",
                    );
                    false
                }
            },
        }
    }

    fn check_patch_instruction(
        &mut self,
        instruction: &crate::PatchInstruction,
        focus: &GateType,
        env: &GateExprEnv,
        value_stack: &mut Vec<ItemId>,
    ) -> bool {
        match instruction.kind {
            crate::PatchInstructionKind::Replace(expr) => {
                let transform_ty = GateType::Arrow {
                    parameter: Box::new(focus.clone()),
                    result: Box::new(focus.clone()),
                };
                if self.expr_matches_with_ambient(expr, env, &transform_ty, focus) {
                    self.check_expr_with_ambient(expr, env, Some(&transform_ty), focus)
                } else {
                    self.check_expr_with_ambient(expr, env, Some(focus), focus)
                }
            }
            crate::PatchInstructionKind::Store(expr) => {
                self.check_expr_with_ambient(expr, env, Some(focus), focus)
            }
            crate::PatchInstructionKind::Remove => {
                // Patch removal is accepted; the focus type confirms the field exists.
                // Result-type shrinking is handled by check_patch_apply_expr which
                // applies Omit to the target type after collecting removed fields.
                let _ = value_stack;
                true
            }
        }
    }

    fn patch_constructor_focus(
        &mut self,
        subject: &GateType,
        constructor: &str,
        span: SourceSpan,
    ) -> Option<GateType> {
        match subject {
            GateType::Option(value) if constructor == "Some" => Some(value.as_ref().clone()),
            GateType::Result { value, .. } if constructor == "Ok" => Some(value.as_ref().clone()),
            GateType::Result { error, .. } if constructor == "Err" => Some(error.as_ref().clone()),
            GateType::Validation { value, .. } if constructor == "Valid" => {
                Some(value.as_ref().clone())
            }
            GateType::Validation { error, .. } if constructor == "Invalid" => {
                Some(error.as_ref().clone())
            }
            GateType::OpaqueItem {
                item, arguments, ..
            } => {
                let Item::Type(type_item) = &self.module.items()[*item] else {
                    self.emit_unknown_patch_constructor(span, constructor, subject);
                    return None;
                };
                let TypeItemBody::Sum(variants) = &type_item.body else {
                    self.emit_unknown_patch_constructor(span, constructor, subject);
                    return None;
                };
                let Some(variant) = variants
                    .iter()
                    .find(|variant| variant.name.text() == constructor)
                else {
                    self.emit_unknown_patch_constructor(span, constructor, subject);
                    return None;
                };
                if variant.fields.len() != 1 {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "constructor selector `{constructor}` currently requires exactly one payload field"
                        ))
                        .with_code(code("unsupported-patch-constructor-shape"))
                        .with_primary_label(
                            span,
                            "patch constructor focus currently continues through single-payload constructors only",
                        ),
                    );
                    return None;
                }
                let substitutions = type_item
                    .parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let Some(field_ty) = self
                    .typing
                    .lower_hir_type(variant.fields[0].ty, &substitutions)
                else {
                    self.emit_unknown_patch_constructor(span, constructor, subject);
                    return None;
                };
                Some(field_ty)
            }
            _ => {
                self.emit_unknown_patch_constructor(span, constructor, subject);
                None
            }
        }
    }

    fn require_default(&mut self, ty: &GateType) -> Result<DefaultEvidence, String> {
        if matches!(ty, GateType::Option(_)) && self.option_default_in_scope {
            return Ok(DefaultEvidence::BuiltinOptionNone);
        }
        if let Some(import) = self.imported_default_value_binding(ty) {
            return Ok(DefaultEvidence::ImportedBinding(import));
        }
        if let Some(body) = self.same_module_default_member_body(ty)? {
            return Ok(DefaultEvidence::SameModuleMemberBody(body));
        }
        match ty {
            GateType::Option(_) => Err(
                "`Option A` only satisfies `Default` here via imported default evidence for `Option` or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Text) => Err(
                "`Text` only satisfies `Default` here via an imported `defaultText` binding or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Int) => Err(
                "`Int` only satisfies `Default` here via an imported `defaultInt` binding or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Bool) => Err(
                "`Bool` only satisfies `Default` here via an imported `defaultBool` binding or a same-module `Default` instance"
                    .to_owned(),
            ),
            _ => Err(
                "resolved-HIR default checking currently accepts same-module `Default` instances and compiler-known imported default evidence only"
                    .to_owned(),
            ),
        }
    }

    fn imported_default_value_binding(&self, ty: &GateType) -> Option<ImportId> {
        let GateType::Primitive(builtin) = ty else {
            return None;
        };
        self.imported_default_values
            .iter()
            .find_map(|binding| (binding.builtin == *builtin).then_some(binding.import))
    }

    fn emit_expr_issues(&mut self, issues: &[GateIssue]) {
        for issue in issues {
            let diagnostic = match issue {
                GateIssue::InvalidPipeStageInput {
                    span,
                    stage,
                    expected,
                    actual,
                } => Diagnostic::error(format!(
                    "`{stage}` stage expects `{actual}` but the current subject is `{expected}`"
                ))
                .with_code(code("invalid-pipe-stage-input"))
                .with_primary_label(
                    *span,
                    "this pipe stage cannot accept the current subject type",
                ),
                GateIssue::InvalidProjection {
                    span,
                    path,
                    subject,
                } => Diagnostic::error(format!(
                    "projection `{path}` cannot be applied to `{subject}`"
                ))
                .with_code(code("invalid-projection"))
                .with_primary_label(
                    *span,
                    "this projection target does not support field access",
                ),
                GateIssue::UnknownField {
                    span,
                    path,
                    subject,
                } => Diagnostic::error(format!(
                    "projection `{path}` is not available on `{subject}`"
                ))
                .with_code(code("unknown-projection-field"))
                .with_primary_label(*span, "this projection refers to a missing record field"),
                GateIssue::AmbientSubjectOutsidePipe { span } => {
                    Diagnostic::error(
                        "`.` is only available when a pipe stage provides an ambient subject",
                    )
                    .with_code(code("ambient-subject-outside-pipe"))
                    .with_primary_label(
                        *span,
                        "use `.` inside a pipe stage or bind the value to a name first",
                    )
                }
                GateIssue::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                } => Diagnostic::error(format!(
                    "domain member `{name}` is ambiguous in this context"
                ))
                .with_code(code("ambiguous-domain-member"))
                .with_primary_label(
                    *span,
                    "add more type context or rename/import an alias for the desired member",
                )
                .with_note(format!("candidates: {}", candidates.join(", "))),
                GateIssue::UnsupportedApplicativeClusterMember { span, actual } => {
                    Diagnostic::error(format!(
                        "`&|>` cluster members must have a supported applicative type, found `{actual}`"
                    ))
                    .with_code(code("unsupported-applicative-cluster-member"))
                    .with_primary_label(
                        *span,
                        "this cluster member does not have a resolved applicative outer type",
                    )
                    .with_note(
                        "resolved-HIR cluster typing currently accepts `List`, `Option`, `Result E`, `Validation E`, `Signal`, and `Task E` members with one shared outer constructor",
                    )
                }
                GateIssue::ApplicativeClusterMismatch {
                    span,
                    expected,
                    actual,
                } => Diagnostic::error(format!(
                    "`&|>` cluster mixes `{expected}` with `{actual}`"
                ))
                .with_code(code("applicative-cluster-mismatch"))
                .with_primary_label(
                    *span,
                    "all members in one cluster must share the same outer applicative constructor",
                ),
                GateIssue::InvalidClusterFinalizer {
                    span,
                    expected_inputs,
                    actual,
                } => Diagnostic::error(format!(
                    "`&|>` cluster finalizer must accept payloads {} in member order, found `{actual}`",
                    expected_inputs
                        .iter()
                        .map(|input| format!("`{input}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .with_code(code("invalid-cluster-finalizer"))
                .with_primary_label(
                    *span,
                    "this finalizer cannot be applied to the current cluster member payloads",
                ),
                GateIssue::CaseBranchTypeMismatch {
                    span,
                    expected,
                    actual,
                } => Diagnostic::error(format!(
                    "case split branches must agree on one result type, found `{expected}` and `{actual}`"
                ))
                .with_code(code("case-branch-type-mismatch"))
                .with_primary_label(
                    *span,
                    "this branch produces a different type than earlier branches in the same case split",
                ),
                GateIssue::AmbiguousDomainOperator {
                    span,
                    operator,
                    candidates,
                } => Diagnostic::error(format!(
                    "binary operator `{operator}` is ambiguous: multiple domain implementations match"
                ))
                .with_code(code("ambiguous-domain-operator"))
                .with_primary_label(
                    *span,
                    "add a type annotation on one operand to disambiguate which domain operator to use",
                )
                .with_note(format!("candidates: {}", candidates.join(", "))),
            };
            self.diagnostics.push(diagnostic);
        }
    }

    fn emit_ambiguous_domain_member(
        &mut self,
        span: SourceSpan,
        reference: &TermReference,
        candidates: &[String],
    ) {
        let name = reference.path.segments().last().text().to_owned();
        self.diagnostics.push(
            Diagnostic::error(format!(
                "domain member `{name}` is ambiguous in this context"
            ))
            .with_code(code("ambiguous-domain-member"))
            .with_primary_label(
                span,
                "add more type context or rename/import an alias for the desired member",
            )
            .with_note(format!("candidates: {}", candidates.join(", "))),
        );
    }

    fn emit_ambiguous_class_member(
        &mut self,
        span: SourceSpan,
        reference: &TermReference,
        candidates: &[String],
    ) {
        let name = reference.path.segments().last().text().to_owned();
        self.diagnostics.push(
            Diagnostic::error(format!(
                "class member `{name}` is ambiguous in this context"
            ))
            .with_code(code("ambiguous-class-member"))
            .with_primary_label(
                span,
                "add more type context or rename a local binding that shadows the intended member",
            )
            .with_note(format!("candidates: {}", candidates.join(", "))),
        );
    }

    fn emit_invalid_unary_operator(
        &mut self,
        span: SourceSpan,
        operator: UnaryOperator,
        actual: Option<&GateType>,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "operator `{}` expects a `Bool` operand, found {}",
                unary_operator_text(operator),
                describe_inferred_type(actual)
            ))
            .with_code(code("invalid-unary-operator"))
            .with_primary_label(span, "use a `Bool` expression here"),
        );
    }

    fn emit_invalid_binary_operator(
        &mut self,
        span: SourceSpan,
        operator: BinaryOperator,
        left: Option<&GateType>,
        right: Option<&GateType>,
        expectation: BinaryOperatorExpectation,
    ) {
        let (expected_operands, label) = match expectation {
            BinaryOperatorExpectation::BoolOperands => (
                "`Bool` operands",
                "both operands must have type `Bool` here",
            ),
            BinaryOperatorExpectation::MatchingNumericOperands => (
                "matching numeric operands",
                "both operands must resolve to the same numeric type here",
            ),
            BinaryOperatorExpectation::CommonTypeOperands => (
                "operands that resolve to one common type",
                "both operands must resolve to one shared type here",
            ),
        };
        let mut diag = Diagnostic::error(format!(
            "operator `{}` expects {expected_operands}, found {} and {}",
            binary_operator_text(operator),
            describe_inferred_type(left),
            describe_inferred_type(right),
        ))
        .with_code(code("invalid-binary-operator"))
        .with_primary_label(span, label);

        // Suggest fixes for common mismatches.
        match expectation {
            BinaryOperatorExpectation::BoolOperands => {
                diag =
                    diag.with_help("logical operators `and`, `or` require both sides to be `Bool`");
            }
            BinaryOperatorExpectation::MatchingNumericOperands => {
                if let (Some(l), Some(r)) = (left, right)
                    && l != r {
                        diag = diag.with_help(
                            "convert one operand so both sides share the same numeric type",
                        );
                    }
            }
            BinaryOperatorExpectation::CommonTypeOperands => {}
        }

        self.diagnostics.push(diag);
    }

    fn handle_constraints(&mut self, constraints: &[TypeConstraint]) -> ConstraintSolveReport {
        let mut report = ConstraintSolveReport::default();
        for constraint in constraints {
            match constraint.class() {
                ConstraintClass::Eq => {
                    self.enqueue_eq_constraint(constraint);
                }
                ConstraintClass::Default => match self.require_default(constraint.subject()) {
                    Ok(evidence) => {
                        if let Some(field_name) = constraint.omitted_field_name() {
                            report.default_record_fields.push(SolvedDefaultRecordField {
                                field_name: field_name.to_owned(),
                                evidence,
                            });
                        }
                    }
                    Err(reason) => {
                        let field_name = constraint.omitted_field_name().unwrap_or("this field");
                        let mut diag = Diagnostic::error(format!(
                                "record literal omits field `{field_name}` but no `Default` instance is in scope for `{}`",
                                constraint.subject()
                            ))
                            .with_code(code("missing-default-instance"))
                            .with_primary_label(
                                constraint.span(),
                                format!("field `{field_name}` must be provided or defaultable here"),
                            )
                            .with_note(reason);
                        if let Some(available) = constraint.available_field_names()
                            && !available.is_empty()
                        {
                            diag = diag
                                .with_note(format!("available fields: {}", available.join(", ")));
                        }
                        self.diagnostics.push(diag);
                    }
                },
            }
        }
        report
    }

    fn enqueue_eq_constraints(&mut self, constraints: &[TypeConstraint]) {
        for constraint in constraints {
            if matches!(constraint.class(), ConstraintClass::Eq) {
                self.enqueue_eq_constraint(constraint);
            }
        }
    }

    fn enqueue_eq_constraint(&mut self, constraint: &TypeConstraint) {
        debug_assert!(matches!(constraint.class(), ConstraintClass::Eq));
        let pending = PendingEqConstraint {
            constraint: constraint.clone(),
            scope: self.current_eq_constraint_scope(),
        };
        if self
            .pending_eq_constraints.contains(&pending)
        {
            return;
        }
        self.pending_eq_constraints.push(pending);
    }

    fn current_eq_constraint_scope(&self) -> EqConstraintScope {
        EqConstraintScope {
            constrained_parameters: self.eq_constrained_parameters.clone(),
        }
    }

    fn solve_pending_eq_constraints(&mut self) {
        let constraints = std::mem::take(&mut self.pending_eq_constraints);
        for pending in constraints {
            if let Err(reason) = self.require_eq_with_scope(
                pending.constraint.subject(),
                &pending.scope,
                &mut Vec::new(),
            ) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "this expression requires `Eq` for `{}`",
                        pending.constraint.subject()
                    ))
                    .with_code(code("missing-eq-instance"))
                    .with_primary_label(
                        pending.constraint.span(),
                        format!(
                            "`{}` does not currently have `Eq` evidence",
                            pending.constraint.subject()
                        ),
                    )
                    .with_note(reason),
                );
            }
        }
    }

    fn fallback_apply_parameter_types(
        &mut self,
        callee: ExprId,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
    ) -> Option<Vec<GateType>> {
        let mut parameter_types = arguments
            .iter()
            .map(|argument| self.inferred_expr_type(*argument, env))
            .collect::<Vec<_>>();
        if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind
            && let Some(named_parameter_types) =
                self.named_function_parameter_types(reference, arguments.len())
            {
                for (slot, named_parameter_ty) in parameter_types
                    .iter_mut()
                    .zip(named_parameter_types.into_iter())
                {
                    if named_parameter_ty.is_some() {
                        *slot = named_parameter_ty;
                    }
                }
            }
        parameter_types.into_iter().collect()
    }

    fn named_function_parameter_types(
        &mut self,
        reference: &TermReference,
        arity: usize,
    ) -> Option<Vec<Option<GateType>>> {
        let crate::ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let parameters = match &self.module.items()[*item_id] {
            Item::Function(item) if item.parameters.len() == arity => item.parameters.clone(),
            _ => return None,
        };
        Some(
            parameters
                .into_iter()
                .map(|parameter| {
                    parameter
                        .annotation
                        .and_then(|annotation| self.typing.lower_annotation(annotation))
                })
                .collect(),
        )
    }

    fn match_function_constraints(
        &mut self,
        function: &FunctionItem,
        arguments: &crate::NonEmpty<ExprId>,
        argument_types: &[GateType],
        expected_result: &GateType,
    ) -> Option<(Vec<GateType>, Vec<ClassConstraintBinding>)> {
        if function.parameters.len() < argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(argument_types.len());
        for ((parameter, argument_expr), actual) in function
            .parameters
            .iter()
            .zip(arguments.iter())
            .zip(argument_types.iter())
        {
            let annotation = parameter.annotation?;
            let payload = signal_name_payload_type(self.module, *argument_expr, actual);
            if let Some(lowered) = self.typing.lower_annotation(annotation) {
                if !lowered.same_shape(actual)
                    && !payload.is_some_and(|payload| lowered.same_shape(payload))
                {
                    return None;
                }
                instantiated_parameters.push(lowered);
                continue;
            }
            let mut direct_bindings = bindings.clone();
            if self
                .typing
                .match_poly_hir_type(annotation, actual, &mut direct_bindings)
            {
                bindings = direct_bindings;
            } else if let Some(payload) = payload {
                let mut payload_bindings = bindings.clone();
                if !self
                    .typing
                    .match_poly_hir_type(annotation, payload, &mut payload_bindings)
                {
                    return None;
                }
                bindings = payload_bindings;
            } else {
                return None;
            }
            instantiated_parameters.push(
                self.typing
                    .instantiate_poly_hir_type(annotation, &bindings)?,
            );
        }
        let result_annotation = function.annotation?;
        if function.parameters.len() == argument_types.len() {
            // Full application: check result type against expected and collect constraints.
            if let Some(lowered) = self.typing.lower_annotation(result_annotation) {
                if !lowered.same_shape(expected_result) {
                    return None;
                }
            } else if !self.typing.match_poly_hir_type(
                result_annotation,
                expected_result,
                &mut bindings,
            ) {
                return None;
            }
            let constraints = function
                .context
                .iter()
                .map(|constraint| self.typing.class_constraint_binding(*constraint, &bindings))
                .collect::<Option<Vec<_>>>()?;
            Some((instantiated_parameters, constraints))
        } else {
            // Partial application: build the curried result type from the remaining parameters
            // and check it against expected_result.
            let remaining_params = &function.parameters[argument_types.len()..];
            let remaining_types = remaining_params
                .iter()
                .map(|p| {
                    p.annotation.and_then(|ann| {
                        self.typing.lower_annotation(ann).or_else(|| {
                            self.typing
                                .instantiate_poly_hir_type_partially(ann, &bindings)
                        })
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            let result_ty = self
                .typing
                .lower_annotation(result_annotation)
                .or_else(|| {
                    self.typing
                        .instantiate_poly_hir_type_partially(result_annotation, &bindings)
                })?;
            let curried_result = self.arrow_type(&remaining_types, &result_ty);
            if !curried_result.same_shape(expected_result)
                && !curried_result.fits_template(expected_result)
            {
                return None;
            }
            // Collect only constraints whose type parameter is fully resolved.
            let constraints = function
                .context
                .iter()
                .filter_map(|constraint| {
                    self.typing.class_constraint_binding(*constraint, &bindings)
                })
                .collect::<Vec<_>>();
            Some((instantiated_parameters, constraints))
        }
    }

    fn arrow_type(&self, parameter_types: &[GateType], result: &GateType) -> GateType {
        let mut current = result.clone();
        for parameter in parameter_types.iter().rev() {
            current = GateType::Arrow {
                parameter: Box::new(parameter.clone()),
                result: Box::new(current),
            };
        }
        current
    }

    fn require_eq_with_scope(
        &mut self,
        ty: &GateType,
        scope: &EqConstraintScope,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<(), String> {
        if self
            .require_compiler_derived_eq_with_scope(ty, scope, item_stack)
            .is_ok()
        {
            return Ok(());
        }
        if let Some(class_item_id) = self.class_item_id_by_name("Eq")
            && self
                .resolve_same_module_instance(class_item_id, ty)?
                .is_some()
            {
                return Ok(());
            }
        self.require_compiler_derived_eq_with_scope(ty, scope, item_stack)
    }

    fn require_compiler_derived_eq(
        &mut self,
        ty: &GateType,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<(), String> {
        let scope = self.current_eq_constraint_scope();
        self.require_compiler_derived_eq_with_scope(ty, &scope, item_stack)
    }

    fn require_compiler_derived_eq_with_scope(
        &mut self,
        ty: &GateType,
        scope: &EqConstraintScope,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<(), String> {
        match ty {
            GateType::Primitive(BuiltinType::Bytes) => {
                Err("`Bytes` does not have a compiler-derived `Eq` instance in v1".to_owned())
            }
            GateType::Primitive(_) => Ok(()),
            GateType::TypeParameter { parameter, name } => {
                if scope.constrained_parameters.contains(parameter) {
                    Ok(())
                } else {
                    Err(format!(
                        "open type parameter `{name}` does not have a compiler-derived `Eq` \
                         instance in v1; add `(Eq {name}) ->` to the function annotation to \
                         require it"
                    ))
                }
            }
            GateType::Tuple(elements) => {
                for element in elements {
                    self.require_eq_with_scope(element, scope, item_stack)?;
                }
                Ok(())
            }
            GateType::Record(fields) => {
                for field in fields {
                    self.require_eq_with_scope(&field.ty, scope, item_stack)?;
                }
                Ok(())
            }
            GateType::List(element) | GateType::Option(element) => {
                self.require_eq_with_scope(element, scope, item_stack)
            }
            GateType::Result { error, value } | GateType::Validation { error, value } => {
                self.require_eq_with_scope(error, scope, item_stack)?;
                self.require_eq_with_scope(value, scope, item_stack)
            }
            GateType::Domain {
                item, arguments, ..
            } => {
                if item_stack.contains(item) {
                    return Ok(());
                }
                let (parameters, carrier) = match &self.module.items()[*item] {
                    Item::Domain(domain) => (domain.parameters.clone(), domain.carrier),
                    _ => return Err(format!("`{ty}` does not refer to a domain declaration")),
                };
                let substitutions = parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<TypeParameterId, GateType>>();
                let Some(carrier) = self.typing.lower_hir_type(carrier, &substitutions) else {
                    return Err(format!(
                        "the carrier type for `{ty}` could not be lowered for Eq checking"
                    ));
                };
                item_stack.push(*item);
                let result = self.require_eq_with_scope(&carrier, scope, item_stack);
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(*item));
                result
            }
            GateType::OpaqueItem {
                item, arguments, ..
            } => {
                if item_stack.contains(item) {
                    return Ok(());
                }
                let (parameters, body) = match &self.module.items()[*item] {
                    Item::Type(item_ty) => (item_ty.parameters.clone(), item_ty.body.clone()),
                    _ => return Err(format!("`{ty}` does not refer to a type declaration")),
                };
                let substitutions = parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<TypeParameterId, GateType>>();
                item_stack.push(*item);
                let result = match body {
                    TypeItemBody::Alias(alias) => {
                        let Some(lowered) = self.typing.lower_hir_type(alias, &substitutions)
                        else {
                            return Err(format!(
                                "the alias body for `{ty}` could not be lowered for Eq checking"
                            ));
                        };
                        self.require_eq_with_scope(&lowered, scope, item_stack)
                    }
                    TypeItemBody::Sum(variants) => {
                        for variant in variants.iter() {
                            for field in &variant.fields {
                                let Some(lowered) =
                                    self.typing.lower_hir_type(field.ty, &substitutions)
                                else {
                                    return Err(format!(
                                        "constructor payloads for `{ty}` could not be lowered for Eq checking"
                                    ));
                                };
                                self.require_eq_with_scope(&lowered, scope, item_stack)?;
                            }
                        }
                        Ok(())
                    }
                };
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(*item));
                result
            }
            GateType::Arrow { .. }
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. } => Err(format!(
                "`{ty}` does not have a compiler-derived `Eq` instance in v1"
            )),
            // Imported types are opaque; their Eq derivation is checked in their
            // defining module, so we optimistically accept them here.
            GateType::OpaqueImport { .. } => Ok(()),
        }
    }

    fn class_item_id_by_name(&self, class_name: &str) -> Option<ItemId> {
        self.module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Class(class_item) if class_item.name.text() == class_name => Some(item_id),
                _ => None,
            })
    }

    fn class_name(&self, class_item_id: ItemId) -> Option<&str> {
        match &self.module.items()[class_item_id] {
            Item::Class(class_item) => Some(class_item.name.text()),
            _ => None,
        }
    }

    fn class_constraint_binding_label(&self, binding: &ClassConstraintBinding) -> String {
        let class_name = self.class_name(binding.class_item).unwrap_or("<class>");
        format!("{class_name} {}", self.type_binding_label(&binding.subject))
    }

    fn emit_missing_instance_requirement(
        &mut self,
        span: SourceSpan,
        class_item_id: ItemId,
        requirement: &ClassConstraintBinding,
        reason: &str,
    ) {
        let class_name = self.class_name(class_item_id).unwrap_or("<class>");
        self.diagnostics.push(
            Diagnostic::error(format!(
                "instance for `{class_name}` is missing required `{}` evidence",
                self.class_constraint_binding_label(requirement)
            ))
            .with_code(code("missing-instance-requirement"))
            .with_primary_label(
                span,
                "add this constraint to the instance context or provide matching evidence",
            )
            .with_note(reason.to_owned()),
        );
    }

    fn type_binding_label(&self, binding: &TypeBinding) -> String {
        match binding {
            TypeBinding::Type(ty) => ty.to_string(),
            TypeBinding::Constructor(binding) => match binding.head() {
                crate::validate::TypeConstructorHead::Builtin(builtin) => {
                    format!("{builtin:?}")
                }
                crate::validate::TypeConstructorHead::Item(item_id) => {
                    match &self.module.items()[item_id] {
                        Item::Type(item) => item.name.text().to_owned(),
                        Item::Domain(item) => item.name.text().to_owned(),
                        Item::Class(item) => item.name.text().to_owned(),
                        _ => "<constructor>".to_owned(),
                    }
                }
                crate::validate::TypeConstructorHead::Import(import_id) => self.module.imports()
                    [import_id]
                    .local_name
                    .text()
                    .to_owned(),
            },
        }
    }

    fn class_member_dispatch(
        &mut self,
        matched: &ClassMemberCallMatch,
    ) -> Option<ResolvedClassMemberDispatch> {
        let implementation =
            self.class_member_implementation(matched.resolution, &matched.evidence.subject)?;
        Some(ResolvedClassMemberDispatch {
            member: matched.resolution,
            subject: matched.evidence.subject.clone(),
            implementation,
        })
    }

    fn class_member_implementation(
        &mut self,
        resolution: ClassMemberResolution,
        subject: &TypeBinding,
    ) -> Option<ClassMemberImplementation> {
        let class_name = self.class_name(resolution.class)?.to_owned();
        if matches!(class_name.as_str(), "Eq" | "Setoid")
            && matches!(subject, TypeBinding::Type(ty) if self.require_compiler_derived_eq(ty, &mut Vec::new()).is_ok())
        {
            return Some(ClassMemberImplementation::Builtin);
        }
        if let Ok(Some((instance_id, instance))) =
            self.resolve_same_module_instance_binding_with_id(resolution.class, subject)
        {
            let Item::Class(class_item) = &self.module.items()[resolution.class] else {
                return None;
            };
            let member_name = class_item.members.get(resolution.member_index)?.name.text();
            let member_index = instance
                .members
                .iter()
                .position(|member| member.name.text() == member_name)?;
            return Some(ClassMemberImplementation::SameModuleInstance {
                instance: instance_id,
                member_index,
            });
        }
        if self.has_builtin_class_instance_binding(class_name.as_str(), subject) {
            return Some(ClassMemberImplementation::Builtin);
        }
        // Check imported instances from other modules.
        if let Some(import) =
            self.resolve_imported_instance_member(&class_name, resolution, subject)
        {
            return Some(ClassMemberImplementation::ImportedInstance { import });
        }
        None
    }

    /// Search through all import bindings for an `InstanceMember` matching the given
    /// class, member, and subject type.
    fn resolve_imported_instance_member(
        &self,
        class_name: &str,
        resolution: ClassMemberResolution,
        subject: &TypeBinding,
    ) -> Option<ImportId> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member_name = class_item.members.get(resolution.member_index)?.name.text();
        let subject_label = self.type_binding_label(subject);
        for (import_id, import) in self.module.imports().iter() {
            if let ImportBindingMetadata::InstanceMember {
                class_name: ic,
                member_name: im,
                subject: is,
                ..
            } = &import.metadata
                && ic.as_ref() == class_name
                    && im.as_ref() == member_name
                    && is.as_ref() == subject_label.as_str()
                {
                    return Some(import_id);
                }
        }
        None
    }

    fn solve_class_constraint_bindings(
        &mut self,
        evidence_span: SourceSpan,
        evidence: &ClassConstraintBinding,
        constraints: &[ClassConstraintBinding],
    ) -> Result<(), String> {
        self.require_class_binding(evidence)?;
        for constraint in constraints {
            self.require_class_binding(constraint).map_err(|reason| {
                format!(
                    "{reason} (required by `{}` at {evidence_span:?})",
                    self.class_constraint_binding_label(constraint)
                )
            })?;
        }
        Ok(())
    }

    fn require_class_binding(&mut self, binding: &ClassConstraintBinding) -> Result<(), String> {
        if self.in_scope_class_constraints.contains(binding) {
            return Ok(());
        }
        let Some(class_name) = self.class_name(binding.class_item).map(str::to_owned) else {
            return Err("constraint does not reference a class item".to_owned());
        };
        if matches!(class_name.as_str(), "Eq" | "Setoid")
            && matches!(&binding.subject, TypeBinding::Type(_))
        {
            let TypeBinding::Type(ty) = &binding.subject else {
                unreachable!();
            };
            if self
                .require_compiler_derived_eq(ty, &mut Vec::new())
                .is_ok()
            {
                return Ok(());
            }
        }
        if self.has_builtin_class_instance_binding(class_name.as_str(), &binding.subject) {
            return Ok(());
        }
        if self
            .resolve_same_module_instance_binding(binding.class_item, &binding.subject)?
            .is_some()
        {
            return Ok(());
        }
        Err(format!(
            "no compiler-provided or same-module `{class_name}` instance matches `{}`",
            self.type_binding_label(&binding.subject)
        ))
    }

    #[allow(dead_code)]
    fn require_class_named(&mut self, class_name: &str, ty: &GateType) -> Result<(), String> {
        if self.has_builtin_class_instance(class_name, ty) {
            return Ok(());
        }
        let Some(class_item_id) = self.class_item_id_by_name(class_name) else {
            return Err(format!(
                "no compiler-provided or same-module `{class_name}` instance matches `{ty}`"
            ));
        };
        if self
            .require_class_binding(&ClassConstraintBinding {
                class_item: class_item_id,
                subject: TypeBinding::Type(ty.clone()),
            })
            .is_ok()
        {
            return Ok(());
        }
        self.resolve_same_module_instance(class_item_id, ty)?
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "no same-module `{class_name}` instance matches `{ty}` after resolved-HIR unification"
                )
            })
    }

    #[allow(dead_code)]
    fn has_builtin_class_instance(&self, class_name: &str, ty: &GateType) -> bool {
        match class_name {
            "Functor" | "Applicative" => matches!(
                ty,
                GateType::List(_)
                    | GateType::Option(_)
                    | GateType::Result { .. }
                    | GateType::Validation { .. }
                    | GateType::Signal(_)
                    | GateType::Task { .. }
            ),
            "Monad" => matches!(
                ty,
                GateType::List(_)
                    | GateType::Option(_)
                    | GateType::Result { .. }
                    | GateType::Task { .. }
            ),
            _ => false,
        }
    }

    fn has_builtin_class_instance_binding(
        &mut self,
        class_name: &str,
        subject: &TypeBinding,
    ) -> bool {
        match subject {
            TypeBinding::Type(ty) => match class_name {
                "Ord" => {
                    matches!(
                        ty,
                        GateType::Primitive(
                            BuiltinType::Int
                                | BuiltinType::Float
                                | BuiltinType::Decimal
                                | BuiltinType::BigInt
                                | BuiltinType::Bool
                                | BuiltinType::Text
                        )
                    ) || matches!(
                        ty,
                        GateType::OpaqueItem { name, .. } if name == "Ordering"
                    )
                }
                "Semigroup" | "Monoid" => matches!(
                    ty,
                    GateType::Primitive(BuiltinType::Text) | GateType::List(_)
                ),
                "Bifunctor" => matches!(ty, GateType::Result { .. } | GateType::Validation { .. }),
                _ => self.has_builtin_class_instance(class_name, ty),
            },
            TypeBinding::Constructor(binding) => match class_name {
                "Functor" | "Apply" | "Applicative" => self.matches_builtin_head(
                    binding,
                    &[
                        BuiltinType::List,
                        BuiltinType::Option,
                        BuiltinType::Result,
                        BuiltinType::Validation,
                        BuiltinType::Signal,
                        BuiltinType::Task,
                    ],
                ),
                "Alt" | "Plus" | "Alternative" => self.matches_builtin_head(
                    binding,
                    &[
                        BuiltinType::List,
                        BuiltinType::Option,
                        BuiltinType::Result,
                        BuiltinType::Validation,
                    ],
                ),
                "Chain" | "Monad" | "ChainRec" => self.matches_builtin_head(
                    binding,
                    &[
                        BuiltinType::List,
                        BuiltinType::Option,
                        BuiltinType::Result,
                        BuiltinType::Task,
                    ],
                ),
                "Foldable" | "Traversable" => self.matches_builtin_head(
                    binding,
                    &[
                        BuiltinType::List,
                        BuiltinType::Option,
                        BuiltinType::Result,
                        BuiltinType::Validation,
                    ],
                ),
                "Filterable" => {
                    self.matches_builtin_head(binding, &[BuiltinType::List, BuiltinType::Option])
                }
                "Bifunctor" => self
                    .matches_builtin_head(binding, &[BuiltinType::Result, BuiltinType::Validation]),
                _ => false,
            },
        }
    }

    fn matches_builtin_head(
        &self,
        binding: &crate::validate::TypeConstructorBinding,
        allowed: &[BuiltinType],
    ) -> bool {
        matches!(binding.head(), crate::validate::TypeConstructorHead::Builtin(builtin) if allowed.contains(&builtin))
    }

    fn resolve_same_module_instance(
        &mut self,
        class_item_id: ItemId,
        ty: &GateType,
    ) -> Result<Option<InstanceItem>, String> {
        let instances = self
            .module
            .items()
            .iter()
            .filter_map(|(_, item)| match item {
                Item::Instance(instance) => Some(instance.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut matches = Vec::new();
        for instance in instances {
            if self.instance_class_item_id(&instance) != Some(class_item_id)
                || instance.arguments.len() != 1
            {
                continue;
            }
            let mut bindings = PolyTypeBindings::new();
            if self
                .typing
                .match_poly_hir_type(*instance.arguments.first(), ty, &mut bindings)
            {
                matches.push(instance);
            }
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(format!(
                "multiple same-module instances match `{ty}` for `{}`; overlapping instances are not yet supported here",
                match &self.module.items()[class_item_id] {
                    Item::Class(class_item) => class_item.name.text(),
                    _ => "<class>",
                }
            )),
        }
    }

    fn resolve_same_module_instance_binding(
        &mut self,
        class_item_id: ItemId,
        subject: &TypeBinding,
    ) -> Result<Option<InstanceItem>, String> {
        self.resolve_same_module_instance_binding_with_id(class_item_id, subject)
            .map(|resolved| resolved.map(|(_, instance)| instance))
    }

    fn resolve_same_module_instance_binding_with_id(
        &mut self,
        class_item_id: ItemId,
        subject: &TypeBinding,
    ) -> Result<Option<(ItemId, InstanceItem)>, String> {
        let instances = self
            .module
            .items()
            .iter()
            .filter_map(|(item_id, item)| match item {
                Item::Instance(instance) => Some((item_id, instance.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut matches = Vec::new();
        for (instance_id, instance) in instances {
            if self.instance_class_item_id(&instance) != Some(class_item_id)
                || instance.arguments.len() != 1
            {
                continue;
            }
            let mut bindings = PolyTypeBindings::new();
            if self.typing.match_poly_type_binding(
                *instance.arguments.first(),
                subject,
                &mut bindings,
            ) {
                matches.push((instance_id, instance));
            }
        }
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(format!(
                "multiple same-module instances match `{}` for `{}`; overlapping instances are not yet supported here",
                self.type_binding_label(subject),
                self.class_name(class_item_id).unwrap_or("<class>")
            )),
        }
    }

    fn same_module_default_member_body(&mut self, ty: &GateType) -> Result<Option<ExprId>, String> {
        let Some(class_item_id) = self.class_item_id_by_name("Default") else {
            return Ok(None);
        };
        let Some(instance) = self.resolve_same_module_instance(class_item_id, ty)? else {
            return Ok(None);
        };
        Ok(instance
            .members
            .iter()
            .find(|member| member.name.text() == "default" && member.parameters.is_empty())
            .map(|member| member.body))
    }

    fn instance_class_item_id(&self, item: &InstanceItem) -> Option<ItemId> {
        let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
            item.class.resolution.as_ref()
        else {
            return None;
        };
        matches!(self.module.items()[*item_id], Item::Class(_)).then_some(*item_id)
    }

    fn instance_argument_bindings(
        &mut self,
        class_item_id: ItemId,
        item: &InstanceItem,
    ) -> Option<PolyTypeBindings> {
        let Item::Class(class_item) = &self.module.items()[class_item_id] else {
            return None;
        };
        if class_item.parameters.len() != item.arguments.len() {
            return None;
        }
        let mut arguments = Vec::with_capacity(item.arguments.len());
        for argument in item.arguments.iter() {
            arguments.push(self.typing.poly_type_binding(*argument)?);
        }
        Some(
            class_item
                .parameters
                .iter()
                .copied()
                .zip(arguments)
                .collect(),
        )
    }

    fn emit_type_mismatch(&mut self, span: SourceSpan, expected: &GateType, actual: &GateType) {
        let mut diag = Diagnostic::error(format!("expected `{expected}` but found `{actual}`"))
            .with_code(code("type-mismatch"))
            .with_primary_label(
                span,
                format!("found `{actual}` here, expected `{expected}`"),
            );

        // Suggest conversions for common primitive mismatches.
        if let (GateType::Primitive(e), GateType::Primitive(a)) = (expected, actual) {
            use crate::hir::BuiltinType;
            match (e, a) {
                (BuiltinType::Text, BuiltinType::Int | BuiltinType::Float) => {
                    diag = diag.with_help("use `toString` to convert a number to text");
                }
                (BuiltinType::Int, BuiltinType::Float) => {
                    diag =
                        diag.with_help("use `round`, `floor`, or `ceil` to convert Float to Int");
                }
                (BuiltinType::Float, BuiltinType::Int) => {
                    diag = diag.with_help("use `toFloat` to convert Int to Float");
                }
                (BuiltinType::Text, BuiltinType::Bool) => {
                    diag = diag.with_help("use `toString` to convert Bool to text");
                }
                _ => {}
            }
        }

        self.diagnostics.push(diag);
    }

    fn emit_type_mismatch_or_unresolved(
        &mut self,
        span: SourceSpan,
        expected: &GateType,
        actual: Option<&GateType>,
    ) {
        match actual {
            Some(actual) => self.emit_type_mismatch(span, expected, actual),
            None => self.diagnostics.push(
                Diagnostic::error(format!(
                    "expected `{expected}` but found {}",
                    describe_inferred_type(None)
                ))
                .with_code(code("type-mismatch"))
                .with_primary_label(
                    span,
                    format!("expected `{expected}` but the type could not be inferred"),
                ),
            ),
        }
    }

    fn emit_invalid_patch_selector(
        &mut self,
        span: SourceSpan,
        selector_kind: &str,
        subject: &GateType,
        note: &str,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "{selector_kind} cannot be applied to `{subject}` in a patch selector"
            ))
            .with_code(code("invalid-patch-selector"))
            .with_primary_label(
                span,
                "this selector segment is not valid for the current focus",
            )
            .with_note(note),
        );
    }

    fn emit_unknown_patch_field(
        &mut self,
        span: SourceSpan,
        field: &str,
        dotted: bool,
        subject: &GateType,
    ) {
        let selector = if dotted {
            format!(".{field}")
        } else {
            field.to_owned()
        };
        self.diagnostics.push(
            Diagnostic::error(format!(
                "field selector `{selector}` is not available on `{subject}`"
            ))
            .with_code(code("unknown-patch-field"))
            .with_primary_label(span, "this patch selector refers to a missing record field"),
        );
    }

    fn emit_unknown_patch_constructor(
        &mut self,
        span: SourceSpan,
        constructor: &str,
        subject: &GateType,
    ) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "constructor selector `{constructor}` is not available on `{subject}`"
            ))
            .with_code(code("unknown-patch-constructor"))
            .with_primary_label(
                span,
                "this patch selector refers to a constructor that does not match the current focus type",
            ),
        );
    }
}
