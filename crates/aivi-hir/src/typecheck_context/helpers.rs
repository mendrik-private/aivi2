pub(crate) fn builtin_type_arity(builtin: BuiltinType) -> usize {
    match builtin {
        BuiltinType::Int
        | BuiltinType::Float
        | BuiltinType::Decimal
        | BuiltinType::BigInt
        | BuiltinType::Bool
        | BuiltinType::Text
        | BuiltinType::Unit
        | BuiltinType::Bytes => 0,
        BuiltinType::List | BuiltinType::Set | BuiltinType::Option | BuiltinType::Signal => 1,
        BuiltinType::Map | BuiltinType::Result | BuiltinType::Validation | BuiltinType::Task => 2,
    }
}

pub(crate) fn type_constructor_arity(head: TypeConstructorHead, module: &Module) -> usize {
    match head {
        TypeConstructorHead::Builtin(builtin) => builtin_type_arity(builtin),
        TypeConstructorHead::Item(item_id) => match &module.items()[item_id] {
            Item::Type(item) => item.parameters.len(),
            Item::Domain(item) => item.parameters.len(),
            _ => 0,
        },
        TypeConstructorHead::Import(import_id) => match &module.imports()[import_id].metadata {
            ImportBindingMetadata::TypeConstructor { kind, .. }
            | ImportBindingMetadata::Domain { kind, .. } => kind.arity(),
            _ => 0,
        },
    }
}

pub(crate) fn item_type_name(item: &Item) -> String {
    match item {
        Item::Type(item) => item.name.text().to_owned(),
        Item::Class(item) => item.name.text().to_owned(),
        Item::Domain(item) => item.name.text().to_owned(),
        Item::SourceProviderContract(item) => {
            item.provider.key().unwrap_or("<provider>").to_owned()
        }
        other => format!("{:?}", other.kind()),
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprEnv {
    pub(crate) locals: HashMap<BindingId, GateType>,
    pub(crate) current_domain: Option<ItemId>,
    pub(crate) equality_evidence: Vec<GateEqualityEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GateEqualityEvidence {
    pub(crate) binding: BindingId,
    pub(crate) span: SourceSpan,
    pub(crate) name: Box<str>,
    pub(crate) ty: GateType,
    pub(crate) subject: GateType,
    pub(crate) member: ClassMemberResolution,
    pub(crate) priority: u8,
}

pub(crate) fn pipe_stage_subject_memo_type(
    _stage: &PipeStage,
    subject: &GateType,
) -> Option<GateType> {
    Some(subject.gate_payload().clone())
}

pub(crate) fn pipe_stage_expr_env(
    env: &GateExprEnv,
    stage: &PipeStage,
    subject: &GateType,
) -> GateExprEnv {
    let mut stage_env = env.clone();
    if let Some(binding) = stage.subject_memo
        && let Some(ty) = pipe_stage_subject_memo_type(stage, subject)
    {
        stage_env.locals.insert(binding, ty);
    }
    stage_env
}

pub(crate) fn extend_pipe_env_with_stage_memos(
    env: &mut GateExprEnv,
    stage: &PipeStage,
    input_subject: &GateType,
    result_subject: &GateType,
) {
    if let Some(binding) = stage.subject_memo
        && let Some(ty) = pipe_stage_subject_memo_type(stage, input_subject)
    {
        env.locals.insert(binding, ty);
    }
    extend_pipe_env_with_stage_result_memo(env, stage, result_subject);
}

pub(crate) fn extend_pipe_env_with_stage_result_memo(
    env: &mut GateExprEnv,
    stage: &PipeStage,
    result_subject: &GateType,
) {
    if let Some(binding) = stage.result_memo {
        env.locals.insert(binding, result_subject.clone());
    }
}

fn extend_pipe_env_with_fanout_segment_memos(
    env: &mut GateExprEnv,
    segment: &crate::PipeFanoutSegment<'_>,
    stage_env: &GateExprEnv,
    input_subject: &GateType,
    result_subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) {
    let Some(carrier) = input_subject.fanout_carrier() else {
        return;
    };
    let Some(element_subject) = input_subject.fanout_element() else {
        return;
    };
    let Some(mapped_element_type) = typing
        .infer_pipe_body(segment.map_expr(), stage_env, element_subject)
        .ty
    else {
        return;
    };
    let mapped_collection_type = typing.apply_fanout_plan(
        FanoutPlanner::plan(FanoutStageKind::Map, carrier),
        mapped_element_type,
    );

    if let Some(binding) = segment.map_stage().result_memo {
        env.locals.insert(binding, mapped_collection_type.clone());
    }
    for stage in segment.filter_stages() {
        if let Some(binding) = stage.result_memo {
            env.locals.insert(binding, mapped_collection_type.clone());
        }
    }
    if let Some(join_stage) = segment.join_stage()
        && let Some(binding) = join_stage.result_memo
    {
        env.locals.insert(binding, result_subject.clone());
    }
}

fn extend_pipe_env_with_subject_stage_memos(
    env: &mut GateExprEnv,
    subject_stage: &crate::PipeSubjectStage<'_>,
    stage_env: &GateExprEnv,
    input_subject: &GateType,
    result_subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) {
    match subject_stage {
        crate::PipeSubjectStage::Single { stage, .. } => {
            extend_pipe_env_with_stage_memos(env, stage, input_subject, result_subject);
        }
        crate::PipeSubjectStage::FanoutSegment(segment) => {
            extend_pipe_env_with_fanout_segment_memos(
                env,
                segment,
                stage_env,
                input_subject,
                result_subject,
                typing,
            );
        }
        crate::PipeSubjectStage::ApplyRun(_) => {}
        crate::PipeSubjectStage::TruthyFalsyPair(pair) => {
            extend_pipe_env_with_stage_memos(
                env,
                pair.start_stage(),
                input_subject,
                result_subject,
            );
        }
        crate::PipeSubjectStage::CaseRun(run) => {
            extend_pipe_env_with_stage_memos(
                env,
                run.start_stage(),
                input_subject,
                result_subject,
            );
        }
    }
}

/// Outcome of one step in a `PipeSubjectWalker` iteration.
///
/// Returned by per-stage callbacks to tell the walker what the new subject type
/// is after the current grouped subject stage (PA-M1).
pub(crate) enum PipeSubjectStepOutcome {
    /// The stage was handled; `new_subject` is the subject type after it.
    Continue { new_subject: Option<GateType> },
    /// Stop walking at this stage index (e.g. when hitting a recurrence
    /// boundary or a stage kind the caller cannot handle).
    Stop,
}

/// Iterator-style helper that walks a pipe expression's stages left-to-right,
/// maintaining the subject type across `|>` transform and `|` tap stages.
///
/// Callers supply a callback that handles operator-specific stages (gate,
/// fanout, truthy/falsy, recurrence, …).  The walker handles the common
/// `Transform` and `Tap` stages so every pass doesn't duplicate that logic
/// (PA-M1).
///
/// # Usage
/// ```ignore
/// PipeSubjectWalker::new(pipe, env, typing).walk(|stage, current, typing| {
///     match stage {
///         crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
///             PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue { … },
///             _ => PipeSubjectStepOutcome::Stop,
///         },
///         _ => PipeSubjectStepOutcome::Stop,
///     }
/// });
/// ```
pub(crate) struct PipeSubjectWalker<'pipe> {
    stages: Vec<crate::PipeSubjectStage<'pipe>>,
    current: Option<GateType>,
    env: GateExprEnv,
}

impl<'pipe> PipeSubjectWalker<'pipe> {
    pub(crate) fn new(
        pipe: &'pipe crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) -> Self {
        let stages = pipe.subject_stages().collect::<Vec<_>>();
        let current = typing.infer_expr(pipe.head, env, None).ty;
        Self {
            stages,
            current,
            env: env.clone(),
        }
    }

    /// Like `new`, but only considers the first `limit` stages of the pipe.
    ///
    /// Used by passes (e.g. recurrence elaboration) that must stop before the
    /// recurrence boundary stages (`RecurStart`/`RecurStep`) which appear at a
    /// known prefix position (PA-M1).
    pub(crate) fn new_with_limit(
        pipe: &'pipe crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
        limit: usize,
    ) -> Self {
        let stages = pipe.subject_stages_with_limit(limit).collect::<Vec<_>>();
        let current = typing.infer_expr(pipe.head, env, None).ty;
        Self {
            stages,
            current,
            env: env.clone(),
        }
    }

    /// Walk all stages, calling `on_stage` for each stage that is not a plain
    /// `Transform` or `Tap`.  Iteration stops when `on_stage` returns
    /// `PipeSubjectStepOutcome::Stop` or when all stages are exhausted.
    ///
    /// Returns the subject type at the point where walking stopped.
    pub(crate) fn walk<F>(
        mut self,
        typing: &mut GateTypeContext<'_>,
        mut on_stage: F,
    ) -> Option<GateType>
    where
        F: FnMut(
            &crate::PipeSubjectStage<'pipe>,
            Option<&GateType>, // current subject (before this stage)
            &GateExprEnv,      // current pipe environment
            &mut GateTypeContext<'_>,
        ) -> PipeSubjectStepOutcome,
    {
        for subject_stage in &self.stages {
            let start_stage = subject_stage.start_stage();
            match subject_stage {
                crate::PipeSubjectStage::Single { stage, .. } => match &stage.kind {
                    PipeStageKind::Transform { expr } => {
                        if let Some(subject) = self.current.clone() {
                            let stage_env = pipe_stage_expr_env(&self.env, stage, &subject);
                            self.current = typing.infer_transform_stage(*expr, &stage_env, &subject);
                            if let Some(result_subject) = self.current.as_ref() {
                                extend_pipe_env_with_stage_memos(
                                    &mut self.env,
                                    stage,
                                    &subject,
                                    result_subject,
                                );
                            }
                        }
                    }
                    PipeStageKind::Tap { expr } => {
                        if let Some(subject) = self.current.clone() {
                            let stage_env = pipe_stage_expr_env(&self.env, stage, &subject);
                            let _ = typing.infer_pipe_body(*expr, &stage_env, &subject);
                            extend_pipe_env_with_stage_memos(&mut self.env, stage, &subject, &subject);
                            self.current = Some(subject);
                        }
                    }
                    _ => {
                        let current_subject = self.current.clone();
                        let stage_env = current_subject.as_ref().map_or_else(
                            || self.env.clone(),
                            |subject| pipe_stage_expr_env(&self.env, start_stage, subject),
                        );
                        match on_stage(subject_stage, current_subject.as_ref(), &stage_env, typing) {
                            PipeSubjectStepOutcome::Continue { new_subject } => {
                                if let (Some(input_subject), Some(result_subject)) =
                                    (current_subject.as_ref(), new_subject.as_ref())
                                {
                                    extend_pipe_env_with_subject_stage_memos(
                                        &mut self.env,
                                        subject_stage,
                                        &stage_env,
                                        input_subject,
                                        result_subject,
                                        typing,
                                    );
                                }
                                self.current = new_subject;
                            }
                            PipeSubjectStepOutcome::Stop => break,
                        }
                    }
                },
                _ => {
                    let current_subject = self.current.clone();
                    let stage_env = current_subject.as_ref().map_or_else(
                        || self.env.clone(),
                        |subject| pipe_stage_expr_env(&self.env, start_stage, subject),
                    );
                    match on_stage(subject_stage, current_subject.as_ref(), &stage_env, typing) {
                        PipeSubjectStepOutcome::Continue { new_subject } => {
                            if let (Some(input_subject), Some(result_subject)) =
                                (current_subject.as_ref(), new_subject.as_ref())
                            {
                                extend_pipe_env_with_subject_stage_memos(
                                    &mut self.env,
                                    subject_stage,
                                    &stage_env,
                                    input_subject,
                                    result_subject,
                                    typing,
                                );
                            }
                            self.current = new_subject;
                        }
                        PipeSubjectStepOutcome::Stop => break,
                    }
                }
            }
        }
        self.current
    }
}

/// Build a `GateExprEnv` from a function item's annotated parameters.
///
/// Shared by all gate/truthy-falsy/recurrence elaboration passes so the
/// parameter-to-type wiring is defined exactly once (PA-I2).
pub(crate) fn gate_env_for_function(
    item: &crate::hir::FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> GateExprEnv {
    let mut env = GateExprEnv::default();
    for parameter in &item.parameters {
        let Some(annotation) = parameter.annotation else {
            continue;
        };
        if let Some(ty) = typing.lower_open_annotation(annotation) {
            env.locals.insert(parameter.binding, ty);
        }
    }
    env
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GateExprInfo {
    pub(crate) ty: Option<GateType>,
    pub(crate) actual: Option<SourceOptionActualType>,
    pub(crate) contains_signal: bool,
    pub(crate) issues: Vec<GateIssue>,
    pub(crate) constraints: Vec<TypeConstraint>,
}

impl GateExprInfo {
    pub(crate) fn merge(&mut self, other: Self) {
        self.contains_signal |= other.contains_signal;
        self.issues.extend(other.issues);
        self.constraints.extend(other.constraints);
    }

    pub(crate) fn actual(&self) -> Option<SourceOptionActualType> {
        self.actual
            .clone()
            .or_else(|| self.ty.as_ref().map(SourceOptionActualType::from_gate_type))
    }

    pub(crate) fn actual_gate_type(&self) -> Option<GateType> {
        self.actual().and_then(|actual| actual.to_gate_type())
    }

    pub(crate) fn set_actual(&mut self, actual: SourceOptionActualType) {
        self.contains_signal |= actual.is_signal();
        self.ty = actual.to_gate_type();
        self.actual = Some(actual);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PipeBodyInference {
    info: GateExprInfo,
    transform_mode: PipeTransformMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PipeFunctionSignatureMatch {
    pub(crate) callee_expr: ExprId,
    pub(crate) explicit_arguments: Vec<ExprId>,
    pub(crate) signal_payload_arguments: Vec<bool>,
    pub(crate) parameter_types: Vec<GateType>,
    pub(crate) result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateIssue {
    UnknownLiteralSuffix {
        span: SourceSpan,
        suffix: String,
    },
    AmbiguousLiteralSuffix {
        span: SourceSpan,
        suffix: String,
        candidates: Vec<String>,
    },
    InvalidPipeStageInput {
        span: SourceSpan,
        stage: &'static str,
        expected: String,
        actual: String,
    },
    AmbientSubjectOutsidePipe {
        span: SourceSpan,
    },
    InvalidProjection {
        span: SourceSpan,
        path: String,
        subject: String,
    },
    UnknownField {
        span: SourceSpan,
        path: String,
        subject: String,
    },
    AmbiguousDomainMember {
        span: SourceSpan,
        name: String,
        candidates: Vec<String>,
    },
    UnsupportedApplicativeClusterMember {
        span: SourceSpan,
        actual: String,
    },
    ApplicativeClusterMismatch {
        span: SourceSpan,
        expected: String,
        actual: String,
    },
    InvalidClusterFinalizer {
        span: SourceSpan,
        expected_inputs: Vec<String>,
        actual: String,
    },
    CaseBranchTypeMismatch {
        span: SourceSpan,
        expected: String,
        actual: String,
    },
    /// Two or more domain operator implementations match the given binary expression.
    /// The caller must emit this issue as a diagnostic and treat the operator result type
    /// as unknown so downstream checking can continue without cascading false errors.
    AmbiguousDomainOperator {
        span: SourceSpan,
        operator: String,
        candidates: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DomainMemberSelection<T> {
    Unique(T),
    Ambiguous,
    NoMatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LiteralSuffixSelection {
    Unique {
        resolution: LiteralSuffixResolution,
        base: LiteralSuffixBase,
        result: GateType,
    },
    Ambiguous {
        candidates: Vec<String>,
    },
    NoMatch {
        candidates: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiteralSuffixCallLowering {
    pub(crate) resolution: LiteralSuffixResolution,
    pub(crate) base: LiteralSuffixBase,
    pub(crate) callee_type: GateType,
    pub(crate) result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DomainMemberCallMatch {
    pub(crate) parameters: Vec<GateType>,
    pub(crate) result: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateProjectionStep {
    RecordField {
        result: GateType,
    },
    DomainMember {
        handle: DomainMemberHandle,
        result: GateType,
    },
}

impl GateProjectionStep {
    pub(crate) fn result(&self) -> &GateType {
        match self {
            Self::RecordField { result } | Self::DomainMember { result, .. } => result,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassConstraintBinding {
    pub(crate) class_item: ItemId,
    pub(crate) subject: TypeBinding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassMemberCallMatch {
    pub(crate) resolution: ClassMemberResolution,
    pub(crate) parameters: Vec<GateType>,
    pub(crate) result: GateType,
    pub(crate) evidence: ClassConstraintBinding,
    pub(crate) constraints: Vec<ClassConstraintBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TruthyFalsySubjectPlan {
    pub(crate) truthy_constructor: BuiltinTerm,
    pub(crate) truthy_payload: Option<GateType>,
    pub(crate) falsy_constructor: BuiltinTerm,
    pub(crate) falsy_payload: Option<GateType>,
}

pub(crate) type TruthyFalsyPairStages<'a> = crate::PipeTruthyFalsyPair<'a>;

pub fn case_pattern_field_types(
    module: &Module,
    callee: &TermReference,
    subject: &GateType,
) -> Option<Vec<GateType>> {
    pub(crate) fn same_module_constructor_fields(
        module: &Module,
        item_id: ItemId,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let Item::Type(item) = &module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let GateType::OpaqueItem {
            item: subject_item,
            arguments,
            ..
        } = subject
        else {
            return None;
        };
        if *subject_item != item_id || item.parameters.len() != arguments.len() {
            return None;
        }
        let variant_name = callee.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut typing = GateTypeContext::new(module);
        variant
            .fields
            .iter()
            .map(|field| typing.lower_hir_type(field.ty, &substitutions))
            .collect()
    }

    match callee.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::True))
        | ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::False)) => {
            matches!(subject, GateType::Primitive(BuiltinType::Bool)).then(Vec::new)
        }
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Some)) => match subject {
            GateType::Option(payload) => Some(vec![payload.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::None)) => {
            matches!(subject, GateType::Option(_)).then(Vec::new)
        }
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Ok)) => match subject {
            GateType::Result { value, .. } => Some(vec![value.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Err)) => match subject {
            GateType::Result { error, .. } => Some(vec![error.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Valid)) => match subject {
            GateType::Validation { value, .. } => Some(vec![value.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Invalid)) => match subject {
            GateType::Validation { error, .. } => Some(vec![error.as_ref().clone()]),
            _ => None,
        },
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            same_module_constructor_fields(module, *item_id, callee, subject)
        }
        ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            // Imported sum constructor (e.g. `SwitchView : View -> UIEvent`).
            // Walk the Arrow chain collecting parameter types; verify the final Named result
            // type matches the subject's OpaqueImport name.
            let binding = module.imports().get(*import_id)?;
            let ty_owned: ImportValueType = match &binding.metadata {
                ImportBindingMetadata::Value { ty } => ty.clone(),
                ImportBindingMetadata::IntrinsicValue { ty, .. } => ty.clone(),
                _ => return None,
            };
            let ctx = GateTypeContext::new(module);
            let mut params: Vec<GateType> = Vec::new();
            let mut cur: &ImportValueType = &ty_owned;
            loop {
                match cur {
                    ImportValueType::Arrow { parameter, result } => {
                        let gate = ctx.lower_import_value_type(parameter);
                        params.push(gate);
                        cur = result.as_ref();
                    }
                    ImportValueType::Named { type_name, .. } => {
                        let matches = match subject {
                            GateType::OpaqueImport { name, .. } => {
                                name.as_str() == type_name.as_str()
                            }
                            _ => false,
                        };
                        return matches.then_some(params);
                    }
                    _ => return None,
                }
            }
        }
        ResolutionState::Resolved(_) | ResolutionState::Unresolved => None,
    }
}

pub fn domain_carrier_type(
    module: &Module,
    item_id: ItemId,
    arguments: &[GateType],
) -> Option<GateType> {
    let Item::Domain(domain) = &module.items()[item_id] else {
        return None;
    };
    if domain.parameters.len() != arguments.len() {
        return None;
    }
    let substitutions = domain
        .parameters
        .iter()
        .copied()
        .zip(arguments.iter().cloned())
        .collect::<HashMap<_, _>>();
    let mut typing = GateTypeContext::new(module);
    typing.lower_hir_type(domain.carrier, &substitutions)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpaqueTypeVariant {
    pub name: Box<str>,
    pub fields: Vec<GateType>,
}

pub fn opaque_type_variants(module: &Module, subject: &GateType) -> Option<Vec<OpaqueTypeVariant>> {
    fn lower_import_value_type_with_substitutions(
        module: &Module,
        ty: &ImportValueType,
        substitutions: &[GateType],
    ) -> GateType {
        match ty {
            ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
            ImportValueType::Tuple(elements) => GateType::Tuple(
                elements
                    .iter()
                    .map(|element| {
                        lower_import_value_type_with_substitutions(module, element, substitutions)
                    })
                    .collect(),
            ),
            ImportValueType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .map(|field| GateRecordField {
                        name: field.name.to_string(),
                        ty: lower_import_value_type_with_substitutions(
                            module,
                            &field.ty,
                            substitutions,
                        ),
                    })
                    .collect(),
            ),
            ImportValueType::Arrow { parameter, result } => GateType::Arrow {
                parameter: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    parameter,
                    substitutions,
                )),
                result: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    result,
                    substitutions,
                )),
            },
            ImportValueType::List(element) => GateType::List(Box::new(
                lower_import_value_type_with_substitutions(module, element, substitutions),
            )),
            ImportValueType::Map { key, value } => GateType::Map {
                key: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    key,
                    substitutions,
                )),
                value: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    value,
                    substitutions,
                )),
            },
            ImportValueType::Set(element) => GateType::Set(Box::new(
                lower_import_value_type_with_substitutions(module, element, substitutions),
            )),
            ImportValueType::Option(element) => GateType::Option(Box::new(
                lower_import_value_type_with_substitutions(module, element, substitutions),
            )),
            ImportValueType::Result { error, value } => GateType::Result {
                error: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    error,
                    substitutions,
                )),
                value: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    value,
                    substitutions,
                )),
            },
            ImportValueType::Validation { error, value } => GateType::Validation {
                error: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    error,
                    substitutions,
                )),
                value: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    value,
                    substitutions,
                )),
            },
            ImportValueType::Signal(element) => GateType::Signal(Box::new(
                lower_import_value_type_with_substitutions(module, element, substitutions),
            )),
            ImportValueType::Task { error, value } => GateType::Task {
                error: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    error,
                    substitutions,
                )),
                value: Box::new(lower_import_value_type_with_substitutions(
                    module,
                    value,
                    substitutions,
                )),
            },
            ImportValueType::TypeVariable { index, name } => substitutions
                .get(*index)
                .cloned()
                .unwrap_or_else(|| GateType::TypeParameter {
                    parameter: TypeParameterId::from_raw(u32::MAX - *index as u32),
                    name: name.clone(),
                }),
            ImportValueType::Named {
                type_name,
                arguments,
                definition,
            } => {
                let lowered_args = arguments
                    .iter()
                    .map(|argument| {
                        lower_import_value_type_with_substitutions(
                            module,
                            argument,
                            substitutions,
                        )
                    })
                    .collect::<Vec<_>>();
                match definition.as_deref() {
                    Some(ImportTypeDefinition::Alias(alias)) => {
                        lower_import_value_type_with_substitutions(module, alias, &lowered_args)
                    }
                    Some(ImportTypeDefinition::Sum(_)) | None => {
                        let import_id = module
                            .imports()
                            .iter()
                            .find(|(_, binding)| {
                                binding.imported_name.text() == type_name
                                    && matches!(
                                        &binding.metadata,
                                        ImportBindingMetadata::TypeConstructor { .. }
                                            | ImportBindingMetadata::AmbientType
                                            | ImportBindingMetadata::BuiltinType(_)
                                            | ImportBindingMetadata::Domain { .. }
                                    )
                            })
                            .map(|(id, _)| id);
                        GateType::OpaqueImport {
                            import: import_id.unwrap_or_else(|| ImportId::from_raw(u32::MAX)),
                            name: type_name.clone(),
                            arguments: lowered_args,
                            definition: definition.clone(),
                        }
                    }
                }
            }
        }
    }

    match subject {
        GateType::OpaqueItem {
            item,
            arguments,
            ..
        } => {
            let Item::Type(type_item) = &module.items()[*item] else {
                return None;
            };
            let TypeItemBody::Sum(variants) = &type_item.body else {
                return None;
            };
            if type_item.parameters.len() != arguments.len() {
                return None;
            }
            let substitutions = type_item
                .parameters
                .iter()
                .copied()
                .zip(arguments.iter().cloned())
                .collect::<HashMap<_, _>>();
            let mut typing = GateTypeContext::new(module);
            variants
                .iter()
                .map(|variant| {
                    Some(OpaqueTypeVariant {
                        name: variant.name.text().into(),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| typing.lower_hir_type(field.ty, &substitutions))
                            .collect::<Option<Vec<_>>>()?,
                    })
                })
                .collect()
        }
        GateType::OpaqueImport {
            definition: Some(definition),
            arguments,
            ..
        } => match definition.as_ref() {
            ImportTypeDefinition::Alias(alias) => {
                opaque_type_variants(
                    module,
                    &lower_import_value_type_with_substitutions(module, alias, arguments),
                )
            }
            ImportTypeDefinition::Sum(variants) => Some(
                variants
                    .iter()
                    .map(|variant| OpaqueTypeVariant {
                        name: variant.name.clone(),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| {
                                lower_import_value_type_with_substitutions(module, field, arguments)
                            })
                            .collect(),
                    })
                    .collect(),
            ),
        },
        _ => None,
    }
}
