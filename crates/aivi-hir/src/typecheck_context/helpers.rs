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
    if let Some(binding) = stage.result_memo {
        env.locals.insert(binding, result_subject.clone());
    }
}

/// Outcome of one step in a `PipeSubjectWalker` iteration.
///
/// Returned by per-stage callbacks to tell the walker how to advance and what
/// the new subject type is after the stage (PA-M1).
pub(crate) enum PipeSubjectStepOutcome {
    /// The stage was handled; `new_subject` is the subject type after the
    /// stage and `advance_by` is how many stage slots to skip (usually 1, but
    /// fanout segments span multiple slots).
    Continue {
        new_subject: Option<GateType>,
        advance_by: usize,
    },
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
/// PipeSubjectWalker::new(pipe, env, typing).walk(|stage_index, stage, current, typing| {
///     match &stage.kind {
///         PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue { … },
///         _ => PipeSubjectStepOutcome::Stop,
///     }
/// });
/// ```
pub(crate) struct PipeSubjectWalker<'pipe> {
    stages: Vec<&'pipe PipeStage>,
    current: Option<GateType>,
    env: GateExprEnv,
}

impl<'pipe> PipeSubjectWalker<'pipe> {
    pub(crate) fn new(
        pipe: &'pipe crate::hir::PipeExpr,
        env: &GateExprEnv,
        typing: &mut GateTypeContext<'_>,
    ) -> Self {
        let stages = pipe.stages.iter().collect::<Vec<_>>();
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
        let stages = pipe.stages.iter().take(limit).collect::<Vec<_>>();
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
            usize,             // stage_index
            &'pipe PipeStage,  // stage
            Option<&GateType>, // current subject (before this stage)
            &GateExprEnv,      // current pipe environment
            &mut GateTypeContext<'_>,
        ) -> PipeSubjectStepOutcome,
    {
        let mut stage_index = 0usize;
        while stage_index < self.stages.len() {
            let stage = self.stages[stage_index];
            match &stage.kind {
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
                    stage_index += 1;
                }
                PipeStageKind::Tap { expr } => {
                    if let Some(subject) = self.current.clone() {
                        let stage_env = pipe_stage_expr_env(&self.env, stage, &subject);
                        let _ = typing.infer_pipe_body(*expr, &stage_env, &subject);
                        extend_pipe_env_with_stage_memos(&mut self.env, stage, &subject, &subject);
                        self.current = Some(subject);
                    }
                    stage_index += 1;
                }
                _ => {
                    let current_subject = self.current.clone();
                    let stage_env = current_subject.as_ref().map_or_else(
                        || self.env.clone(),
                        |subject| pipe_stage_expr_env(&self.env, stage, subject),
                    );
                    match on_stage(
                        stage_index,
                        stage,
                        current_subject.as_ref(),
                        &stage_env,
                        typing,
                    ) {
                        PipeSubjectStepOutcome::Continue {
                            new_subject,
                            advance_by,
                        } => {
                            if let (Some(input_subject), Some(result_subject)) =
                                (current_subject.as_ref(), new_subject.as_ref())
                            {
                                extend_pipe_env_with_stage_memos(
                                    &mut self.env,
                                    stage,
                                    input_subject,
                                    result_subject,
                                );
                            }
                            self.current = new_subject;
                            stage_index += advance_by;
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct TruthyFalsyPairStages<'a> {
    pub(crate) truthy_index: usize,
    pub(crate) truthy_stage: &'a crate::hir::PipeStage,
    pub(crate) truthy_expr: ExprId,
    pub(crate) falsy_index: usize,
    pub(crate) falsy_stage: &'a crate::hir::PipeStage,
    pub(crate) falsy_expr: ExprId,
    pub(crate) next_index: usize,
}

pub(crate) fn truthy_falsy_pair_start_stage<'a>(
    pair: &TruthyFalsyPairStages<'a>,
) -> &'a crate::hir::PipeStage {
    if pair.truthy_index < pair.falsy_index {
        pair.truthy_stage
    } else {
        pair.falsy_stage
    }
}

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

