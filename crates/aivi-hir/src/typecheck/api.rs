#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintClass {
    Eq,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConstraintOrigin {
    Expression,
    RecordOmittedField {
        field_name: String,
        available_fields: Vec<String>,
    },
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
        available_fields: Vec<String>,
    ) -> Self {
        Self {
            span,
            class: ConstraintClass::Default,
            subject,
            origin: ConstraintOrigin::RecordOmittedField {
                field_name: field_name.into(),
                available_fields,
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
            ConstraintOrigin::RecordOmittedField { field_name, .. } => Some(field_name),
        }
    }

    fn available_field_names(&self) -> Option<&[String]> {
        match &self.origin {
            ConstraintOrigin::Expression => None,
            ConstraintOrigin::RecordOmittedField {
                available_fields, ..
            } => Some(available_fields),
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
    ImportedInstance {
        import: ImportId,
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

fn signal_annotation_payload(annotation: Option<&GateType>) -> Option<&GateType> {
    match annotation {
        Some(GateType::Signal(payload)) => Some(payload.as_ref()),
        _ => None,
    }
}

pub fn signal_payload_type(module: &Module, item: &SignalItem) -> Option<GateType> {
    let mut typing = GateTypeContext::new(module);
    let expected = item
        .annotation
        .and_then(|annotation| typing.lower_annotation(annotation));
    signal_annotation_payload(expected.as_ref())
        .cloned()
        .or_else(|| {
            item.body
                .and_then(|body| typing.infer_expr(body, &GateExprEnv::default(), None).ty)
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
