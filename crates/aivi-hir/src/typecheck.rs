use std::collections::{HashMap, HashSet};

use aivi_base::{Diagnostic, DiagnosticCode, SourceSpan};

use crate::{
    domain_operator_elaboration::select_domain_binary_operator,
    hir::{
        BinaryOperator, BuiltinTerm, BuiltinType, ClassMemberResolution, ExprKind, FunctionItem,
        ImportBindingMetadata, ImportBundleKind, InstanceItem, InstanceMember, Item, MapExpr,
        Module, Name, NamePath, ProjectionBase, RecordExpr, RecordExprField, RecordFieldSurface,
        ResolutionState, SignalItem, TermReference, TermResolution, TypeItemBody, TypeResolution,
        UnaryOperator, ValueItem,
    },
    ids::{ExprId, ImportId, ItemId, TypeId, TypeParameterId},
    validate::{
        ClassConstraintBinding, ClassMemberCallMatch, DomainMemberSelection, GateExprEnv,
        GateIssue, GateRecordField, GateType, GateTypeContext, PolyTypeBindings, TypeBinding,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintClass {
    Eq,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConstraintOrigin {
    Expression,
    RecordOmittedField { field_name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefaultEvidence {
    BuiltinOptionNone,
    ImportedBinding(ImportId),
    SameModuleMemberBody(ExprId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ConstraintSolveReport {
    default_record_fields: Vec<SolvedDefaultRecordField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SolvedDefaultRecordField {
    field_name: String,
    evidence: DefaultEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DefaultRecordElision {
    record_expr: ExprId,
    fields: Vec<SolvedDefaultRecordField>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct EqConstraintScope {
    constrained_parameters: HashSet<TypeParameterId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingEqConstraint {
    constraint: TypeConstraint,
    scope: EqConstraintScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImportedDefaultValue {
    builtin: BuiltinType,
    import: ImportId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeConstraint {
    span: SourceSpan,
    class: ConstraintClass,
    subject: GateType,
    origin: ConstraintOrigin,
}

impl TypeConstraint {
    pub(crate) fn eq(span: SourceSpan, subject: GateType) -> Self {
        Self {
            span,
            class: ConstraintClass::Eq,
            subject,
            origin: ConstraintOrigin::Expression,
        }
    }

    pub(crate) fn default_record_field(
        span: SourceSpan,
        field_name: impl Into<String>,
        subject: GateType,
    ) -> Self {
        Self {
            span,
            class: ConstraintClass::Default,
            subject,
            origin: ConstraintOrigin::RecordOmittedField {
                field_name: field_name.into(),
            },
        }
    }

    pub fn span(&self) -> SourceSpan {
        self.span
    }

    pub fn class(&self) -> &ConstraintClass {
        &self.class
    }

    pub fn subject(&self) -> &GateType {
        &self.subject
    }

    fn omitted_field_name(&self) -> Option<&str> {
        match &self.origin {
            ConstraintOrigin::Expression => None,
            ConstraintOrigin::RecordOmittedField { field_name } => Some(field_name),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeCheckReport {
    diagnostics: Vec<Diagnostic>,
    elisions: Vec<DefaultRecordElision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassMemberImplementation {
    Builtin,
    SameModuleInstance {
        instance: ItemId,
        member_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedClassMemberDispatch {
    pub member: ClassMemberResolution,
    pub subject: TypeBinding,
    pub implementation: ClassMemberImplementation,
}

impl TypeCheckReport {
    fn new(diagnostics: Vec<Diagnostic>, elisions: Vec<DefaultRecordElision>) -> Self {
        Self {
            diagnostics,
            elisions,
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

pub fn typecheck_module(module: &Module) -> TypeCheckReport {
    let mut checker = TypeChecker::new(module);
    checker.run();
    TypeCheckReport::new(checker.diagnostics, checker.default_record_elisions)
}

/// Applies the default-record-field elisions computed by [`typecheck_module`] to `module`,
/// returning the elaborated module with synthesized fields injected.
pub fn apply_defaults(module: &Module, report: &TypeCheckReport) -> Module {
    apply_default_record_elisions(module, &report.elisions)
}

pub fn elaborate_default_record_fields(module: &Module) -> Module {
    apply_defaults(module, &typecheck_module(module))
}

pub(crate) fn expression_matches(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    expected: &GateType,
) -> bool {
    let mut checker = TypeChecker::new(module);
    let matched = checker.check_expr(expr_id, env, Some(expected), &mut Vec::new());
    checker.solve_pending_eq_constraints();
    matched && checker.diagnostics.is_empty()
}

fn signal_name_payload_type<'a>(
    module: &Module,
    expr_id: ExprId,
    actual: &'a GateType,
) -> Option<&'a GateType> {
    matches!(module.exprs()[expr_id].kind, ExprKind::Name(_))
        .then_some(actual)
        .and_then(|actual| match actual {
            GateType::Signal(payload) => Some(payload.as_ref()),
            _ => None,
        })
}

pub(crate) fn resolve_class_member_dispatch(
    module: &Module,
    reference: &TermReference,
    argument_types: &[GateType],
    expected_result: Option<&GateType>,
) -> Option<ResolvedClassMemberDispatch> {
    let mut checker = TypeChecker::new(module);
    match checker
        .typing
        .select_class_member_call(reference, argument_types, expected_result)?
    {
        DomainMemberSelection::Unique(matched) => checker
            .solve_class_constraint_bindings(
                reference.span(),
                &matched.evidence,
                &matched.constraints,
            )
            .ok()
            .and_then(|()| checker.class_member_dispatch(&matched)),
        DomainMemberSelection::Ambiguous | DomainMemberSelection::NoMatch => None,
    }
}

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

    fn run(&mut self) {
        let items = self
            .module
            .items()
            .iter()
            .map(|(_, item)| item.clone())
            .collect::<Vec<_>>();
        for item in items {
            match item {
                Item::Value(item) => self.check_value_item(&item),
                Item::Function(item) => self.check_function_item(&item),
                Item::Signal(item) => self.check_signal_item(&item),
                Item::Instance(item) => self.check_instance_item(&item),
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Use(_)
                | Item::Export(_) => {}
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
            if use_item.module.to_string() != "aivi.defaults" {
                continue;
            }
            for import_id in use_item.imports.iter().copied() {
                let import = &module.imports()[import_id];
                match import.imported_name.text() {
                    "Option"
                        if matches!(
                            &import.metadata,
                            ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
                        ) =>
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

    fn check_function_item(&mut self, item: &FunctionItem) {
        let context = self.constraint_bindings(&item.context, &PolyTypeBindings::new());
        self.with_class_constraint_scope(context, |this| {
            let mut env = GateExprEnv::default();
            for parameter in &item.parameters {
                let Some(annotation) = parameter.annotation else {
                    continue;
                };
                let Some(parameter_ty) = this.typing.lower_open_annotation(annotation) else {
                    continue;
                };
                env.locals.insert(parameter.binding, parameter_ty);
            }
            let expected = item
                .annotation
                .and_then(|annotation| this.typing.lower_open_annotation(annotation));
            this.check_expr(item.body, &env, expected.as_ref(), &mut Vec::new());
        });
    }

    fn check_signal_item(&mut self, item: &SignalItem) {
        let Some(body) = item.body else {
            return;
        };
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        match expected.as_ref() {
            Some(annotation @ GateType::Signal(payload)) => {
                let checkpoint = self.diagnostics.len();
                if self.check_expr(
                    body,
                    &GateExprEnv::default(),
                    Some(annotation),
                    &mut Vec::new(),
                ) {
                    return;
                }
                self.diagnostics.truncate(checkpoint);
                self.check_expr(
                    body,
                    &GateExprEnv::default(),
                    Some(payload.as_ref()),
                    &mut Vec::new(),
                );
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
            }
            None => {
                self.check_inferred_expr(body, &GateExprEnv::default(), None);
            }
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
        if let Some(result) = self.check_operator_expr(expr_id, env, expected, value_stack) {
            return result;
        }

        if let Some(expected) = expected {
            if let Some(result) =
                self.check_expected_special_case(expr_id, env, expected, value_stack)
            {
                return result;
            }
        }

        self.check_inferred_expr(expr_id, env, expected)
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
        {
            if let Some(domain_operator) = select_domain_binary_operator(
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
        if !left_ok || !right_ok {
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
        }

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
        let (parameters, annotated, body) = match &self.module.items()[*item_id] {
            Item::Function(item) => (
                item.parameters.clone(),
                item.annotation.is_some(),
                item.body,
            ),
            _ => return None,
        };
        if annotated || value_stack.contains(item_id) {
            return None;
        }
        let (parameter_types, result_expected) =
            self.expected_function_signature(expected, parameters.len())?;
        let mut env = GateExprEnv::default();
        for (parameter, expected_parameter_ty) in parameters.iter().zip(parameter_types.iter()) {
            if let Some(annotation) = parameter.annotation {
                let Some(parameter_ty) = self.typing.lower_annotation(annotation) else {
                    return None;
                };
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
            let Some(argument_ty) = info.ty.clone().or_else(|| info.actual_gate_type()) else {
                return None;
            };
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
            let Some(argument_ty) = info.ty.clone().or_else(|| info.actual_gate_type()) else {
                return None;
            };
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
            let Some(argument_ty) = info.ty else {
                return None;
            };
            argument_types.push(argument_ty);
        }
        let Some((matched_parameters, constraints)) =
            self.match_function_constraints(&function, arguments, &argument_types, expected)
        else {
            return None;
        };
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
                if !result_ty.same_shape(expected) {
                    self.emit_type_mismatch(
                        self.module.exprs()[expr_id].span,
                        expected,
                        &result_ty,
                    );
                    return Some(false);
                }
                parameter_types
            }
            None => {
                let parameter_types =
                    self.fallback_apply_parameter_types(callee, &arguments, env)?;
                let callee_expected = self.arrow_type(&parameter_types, expected);
                let checkpoint = self.diagnostics.len();
                if !self.check_expr(callee, env, Some(&callee_expected), value_stack) {
                    return (self.diagnostics.len() != checkpoint).then_some(false);
                }
                parameter_types
            }
        };

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
        let mut seen = HashMap::<String, SourceSpan>::new();
        let mut ok = true;

        for field in &record.fields {
            let label = field.label.text();
            let Some(expected_ty) = expected.get(label) else {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "record literal provides unexpected field `{label}`"
                    ))
                    .with_code(code("unexpected-record-field"))
                    .with_primary_label(
                        field.span,
                        "this field is not part of the expected closed record type",
                    ),
                );
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
        let Some(subject) = subject else {
            return None;
        };

        if !self.check_expected_expr(*base_expr, env, &subject, value_stack) {
            return Some(false);
        }

        match self.project_type(&subject, path) {
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

    fn project_type(&self, subject: &GateType, path: &NamePath) -> Result<GateType, GateIssue> {
        let mut current = subject.clone();
        for segment in path.segments().iter() {
            let GateType::Record(fields) = &current else {
                return Err(GateIssue::InvalidProjection {
                    span: path.span(),
                    path: projection_path_text(path),
                    subject: current.to_string(),
                });
            };
            let Some(field) = fields.iter().find(|field| field.name == segment.text()) else {
                return Err(GateIssue::UnknownField {
                    span: path.span(),
                    path: projection_path_text(path),
                    subject: current.to_string(),
                });
            };
            current = field.ty.clone();
        }
        Ok(current)
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
                "`Option A` only satisfies `Default` here via `use aivi.defaults (Option)` or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Text) => Err(
                "`Text` only satisfies `Default` here via `use aivi.defaults (defaultText)` or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Int) => Err(
                "`Int` only satisfies `Default` here via `use aivi.defaults (defaultInt)` or a same-module `Default` instance"
                    .to_owned(),
            ),
            GateType::Primitive(BuiltinType::Bool) => Err(
                "`Bool` only satisfies `Default` here via `use aivi.defaults (defaultBool)` or a same-module `Default` instance"
                    .to_owned(),
            ),
            _ => Err(
                "resolved-HIR default checking currently accepts same-module `Default` instances and the compiler-known `aivi.defaults` imports only"
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
        self.diagnostics.push(
            Diagnostic::error(format!(
                "operator `{}` expects {expected_operands}, found {} and {}",
                binary_operator_text(operator),
                describe_inferred_type(left),
                describe_inferred_type(right),
            ))
            .with_code(code("invalid-binary-operator"))
            .with_primary_label(span, label),
        );
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
                        self.diagnostics.push(
                                Diagnostic::error(format!(
                                    "record literal omits field `{field_name}` but no `Default` instance is in scope for `{}`",
                                    constraint.subject()
                                ))
                                .with_code(code("missing-default-instance"))
                                .with_primary_label(
                                    constraint.span(),
                                    format!("field `{field_name}` must be provided or defaultable here"),
                                )
                                .with_note(reason),
                            );
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
            .pending_eq_constraints
            .iter()
            .any(|existing| *existing == pending)
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
        if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind {
            if let Some(named_parameter_types) =
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
        if function.parameters.len() != argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(function.parameters.len());
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
        if let Some(class_item_id) = self.class_item_id_by_name("Eq") {
            if self
                .resolve_same_module_instance(class_item_id, ty)?
                .is_some()
            {
                return Ok(());
            }
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
                                    self.typing.lower_hir_type(*field, &substitutions)
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
        self.diagnostics.push(
            Diagnostic::error(format!("expected `{expected}` but found `{actual}`"))
                .with_code(code("type-mismatch"))
                .with_primary_label(span, "this expression has the wrong type"),
        );
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
                .with_primary_label(span, "this expression has the wrong type"),
            ),
        }
    }
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

fn is_numeric_gate_type(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(
            BuiltinType::Int | BuiltinType::Float | BuiltinType::Decimal | BuiltinType::BigInt
        )
    )
}

fn unary_operator_text(operator: UnaryOperator) -> &'static str {
    match operator {
        UnaryOperator::Not => "not",
    }
}

fn binary_operator_text(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Modulo => "%",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::LessThan => "<",
        BinaryOperator::GreaterThanOrEqual => ">=",
        BinaryOperator::LessThanOrEqual => "<=",
        BinaryOperator::Equals => "==",
        BinaryOperator::NotEquals => "!=",
        BinaryOperator::And => "and",
        BinaryOperator::Or => "or",
    }
}

fn describe_inferred_type(ty: Option<&GateType>) -> String {
    ty.map(|ty| format!("`{ty}`"))
        .unwrap_or_else(|| "an unresolved expression".to_owned())
}

fn projection_path_text(path: &NamePath) -> String {
    format!(
        ".{}",
        path.segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".")
    )
}

// KNOWN ISSUE: This function mutates the module (by synthesizing and injecting new record
// fields) during the type-checking phase. Because it alters the structure of the module
// that was passed into the type checker, running type checking a second time on the
// elaborated module will observe different record expressions than the first run, making
// type checking non-idempotent. The elaboration of default record fields should be moved
// to a separate, explicit elaboration pass that runs after type checking completes, so
// that the type checker itself remains a pure read-only query over the module.
fn apply_default_record_elisions(module: &Module, elisions: &[DefaultRecordElision]) -> Module {
    if elisions.is_empty() {
        return module.clone();
    }

    let mut module = module.clone();
    for elision in elisions {
        let record_span = module.exprs()[elision.record_expr].span;
        let synthesized_fields = elision
            .fields
            .iter()
            .map(|field| synthesize_default_record_field(&mut module, record_span, field))
            .collect::<Vec<_>>();
        let Some(expr) = module.arenas.exprs.get_mut(elision.record_expr) else {
            continue;
        };
        let ExprKind::Record(record) = &mut expr.kind else {
            continue;
        };
        record.fields.extend(synthesized_fields);
    }
    module
}

fn synthesize_default_record_field(
    module: &mut Module,
    record_span: SourceSpan,
    field: &SolvedDefaultRecordField,
) -> RecordExprField {
    let label = Name::new(field.field_name.clone(), record_span)
        .expect("typechecked record field names must stay valid");
    let value = match field.evidence {
        DefaultEvidence::BuiltinOptionNone => {
            alloc_builtin_default_expr(module, record_span, BuiltinTerm::None, "None")
        }
        DefaultEvidence::ImportedBinding(import) => {
            alloc_import_default_expr(module, record_span, import)
        }
        DefaultEvidence::SameModuleMemberBody(body) => body,
    };
    RecordExprField {
        span: record_span,
        label,
        value,
        surface: RecordFieldSurface::Defaulted,
    }
}

fn alloc_builtin_default_expr(
    module: &mut Module,
    span: SourceSpan,
    builtin: BuiltinTerm,
    text: &str,
) -> ExprId {
    let path = NamePath::from_vec(vec![
        Name::new(text, span).expect("builtin default term name must stay valid"),
    ])
    .expect("builtin default term path must stay valid");
    module
        .alloc_expr(crate::Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                path,
                TermResolution::Builtin(builtin),
            )),
        })
        .expect("default-record elaboration should fit inside the expression arena")
}

fn alloc_import_default_expr(module: &mut Module, span: SourceSpan, import: ImportId) -> ExprId {
    let local_name = module.imports()[import].local_name.text().to_owned();
    let path = NamePath::from_vec(vec![
        Name::new(local_name, span).expect("default import local name must stay valid"),
    ])
    .expect("default import path must stay valid");
    module
        .alloc_expr(crate::Expr {
            span,
            kind: ExprKind::Name(TermReference::resolved(
                path,
                TermResolution::Import(import),
            )),
        })
        .expect("default-record elaboration should fit inside the expression arena")
}

#[cfg(test)]
mod tests {
    use aivi_base::{DiagnosticCode, FileId, SourceDatabase, SourceSpan};
    use aivi_syntax::parse_module;

    use crate::{BuiltinType, Item, PipeTransformMode, RecordFieldSurface, lower_module};

    use super::*;

    fn typecheck_text(path: &str, text: &str) -> TypeCheckReport {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "typecheck input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "typecheck input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        typecheck_module(lowered.module())
    }

    fn typecheck_and_elaborate_text(path: &str, text: &str) -> (TypeCheckReport, Module) {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "typecheck input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "typecheck input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let lowered_module = lowered.module().clone();
        let report = typecheck_module(&lowered_module);
        let elaborated = apply_defaults(&lowered_module, &report);
        (report, elaborated)
    }

    fn lowered_module_text(path: &str, text: &str) -> Module {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "module input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "module input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        lowered.module().clone()
    }

    fn unit_span() -> SourceSpan {
        SourceSpan::default()
    }

    fn test_name(text: &str) -> crate::Name {
        crate::Name::new(text, unit_span()).expect("test name should stay valid")
    }

    fn test_path(text: &str) -> crate::NamePath {
        crate::NamePath::from_vec(vec![test_name(text)]).expect("single-segment path")
    }

    fn builtin_type(module: &mut Module, builtin: BuiltinType) -> crate::TypeId {
        let builtin_name = match builtin {
            BuiltinType::Int => "Int",
            BuiltinType::Float => "Float",
            BuiltinType::Decimal => "Decimal",
            BuiltinType::BigInt => "BigInt",
            BuiltinType::Bool => "Bool",
            BuiltinType::Text => "Text",
            BuiltinType::Unit => "Unit",
            BuiltinType::Bytes => "Bytes",
            BuiltinType::List => "List",
            BuiltinType::Map => "Map",
            BuiltinType::Set => "Set",
            BuiltinType::Option => "Option",
            BuiltinType::Result => "Result",
            BuiltinType::Validation => "Validation",
            BuiltinType::Signal => "Signal",
            BuiltinType::Task => "Task",
        };
        module
            .alloc_type(crate::TypeNode {
                span: unit_span(),
                kind: crate::TypeKind::Name(crate::TypeReference::resolved(
                    test_path(builtin_name),
                    crate::TypeResolution::Builtin(builtin),
                )),
            })
            .expect("builtin type allocation should fit")
    }

    fn type_parameter(module: &mut Module, text: &str) -> crate::TypeParameterId {
        module
            .alloc_type_parameter(crate::TypeParameter {
                span: unit_span(),
                name: test_name(text),
            })
            .expect("type parameter allocation should fit")
    }

    fn type_parameter_type(
        module: &mut Module,
        parameter: crate::TypeParameterId,
        text: &str,
    ) -> crate::TypeId {
        module
            .alloc_type(crate::TypeNode {
                span: unit_span(),
                kind: crate::TypeKind::Name(crate::TypeReference::resolved(
                    test_path(text),
                    crate::TypeResolution::TypeParameter(parameter),
                )),
            })
            .expect("type parameter reference allocation should fit")
    }

    fn applied_type(
        module: &mut Module,
        callee: crate::TypeId,
        argument: crate::TypeId,
    ) -> crate::TypeId {
        module
            .alloc_type(crate::TypeNode {
                span: unit_span(),
                kind: crate::TypeKind::Apply {
                    callee,
                    arguments: crate::NonEmpty::new(argument, Vec::new()),
                },
            })
            .expect("applied type allocation should fit")
    }

    fn builtin_term_expr(
        module: &mut Module,
        builtin: crate::BuiltinTerm,
        text: &str,
    ) -> crate::ExprId {
        module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path(text),
                    crate::TermResolution::Builtin(builtin),
                )),
            })
            .expect("builtin term allocation should fit")
    }

    #[test]
    fn typecheck_allows_option_default_record_elision() {
        let report = typecheck_text(
            "record-elision.aivi",
            "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             value name = \"Ada\"\n\
             value nickname = Some \"Countess\"\n\
             value profile:Profile = { name, nickname }\n",
        );
        assert!(
            report.is_ok(),
            "expected defaulted record elision to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_elaborates_option_default_record_elision_into_explicit_fields() {
        let (report, module) = typecheck_and_elaborate_text(
            "record-elision-hir.aivi",
            "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             value name = \"Ada\"\n\
             value nickname = Some \"Countess\"\n\
             value profile:Profile = { name, nickname }\n",
        );
        assert!(
            report.is_ok(),
            "expected defaulted record elision to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );

        let module = &module;
        let profile = value_body(module, "profile");
        let ExprKind::Record(record) = &module.exprs()[profile].kind else {
            panic!("expected `profile` to stay a record literal");
        };
        assert_eq!(
            record.fields.len(),
            3,
            "expected omitted bio field to be synthesized"
        );
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.label.text())
                .collect::<Vec<_>>(),
            vec!["name", "nickname", "bio"]
        );
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.surface)
                .collect::<Vec<_>>(),
            vec![
                RecordFieldSurface::Shorthand,
                RecordFieldSurface::Shorthand,
                RecordFieldSurface::Defaulted,
            ]
        );
        let defaulted_value = record.fields[2].value;
        match &module.exprs()[defaulted_value].kind {
            ExprKind::Name(reference) => assert!(matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::None))
            )),
            other => panic!("expected synthesized option default to be `None`, found {other:?}"),
        }
    }

    #[test]
    fn typecheck_reports_missing_eq_for_map_equality() {
        let report = typecheck_text(
            "map-equality.aivi",
            "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 1 }\n\
             value same:Bool = left == right\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "missing-eq-instance"))
            }),
            "expected missing Eq diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_missing_eq_for_map_inequality() {
        let report = typecheck_text(
            "map-inequality.aivi",
            "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 2 }\n\
             value different:Bool = left != right\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "missing-eq-instance"))
            }),
            "expected missing Eq diagnostic for !=, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn expression_matches_solves_deferred_eq_constraints() {
        let module = lowered_module_text(
            "expression-matches-map-equality.aivi",
            "value left = Map { \"id\": 1 }\n\
             value right = Map { \"id\": 1 }\n\
             value same:Bool = left == right\n",
        );
        assert!(
            !expression_matches(
                &module,
                value_body(&module, "same"),
                &GateExprEnv::default(),
                &GateType::Primitive(BuiltinType::Bool),
            ),
            "expected expression_matches to reject deferred missing Eq evidence"
        );
    }

    #[test]
    fn typecheck_accepts_same_module_eq_instances_for_nonstructural_types() {
        let report = typecheck_text(
            "same-module-eq-instance.aivi",
            "class Eq A\n\
             \x20\x20\x20\x20(==) : A -> A -> Bool\n\
             type Blob = Blob Bytes\n\
             fun blobEquals:Bool left:Blob right:Blob =>\n\
             \x20\x20\x20\x20True\n\
             instance Eq Blob\n\
             \x20\x20\x20\x20(==) left right = blobEquals left right\n\
             fun compare:Bool left:Blob right:Blob =>\n\
             \x20\x20\x20\x20left == right\n",
        );
        assert!(
            report.is_ok(),
            "expected same-module Eq instance to satisfy equality, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_equality_in_instance_member_bodies() {
        let report = typecheck_text(
            "instance-member-equality.aivi",
            "class Compare A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             type Label = Label Text\n\
             instance Compare Label\n\
             \x20\x20\x20\x20same left right = left == right\n",
        );
        assert!(
            report.is_ok(),
            "expected equality inside instance members to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_class_requirements_in_generic_instance_bodies() {
        let report = typecheck_text(
            "class-require-instance-context.aivi",
            "class Container A\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             instance Eq A -> Container A\n\
             \x20\x20\x20\x20same left right = left == right\n",
        );
        assert!(
            report.is_ok(),
            "expected class `require` constraints to typecheck inside generic instance bodies, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_missing_instance_requirement_for_class_requirements() {
        let report = typecheck_text(
            "class-require-missing-instance.aivi",
            "class Container A\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             instance Container Bytes\n\
             \x20\x20\x20\x20same left right = True\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "missing-instance-requirement"))
            }),
            "expected class `require` constraints to reject unsatisfied instances, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_instance_member_operator_operand_mismatch() {
        let report = typecheck_text(
            "instance-member-operator-mismatch.aivi",
            "class Ready A\n\
             \x20\x20\x20\x20ready : A -> Bool\n\
             type Blob = Blob Bytes\n\
             instance Ready Blob\n\
             \x20\x20\x20\x20ready blob = blob and True\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected instance member operator mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_invalid_unary_operator_without_resolved_operand_type() {
        let report = typecheck_text(
            "invalid-unary-operator.aivi",
            "value broken:Bool = not None\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-unary-operator"))
            }),
            "expected invalid unary operator diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_prelude_functor_map_calls() {
        let report = typecheck_text(
            "prelude-map-call.aivi",
            "fun increment:Int n:Int => n + 1\n\
             value mapped:Option Int = map increment (Some 1)\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient prelude Functor map call to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_prelude_foldable_reduce_calls() {
        let report = typecheck_text(
            "prelude-reduce-call.aivi",
            "fun add:Int acc:Int item:Int => acc + item\n\
             value joined:Text = reduce append empty [\"hel\", \"lo\"]\n\
             value total:Int = reduce add 10 (Some 2)\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient prelude Foldable reduce calls to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_class_member_names_from_expected_arrow_types() {
        let report = typecheck_text(
            "class-member-name-expected-arrow.aivi",
            "value pureOption:(Int -> Option Int) = pure\n",
        );
        assert!(
            report.is_ok(),
            "expected class member names to resolve from expected arrows, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_function_signature_constraints_at_call_sites() {
        let report = typecheck_text(
            "function-signature-constraints.aivi",
            "fun same:Eq A -> Bool x:A => True\n\
             value sameText:Bool = same \"Ada\"\n",
        );
        assert!(
            report.is_ok(),
            "expected signature constraints to solve at call sites, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_class_requirements_in_function_contexts() {
        let report = typecheck_text(
            "class-require-function-context.aivi",
            "class Container A\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             fun delegated:Container A -> Bool left:A right:A =>\n\
             \x20\x20\x20\x20left == right\n\
             instance Container Text\n\
             \x20\x20\x20\x20same left right = left == right\n\
             value sameText:Bool = delegated \"Ada\" \"Grace\"\n",
        );
        assert!(
            report.is_ok(),
            "expected class `require` constraints to propagate through function contexts, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_expands_class_requirements_into_eq_bindings() {
        let module = lowered_module_text(
            "class-require-expansion.aivi",
            "class Container A\n\
             \x20\x20\x20\x20require Eq A\n\
             \x20\x20\x20\x20same : A -> A -> Bool\n\
             fun delegated:Container A -> Bool left:A right:A =>\n\
             \x20\x20\x20\x20left == right\n",
        );
        let function = module
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Function(item) if item.name.text() == "delegated" => Some(item.clone()),
                _ => None,
            })
            .expect("delegated function should lower");
        let mut checker = TypeChecker::new(&module);
        let bindings = checker.constraint_bindings(&function.context, &PolyTypeBindings::new());
        let expanded = checker.expand_class_constraint_bindings(bindings);
        let labels = expanded
            .iter()
            .map(|binding| checker.class_constraint_binding_label(binding))
            .collect::<Vec<_>>();
        let context_kinds = function
            .context
            .iter()
            .map(|constraint| format!("{:?}", module.types()[*constraint].kind))
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label == "Eq A"),
            "expected `Container A` to imply `Eq A`, got context len {} kinds {:?} and labels {labels:?}",
            function.context.len(),
            context_kinds
        );
    }

    #[test]
    fn typecheck_accepts_ord_comparison_for_text() {
        let report = typecheck_text(
            "ord-text-comparison.aivi",
            "value ordered:Bool = \"a\" < \"b\"\n",
        );
        assert!(
            report.is_ok(),
            "expected Ord-backed text comparison to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_invalid_binary_operator_for_non_ord_comparison() {
        let report = typecheck_text(
            "invalid-binary-operator.aivi",
            "value broken:Bool = [1] < [2]\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-binary-operator"))
            }),
            "expected invalid binary operator diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_value_annotation_mismatch() {
        let report = typecheck_text("value-mismatch.aivi", "value answer:Text = 42\n");
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected type mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_unannotated_function_name_from_expected_arrow() {
        let report = typecheck_text(
            "function-name-expected-arrow.aivi",
            "fun keep x => x\n\
             value chosen:(Option Int -> Option Int) = keep\n",
        );
        assert!(
            report.is_ok(),
            "expected unannotated function name to typecheck from expected arrow, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_unannotated_function_application_from_expected_result() {
        let report = typecheck_text(
            "function-application-expected-result.aivi",
            "fun keepNone opt:Option Int => None\n\
             value result:Option Int = keepNone None\n",
        );
        assert!(
            report.is_ok(),
            "expected unannotated function application to typecheck from expected result, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_function_application_with_expected_builtin_hole_argument() {
        let report = typecheck_text(
            "function-application-expected-hole.aivi",
            "fun keep:Option Int opt:Option Int => opt\n\
             value result:Option Int = keep None\n",
        );
        assert!(
            report.is_ok(),
            "expected keep None to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_function_application_result_mismatch() {
        let report = typecheck_text(
            "function-application-result-mismatch.aivi",
            "fun keep:Option Int opt:Option Int => opt\n\
             value result:Option Text = keep None\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected type mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_missing_default_instance_via_constraint_solver() {
        let report = typecheck_text(
            "missing-default-instance.aivi",
            "type Nickname = Nickname Text\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "missing-default-instance"))
                    && diagnostic.message.contains("nickname")
            }),
            "expected missing Default diagnostic from constraint solver, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_same_module_default_instances_for_record_elision() {
        let report = typecheck_text(
            "same-module-default-instance.aivi",
            "class Default A\n\
             \x20\x20\x20\x20default : A\n\
             type Nickname = Nickname Text\n\
             instance Default Nickname\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
        );
        assert!(
            report.is_ok(),
            "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_imported_default_values_for_record_elision() {
        let report = typecheck_text(
            "imported-default-values.aivi",
            "use aivi.defaults (defaultText as emptyText, defaultInt, defaultBool as disabled)\n\
             type Settings = {\n\
                 title: Text,\n\
                 retries: Int,\n\
                 enabled: Bool,\n\
                 label: Text\n\
             }\n\
             value title = \"AIVI\"\n\
             value settings:Settings = { title }\n",
        );
        assert!(
            report.is_ok(),
            "expected imported aivi.defaults values to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_ambient_default_class_for_record_elision() {
        let report = typecheck_text(
            "ambient-default-instance.aivi",
            "type Nickname = Nickname Text\n\
             instance Default Nickname\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value user:User = { name: \"Ada\" }\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient Default class to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_elaborates_imported_default_values_into_explicit_fields() {
        let (report, module) = typecheck_and_elaborate_text(
            "imported-default-values-hir.aivi",
            "use aivi.defaults (defaultText as emptyText, defaultInt, defaultBool as disabled)\n\
             type Settings = {\n\
                 title: Text,\n\
                 retries: Int,\n\
                 enabled: Bool,\n\
                 label: Text\n\
             }\n\
             value title = \"AIVI\"\n\
             value settings:Settings = { title }\n",
        );
        assert!(
            report.is_ok(),
            "expected imported aivi.defaults values to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );

        let settings = value_body(&module, "settings");
        let ExprKind::Record(record) = &module.exprs()[settings].kind else {
            panic!("expected `settings` to stay a record literal");
        };
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.label.text())
                .collect::<Vec<_>>(),
            vec!["title", "retries", "enabled", "label"]
        );
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.surface)
                .collect::<Vec<_>>(),
            vec![
                RecordFieldSurface::Shorthand,
                RecordFieldSurface::Defaulted,
                RecordFieldSurface::Defaulted,
                RecordFieldSurface::Defaulted,
            ]
        );

        let empty_text = import_binding_id(&module, "emptyText");
        let default_int = import_binding_id(&module, "defaultInt");
        let disabled = import_binding_id(&module, "disabled");
        for (label, expected_import) in [
            ("retries", default_int),
            ("enabled", disabled),
            ("label", empty_text),
        ] {
            let value = record
                .fields
                .iter()
                .find(|field| field.label.text() == label)
                .map(|field| field.value)
                .expect("expected synthesized field to exist");
            match &module.exprs()[value].kind {
                ExprKind::Name(reference) => assert!(matches!(
                    reference.resolution.as_ref(),
                    ResolutionState::Resolved(TermResolution::Import(import_id))
                        if *import_id == expected_import
                )),
                other => panic!(
                    "expected synthesized imported default for `{label}` to stay a name reference, found {other:?}"
                ),
            }
        }
    }

    #[test]
    fn typecheck_elaborates_same_module_default_instances_into_explicit_fields() {
        let (report, module) = typecheck_and_elaborate_text(
            "same-module-default-instance-hir.aivi",
            "class Default A\n\
             \x20\x20\x20\x20default : A\n\
             type Nickname = Nickname Text\n\
             instance Default Nickname\n\
             \x20\x20\x20\x20default = Nickname \"\"\n\
             type User = {\n\
                 name: Text,\n\
                 nickname: Nickname\n\
             }\n\
             value name = \"Ada\"\n\
             value user:User = { name }\n",
        );
        assert!(
            report.is_ok(),
            "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );

        let module = &module;
        let user = value_body(module, "user");
        let ExprKind::Record(record) = &module.exprs()[user].kind else {
            panic!("expected `user` to stay a record literal");
        };
        assert_eq!(
            record
                .fields
                .iter()
                .map(|field| field.label.text())
                .collect::<Vec<_>>(),
            vec!["name", "nickname"]
        );
        assert_eq!(record.fields[1].surface, RecordFieldSurface::Defaulted);

        let default_body = same_module_default_body(module, "default");
        assert_eq!(
            record.fields[1].value, default_body,
            "same-module Default synthesis should reuse the validated instance member body"
        );
    }

    #[test]
    fn typecheck_reports_same_module_constructor_argument_mismatch() {
        let report = typecheck_text(
            "same-module-constructor-mismatch.aivi",
            "type Box A = Box A\n\
             value wrapped:(Box Text) = Box 42\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            }),
            "expected same-module constructor mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_mixed_applicative_cluster_members() {
        let report = typecheck_text(
            "mixed-applicative-cluster.aivi",
            "type NamePair = NamePair Text Text\n\
             value first:(Option Text) = Some \"Ada\"\n\
             signal last = \"Lovelace\"\n\
             value broken =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "applicative-cluster-mismatch"))
            }),
            "expected applicative cluster mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_partial_builtin_applicative_clusters() {
        let report = typecheck_text(
            "partial-builtin-clusters.aivi",
            "type NamePair = NamePair Text Text\n\
             value first = Some \"Ada\"\n\
             value last = None\n\
             value maybePair:Option NamePair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             value okFirst = Ok \"Ada\"\n\
             value errLast = Err \"missing\"\n\
             value resultPair:Result Text NamePair =\n\
              &|> okFirst\n\
              &|> errLast\n\
               |> NamePair\n",
        );
        assert!(
            report.is_ok(),
            "expected partial builtin clusters to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_case_branch_type_mismatch() {
        let report = typecheck_text(
            "case-branch-type-mismatch.aivi",
            r#"type Screen =
  | Loading
  | Ready Text
value current:Screen = Loading
value broken =
    current
     ||> Loading -> 0
     ||> Ready title -> title
"#,
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "case-branch-type-mismatch"))
            }),
            "expected case branch type mismatch diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_partial_builtin_case_runs() {
        let report = typecheck_text(
            "partial-builtin-case-runs.aivi",
            r#"type Screen =
  | Loading
  | Ready Text
  | Failed Text
value current:Screen = Loading
value maybeLabel:Option Text =
    current
     ||> Loading -> None
     ||> Ready title -> Some title
     ||> Failed reason -> Some reason
value resultLabel:Result Text Text =
    current
     ||> Loading -> Ok "loading"
     ||> Ready title -> Ok title
     ||> Failed reason -> Err reason
"#,
        );
        assert!(
            report.is_ok(),
            "expected partial builtin case runs to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_applied_calls_in_case_branches() {
        let report = typecheck_text(
            "applied-call-case-branches.aivi",
            r#"fun addOne:Int n:Int => n + 1
value x:Int =
    0
     ||> 0 -> addOne 0
     ||> _ -> 1
"#,
        );
        assert!(
            report.is_ok(),
            "expected applied calls in case branches to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_applied_calls_in_truthy_falsy_branches() {
        let report = typecheck_text(
            "applied-call-truthy-falsy-branches.aivi",
            r#"fun addOne:Int n:Int => n + 1
value x:Int =
    True
     T|> addOne 0
     F|> 1
"#,
        );
        assert!(
            report.is_ok(),
            "expected applied calls in truthy/falsy branches to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_generic_record_projection_in_function_body() {
        let report = typecheck_text(
            "generic-record-projection-in-function-body.aivi",
            r#"type TakeAcc A = {
    n: Int,
    items: List A
}
fun remaining:Int acc:(TakeAcc A) => acc.n
fun items:(List A) acc:(TakeAcc A) => acc.items
"#,
        );
        assert!(
            report.is_ok(),
            "expected generic record projection in function bodies to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_polymorphic_pipe_transforms() {
        let mut module = Module::new(FileId::new(0));
        let option_type = builtin_type(&mut module, BuiltinType::Option);
        let int_type = builtin_type(&mut module, BuiltinType::Int);
        let text_type = builtin_type(&mut module, BuiltinType::Text);
        let parameter = type_parameter(&mut module, "A");
        let a_type = type_parameter_type(&mut module, parameter, "A");
        let option_a_type = applied_type(&mut module, option_type, a_type);
        let binding = module
            .alloc_binding(crate::Binding {
                span: unit_span(),
                name: test_name("value"),
                kind: crate::BindingKind::FunctionParameter,
            })
            .expect("binding allocation should fit");
        let local_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("value"),
                    crate::TermResolution::Local(binding),
                )),
            })
            .expect("local expression allocation should fit");
        let some_expr = builtin_term_expr(&mut module, crate::BuiltinTerm::Some, "Some");
        let wrap_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Apply {
                    callee: some_expr,
                    arguments: crate::NonEmpty::new(local_expr, Vec::new()),
                },
            })
            .expect("wrap body allocation should fit");
        let wrap = module
            .push_item(crate::Item::Function(crate::FunctionItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("wrap"),
                type_parameters: vec![parameter],
                context: Vec::new(),
                parameters: vec![crate::FunctionParameter {
                    span: unit_span(),
                    binding,
                    annotation: Some(a_type),
                }],
                annotation: Some(option_a_type),
                body: wrap_body,
            }))
            .expect("function allocation should fit");
        let wrap_ref_number = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("wrap"),
                    crate::TermResolution::Item(wrap),
                )),
            })
            .expect("wrap reference allocation should fit");
        let maybe_number_head = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
            })
            .expect("integer allocation should fit");
        let maybe_number_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Pipe(crate::PipeExpr {
                    head: maybe_number_head,
                    stages: crate::NonEmpty::new(
                        crate::PipeStage {
                            span: unit_span(),
                            subject_memo: None,
                            result_memo: None,
                            kind: crate::PipeStageKind::Transform {
                                expr: wrap_ref_number,
                            },
                        },
                        Vec::new(),
                    ),
                }),
            })
            .expect("pipe allocation should fit");
        let option_int_type = applied_type(&mut module, option_type, int_type);
        let _maybe_number = module
            .push_item(crate::Item::Value(crate::ValueItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("maybeNumber"),
                annotation: Some(option_int_type),
                body: maybe_number_body,
            }))
            .expect("value allocation should fit");
        let wrap_ref_label = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("wrap"),
                    crate::TermResolution::Item(wrap),
                )),
            })
            .expect("wrap reference allocation should fit");
        let maybe_label_head = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Text(crate::TextLiteral {
                    segments: vec![crate::TextSegment::Text(crate::TextFragment {
                        raw: "Ada".into(),
                        span: unit_span(),
                    })],
                }),
            })
            .expect("text allocation should fit");
        let maybe_label_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Pipe(crate::PipeExpr {
                    head: maybe_label_head,
                    stages: crate::NonEmpty::new(
                        crate::PipeStage {
                            span: unit_span(),
                            subject_memo: None,
                            result_memo: None,
                            kind: crate::PipeStageKind::Transform {
                                expr: wrap_ref_label,
                            },
                        },
                        Vec::new(),
                    ),
                }),
            })
            .expect("pipe allocation should fit");
        let option_text_type = applied_type(&mut module, option_type, text_type);
        let _maybe_label = module
            .push_item(crate::Item::Value(crate::ValueItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("maybeLabel"),
                annotation: Some(option_text_type),
                body: maybe_label_body,
            }))
            .expect("value allocation should fit");

        let report = typecheck_module(&module);
        assert!(
            report.is_ok(),
            "expected polymorphic pipe transforms to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_infers_callable_and_replacement_pipe_transforms() {
        let mut module = Module::new(FileId::new(0));
        let int_type = builtin_type(&mut module, BuiltinType::Int);
        let binding = module
            .alloc_binding(crate::Binding {
                span: unit_span(),
                name: test_name("value"),
                kind: crate::BindingKind::FunctionParameter,
            })
            .expect("binding allocation should fit");
        let local_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("value"),
                    crate::TermResolution::Local(binding),
                )),
            })
            .expect("local expression allocation should fit");
        let add_one = module
            .push_item(crate::Item::Function(crate::FunctionItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("addOne"),
                type_parameters: Vec::new(),
                context: Vec::new(),
                parameters: vec![crate::FunctionParameter {
                    span: unit_span(),
                    binding,
                    annotation: Some(int_type),
                }],
                annotation: Some(int_type),
                body: local_expr,
            }))
            .expect("function allocation should fit");
        let callable_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("addOne"),
                    crate::TermResolution::Item(add_one),
                )),
            })
            .expect("callable expression allocation should fit");
        let replacement_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Text(crate::TextLiteral {
                    segments: vec![crate::TextSegment::Text(crate::TextFragment {
                        raw: "done".into(),
                        span: unit_span(),
                    })],
                }),
            })
            .expect("replacement expression allocation should fit");

        let mut typing = GateTypeContext::new(&module);
        let env = GateExprEnv::default();
        let subject = GateType::Primitive(BuiltinType::Int);

        assert_eq!(
            typing.infer_transform_stage_mode(callable_expr, &env, &subject),
            PipeTransformMode::Apply
        );
        assert_eq!(
            typing.infer_transform_stage(callable_expr, &env, &subject),
            Some(GateType::Primitive(BuiltinType::Int))
        );
        assert_eq!(
            typing.infer_transform_stage_mode(replacement_expr, &env, &subject),
            PipeTransformMode::Replace
        );
        assert_eq!(
            typing.infer_transform_stage(replacement_expr, &env, &subject),
            Some(GateType::Primitive(BuiltinType::Text))
        );
    }

    #[test]
    fn typecheck_accepts_polymorphic_function_application() {
        let mut module = Module::new(FileId::new(0));
        let option_type = builtin_type(&mut module, BuiltinType::Option);
        let int_type = builtin_type(&mut module, BuiltinType::Int);
        let text_type = builtin_type(&mut module, BuiltinType::Text);
        let parameter = type_parameter(&mut module, "A");
        let a_type = type_parameter_type(&mut module, parameter, "A");
        let option_a_type = applied_type(&mut module, option_type, a_type);
        let binding = module
            .alloc_binding(crate::Binding {
                span: unit_span(),
                name: test_name("value"),
                kind: crate::BindingKind::FunctionParameter,
            })
            .expect("binding allocation should fit");
        let local_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("value"),
                    crate::TermResolution::Local(binding),
                )),
            })
            .expect("local expression allocation should fit");
        let some_expr = builtin_term_expr(&mut module, crate::BuiltinTerm::Some, "Some");
        let wrap_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Apply {
                    callee: some_expr,
                    arguments: crate::NonEmpty::new(local_expr, Vec::new()),
                },
            })
            .expect("wrap body allocation should fit");
        let wrap = module
            .push_item(crate::Item::Function(crate::FunctionItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("wrap"),
                type_parameters: vec![parameter],
                context: Vec::new(),
                parameters: vec![crate::FunctionParameter {
                    span: unit_span(),
                    binding,
                    annotation: Some(a_type),
                }],
                annotation: Some(option_a_type),
                body: wrap_body,
            }))
            .expect("function allocation should fit");
        let wrap_ref_number = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("wrap"),
                    crate::TermResolution::Item(wrap),
                )),
            })
            .expect("wrap reference allocation should fit");
        let number_argument = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
            })
            .expect("integer allocation should fit");
        let maybe_number_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Apply {
                    callee: wrap_ref_number,
                    arguments: crate::NonEmpty::new(number_argument, Vec::new()),
                },
            })
            .expect("application allocation should fit");
        let option_int_type = applied_type(&mut module, option_type, int_type);
        let _maybe_number = module
            .push_item(crate::Item::Value(crate::ValueItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("maybeNumber"),
                annotation: Some(option_int_type),
                body: maybe_number_body,
            }))
            .expect("value allocation should fit");
        let wrap_ref_label = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("wrap"),
                    crate::TermResolution::Item(wrap),
                )),
            })
            .expect("wrap reference allocation should fit");
        let label_argument = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Text(crate::TextLiteral {
                    segments: vec![crate::TextSegment::Text(crate::TextFragment {
                        raw: "Ada".into(),
                        span: unit_span(),
                    })],
                }),
            })
            .expect("text allocation should fit");
        let maybe_label_body = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Apply {
                    callee: wrap_ref_label,
                    arguments: crate::NonEmpty::new(label_argument, Vec::new()),
                },
            })
            .expect("application allocation should fit");
        let option_text_type = applied_type(&mut module, option_type, text_type);
        let _maybe_label = module
            .push_item(crate::Item::Value(crate::ValueItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("maybeLabel"),
                annotation: Some(option_text_type),
                body: maybe_label_body,
            }))
            .expect("value allocation should fit");

        let report = typecheck_module(&module);
        assert!(
            report.is_ok(),
            "expected polymorphic function application to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_expected_polymorphic_ambient_helper_application() {
        let report = typecheck_text(
            "expected-polymorphic-ambient-helper-application.aivi",
            "fun even:Bool n:Int => n == 2 or n == 4\n\
             value maybeName:Option Text = Some \"Ada\"\n\
             value numbers:List Int = [1, 2, 3, 4]\n\
             value chosenName:Text = __aivi_option_getOrElse \"guest\" maybeName\n\
             value count:Int = __aivi_list_length numbers\n\
             value firstNumber:Option Int = __aivi_list_head numbers\n\
             value hasEven:Bool = __aivi_list_any even numbers\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient polymorphic helper application to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_allows_signal_names_in_direct_function_calls() {
        let report = typecheck_text(
            "signal-name-direct-call.aivi",
            "signal direction : Signal Int = 1\n\
             fun step:Int x:Int => x\n\
             fun current:Int tick:Unit => step direction\n",
        );
        assert!(
            report.is_ok(),
            "expected direct function application to accept a signal payload name, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_invalid_pipe_stage_input_for_transforms() {
        let report = typecheck_text(
            "invalid-pipe-stage-transform.aivi",
            "fun describe:Text n:Int => \"count\"\n\
             value broken:Text = \"Ada\" |> describe\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-pipe-stage-input"))
            }),
            "expected invalid pipe stage input diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_invalid_pipe_stage_input_for_taps() {
        let report = typecheck_text(
            "invalid-pipe-stage-tap.aivi",
            "fun describe:Text n:Int => \"count\"\n\
             value broken:Text = \"Ada\" | describe\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-pipe-stage-input"))
            }),
            "expected invalid pipe stage input diagnostic for tap, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_higher_kinded_instance_member_signatures() {
        let report = typecheck_text(
            "higher-kinded-instance-members.aivi",
            "class Applicative F\n\
             \x20\x20\x20\x20pureInt : F Int\n\
             instance Applicative Option\n\
             \x20\x20\x20\x20pureInt = Some 1\n\
             class Functor F\n\
             \x20\x20\x20\x20labelInt : F Int\n\
             instance Functor (Result Text)\n\
             \x20\x20\x20\x20labelInt = Ok 1\n",
        );
        assert!(
            report.is_ok(),
            "expected higher-kinded instance member signatures to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_resolves_partial_same_module_instances_generically() {
        let module = lowered_module_text(
            "partial-same-module-instances.aivi",
            "class Applicative F\n\
             \x20\x20\x20\x20pureInt : F Int\n\
             instance Applicative Option\n\
             \x20\x20\x20\x20pureInt = Some 1\n\
             class Monad F\n\
             \x20\x20\x20\x20labelInt : F Int\n\
             instance Monad (Result Text)\n\
             \x20\x20\x20\x20labelInt = Ok 1\n",
        );
        let mut checker = TypeChecker::new(&module);
        assert!(
            checker
                .require_class_named(
                    "Applicative",
                    &GateType::Option(Box::new(GateType::Primitive(BuiltinType::Int)))
                )
                .is_ok(),
            "expected general class resolution to accept same-module `Applicative Option`"
        );
        assert!(
            checker
                .require_class_named(
                    "Monad",
                    &GateType::Result {
                        error: Box::new(GateType::Primitive(BuiltinType::Text)),
                        value: Box::new(GateType::Primitive(BuiltinType::Int)),
                    },
                )
                .is_ok(),
            "expected general class resolution to accept same-module `Monad (Result Text)`"
        );
    }

    #[test]
    fn typecheck_accepts_projection_from_unannotated_record_values() {
        let report = typecheck_text(
            "projection-from-record-value.aivi",
            "value profile = { name: \"Ada\", age: 36 }\n\
             value name:Text = profile.name\n",
        );
        assert!(
            report.is_ok(),
            "expected projection from an unannotated record value to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_unknown_field_from_unannotated_record_projection() {
        let report = typecheck_text(
            "projection-unknown-field.aivi",
            "value profile = { name: \"Ada\", age: 36 }\n\
             value missing:Text = profile.missing\n",
        );
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "unknown-projection-field"))
            }),
            "expected unknown projection field diagnostic, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_collection_literals_with_expected_shapes() {
        let report = typecheck_text(
            "expected-collection-literals.aivi",
            "value pair:(Option Int, Result Text Int) = (None, Ok 1)\n\
             value items:List (Option Int) = [None, Some 2]\n\
             value headers:Map Text (Option Int) = Map { \"primary\": None, \"backup\": Some 3 }\n\
             value tags:Set (Option Int) = Set [None, Some 4]\n",
        );
        assert!(
            report.is_ok(),
            "expected collection literals to use their expected shapes bidirectionally, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_collection_literal_element_mismatches() {
        let report = typecheck_text(
            "expected-collection-literal-mismatches.aivi",
            "value pair:(Option Int, Result Text Int) = (Some \"Ada\", Ok \"Ada\")\n\
             value items:List (Option Int) = [Some \"Ada\"]\n\
             value headers:Map Text (Option Int) = Map { \"primary\": Some \"Ada\" }\n\
             value tags:Set (Option Int) = Set [Some \"Ada\"]\n",
        );
        let mismatch_count = report
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            })
            .count();
        assert!(
            mismatch_count >= 4,
            "expected collection literal mismatches to surface type mismatches, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_builtin_noninteger_literals_with_matching_annotations() {
        let report = typecheck_text(
            "builtin-noninteger-literals-valid.aivi",
            "value pi:Float = 3.14\n\
             value amount:Decimal = 19.25d\n\
             value whole:Decimal = 19d\n\
             value count:BigInt = 123n\n",
        );
        assert!(
            report.is_ok(),
            "expected builtin noninteger literals to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_noninteger_literal_type_mismatches() {
        let report = typecheck_text(
            "builtin-noninteger-literals-invalid.aivi",
            "value pi:Float = 19.25d\n\
             value amount:Decimal = 3.14\n\
             value count:BigInt = 42\n",
        );
        let mismatch_count = report
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(DiagnosticCode::new("hir", "type-mismatch"))
            })
            .count();
        assert!(
            mismatch_count >= 3,
            "expected noninteger literal mismatches to surface type mismatches, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    fn value_body(module: &Module, name: &str) -> ExprId {
        module
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Value(value) if value.name.text() == name => Some(value.body),
                _ => None,
            })
            .expect("expected value item to exist")
    }

    fn same_module_default_body(module: &Module, member_name: &str) -> ExprId {
        module
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Instance(instance) => instance
                    .members
                    .iter()
                    .find(|member| member.name.text() == member_name)
                    .map(|member| member.body),
                _ => None,
            })
            .expect("expected same-module Default member to exist")
    }

    fn import_binding_id(module: &Module, local_name: &str) -> ImportId {
        module
            .imports()
            .iter()
            .find_map(|(import_id, import)| {
                (import.local_name.text() == local_name).then_some(import_id)
            })
            .expect("expected import binding to exist")
    }
}
