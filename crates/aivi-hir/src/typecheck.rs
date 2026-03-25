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
    ids::{ExprId, ItemId, TypeParameterId},
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
    elaborated_module: Module,
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
    pub fn new(elaborated_module: Module, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            elaborated_module,
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn elaborated_module(&self) -> &Module {
        &self.elaborated_module
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn into_elaborated_module(self) -> Module {
        self.elaborated_module
    }

    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

pub fn typecheck_module(module: &Module) -> TypeCheckReport {
    let mut checker = TypeChecker::new(module);
    checker.run();
    let elaborated_module = checker.build_elaborated_module();
    TypeCheckReport::new(elaborated_module, checker.diagnostics)
}

pub fn elaborate_default_record_fields(module: &Module) -> Module {
    typecheck_module(module).into_elaborated_module()
}

pub(crate) fn expression_matches(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    expected: &GateType,
) -> bool {
    let mut checker = TypeChecker::new(module);
    checker.check_expr(expr_id, env, Some(expected), &mut Vec::new())
        && checker.diagnostics.is_empty()
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
    default_record_elisions: Vec<DefaultRecordElision>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOperatorExpectation {
    BoolOperands,
    MatchingNumericOperands,
    CommonTypeOperands,
}

impl<'a> TypeChecker<'a> {
    fn new(module: &'a Module) -> Self {
        let option_default_in_scope = module.imports().iter().any(|(_, import)| {
            matches!(
                import.metadata,
                ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
            )
        });
        Self {
            module,
            typing: GateTypeContext::new(module),
            diagnostics: Vec::new(),
            option_default_in_scope,
            default_record_elisions: Vec::new(),
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
    }

    fn build_elaborated_module(&self) -> Module {
        apply_default_record_elisions(self.module, &self.default_record_elisions)
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
        let mut env = GateExprEnv::default();
        for parameter in &item.parameters {
            let Some(annotation) = parameter.annotation else {
                continue;
            };
            let Some(parameter_ty) = self.typing.lower_open_annotation(annotation) else {
                continue;
            };
            env.locals.insert(parameter.binding, parameter_ty);
        }
        let expected = item
            .annotation
            .and_then(|annotation| self.typing.lower_open_annotation(annotation));
        self.check_expr(item.body, &env, expected.as_ref(), &mut Vec::new());
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
        let Item::Class(class_item) = &self.module.items()[class_item_id] else {
            return;
        };
        let expected_members = class_item
            .members
            .iter()
            .map(|member| (member.name.text().to_owned(), member.annotation))
            .collect::<HashMap<_, _>>();
        for member in &item.members {
            let Some(annotation) = expected_members.get(member.name.text()).copied() else {
                continue;
            };
            let Some(expected) = self
                .typing
                .instantiate_poly_hir_type(annotation, &argument_bindings)
            else {
                continue;
            };
            self.check_instance_member(member, &expected);
        }
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
        self.solve_constraints(&info.constraints);

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
            | BinaryOperator::LessThan => self.check_numeric_binary_expr(
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
                BinaryOperator::GreaterThan | BinaryOperator::LessThan
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
            BinaryOperator::GreaterThan | BinaryOperator::LessThan => {
                GateType::Primitive(BuiltinType::Bool)
            }
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

        self.solve_constraints(&[TypeConstraint::eq(
            self.module.exprs()[expr_id].span,
            operand_ty,
        )]);
        let bool_ty = GateType::Primitive(BuiltinType::Bool);
        self.check_result_type(expr_id, expected, &bool_ty)
    }

    fn inferred_expr_type(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        self.typing.infer_expr(expr_id, env, None).ty
    }

    fn inferred_expr_shape(&mut self, expr_id: ExprId, env: &GateExprEnv) -> Option<GateType> {
        let info = self.typing.infer_expr(expr_id, env, None);
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
                    let solved = self.solve_constraints(&constraints);
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
            self.solve_constraints(&info.constraints);
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
            self.solve_constraints(&info.constraints);
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
            self.solve_constraints(&info.constraints);
            let Some(argument_ty) = info.ty else {
                return None;
            };
            argument_types.push(argument_ty);
        }
        let Some((matched_parameters, constraints)) =
            self.match_function_constraints(&function, &argument_types, expected)
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
        self.solve_constraints(&callee_info.constraints);
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
        let mut seen = HashSet::<String>::new();
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
            seen.insert(label.to_owned());
            ok &= self.check_expr(field.value, env, Some(*expected_ty), value_stack);
        }

        for field in expected_fields {
            if seen.contains(&field.name) {
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
        self.solve_constraints(&base_info.constraints);
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
        self.solve_constraints(&callee_info.constraints);

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
        if let Some(body) = self.same_module_default_member_body(ty)? {
            return Ok(DefaultEvidence::SameModuleMemberBody(body));
        }
        match ty {
            GateType::Option(_) => Err(
                "`Option A` only satisfies `Default` here via `use aivi.defaults (Option)` or a same-module `Default` instance"
                    .to_owned(),
            ),
            _ => Err(
                "resolved-HIR default checking currently accepts same-module `Default` instances only"
                    .to_owned(),
            ),
        }
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

    fn solve_constraints(&mut self, constraints: &[TypeConstraint]) -> ConstraintSolveReport {
        let mut report = ConstraintSolveReport::default();
        for constraint in constraints {
            match constraint.class() {
                ConstraintClass::Eq => {
                    if let Err(reason) = self.require_eq(constraint.subject(), &mut Vec::new()) {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "this expression requires `Eq` for `{}`",
                                constraint.subject()
                            ))
                            .with_code(code("missing-eq-instance"))
                            .with_primary_label(
                                constraint.span(),
                                format!(
                                    "`{}` does not currently have `Eq` evidence",
                                    constraint.subject()
                                ),
                            )
                            .with_note(reason),
                        );
                    }
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
        argument_types: &[GateType],
        expected_result: &GateType,
    ) -> Option<(Vec<GateType>, Vec<ClassConstraintBinding>)> {
        if function.parameters.len() != argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(function.parameters.len());
        for (parameter, actual) in function.parameters.iter().zip(argument_types.iter()) {
            let annotation = parameter.annotation?;
            if let Some(lowered) = self.typing.lower_annotation(annotation) {
                if !lowered.same_shape(actual) {
                    return None;
                }
                instantiated_parameters.push(lowered);
                continue;
            }
            if !self
                .typing
                .match_poly_hir_type(annotation, actual, &mut bindings)
            {
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

    fn require_eq(&mut self, ty: &GateType, item_stack: &mut Vec<ItemId>) -> Result<(), String> {
        if self.require_compiler_derived_eq(ty, item_stack).is_ok() {
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
        self.require_compiler_derived_eq(ty, item_stack)
    }

    fn require_compiler_derived_eq(
        &mut self,
        ty: &GateType,
        item_stack: &mut Vec<ItemId>,
    ) -> Result<(), String> {
        match ty {
            GateType::Primitive(BuiltinType::Bytes) => {
                Err("`Bytes` does not have a compiler-derived `Eq` instance in v1".to_owned())
            }
            GateType::Primitive(_) => Ok(()),
            GateType::TypeParameter { name, .. } => Err(format!(
                "open type parameter `{name}` does not have a compiler-derived `Eq` instance in v1"
            )),
            GateType::Tuple(elements) => {
                for element in elements {
                    self.require_eq(element, item_stack)?;
                }
                Ok(())
            }
            GateType::Record(fields) => {
                for field in fields {
                    self.require_eq(&field.ty, item_stack)?;
                }
                Ok(())
            }
            GateType::List(element) | GateType::Option(element) => {
                self.require_eq(element, item_stack)
            }
            GateType::Result { error, value } | GateType::Validation { error, value } => {
                self.require_eq(error, item_stack)?;
                self.require_eq(value, item_stack)
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
                let result = self.require_eq(&carrier, item_stack);
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
                        self.require_eq(&lowered, item_stack)
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
                                self.require_eq(&lowered, item_stack)?;
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
        if self.has_builtin_class_instance_binding(class_name.as_str(), subject) {
            return Some(ClassMemberImplementation::Builtin);
        }
        let (instance_id, instance) = self
            .resolve_same_module_instance_binding_with_id(resolution.class, subject)
            .ok()??;
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member_name = class_item.members.get(resolution.member_index)?.name.text();
        let member_index = instance
            .members
            .iter()
            .position(|member| member.name.text() == member_name)?;
        Some(ClassMemberImplementation::SameModuleInstance {
            instance: instance_id,
            member_index,
        })
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
                "Filterable" => self.matches_builtin_head(
                    binding,
                    &[BuiltinType::List, BuiltinType::Option],
                ),
                "Bifunctor" => self.matches_builtin_head(
                    binding,
                    &[BuiltinType::Result, BuiltinType::Validation],
                ),
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

#[cfg(test)]
mod tests {
    use aivi_base::{DiagnosticCode, SourceDatabase};
    use aivi_syntax::parse_module;

    use crate::{Item, RecordFieldSurface, lower_module};

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
             val name = \"Ada\"\n\
             val nickname = Some \"Countess\"\n\
             val profile:Profile = { name, nickname }\n",
        );
        assert!(
            report.is_ok(),
            "expected defaulted record elision to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_elaborates_option_default_record_elision_into_explicit_fields() {
        let report = typecheck_text(
            "record-elision-hir.aivi",
            "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             val name = \"Ada\"\n\
             val nickname = Some \"Countess\"\n\
             val profile:Profile = { name, nickname }\n",
        );
        assert!(
            report.is_ok(),
            "expected defaulted record elision to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );

        let module = report.elaborated_module();
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
            "val left = Map { \"id\": 1 }\n\
             val right = Map { \"id\": 1 }\n\
             val same:Bool = left == right\n",
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
            "val left = Map { \"id\": 1 }\n\
             val right = Map { \"id\": 2 }\n\
             val different:Bool = left != right\n",
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
    fn typecheck_accepts_same_module_eq_instances_for_nonstructural_types() {
        let report = typecheck_text(
            "same-module-eq-instance.aivi",
            "class Eq A\n\
             \x20\x20\x20\x20(==) : A -> A -> Bool\n\
             type Blob = Blob Bytes\n\
             fun blobEquals:Bool #left:Blob #right:Blob =>\n\
             \x20\x20\x20\x20True\n\
             instance Eq Blob\n\
             \x20\x20\x20\x20(==) left right = blobEquals left right\n\
             fun compare:Bool #left:Blob #right:Blob =>\n\
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
            "val broken:Bool = not None\n",
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
            "fun increment:Int #value:Int => value + 1\n\
             val mapped:Option Int = map increment (Some 1)\n",
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
            "fun add:Int #acc:Int #value:Int => acc + value\n\
             val joined:Text = reduce append empty [\"hel\", \"lo\"]\n\
             val total:Int = reduce add 10 (Some 2)\n",
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
            "val pureOption:(Int -> Option Int) = pure\n",
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
            "fun same:Eq A => Bool #value:A => True\n\
             val sameText:Bool = same \"Ada\"\n",
        );
        assert!(
            report.is_ok(),
            "expected signature constraints to solve at call sites, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_ord_comparison_for_text() {
        let report = typecheck_text(
            "ord-text-comparison.aivi",
            "val ordered:Bool = \"a\" < \"b\"\n",
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
            "val broken:Bool = [1] < [2]\n",
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
        let report = typecheck_text("value-mismatch.aivi", "val answer:Text = 42\n");
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
            "fun keep #value => value\n\
             val chosen:(Option Int -> Option Int) = keep\n",
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
            "fun keepNone #value:Option Int => None\n\
             val result:Option Int = keepNone None\n",
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
            "fun keep:Option Int #value:Option Int => value\n\
             val result:Option Int = keep None\n",
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
            "fun keep:Option Int #value:Option Int => value\n\
             val result:Option Text = keep None\n",
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
             val name = \"Ada\"\n\
             val user:User = { name }\n",
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
             val name = \"Ada\"\n\
             val user:User = { name }\n",
        );
        assert!(
            report.is_ok(),
            "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
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
             val user:User = { name: \"Ada\" }\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient Default class to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_elaborates_same_module_default_instances_into_explicit_fields() {
        let report = typecheck_text(
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
             val name = \"Ada\"\n\
             val user:User = { name }\n",
        );
        assert!(
            report.is_ok(),
            "expected same-module Default instance to satisfy record elision, got diagnostics: {:?}",
            report.diagnostics()
        );

        let module = report.elaborated_module();
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
             val wrapped:(Box Text) = Box 42\n",
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
             val first:(Option Text) = Some \"Ada\"\n\
             sig last = \"Lovelace\"\n\
             val broken =\n\
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
             val first = Some \"Ada\"\n\
             val last = None\n\
             val maybePair:Option NamePair =\n\
              &|> first\n\
              &|> last\n\
               |> NamePair\n\
             val okFirst = Ok \"Ada\"\n\
             val errLast = Err \"missing\"\n\
             val resultPair:Result Text NamePair =\n\
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
val current:Screen = Loading
val broken =
    current
     ||> Loading => 0
     ||> Ready title => title
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
val current:Screen = Loading
val maybeLabel:Option Text =
    current
     ||> Loading => None
     ||> Ready title => Some title
     ||> Failed reason => Some reason
val resultLabel:Result Text Text =
    current
     ||> Loading => Ok "loading"
     ||> Ready title => Ok title
     ||> Failed reason => Err reason
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
            r#"fun addOne:Int #n:Int => n + 1
val value:Int =
    0
     ||> 0 => addOne 0
     ||> _ => 1
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
            r#"fun addOne:Int #n:Int => n + 1
val value:Int =
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
fun remaining:Int #acc:(TakeAcc A) => acc.n
fun items:(List A) #acc:(TakeAcc A) => acc.items
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
        let report = typecheck_text(
            "polymorphic-pipe-transforms.aivi",
            "fun wrap:(Option A) #value:A => Some value\n\
             val maybeNumber:Option Int = 1 |> wrap\n\
             val maybeLabel:Option Text = \"Ada\" |> wrap\n",
        );
        assert!(
            report.is_ok(),
            "expected polymorphic pipe transforms to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_accepts_polymorphic_function_application() {
        let report = typecheck_text(
            "polymorphic-function-application.aivi",
            "fun wrap:(Option A) #value:A => Some value\n\
             val maybeNumber:Option Int = wrap 1\n\
             val maybeLabel:Option Text = wrap \"Ada\"\n",
        );
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
            "fun even:Bool #value:Int => value == 2 or value == 4\n\
             val maybeName:Option Text = Some \"Ada\"\n\
             val numbers:List Int = [1, 2, 3, 4]\n\
             val chosenName:Text = __aivi_option_getOrElse \"guest\" maybeName\n\
             val count:Int = __aivi_list_length numbers\n\
             val firstNumber:Option Int = __aivi_list_head numbers\n\
             val hasEven:Bool = __aivi_list_any even numbers\n",
        );
        assert!(
            report.is_ok(),
            "expected ambient polymorphic helper application to typecheck, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn typecheck_reports_invalid_pipe_stage_input_for_transforms() {
        let report = typecheck_text(
            "invalid-pipe-stage-transform.aivi",
            "fun describe:Text #value:Int => \"count\"\n\
             val broken:Text = \"Ada\" |> describe\n",
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
            "fun describe:Text #value:Int => \"count\"\n\
             val broken:Text = \"Ada\" | describe\n",
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
            "val profile = { name: \"Ada\", age: 36 }\n\
             val name:Text = profile.name\n",
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
            "val profile = { name: \"Ada\", age: 36 }\n\
             val missing:Text = profile.missing\n",
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
            "val pair:(Option Int, Result Text Int) = (None, Ok 1)\n\
             val items:List (Option Int) = [None, Some 2]\n\
             val headers:Map Text (Option Int) = Map { \"primary\": None, \"backup\": Some 3 }\n\
             val tags:Set (Option Int) = Set [None, Some 4]\n",
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
            "val pair:(Option Int, Result Text Int) = (Some \"Ada\", Ok \"Ada\")\n\
             val items:List (Option Int) = [Some \"Ada\"]\n\
             val headers:Map Text (Option Int) = Map { \"primary\": Some \"Ada\" }\n\
             val tags:Set (Option Int) = Set [Some \"Ada\"]\n",
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
            "val pi:Float = 3.14\n\
             val amount:Decimal = 19.25d\n\
             val whole:Decimal = 19d\n\
             val count:BigInt = 123n\n",
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
            "val pi:Float = 19.25d\n\
             val amount:Decimal = 3.14\n\
             val count:BigInt = 42\n",
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
}
