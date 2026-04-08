use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::fmt;

use aivi_base::SourceSpan;
use aivi_typing::{
    FanoutCarrier, FanoutPlan, FanoutPlanner, FanoutResultKind, FanoutStageKind, GateCarrier,
    GatePlanner, GateResultKind, RecurrenceTargetEvidence, RecurrenceWakeupKind,
    SourceTypeParameter,
};

use crate::{
    domain_operator_elaboration::{binary_operator_text, select_domain_binary_operator},
    function_inference::{
        FunctionCallEvidence, FunctionSignatureEvidence, infer_same_module_function_types,
        supports_same_module_function_inference,
    },
    hir::{
        ApplicativeSpineHead, BuiltinTerm, BuiltinType, ClassMemberResolution,
        CustomSourceRecurrenceWakeup, DomainMemberHandle, DomainMemberKind, DomainMemberResolution,
        ExprKind, ImportBindingMetadata, ImportTypeDefinition, ImportValueType, IntrinsicValue,
        Item, Module, Name, NamePath, PatternKind, PipeStage, PipeStageKind, PipeTransformMode,
        ProjectionBase, ResolutionState, TermReference, TermResolution, TextSegment, TypeItemBody,
        TypeKind, TypeReference, TypeResolution, TypeVariantField,
    },
    ids::{BindingId, ClusterId, ExprId, ImportId, ItemId, PatternId, TypeId, TypeParameterId},
    source_contract_resolution::{ResolvedSourceContractType, ResolvedSourceTypeConstructor},
    typecheck::{TypeConstraint, expression_matches},
    validate::{
        CaseConstructorKey, CaseConstructorShape, CasePatternCoverage, CaseSubjectShape,
        GateRecordField, RecurrenceTargetHint, ValidationMode, Validator, builtin_type_name,
    },
};

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
    stage: &PipeStage,
    subject: &GateType,
) -> Option<GateType> {
    if !stage.supports_memos() {
        return None;
    }
    match stage.kind {
        PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
            Some(subject.gate_payload().clone())
        }
        _ => None,
    }
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
    if stage.supports_memos()
        && let Some(binding) = stage.result_memo
    {
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
                _ => match on_stage(stage_index, stage, self.current.as_ref(), &self.env, typing) {
                    PipeSubjectStepOutcome::Continue {
                        new_subject,
                        advance_by,
                    } => {
                        self.current = new_subject;
                        stage_index += advance_by;
                    }
                    PipeSubjectStepOutcome::Stop => break,
                },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateType {
    Primitive(BuiltinType),
    TypeParameter {
        parameter: TypeParameterId,
        name: String,
    },
    Tuple(Vec<GateType>),
    Record(Vec<GateRecordField>),
    Arrow {
        parameter: Box<GateType>,
        result: Box<GateType>,
    },
    List(Box<GateType>),
    Map {
        key: Box<GateType>,
        value: Box<GateType>,
    },
    Set(Box<GateType>),
    Option(Box<GateType>),
    Result {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Validation {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Signal(Box<GateType>),
    Task {
        error: Box<GateType>,
        value: Box<GateType>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<GateType>,
    },
    /// An imported type constructor or domain from another module.
    OpaqueImport {
        import: ImportId,
        name: String,
        arguments: Vec<GateType>,
        definition: Option<Box<ImportTypeDefinition>>,
    },
}

impl GateType {
    pub(crate) fn is_bool(&self) -> bool {
        matches!(self, Self::Primitive(BuiltinType::Bool))
    }

    pub(crate) fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn gate_carrier(&self) -> GateCarrier {
        match self {
            Self::Signal(_) => GateCarrier::Signal,
            _ => GateCarrier::Ordinary,
        }
    }

    pub(crate) fn gate_payload(&self) -> &Self {
        match self {
            Self::Signal(inner) => inner,
            other => other,
        }
    }

    pub(crate) fn fanout_carrier(&self) -> Option<FanoutCarrier> {
        match self {
            Self::List(_) => Some(FanoutCarrier::Ordinary),
            Self::Signal(inner) if matches!(inner.as_ref(), Self::List(_)) => {
                Some(FanoutCarrier::Signal)
            }
            _ => None,
        }
    }

    pub(crate) fn fanout_element(&self) -> Option<&Self> {
        match self {
            Self::List(element) => Some(element),
            Self::Signal(inner) => match inner.as_ref() {
                Self::List(element) => Some(element),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn recurrence_target_evidence(&self) -> Option<RecurrenceTargetEvidence> {
        match self {
            Self::Signal(_) => Some(RecurrenceTargetEvidence::ExplicitSignalAnnotation),
            Self::Task { .. } => Some(RecurrenceTargetEvidence::ExplicitTaskAnnotation),
            _ => None,
        }
    }

    /// Extract the canonical name and arguments from a user-defined type variant
    /// (Domain, OpaqueItem, OpaqueImport). Returns None for builtin/primitive types.
    fn named_type_parts(&self) -> Option<(&str, &[GateType])> {
        match self {
            Self::Domain {
                name, arguments, ..
            }
            | Self::OpaqueItem {
                name, arguments, ..
            }
            | Self::OpaqueImport {
                name, arguments, ..
            } => Some((name.as_str(), arguments.as_slice())),
            _ => None,
        }
    }

    pub(crate) fn same_shape(&self, other: &Self) -> bool {
        let mut left_to_right = HashMap::new();
        let mut right_to_left = HashMap::new();
        Self::same_shape_inner(self, other, &mut left_to_right, &mut right_to_left)
    }

    /// Substitute every occurrence of `param` with `replacement` throughout this type.
    pub(crate) fn substitute_type_parameter(
        &self,
        param: TypeParameterId,
        replacement: &GateType,
    ) -> GateType {
        self.substitute_type_parameters(&HashMap::from([(param, replacement.clone())]))
    }

    /// Substitute multiple type parameters simultaneously using the given map.
    pub(crate) fn substitute_type_parameters(
        &self,
        subs: &HashMap<TypeParameterId, GateType>,
    ) -> GateType {
        if subs.is_empty() {
            return self.clone();
        }
        match self {
            Self::TypeParameter { parameter, .. } => {
                subs.get(parameter).cloned().unwrap_or_else(|| self.clone())
            }
            Self::Primitive(_) => self.clone(),
            Self::Arrow { parameter, result } => Self::Arrow {
                parameter: Box::new(parameter.substitute_type_parameters(subs)),
                result: Box::new(result.substitute_type_parameters(subs)),
            },
            Self::List(element) => Self::List(Box::new(element.substitute_type_parameters(subs))),
            Self::Option(element) => {
                Self::Option(Box::new(element.substitute_type_parameters(subs)))
            }
            Self::Signal(element) => {
                Self::Signal(Box::new(element.substitute_type_parameters(subs)))
            }
            Self::Tuple(elements) => Self::Tuple(
                elements
                    .iter()
                    .map(|e| e.substitute_type_parameters(subs))
                    .collect(),
            ),
            Self::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|f| GateRecordField {
                        name: f.name.clone(),
                        ty: f.ty.substitute_type_parameters(subs),
                    })
                    .collect(),
            ),
            Self::Map { key, value } => Self::Map {
                key: Box::new(key.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Set(element) => Self::Set(Box::new(element.substitute_type_parameters(subs))),
            Self::Result { error, value } => Self::Result {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Validation { error, value } => Self::Validation {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Task { error, value } => Self::Task {
                error: Box::new(error.substitute_type_parameters(subs)),
                value: Box::new(value.substitute_type_parameters(subs)),
            },
            Self::Domain {
                item,
                name,
                arguments,
            } => Self::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
            },
            Self::OpaqueItem {
                item,
                name,
                arguments,
            } => Self::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
            },
            Self::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Self::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(|a| a.substitute_type_parameters(subs))
                    .collect(),
                definition: definition.clone(),
            },
        }
    }

    /// Returns true when `self` (a concrete type) is a valid instantiation of `template`, treating
    /// any `TypeParameter` in `template` as an unconstrained wildcard.
    pub(crate) fn has_type_params(&self) -> bool {
        match self {
            Self::TypeParameter { .. } => true,
            Self::Primitive(_) => false,
            Self::Arrow { parameter, result } => {
                parameter.has_type_params() || result.has_type_params()
            }
            Self::List(e) | Self::Option(e) | Self::Signal(e) | Self::Set(e) => e.has_type_params(),
            Self::Tuple(elements) => elements.iter().any(|e| e.has_type_params()),
            Self::Record(fields) => fields.iter().any(|f| f.ty.has_type_params()),
            Self::Map { key, value } => key.has_type_params() || value.has_type_params(),
            Self::Result { error, value }
            | Self::Validation { error, value }
            | Self::Task { error, value } => error.has_type_params() || value.has_type_params(),
            Self::Domain { arguments, .. }
            | Self::OpaqueItem { arguments, .. }
            | Self::OpaqueImport { arguments, .. } => arguments.iter().any(|a| a.has_type_params()),
        }
    }

    pub(crate) fn fits_template(&self, template: &Self) -> bool {
        match template {
            Self::TypeParameter { .. } => true,
            Self::Primitive(_) => self == template,
            Self::Arrow {
                parameter: tp,
                result: tr,
            } => match self {
                Self::Arrow {
                    parameter: sp,
                    result: sr,
                } => sp.fits_template(tp) && sr.fits_template(tr),
                _ => false,
            },
            Self::List(te) => match self {
                Self::List(se) => se.fits_template(te),
                _ => false,
            },
            Self::Option(te) => match self {
                Self::Option(se) => se.fits_template(te),
                _ => false,
            },
            Self::Signal(te) => match self {
                Self::Signal(se) => se.fits_template(te),
                _ => false,
            },
            Self::Set(te) => match self {
                Self::Set(se) => se.fits_template(te),
                _ => false,
            },
            Self::Tuple(tes) => match self {
                Self::Tuple(ses) => {
                    ses.len() == tes.len()
                        && ses.iter().zip(tes.iter()).all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::Record(tfields) => match self {
                Self::Record(sfields) => {
                    sfields.len() == tfields.len()
                        && sfields
                            .iter()
                            .zip(tfields.iter())
                            .all(|(s, t)| s.name == t.name && s.ty.fits_template(&t.ty))
                }
                _ => false,
            },
            Self::Map { key: tk, value: tv } => match self {
                Self::Map { key: sk, value: sv } => sk.fits_template(tk) && sv.fits_template(tv),
                _ => false,
            },
            Self::Result {
                error: te,
                value: tv,
            } => match self {
                Self::Result {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Validation {
                error: te,
                value: tv,
            } => match self {
                Self::Validation {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Task {
                error: te,
                value: tv,
            } => match self {
                Self::Task {
                    error: se,
                    value: sv,
                } => se.fits_template(te) && sv.fits_template(tv),
                _ => false,
            },
            Self::Domain {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::OpaqueItem {
                arguments: targs,
                name: tname,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
            Self::OpaqueImport {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.fits_template(t))
                }
                _ => false,
            },
        }
    }

    /// Structurally match `self` (concrete) against `template` (may contain TypeParameter nodes),
    /// collecting the bindings.  Returns `true` when matching succeeds and all TypeParameter
    /// nodes receive consistent bindings.
    pub(crate) fn unify_type_params(
        &self,
        template: &Self,
        bindings: &mut HashMap<TypeParameterId, GateType>,
    ) -> bool {
        match template {
            Self::TypeParameter { parameter, .. } => match bindings.get(parameter) {
                Some(existing) => existing.same_shape(self),
                None => {
                    bindings.insert(*parameter, self.clone());
                    true
                }
            },
            Self::Primitive(_) => self == template,
            Self::Arrow {
                parameter: tp,
                result: tr,
            } => match self {
                Self::Arrow {
                    parameter: sp,
                    result: sr,
                } => sp.unify_type_params(tp, bindings) && sr.unify_type_params(tr, bindings),
                _ => false,
            },
            Self::List(te) => match self {
                Self::List(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Option(te) => match self {
                Self::Option(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Signal(te) => match self {
                Self::Signal(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Set(te) => match self {
                Self::Set(se) => se.unify_type_params(te, bindings),
                _ => false,
            },
            Self::Tuple(tes) => match self {
                Self::Tuple(ses) => {
                    ses.len() == tes.len()
                        && ses
                            .iter()
                            .zip(tes.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::Record(tfields) => match self {
                Self::Record(sfields) => {
                    sfields.len() == tfields.len()
                        && sfields.iter().zip(tfields.iter()).all(|(s, t)| {
                            s.name == t.name && s.ty.unify_type_params(&t.ty, bindings)
                        })
                }
                _ => false,
            },
            Self::Map { key: tk, value: tv } => match self {
                Self::Map { key: sk, value: sv } => {
                    sk.unify_type_params(tk, bindings) && sv.unify_type_params(tv, bindings)
                }
                _ => false,
            },
            Self::Result {
                error: te,
                value: tv,
            } => match self {
                Self::Result {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Validation {
                error: te,
                value: tv,
            } => match self {
                Self::Validation {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Task {
                error: te,
                value: tv,
            } => match self {
                Self::Task {
                    error: se,
                    value: sv,
                } => se.unify_type_params(te, bindings) && sv.unify_type_params(tv, bindings),
                _ => false,
            },
            Self::Domain {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::OpaqueItem {
                arguments: targs,
                name: tname,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
            Self::OpaqueImport {
                name: tname,
                arguments: targs,
                ..
            } => match self.named_type_parts() {
                Some((sname, sargs)) => {
                    sname == tname
                        && sargs.len() == targs.len()
                        && sargs
                            .iter()
                            .zip(targs.iter())
                            .all(|(s, t)| s.unify_type_params(t, bindings))
                }
                _ => false,
            },
        }
    }

    pub(crate) fn same_shape_inner(
        left: &Self,
        right: &Self,
        left_to_right: &mut HashMap<TypeParameterId, TypeParameterId>,
        right_to_left: &mut HashMap<TypeParameterId, TypeParameterId>,
    ) -> bool {
        match (left, right) {
            (Self::Primitive(left), Self::Primitive(right)) => left == right,
            (
                Self::TypeParameter {
                    parameter: left_parameter,
                    ..
                },
                Self::TypeParameter {
                    parameter: right_parameter,
                    ..
                },
            ) => match (
                left_to_right.get(left_parameter),
                right_to_left.get(right_parameter),
            ) {
                (Some(mapped_right), Some(mapped_left)) => {
                    mapped_right == right_parameter && mapped_left == left_parameter
                }
                (None, None) => {
                    left_to_right.insert(*left_parameter, *right_parameter);
                    right_to_left.insert(*right_parameter, *left_parameter);
                    true
                }
                _ => false,
            },
            (Self::Tuple(left), Self::Tuple(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        Self::same_shape_inner(left, right, left_to_right, right_to_left)
                    })
            }
            (Self::Record(left), Self::Record(right)) => {
                left.len() == right.len()
                    && left.iter().zip(right.iter()).all(|(left, right)| {
                        left.name == right.name
                            && Self::same_shape_inner(
                                &left.ty,
                                &right.ty,
                                left_to_right,
                                right_to_left,
                            )
                    })
            }
            (
                Self::Arrow {
                    parameter: left_parameter,
                    result: left_result,
                },
                Self::Arrow {
                    parameter: right_parameter,
                    result: right_result,
                },
            ) => {
                Self::same_shape_inner(
                    left_parameter,
                    right_parameter,
                    left_to_right,
                    right_to_left,
                ) && Self::same_shape_inner(left_result, right_result, left_to_right, right_to_left)
            }
            (Self::List(left), Self::List(right))
            | (Self::Set(left), Self::Set(right))
            | (Self::Option(left), Self::Option(right))
            | (Self::Signal(left), Self::Signal(right)) => {
                Self::same_shape_inner(left, right, left_to_right, right_to_left)
            }
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_key, right_key, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
            }
            (
                Self::Result {
                    error: left_error,
                    value: left_value,
                },
                Self::Result {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            )
            | (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => {
                Self::same_shape_inner(left_error, right_error, left_to_right, right_to_left)
                    && Self::same_shape_inner(left_value, right_value, left_to_right, right_to_left)
            }
            (
                Self::Domain {
                    item: left_item,
                    name: left_name,
                    arguments: left_arguments,
                },
                Self::Domain {
                    item: right_item,
                    name: right_name,
                    arguments: right_arguments,
                },
            ) => {
                (left_item == right_item || left_name == right_name)
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            (
                Self::OpaqueItem {
                    item: left_item,
                    arguments: left_arguments,
                    ..
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) => {
                left_item == right_item
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            (
                Self::OpaqueImport {
                    import: left_import,
                    arguments: left_arguments,
                    ..
                },
                Self::OpaqueImport {
                    import: right_import,
                    arguments: right_arguments,
                    ..
                },
            ) => {
                left_import == right_import
                    && left_arguments.len() == right_arguments.len()
                    && left_arguments
                        .iter()
                        .zip(right_arguments.iter())
                        .all(|(left, right)| {
                            Self::same_shape_inner(left, right, left_to_right, right_to_left)
                        })
            }
            // Cross-variant name-based equivalence: Domain, OpaqueItem, and
            // OpaqueImport all represent the same logical type when their canonical
            // names and argument shapes agree.  This covers ambient-prelude types
            // versus stdlib-imported types across all variant combinations.
            _ => match (left.named_type_parts(), right.named_type_parts()) {
                (Some((ln, la)), Some((rn, ra))) => {
                    ln == rn
                        && la.len() == ra.len()
                        && la.iter().zip(ra.iter()).all(|(l, r)| {
                            Self::same_shape_inner(l, r, left_to_right, right_to_left)
                        })
                }
                _ => false,
            },
        }
    }

    pub(crate) fn constructor_view(&self) -> Option<(TypeConstructorHead, Vec<GateType>)> {
        match self {
            Self::List(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::List),
                vec![element.as_ref().clone()],
            )),
            Self::Map { key, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Map),
                vec![key.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Set(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Set),
                vec![element.as_ref().clone()],
            )),
            Self::Option(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Option),
                vec![element.as_ref().clone()],
            )),
            Self::Result { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Result),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Validation { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Validation),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Signal(element) => Some((
                TypeConstructorHead::Builtin(BuiltinType::Signal),
                vec![element.as_ref().clone()],
            )),
            Self::Task { error, value } => Some((
                TypeConstructorHead::Builtin(BuiltinType::Task),
                vec![error.as_ref().clone(), value.as_ref().clone()],
            )),
            Self::Domain {
                item, arguments, ..
            }
            | Self::OpaqueItem {
                item, arguments, ..
            } => Some((TypeConstructorHead::Item(*item), arguments.clone())),
            Self::OpaqueImport {
                import, arguments, ..
            } => Some((TypeConstructorHead::Import(*import), arguments.clone())),
            Self::Primitive(_)
            | Self::TypeParameter { .. }
            | Self::Tuple(_)
            | Self::Record(_)
            | Self::Arrow { .. } => None,
        }
    }
}

impl fmt::Display for GateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateType::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            GateType::TypeParameter { name, .. } => write!(f, "{name}"),
            GateType::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            GateType::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            GateType::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            GateType::List(element) => write!(f, "List {element}"),
            GateType::Map { key, value } => write!(f, "Map {key} {value}"),
            GateType::Set(element) => write!(f, "Set {element}"),
            GateType::Option(element) => write!(f, "Option {element}"),
            GateType::Result { error, value } => write!(f, "Result {error} {value}"),
            GateType::Validation { error, value } => {
                write!(f, "Validation {error} {value}")
            }
            GateType::Signal(element) => write!(f, "Signal {element}"),
            GateType::Task { error, value } => write!(f, "Task {error} {value}"),
            GateType::Domain {
                name, arguments, ..
            }
            | GateType::OpaqueItem {
                name, arguments, ..
            }
            | GateType::OpaqueImport {
                name, arguments, ..
            } => {
                write!(f, "{name}")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ApplicativeClusterKind {
    List,
    Option,
    Result { error: SourceOptionActualType },
    Validation { error: SourceOptionActualType },
    Signal,
    Task { error: SourceOptionActualType },
}

impl ApplicativeClusterKind {
    pub(crate) fn from_member_actual(
        actual: &SourceOptionActualType,
    ) -> Option<(Self, SourceOptionActualType)> {
        match actual {
            SourceOptionActualType::List(element) => Some((Self::List, element.as_ref().clone())),
            SourceOptionActualType::Option(element) => {
                Some((Self::Option, element.as_ref().clone()))
            }
            SourceOptionActualType::Result { error, value } => Some((
                Self::Result {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Validation { error, value } => Some((
                Self::Validation {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Signal(element) => {
                Some((Self::Signal, element.as_ref().clone()))
            }
            SourceOptionActualType::Task { error, value } => Some((
                Self::Task {
                    error: error.as_ref().clone(),
                },
                value.as_ref().clone(),
            )),
            SourceOptionActualType::Hole
            | SourceOptionActualType::Primitive(_)
            | SourceOptionActualType::Tuple(_)
            | SourceOptionActualType::Record(_)
            | SourceOptionActualType::Arrow { .. }
            | SourceOptionActualType::Map { .. }
            | SourceOptionActualType::Set(_)
            | SourceOptionActualType::Domain { .. }
            | SourceOptionActualType::OpaqueItem { .. }
            | SourceOptionActualType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::List, Self::List) => Some(Self::List),
            (Self::Option, Self::Option) => Some(Self::Option),
            (Self::Signal, Self::Signal) => Some(Self::Signal),
            (Self::Result { error: left }, Self::Result { error: right }) => Some(Self::Result {
                error: left.unify(right)?,
            }),
            (Self::Validation { error: left }, Self::Validation { error: right }) => {
                Some(Self::Validation {
                    error: left.unify(right)?,
                })
            }
            (Self::Task { error: left }, Self::Task { error: right }) => Some(Self::Task {
                error: left.unify(right)?,
            }),
            _ => None,
        }
    }

    pub(crate) fn wrap_actual(&self, payload: SourceOptionActualType) -> SourceOptionActualType {
        match self {
            Self::List => SourceOptionActualType::List(Box::new(payload)),
            Self::Option => SourceOptionActualType::Option(Box::new(payload)),
            Self::Result { error } => SourceOptionActualType::Result {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Validation { error } => SourceOptionActualType::Validation {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
            Self::Signal => SourceOptionActualType::Signal(Box::new(payload)),
            Self::Task { error } => SourceOptionActualType::Task {
                error: Box::new(error.clone()),
                value: Box::new(payload),
            },
        }
    }

    pub(crate) fn surface(&self) -> String {
        match self {
            Self::List => "List _".to_owned(),
            Self::Option => "Option _".to_owned(),
            Self::Result { error } => format!("Result {error} _"),
            Self::Validation { error } => format!("Validation {error} _"),
            Self::Signal => "Signal _".to_owned(),
            Self::Task { error } => format!("Task {error} _"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeBinding {
    Type(GateType),
    Constructor(TypeConstructorBinding),
}

impl TypeBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Type(left), Self::Type(right)) => left.same_shape(right),
            (Self::Constructor(left), Self::Constructor(right)) => left.matches(right),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeConstructorBinding {
    head: TypeConstructorHead,
    arguments: Vec<GateType>,
}

impl TypeConstructorBinding {
    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.head == other.head
            && self.arguments.len() == other.arguments.len()
            && self
                .arguments
                .iter()
                .zip(other.arguments.iter())
                .all(|(left, right)| left.same_shape(right))
    }

    pub fn head(&self) -> TypeConstructorHead {
        self.head
    }

    pub fn arguments(&self) -> &[GateType] {
        &self.arguments
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeConstructorHead {
    Builtin(BuiltinType),
    Item(ItemId),
    Import(ImportId),
}

pub(crate) type PolyTypeBindings = HashMap<TypeParameterId, TypeBinding>;

pub(crate) struct GateTypeContext<'a> {
    module: &'a Module,
    item_types: HashMap<ItemId, Option<GateType>>,
    item_actuals: HashMap<ItemId, Option<SourceOptionActualType>>,
    inferred_function_types: Option<HashMap<ItemId, GateType>>,
    function_call_evidence: Vec<FunctionCallEvidence>,
    function_signature_evidence: Vec<FunctionSignatureEvidence>,
    allow_function_inference: bool,
}

impl<'a> GateTypeContext<'a> {
    pub(crate) fn new(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
            item_actuals: HashMap::new(),
            inferred_function_types: None,
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference: true,
        }
    }

    pub(crate) fn new_for_function_inference(module: &'a Module) -> Self {
        Self {
            module,
            item_types: HashMap::new(),
            item_actuals: HashMap::new(),
            inferred_function_types: Some(HashMap::new()),
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference: false,
        }
    }

    pub(crate) fn with_seeded_item_types(
        module: &'a Module,
        item_types: HashMap<ItemId, GateType>,
        allow_function_inference: bool,
    ) -> Self {
        Self {
            module,
            item_types: item_types
                .into_iter()
                .map(|(item_id, ty)| (item_id, Some(ty)))
                .collect(),
            item_actuals: HashMap::new(),
            inferred_function_types: Some(HashMap::new()),
            function_call_evidence: Vec::new(),
            function_signature_evidence: Vec::new(),
            allow_function_inference,
        }
    }

    fn inferred_function_types(&mut self) -> &HashMap<ItemId, GateType> {
        self.inferred_function_types
            .get_or_insert_with(|| infer_same_module_function_types(self.module))
    }

    pub(crate) fn record_function_call_evidence(&mut self, evidence: FunctionCallEvidence) {
        self.function_call_evidence.push(evidence);
    }

    pub(crate) fn take_function_call_evidence(&mut self) -> Vec<FunctionCallEvidence> {
        std::mem::take(&mut self.function_call_evidence)
    }

    pub(crate) fn record_function_signature_evidence(
        &mut self,
        evidence: FunctionSignatureEvidence,
    ) {
        self.function_signature_evidence.push(evidence);
    }

    pub(crate) fn take_function_signature_evidence(&mut self) -> Vec<FunctionSignatureEvidence> {
        std::mem::take(&mut self.function_signature_evidence)
    }

    pub(crate) fn fanout_carrier(&self, subject: &GateType) -> Option<FanoutCarrier> {
        subject.fanout_carrier()
    }

    pub(crate) fn gate_carrier(&self, subject: &GateType) -> GateCarrier {
        subject.gate_carrier()
    }

    pub(crate) fn truthy_falsy_subject_plan(
        &self,
        subject: &GateType,
    ) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Signal(inner) => Self::truthy_falsy_ordinary_subject_plan(inner),
            other => Self::truthy_falsy_ordinary_subject_plan(other),
        }
    }

    fn arrow_parameter_types(ty: &GateType, arity: usize) -> Option<Vec<GateType>> {
        let mut current = ty;
        let mut parameter_types = Vec::with_capacity(arity);
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameter_types.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some(parameter_types)
    }

    fn arrow_result_type(ty: &GateType, arity: usize) -> Option<GateType> {
        let mut current = ty;
        for _ in 0..arity {
            let GateType::Arrow { result, .. } = current else {
                return None;
            };
            current = result.as_ref();
        }
        Some(current.clone())
    }

    pub(crate) fn truthy_falsy_ordinary_subject_plan(
        subject: &GateType,
    ) -> Option<TruthyFalsySubjectPlan> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::True,
                truthy_payload: None,
                falsy_constructor: BuiltinTerm::False,
                falsy_payload: None,
            }),
            GateType::Option(payload) => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Some,
                truthy_payload: Some(payload.as_ref().clone()),
                falsy_constructor: BuiltinTerm::None,
                falsy_payload: None,
            }),
            GateType::Result { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Ok,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Err,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Validation { error, value } => Some(TruthyFalsySubjectPlan {
                truthy_constructor: BuiltinTerm::Valid,
                truthy_payload: Some(value.as_ref().clone()),
                falsy_constructor: BuiltinTerm::Invalid,
                falsy_payload: Some(error.as_ref().clone()),
            }),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. }
            | GateType::OpaqueItem { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn apply_truthy_falsy_result_type(
        &self,
        subject: &GateType,
        result: GateType,
    ) -> GateType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => GateType::Signal(Box::new(result)),
        }
    }

    pub(crate) fn apply_truthy_falsy_result_actual(
        &self,
        subject: &GateType,
        result: SourceOptionActualType,
    ) -> SourceOptionActualType {
        match self.gate_carrier(subject) {
            GateCarrier::Ordinary => result,
            GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(result)),
        }
    }

    pub(crate) fn case_subject_shape(&mut self, subject: &GateType) -> Option<CaseSubjectShape> {
        match subject {
            GateType::Primitive(BuiltinType::Bool) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::True),
                        display: "True".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::False),
                        display: "False".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Option(payload) => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Some),
                        display: "Some".to_owned(),
                        span: None,
                        field_types: Some(vec![payload.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::None),
                        display: "None".to_owned(),
                        span: None,
                        field_types: Some(Vec::new()),
                    },
                ],
            }),
            GateType::Result { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Ok),
                        display: "Ok".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Err),
                        display: "Err".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::Validation { error, value } => Some(CaseSubjectShape {
                constructors: vec![
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Valid),
                        display: "Valid".to_owned(),
                        span: None,
                        field_types: Some(vec![value.as_ref().clone()]),
                    },
                    CaseConstructorShape {
                        key: CaseConstructorKey::Builtin(BuiltinTerm::Invalid),
                        display: "Invalid".to_owned(),
                        span: None,
                        field_types: Some(vec![error.as_ref().clone()]),
                    },
                ],
            }),
            GateType::OpaqueItem {
                item, arguments, ..
            } => self.same_module_case_subject_shape(*item, arguments),
            GateType::Primitive(_)
            | GateType::TypeParameter { .. }
            | GateType::Tuple(_)
            | GateType::Record(_)
            | GateType::Arrow { .. }
            | GateType::List(_)
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Signal(_)
            | GateType::Task { .. }
            | GateType::Domain { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn same_module_case_subject_shape(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
    ) -> Option<CaseSubjectShape> {
        let Item::Type(item) = &self.module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        if item.parameters.len() != arguments.len() {
            return None;
        }
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let constructors = variants
            .iter()
            .map(|variant| CaseConstructorShape {
                key: CaseConstructorKey::SameModuleVariant {
                    item: item_id,
                    name: variant.name.text().to_owned(),
                },
                display: variant.name.text().to_owned(),
                span: Some(variant.span),
                field_types: self.lower_case_variant_fields(
                    &variant.fields.iter().map(|f| f.ty).collect::<Vec<_>>(),
                    &substitutions,
                ),
            })
            .collect::<Vec<_>>();
        Some(CaseSubjectShape { constructors })
    }

    pub(crate) fn lower_case_variant_fields(
        &mut self,
        fields: &[TypeId],
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<Vec<GateType>> {
        let mut lowered = Vec::with_capacity(fields.len());
        for field in fields {
            lowered.push(self.lower_type(*field, substitutions, &mut Vec::new(), false)?);
        }
        Some(lowered)
    }

    pub(crate) fn case_pattern_coverage(
        &mut self,
        pattern_id: PatternId,
        subject: &CaseSubjectShape,
    ) -> CasePatternCoverage {
        let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
            return CasePatternCoverage::None;
        };
        match pattern.kind {
            PatternKind::Wildcard | PatternKind::Binding(_) => CasePatternCoverage::CatchAll,
            PatternKind::Constructor { callee, .. } | PatternKind::UnresolvedName(callee) => {
                let Some(key) = case_constructor_key(&callee) else {
                    return CasePatternCoverage::None;
                };
                if subject.constructor(&key).is_some() {
                    CasePatternCoverage::Constructor(key)
                } else {
                    CasePatternCoverage::None
                }
            }
            PatternKind::Integer(_)
            | PatternKind::Text(_)
            | PatternKind::Tuple(_)
            | PatternKind::List { .. }
            | PatternKind::Record(_) => CasePatternCoverage::None,
        }
    }

    pub(crate) fn case_pattern_bindings(
        &mut self,
        pattern_id: PatternId,
        subject: &GateType,
    ) -> GateExprEnv {
        let mut env = GateExprEnv::default();
        let mut work = vec![(pattern_id, subject.clone())];
        while let Some((pattern_id, subject_ty)) = work.pop() {
            let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
                continue;
            };
            match pattern.kind {
                PatternKind::Wildcard
                | PatternKind::Integer(_)
                | PatternKind::Text(_)
                | PatternKind::UnresolvedName(_) => {}
                PatternKind::Binding(binding) => {
                    env.locals.insert(binding.binding, subject_ty);
                }
                PatternKind::Tuple(elements) => {
                    let GateType::Tuple(subject_elements) = &subject_ty else {
                        continue;
                    };
                    if elements.len() != subject_elements.len() {
                        continue;
                    }
                    let element_pairs = elements
                        .iter()
                        .zip(subject_elements.iter())
                        .collect::<Vec<_>>();
                    for (element, element_ty) in element_pairs.into_iter().rev() {
                        work.push((*element, element_ty.clone()));
                    }
                }
                PatternKind::List { elements, rest } => {
                    let GateType::List(element_ty) = &subject_ty else {
                        continue;
                    };
                    for element in elements.into_iter().rev() {
                        work.push((element, element_ty.as_ref().clone()));
                    }
                    if let Some(rest) = rest {
                        work.push((rest, subject_ty));
                    }
                }
                PatternKind::Record(fields) => {
                    // Collect (name, type) pairs from either a local record or an imported one.
                    let record_pairs: Option<Vec<(String, GateType)>> = match &subject_ty {
                        GateType::Record(subject_fields) => Some(
                            subject_fields
                                .iter()
                                .map(|f| (f.name.clone(), f.ty.clone()))
                                .collect(),
                        ),
                        GateType::OpaqueImport { import, .. } => {
                            match self.module.imports().get(*import).map(|b| &b.metadata) {
                                Some(ImportBindingMetadata::TypeConstructor {
                                    fields: Some(import_fields),
                                    ..
                                }) => Some(
                                    import_fields
                                        .iter()
                                        .map(|f| {
                                            (
                                                f.name.to_string(),
                                                self.lower_import_value_type(&f.ty),
                                            )
                                        })
                                        .collect(),
                                ),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    let Some(record_pairs) = record_pairs else {
                        continue;
                    };
                    for field in fields.into_iter().rev() {
                        let Some(field_ty) = record_pairs
                            .iter()
                            .find(|(name, _)| name.as_str() == field.label.text())
                            .map(|(_, ty)| ty.clone())
                        else {
                            continue;
                        };
                        work.push((field.pattern, field_ty));
                    }
                }
                PatternKind::Constructor { callee, arguments } => {
                    let Some(field_types) = self.case_pattern_field_types(&callee, &subject_ty)
                    else {
                        continue;
                    };
                    if field_types.len() != arguments.len() {
                        continue;
                    }
                    for (argument, field_ty) in arguments.into_iter().zip(field_types).rev() {
                        work.push((argument, field_ty));
                    }
                }
            }
        }
        env
    }

    pub(crate) fn case_pattern_field_types(
        &mut self,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let key = case_constructor_key(callee)?;
        // For imported constructors, extract field types from the Arrow chain in the import type.
        if let CaseConstructorKey::ImportedVariant { import, .. } = &key {
            return Some(self.import_constructor_field_types(*import));
        }
        let subject = self.case_subject_shape(subject)?;
        subject.constructor(&key)?.field_types.clone()
    }

    fn import_constructor_field_types(&mut self, import_id: ImportId) -> Vec<GateType> {
        let ty = {
            let binding = &self.module.imports()[import_id];
            match &binding.metadata {
                crate::ImportBindingMetadata::Value { ty } => ty.clone(),
                _ => return Vec::new(),
            }
        };
        // Collect all Arrow `parameter` types from the chain; the final `result` is the sum type.
        let mut fields = Vec::new();
        let mut current = &ty;
        loop {
            match current {
                ImportValueType::Arrow { parameter, result } => {
                    fields.push(self.lower_import_value_type(parameter));
                    current = result;
                }
                _ => break,
            }
        }
        fields
    }

    pub(crate) fn apply_fanout_plan(&self, plan: FanoutPlan, subject: GateType) -> GateType {
        match plan.result() {
            FanoutResultKind::MappedCollection => {
                let mapped_collection = GateType::List(Box::new(subject));
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(mapped_collection))
                } else {
                    mapped_collection
                }
            }
            FanoutResultKind::JoinedValue => {
                if plan.lifts_pointwise() {
                    GateType::Signal(Box::new(subject))
                } else {
                    subject
                }
            }
        }
    }

    pub(crate) fn apply_gate_plan(
        &self,
        plan: aivi_typing::GatePlan,
        subject: &GateType,
    ) -> GateType {
        match plan.result() {
            GateResultKind::OptionWrappedSubject => GateType::Option(Box::new(subject.clone())),
            GateResultKind::PreservedSignalSubject => match subject {
                GateType::Signal(_) => subject.clone(),
                other => GateType::Signal(Box::new(other.clone())),
            },
        }
    }

    pub(crate) fn lower_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), false)
    }

    pub(crate) fn lower_open_annotation(&mut self, ty: TypeId) -> Option<GateType> {
        self.lower_type(ty, &HashMap::new(), &mut Vec::new(), true)
    }

    pub(crate) fn lower_hir_type(
        &mut self,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        self.lower_type(ty, substitutions, &mut Vec::new(), false)
    }

    pub(crate) fn poly_type_binding(&mut self, ty: TypeId) -> Option<TypeBinding> {
        if let Some(lowered) = self.lower_annotation(ty) {
            return Some(TypeBinding::Type(lowered));
        }
        let mut item_stack = Vec::new();
        self.partial_type_constructor_binding(ty, &mut item_stack)
            .map(TypeBinding::Constructor)
    }

    pub(crate) fn open_poly_type_binding(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<TypeBinding> {
        self.instantiate_poly_type_binding(ty, bindings)
            .or_else(|| {
                self.lower_open_annotation(ty)
                    .map(TypeBinding::Type)
                    .or_else(|| {
                        let mut item_stack = Vec::new();
                        self.partial_type_constructor_binding(ty, &mut item_stack)
                            .map(TypeBinding::Constructor)
                    })
            })
    }

    pub(crate) fn instantiate_poly_hir_type(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type(ty, bindings, &mut Vec::new())
    }

    pub(crate) fn instantiate_poly_hir_type_partially(
        &mut self,
        ty: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_poly_type_partially(ty, bindings, &mut Vec::new())
    }

    pub(crate) fn lower_poly_type_partially(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return Some(lowered);
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.get(parameter) {
                        Some(TypeBinding::Type(ty)) => Some(ty.clone()),
                        Some(TypeBinding::Constructor(binding)) => {
                            let arity = type_constructor_arity(binding.head, self.module);
                            (binding.arguments.len() == arity)
                                .then(|| {
                                    self.apply_type_constructor(
                                        binding.head,
                                        &binding.arguments,
                                        item_stack,
                                    )
                                })
                                .flatten()
                        }
                        None => Some(GateType::TypeParameter {
                            parameter: *parameter,
                            name: self.module.type_parameters()[*parameter]
                                .name
                                .text()
                                .to_owned(),
                        }),
                    }
                }
                ResolutionState::Resolved(TypeResolution::Builtin(
                    builtin @ (BuiltinType::Int
                    | BuiltinType::Float
                    | BuiltinType::Decimal
                    | BuiltinType::BigInt
                    | BuiltinType::Bool
                    | BuiltinType::Text
                    | BuiltinType::Unit
                    | BuiltinType::Bytes),
                )) => Some(GateType::Primitive(*builtin)),
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    self.lower_type_item(*item_id, &[], item_stack, true)
                }
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type_partially(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type_partially(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => self
                .lower_poly_record_row_transform_partially(
                    transform, *source, bindings, item_stack,
                ),
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(
                    self.lower_poly_type_partially(*parameter, bindings, item_stack)?,
                ),
                result: Box::new(self.lower_poly_type_partially(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(
                                    binding.head,
                                    &all_arguments,
                                    item_stack,
                                )
                            })
                            .flatten()
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() != arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments.push(
                                self.lower_poly_type_partially(*argument, bindings, item_stack)?,
                            );
                        }
                        self.apply_type_constructor(head, &lowered_arguments, item_stack)
                    }
                }
            }
        }
    }

    pub(crate) fn match_poly_hir_type(
        &mut self,
        ty: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        self.match_poly_hir_type_inner(ty, actual, bindings, &mut Vec::new())
    }

    pub(crate) fn match_poly_type_binding(
        &mut self,
        ty: TypeId,
        actual: &TypeBinding,
        bindings: &mut PolyTypeBindings,
    ) -> bool {
        if let Some(candidate) = self.instantiate_poly_type_binding(ty, bindings) {
            return candidate.matches(actual);
        }
        match (&self.module.types()[ty].kind, actual) {
            (TypeKind::Name(reference), _) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            (TypeKind::Apply { callee, arguments }, TypeBinding::Constructor(actual_binding)) => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return false;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Item(_)) => {
                        let Some((head, _)) = self.type_constructor_head_and_arity(*callee) else {
                            return false;
                        };
                        if head != actual_binding.head()
                            || arguments.len() != actual_binding.arguments.len()
                        {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments.iter().zip(actual_binding.arguments.iter()).all(
                            |(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            },
                        )
                    }
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let Some(prefix_len) =
                            actual_binding.arguments.len().checked_sub(arguments.len())
                        else {
                            return false;
                        };
                        let prefix = TypeBinding::Constructor(TypeConstructorBinding {
                            head: actual_binding.head(),
                            arguments: actual_binding.arguments[..prefix_len].to_vec(),
                        });
                        let matches_prefix = match bindings.entry(*parameter) {
                            Entry::Occupied(entry) => entry.get().matches(&prefix),
                            Entry::Vacant(entry) => {
                                entry.insert(prefix);
                                true
                            }
                        };
                        if !matches_prefix {
                            return false;
                        }
                        let mut item_stack = Vec::new();
                        arguments
                            .iter()
                            .zip(actual_binding.arguments[prefix_len..].iter())
                            .all(|(argument, actual_argument)| {
                                self.match_poly_hir_type_inner(
                                    *argument,
                                    actual_argument,
                                    bindings,
                                    &mut item_stack,
                                )
                            })
                    }
                    _ => false,
                }
            }
            (TypeKind::Tuple(_), _)
            | (TypeKind::Record(_), _)
            | (TypeKind::RecordTransform { .. }, _)
            | (TypeKind::Arrow { .. }, _)
            | (TypeKind::Apply { .. }, TypeBinding::Type(_)) => false,
        }
    }

    pub(crate) fn recurrence_target_hint_for_annotation(
        &mut self,
        annotation: TypeId,
    ) -> Option<RecurrenceTargetHint> {
        let ty = self.lower_annotation(annotation)?;
        Some(match ty.recurrence_target_evidence() {
            Some(evidence) => RecurrenceTargetHint::Evidence(evidence),
            None => RecurrenceTargetHint::UnsupportedType {
                ty,
                span: self.module.types()[annotation].span,
            },
        })
    }

    pub(crate) fn item_value_type(&mut self, item_id: ItemId) -> Option<GateType> {
        if let Some(cached) = self.item_types.get(&item_id) {
            return cached.clone();
        }
        self.item_types.insert(item_id, None);
        let ty = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| self.infer_expr(item.body, &GateExprEnv::default(), None).ty),
            Item::Function(item) => {
                let mut env = GateExprEnv::default();
                let mut parameters = Vec::with_capacity(item.parameters.len());
                for parameter in &item.parameters {
                    let parameter_ty = match parameter.annotation {
                        Some(annotation) => self.lower_open_annotation(annotation)?,
                        None => {
                            if !self.allow_function_inference
                                || !supports_same_module_function_inference(item)
                            {
                                return None;
                            }
                            let inferred = self.inferred_function_types().get(&item_id).cloned()?;
                            let parameter_types =
                                Self::arrow_parameter_types(&inferred, item.parameters.len())?;
                            parameter_types.get(parameters.len())?.clone()
                        }
                    };
                    env.locals.insert(parameter.binding, parameter_ty.clone());
                    parameters.push(parameter_ty);
                }
                let result = item
                    .annotation
                    .and_then(|annotation| self.lower_open_annotation(annotation))
                    .or_else(|| {
                        if self.allow_function_inference
                            && supports_same_module_function_inference(item)
                        {
                            let inferred = self.inferred_function_types().get(&item_id).cloned();
                            inferred
                                .and_then(|ty| Self::arrow_result_type(&ty, item.parameters.len()))
                        } else {
                            self.infer_expr(item.body, &env, None).ty
                        }
                    })?;
                let mut ty = result;
                for parameter in parameters.into_iter().rev() {
                    ty = GateType::Arrow {
                        parameter: Box::new(parameter),
                        result: Box::new(ty),
                    };
                }
                Some(ty)
            }
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    let body_ty = self.infer_expr(body, &GateExprEnv::default(), None).ty?;
                    // Signal pipe propagation already wraps the body type in
                    // Signal; avoid double-wrapping Signal(Signal(T)).
                    Some(if matches!(body_ty, GateType::Signal(_)) {
                        body_ty
                    } else {
                        GateType::Signal(Box::new(body_ty))
                    })
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        self.item_types.insert(item_id, ty.clone());
        ty
    }

    pub(crate) fn item_value_actual(&mut self, item_id: ItemId) -> Option<SourceOptionActualType> {
        if let Some(cached) = self.item_actuals.get(&item_id) {
            return cached.clone();
        }
        self.item_actuals.insert(item_id, None);
        let actual = match &self.module.items()[item_id] {
            Item::Value(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    self.infer_expr(item.body, &GateExprEnv::default(), None)
                        .actual()
                }),
            Item::Function(_) => self
                .item_value_type(item_id)
                .map(|ty| SourceOptionActualType::from_gate_type(&ty)),
            Item::Signal(item) => item
                .annotation
                .and_then(|annotation| self.lower_annotation(annotation))
                .map(|ty| SourceOptionActualType::from_gate_type(&ty))
                .or_else(|| {
                    if item.source_metadata.is_some() {
                        return None;
                    }
                    let body = item.body?;
                    let body_actual = self
                        .infer_expr(body, &GateExprEnv::default(), None)
                        .actual()?;
                    // Signal pipe propagation already wraps the body actual in
                    // Signal; avoid double-wrapping Signal(Signal(T)).
                    Some(
                        if matches!(body_actual, SourceOptionActualType::Signal(_)) {
                            body_actual
                        } else {
                            SourceOptionActualType::Signal(Box::new(body_actual))
                        },
                    )
                }),
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        self.item_actuals.insert(item_id, actual.clone());
        actual
    }

    pub(crate) fn finalize_expr_info(&self, mut info: GateExprInfo) -> GateExprInfo {
        if let Some(ty) = info.ty.as_ref() {
            let actual_matches_ty = info
                .actual
                .as_ref()
                .and_then(SourceOptionActualType::to_gate_type)
                .is_some_and(|actual| actual.same_shape(ty));
            if !actual_matches_ty {
                info.actual = Some(SourceOptionActualType::from_gate_type(ty));
            }
        }
        info.contains_signal |= info.ty.as_ref().is_some_and(GateType::is_signal)
            || info
                .actual
                .as_ref()
                .is_some_and(SourceOptionActualType::is_signal);
        info
    }

    pub(crate) fn import_value_type(&self, import_id: ImportId) -> Option<GateType> {
        let import = &self.module.imports()[import_id];
        if let Some(ty) = import.callable_type.as_ref() {
            return Some(self.lower_import_value_type(ty));
        }
        match &import.metadata {
            ImportBindingMetadata::Value { ty }
            | ImportBindingMetadata::IntrinsicValue { ty, .. } => {
                Some(self.lower_import_value_type(ty))
            }
            ImportBindingMetadata::InstanceMember { ty, .. } => {
                Some(self.lower_import_value_type(ty))
            }
            ImportBindingMetadata::TypeConstructor { .. }
            | ImportBindingMetadata::Domain { .. }
            | ImportBindingMetadata::AmbientValue { .. }
            | ImportBindingMetadata::OpaqueValue
            | ImportBindingMetadata::BuiltinType(_)
            | ImportBindingMetadata::BuiltinTerm(_)
            | ImportBindingMetadata::AmbientType
            | ImportBindingMetadata::Bundle(_)
            | ImportBindingMetadata::Unknown => None,
        }
    }

    /// Like `import_value_type`, but also resolves `AmbientValue` imports by
    /// looking up the prelude item they reference.  Used in pipe stage inference
    /// where we need the type of functions like `list.map` (which is registered
    /// as `AmbientValue { name: "__aivi_list_map" }` rather than carrying an
    /// explicit `ImportValueType`).
    pub(crate) fn import_value_type_with_ambient(
        &mut self,
        import_id: ImportId,
    ) -> Option<GateType> {
        if let Some(ty) = self.import_value_type(import_id) {
            return Some(ty);
        }
        // For AmbientValue imports, look up the prelude item by name.
        let ambient_name: String = match &self.module.imports()[import_id].metadata {
            ImportBindingMetadata::AmbientValue { name } => name.to_string(),
            _ => return None,
        };
        let item_id = self
            .module
            .items()
            .iter()
            .find_map(|(id, item)| match item {
                Item::Function(f) if f.name.text() == ambient_name => Some(id),
                Item::Value(v) if v.name.text() == ambient_name => Some(id),
                _ => None,
            })?;
        self.item_value_type(item_id)
    }

    fn imported_type_definition(&self, import: ImportId) -> Option<Box<ImportTypeDefinition>> {
        match &self.module.imports()[import].metadata {
            ImportBindingMetadata::TypeConstructor {
                definition: Some(definition),
                ..
            } => Some(Box::new(definition.clone())),
            _ => None,
        }
    }

    fn opaque_import_type(
        &self,
        import: ImportId,
        name: String,
        arguments: Vec<GateType>,
    ) -> GateType {
        GateType::OpaqueImport {
            import,
            name,
            arguments,
            definition: self.imported_type_definition(import),
        }
    }

    pub(crate) fn lower_import_value_type(&self, ty: &ImportValueType) -> GateType {
        match ty {
            ImportValueType::Primitive(builtin) => GateType::Primitive(*builtin),
            ImportValueType::Tuple(elements) => GateType::Tuple(
                elements
                    .iter()
                    .map(|element| self.lower_import_value_type(element))
                    .collect(),
            ),
            ImportValueType::Record(fields) => GateType::Record(
                fields
                    .iter()
                    .map(|field| GateRecordField {
                        name: field.name.to_string(),
                        ty: self.lower_import_value_type(&field.ty),
                    })
                    .collect(),
            ),
            ImportValueType::Arrow { parameter, result } => GateType::Arrow {
                parameter: Box::new(self.lower_import_value_type(parameter)),
                result: Box::new(self.lower_import_value_type(result)),
            },
            ImportValueType::List(element) => {
                GateType::List(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Map { key, value } => GateType::Map {
                key: Box::new(self.lower_import_value_type(key)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Set(element) => {
                GateType::Set(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Option(element) => {
                GateType::Option(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Result { error, value } => GateType::Result {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Validation { error, value } => GateType::Validation {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::Signal(element) => {
                GateType::Signal(Box::new(self.lower_import_value_type(element)))
            }
            ImportValueType::Task { error, value } => GateType::Task {
                error: Box::new(self.lower_import_value_type(error)),
                value: Box::new(self.lower_import_value_type(value)),
            },
            ImportValueType::TypeVariable { index, name } => GateType::TypeParameter {
                parameter: TypeParameterId::from_raw(u32::MAX - *index as u32),
                name: name.clone(),
            },
            ImportValueType::Named {
                type_name,
                arguments,
                definition,
            } => {
                let lowered_args: Vec<GateType> = arguments
                    .iter()
                    .map(|arg| self.lower_import_value_type(arg))
                    .collect();
                // Find the import that provides this type name.
                let import_id = self
                    .module
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
                if let Some(import) = import_id {
                    self.opaque_import_type(import, type_name.clone(), lowered_args)
                } else {
                    // Fallback: create an opaque import with a sentinel; the type checker
                    // will treat this as an unknown opaque type.
                    GateType::OpaqueImport {
                        import: ImportId::from_raw(u32::MAX),
                        name: type_name.clone(),
                        arguments: lowered_args,
                        definition: definition.clone(),
                    }
                }
            }
        }
    }

    pub(crate) fn intrinsic_value_type(&self, value: IntrinsicValue) -> GateType {
        fn primitive(builtin: BuiltinType) -> GateType {
            GateType::Primitive(builtin)
        }

        fn synthetic_type_parameter(index: usize) -> GateType {
            GateType::TypeParameter {
                parameter: TypeParameterId::from_raw(u32::MAX - index as u32),
                name: format!("T{}", index + 1),
            }
        }

        fn arrow(parameter: GateType, result: GateType) -> GateType {
            GateType::Arrow {
                parameter: Box::new(parameter),
                result: Box::new(result),
            }
        }

        fn task(error: GateType, value: GateType) -> GateType {
            GateType::Task {
                error: Box::new(error),
                value: Box::new(value),
            }
        }

        fn option(element: GateType) -> GateType {
            GateType::Option(Box::new(element))
        }

        fn list(element: GateType) -> GateType {
            GateType::List(Box::new(element))
        }

        fn map(key: GateType, value: GateType) -> GateType {
            GateType::Map {
                key: Box::new(key),
                value: Box::new(value),
            }
        }

        fn record(fields: Vec<(&str, GateType)>) -> GateType {
            GateType::Record(
                fields
                    .into_iter()
                    .map(|(name, ty)| GateRecordField {
                        name: name.to_owned(),
                        ty,
                    })
                    .collect(),
            )
        }

        fn db_connection_type() -> GateType {
            record(vec![("database", primitive(BuiltinType::Text))])
        }

        fn db_param_type() -> GateType {
            record(vec![
                ("kind", primitive(BuiltinType::Text)),
                ("bool", option(primitive(BuiltinType::Bool))),
                ("int", option(primitive(BuiltinType::Int))),
                ("float", option(primitive(BuiltinType::Float))),
                ("decimal", option(primitive(BuiltinType::Decimal))),
                ("bigInt", option(primitive(BuiltinType::BigInt))),
                ("text", option(primitive(BuiltinType::Text))),
                ("bytes", option(primitive(BuiltinType::Bytes))),
            ])
        }

        fn db_statement_type() -> GateType {
            record(vec![
                ("sql", primitive(BuiltinType::Text)),
                ("arguments", list(db_param_type())),
            ])
        }

        fn db_rows_type() -> GateType {
            list(map(
                primitive(BuiltinType::Text),
                primitive(BuiltinType::Text),
            ))
        }

        match value {
            IntrinsicValue::TupleConstructor { arity } => {
                let elements: Vec<_> = (0..arity).map(synthetic_type_parameter).collect();
                let mut ty = GateType::Tuple(elements.clone());
                for element in elements.into_iter().rev() {
                    ty = arrow(element, ty);
                }
                ty
            }
            IntrinsicValue::CustomCapabilityCommand(_) => {
                unreachable!("custom capability commands should type through synthetic imports")
            }
            IntrinsicValue::RandomBytes => arrow(
                primitive(BuiltinType::Int),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::RandomInt => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
                ),
            ),
            IntrinsicValue::StdoutWrite | IntrinsicValue::StderrWrite => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::FsWriteText => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsWriteBytes => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Bytes),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsCreateDirAll | IntrinsicValue::FsDeleteFile => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::DbParamBool => arrow(primitive(BuiltinType::Bool), db_param_type()),
            IntrinsicValue::DbParamInt => arrow(primitive(BuiltinType::Int), db_param_type()),
            IntrinsicValue::DbParamFloat => arrow(primitive(BuiltinType::Float), db_param_type()),
            IntrinsicValue::DbParamDecimal => {
                arrow(primitive(BuiltinType::Decimal), db_param_type())
            }
            IntrinsicValue::DbParamBigInt => arrow(primitive(BuiltinType::BigInt), db_param_type()),
            IntrinsicValue::DbParamText => arrow(primitive(BuiltinType::Text), db_param_type()),
            IntrinsicValue::DbParamBytes => arrow(primitive(BuiltinType::Bytes), db_param_type()),
            IntrinsicValue::DbStatement => arrow(
                primitive(BuiltinType::Text),
                arrow(list(db_param_type()), db_statement_type()),
            ),
            IntrinsicValue::DbQuery => arrow(
                db_connection_type(),
                arrow(
                    db_statement_type(),
                    task(primitive(BuiltinType::Text), db_rows_type()),
                ),
            ),
            IntrinsicValue::DbCommit => arrow(
                db_connection_type(),
                arrow(
                    list(primitive(BuiltinType::Text)),
                    arrow(
                        list(db_statement_type()),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                    ),
                ),
            ),
            IntrinsicValue::FloatFloor
            | IntrinsicValue::FloatCeil
            | IntrinsicValue::FloatRound
            | IntrinsicValue::FloatSqrt
            | IntrinsicValue::FloatAbs => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatToInt => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Int))
            }
            IntrinsicValue::FloatFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatToText => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Text))
            }
            IntrinsicValue::FloatParseText => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Float))),
            ),
            IntrinsicValue::FsReadText => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::FsReadDir => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    GateType::List(Box::new(primitive(BuiltinType::Text))),
                ),
            ),
            IntrinsicValue::FsExists => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::FsReadBytes => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::FsRename => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsCopy => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::FsDeleteDir => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
            ),
            IntrinsicValue::PathParent => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathFilename => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathStem => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathExtension => arrow(
                primitive(BuiltinType::Text),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::PathJoin => arrow(
                primitive(BuiltinType::Text),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::PathIsAbsolute => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bool))
            }
            IntrinsicValue::PathNormalize => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::BytesLength => {
                arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Int))
            }
            IntrinsicValue::BytesGet => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Bytes),
                    GateType::Option(Box::new(primitive(BuiltinType::Int))),
                ),
            ),
            IntrinsicValue::BytesSlice => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Bytes)),
                ),
            ),
            IntrinsicValue::BytesAppend => arrow(
                primitive(BuiltinType::Bytes),
                arrow(primitive(BuiltinType::Bytes), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::BytesFromText => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes))
            }
            IntrinsicValue::BytesToText => arrow(
                primitive(BuiltinType::Bytes),
                GateType::Option(Box::new(primitive(BuiltinType::Text))),
            ),
            IntrinsicValue::BytesRepeat => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::BytesEmpty => primitive(BuiltinType::Bytes),
            IntrinsicValue::JsonValidate => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::JsonGet => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        GateType::Option(Box::new(primitive(BuiltinType::Text))),
                    ),
                ),
            ),
            IntrinsicValue::JsonAt => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Int),
                    task(
                        primitive(BuiltinType::Text),
                        GateType::Option(Box::new(primitive(BuiltinType::Text))),
                    ),
                ),
            ),
            IntrinsicValue::JsonKeys => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    GateType::List(Box::new(primitive(BuiltinType::Text))),
                ),
            ),
            IntrinsicValue::JsonPretty => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::JsonMinify => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::XdgDataHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgConfigHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgCacheHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgStateHome => primitive(BuiltinType::Text),
            IntrinsicValue::XdgRuntimeDir => {
                GateType::Option(Box::new(primitive(BuiltinType::Text)))
            }
            IntrinsicValue::XdgDataDirs => GateType::List(Box::new(primitive(BuiltinType::Text))),
            IntrinsicValue::XdgConfigDirs => GateType::List(Box::new(primitive(BuiltinType::Text))),
            // Text intrinsics
            IntrinsicValue::TextLength | IntrinsicValue::TextByteLen => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Int))
            }
            IntrinsicValue::TextSlice => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Int),
                    arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextFind => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    option(primitive(BuiltinType::Int)),
                ),
            ),
            IntrinsicValue::TextContains
            | IntrinsicValue::TextStartsWith
            | IntrinsicValue::TextEndsWith => arrow(
                primitive(BuiltinType::Text),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::TextToUpper
            | IntrinsicValue::TextToLower
            | IntrinsicValue::TextTrim
            | IntrinsicValue::TextTrimStart
            | IntrinsicValue::TextTrimEnd => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextReplace | IntrinsicValue::TextReplaceAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextSplit => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    list(primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TextRepeat => arrow(
                primitive(BuiltinType::Int),
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::TextFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextParseInt => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::TextFromBool => {
                arrow(primitive(BuiltinType::Bool), primitive(BuiltinType::Text))
            }
            IntrinsicValue::TextParseBool => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::Bool)),
            ),
            IntrinsicValue::TextConcat => arrow(
                list(primitive(BuiltinType::Text)),
                primitive(BuiltinType::Text),
            ),
            // Float transcendental intrinsics
            IntrinsicValue::FloatSin
            | IntrinsicValue::FloatCos
            | IntrinsicValue::FloatTan
            | IntrinsicValue::FloatAtan
            | IntrinsicValue::FloatExp
            | IntrinsicValue::FloatTrunc
            | IntrinsicValue::FloatFrac => {
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float))
            }
            IntrinsicValue::FloatAsin | IntrinsicValue::FloatAcos => arrow(
                primitive(BuiltinType::Float),
                option(primitive(BuiltinType::Float)),
            ),
            IntrinsicValue::FloatLog | IntrinsicValue::FloatLog2 | IntrinsicValue::FloatLog10 => {
                arrow(
                    primitive(BuiltinType::Float),
                    option(primitive(BuiltinType::Float)),
                )
            }
            IntrinsicValue::FloatAtan2 | IntrinsicValue::FloatHypot => arrow(
                primitive(BuiltinType::Float),
                arrow(primitive(BuiltinType::Float), primitive(BuiltinType::Float)),
            ),
            IntrinsicValue::FloatPow => arrow(
                primitive(BuiltinType::Float),
                arrow(
                    primitive(BuiltinType::Float),
                    option(primitive(BuiltinType::Float)),
                ),
            ),
            // Time intrinsics
            IntrinsicValue::TimeNowMs | IntrinsicValue::TimeMonotonicMs => {
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Int))
            }
            IntrinsicValue::TimeFormat => arrow(
                primitive(BuiltinType::Int),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::TimeParse => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
                ),
            ),
            // Env intrinsics
            IntrinsicValue::EnvGet => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    option(primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::EnvList => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    list(GateType::Tuple(vec![
                        primitive(BuiltinType::Text),
                        primitive(BuiltinType::Text),
                    ])),
                ),
            ),
            // Log intrinsics
            IntrinsicValue::LogEmit => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                ),
            ),
            IntrinsicValue::LogEmitContext => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        list(GateType::Tuple(vec![
                            primitive(BuiltinType::Text),
                            primitive(BuiltinType::Text),
                        ])),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Unit)),
                    ),
                ),
            ),
            // Random float intrinsic
            IntrinsicValue::RandomFloat => {
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Float))
            }
            // I18n intrinsics
            IntrinsicValue::I18nTranslate => {
                arrow(primitive(BuiltinType::Text), primitive(BuiltinType::Text))
            }
            IntrinsicValue::I18nTranslatePlural => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(primitive(BuiltinType::Int), primitive(BuiltinType::Text)),
                ),
            ),
            // Regex intrinsics
            IntrinsicValue::RegexIsMatch => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Bool)),
                ),
            ),
            IntrinsicValue::RegexFind => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        option(primitive(BuiltinType::Int)),
                    ),
                ),
            ),
            IntrinsicValue::RegexFindText => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        option(primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::RegexFindAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(
                        primitive(BuiltinType::Text),
                        list(primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::RegexReplace | IntrinsicValue::RegexReplaceAll => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        primitive(BuiltinType::Text),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::HttpGet | IntrinsicValue::HttpDelete => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
            ),
            IntrinsicValue::HttpGetBytes => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Bytes)),
            ),
            IntrinsicValue::HttpGetStatus => arrow(
                primitive(BuiltinType::Text),
                task(primitive(BuiltinType::Text), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::HttpPost | IntrinsicValue::HttpPut => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    arrow(
                        primitive(BuiltinType::Text),
                        task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                    ),
                ),
            ),
            IntrinsicValue::HttpPostJson => arrow(
                primitive(BuiltinType::Text),
                arrow(
                    primitive(BuiltinType::Text),
                    task(primitive(BuiltinType::Text), primitive(BuiltinType::Text)),
                ),
            ),
            IntrinsicValue::HttpHead => arrow(
                primitive(BuiltinType::Text),
                task(
                    primitive(BuiltinType::Text),
                    list(GateType::Tuple(vec![
                        primitive(BuiltinType::Text),
                        primitive(BuiltinType::Text),
                    ])),
                ),
            ),
            IntrinsicValue::BigIntFromInt => {
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::BigInt))
            }
            IntrinsicValue::BigIntFromText => arrow(
                primitive(BuiltinType::Text),
                option(primitive(BuiltinType::BigInt)),
            ),
            IntrinsicValue::BigIntToInt => arrow(
                primitive(BuiltinType::BigInt),
                option(primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::BigIntToText => {
                arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Text))
            }
            IntrinsicValue::BigIntAdd | IntrinsicValue::BigIntSub | IntrinsicValue::BigIntMul => {
                arrow(
                    primitive(BuiltinType::BigInt),
                    arrow(
                        primitive(BuiltinType::BigInt),
                        primitive(BuiltinType::BigInt),
                    ),
                )
            }
            IntrinsicValue::BigIntDiv | IntrinsicValue::BigIntMod => arrow(
                primitive(BuiltinType::BigInt),
                arrow(
                    primitive(BuiltinType::BigInt),
                    option(primitive(BuiltinType::BigInt)),
                ),
            ),
            IntrinsicValue::BigIntPow => arrow(
                primitive(BuiltinType::BigInt),
                arrow(primitive(BuiltinType::Int), primitive(BuiltinType::BigInt)),
            ),
            IntrinsicValue::BigIntNeg | IntrinsicValue::BigIntAbs => arrow(
                primitive(BuiltinType::BigInt),
                primitive(BuiltinType::BigInt),
            ),
            IntrinsicValue::BigIntCmp => arrow(
                primitive(BuiltinType::BigInt),
                arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Int)),
            ),
            IntrinsicValue::BigIntEq | IntrinsicValue::BigIntGt | IntrinsicValue::BigIntLt => {
                arrow(
                    primitive(BuiltinType::BigInt),
                    arrow(primitive(BuiltinType::BigInt), primitive(BuiltinType::Bool)),
                )
            }
        }
    }

    pub(crate) fn domain_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<DomainMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn domain_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.domain_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.domain_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn infer_domain_member_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let candidates = self.domain_member_candidates(reference)?;
        if candidates.len() != 1 {
            return None;
        }
        self.lower_domain_member_annotation(candidates[0], &HashMap::new())
    }

    pub(crate) fn select_domain_member_name(
        &mut self,
        reference: &TermReference,
        expected: &GateType,
    ) -> Option<DomainMemberSelection<GateType>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_name_candidate(resolution, expected)
            }),
        )
    }

    pub(crate) fn select_domain_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<DomainMemberCallMatch>> {
        let candidates = self.domain_member_candidates(reference)?;
        Some(
            self.select_domain_member_candidate(candidates, |this, resolution| {
                this.match_domain_member_call_candidate(resolution, argument_types, expected_result)
            }),
        )
    }

    pub(crate) fn class_member_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<ClassMemberResolution>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::ClassMember(resolution)) => {
                Some(vec![*resolution])
            }
            ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Item(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
            | ResolutionState::Resolved(TermResolution::Builtin(_)) => None,
        }
    }

    pub(crate) fn class_member_candidate_labels(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<String>> {
        self.class_member_candidates(reference).map(|candidates| {
            candidates
                .into_iter()
                .filter_map(|candidate| self.class_member_label(candidate))
                .collect()
        })
    }

    pub(crate) fn hoisted_import_candidates(
        &self,
        reference: &TermReference,
    ) -> Option<Vec<ImportId>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(candidates)) => {
                Some(candidates.iter().copied().collect())
            }
            _ => None,
        }
    }

    /// Try to select a unique hoisted import candidate by comparing each
    /// candidate's value type against `expected`.  Returns the resolved
    /// `GateRuntimeReference::Import` on success, or the diagnostic
    /// context on failure.
    pub(crate) fn select_hoisted_import(
        &mut self,
        reference: &TermReference,
        expected: Option<&GateType>,
    ) -> Option<ImportId> {
        let candidates = self.hoisted_import_candidates(reference)?;
        let mut matches = Vec::new();
        for import_id in candidates {
            if let Some(ty) = self.import_value_type_with_ambient(import_id) {
                let fits = match expected {
                    Some(expected) => expected.fits_template(&ty),
                    None => true,
                };
                if fits {
                    matches.push(import_id);
                }
            }
        }
        match matches.len() {
            1 => matches.pop(),
            _ => None,
        }
    }

    pub(crate) fn select_class_member_call(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberSelection<ClassMemberCallMatch>> {
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) =
                self.match_class_member_call_candidate(candidate, argument_types, expected_result)
            {
                matches.push(matched);
            }
        }
        Some(match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one class member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        })
    }

    pub(crate) fn match_class_member_call_candidate(
        &mut self,
        resolution: ClassMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<ClassMemberCallMatch> {
        let (class_parameter, member_annotation, member_context) =
            self.class_member_signature(resolution)?;
        let mut bindings = PolyTypeBindings::new();
        let mut current = member_annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            if !self.match_poly_hir_type(parameter, argument, &mut bindings) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result
            && !self.match_poly_hir_type(current, expected, &mut bindings)
        {
            return None;
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            parameters.push(self.instantiate_poly_hir_type(parameter, &bindings)?);
        }
        let result = self.instantiate_poly_hir_type(current, &bindings)?;
        if let Some(expected) = expected_result
            && !result.same_shape(expected)
        {
            return None;
        }

        let evidence = ClassConstraintBinding {
            class_item: resolution.class,
            subject: bindings.get(&class_parameter)?.clone(),
        };
        let constraints = member_context
            .iter()
            .map(|constraint| self.class_constraint_binding(*constraint, &bindings))
            .collect::<Option<Vec<_>>>()?;
        Some(ClassMemberCallMatch {
            resolution,
            parameters,
            result,
            evidence,
            constraints,
        })
    }

    pub(crate) fn class_member_signature(
        &self,
        resolution: ClassMemberResolution,
    ) -> Option<(TypeParameterId, TypeId, Vec<TypeId>)> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        let context = class_item
            .superclasses
            .iter()
            .chain(class_item.param_constraints.iter())
            .chain(member.context.iter())
            .copied()
            .collect();
        Some((*class_item.parameters.first(), member.annotation, context))
    }

    pub(crate) fn class_member_label(&self, resolution: ClassMemberResolution) -> Option<String> {
        let Item::Class(class_item) = &self.module.items()[resolution.class] else {
            return None;
        };
        let member = class_item.members.get(resolution.member_index)?;
        Some(format!("{}.{}", class_item.name.text(), member.name.text()))
    }

    pub(crate) fn class_constraint_binding(
        &mut self,
        constraint: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<ClassConstraintBinding> {
        let (class_item, subject) = self.class_constraint_parts(constraint)?;
        Some(ClassConstraintBinding {
            class_item,
            subject: self.instantiate_poly_type_binding(subject, bindings)?,
        })
    }

    pub(crate) fn open_class_constraint_binding(
        &mut self,
        constraint: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<ClassConstraintBinding> {
        let (class_item, subject) = self.class_constraint_parts(constraint)?;
        Some(ClassConstraintBinding {
            class_item,
            subject: self.open_poly_type_binding(subject, bindings)?,
        })
    }

    pub(crate) fn class_constraint_parts(&self, constraint: TypeId) -> Option<(ItemId, TypeId)> {
        let ty = self.module.types().get(constraint)?;
        match &ty.kind {
            TypeKind::Apply { callee, arguments } if arguments.len() == 1 => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
                    reference.resolution.as_ref()
                else {
                    return None;
                };
                matches!(self.module.items()[*item_id], Item::Class(_))
                    .then_some((*item_id, *arguments.first()))
            }
            _ => None,
        }
    }

    pub(crate) fn select_domain_member_candidate<T>(
        &mut self,
        candidates: Vec<DomainMemberResolution>,
        mut matcher: impl FnMut(&mut Self, DomainMemberResolution) -> Option<T>,
    ) -> DomainMemberSelection<T> {
        let mut matches = Vec::new();
        for candidate in candidates {
            if let Some(matched) = matcher(self, candidate) {
                matches.push(matched);
            }
        }
        match matches.len() {
            0 => DomainMemberSelection::NoMatch,
            1 => DomainMemberSelection::Unique(
                matches
                    .pop()
                    .expect("exactly one domain member match should be available"),
            ),
            _ => DomainMemberSelection::Ambiguous,
        }
    }

    pub(crate) fn match_domain_member_name_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        expected: &GateType,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut item_stack = Vec::new();
        if !self.match_hir_type(annotation, expected, &mut substitutions, &mut item_stack) {
            return None;
        }
        let lowered = self.lower_domain_member_annotation(resolution, &substitutions)?;
        lowered.same_shape(expected).then_some(lowered)
    }

    pub(crate) fn match_domain_member_call_candidate(
        &mut self,
        resolution: DomainMemberResolution,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<DomainMemberCallMatch> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut substitutions = HashMap::new();
        let mut current = annotation;
        let mut parameter_type_ids = Vec::with_capacity(argument_types.len());
        for argument in argument_types {
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                return None;
            };
            let mut item_stack = Vec::new();
            if !self.match_hir_type(parameter, argument, &mut substitutions, &mut item_stack) {
                return None;
            }
            parameter_type_ids.push(parameter);
            current = result;
        }
        if let Some(expected) = expected_result {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(current, expected, &mut substitutions, &mut item_stack) {
                return None;
            }
        }

        let mut parameters = Vec::with_capacity(parameter_type_ids.len());
        for parameter in parameter_type_ids {
            let mut item_stack = Vec::new();
            parameters.push(self.lower_type(parameter, &substitutions, &mut item_stack, false)?);
        }
        let mut item_stack = Vec::new();
        let result = self.lower_type(current, &substitutions, &mut item_stack, false)?;
        if let Some(expected) = expected_result {
            if !result.same_shape(expected) {
                return None;
            }
        }
        Some(DomainMemberCallMatch { parameters, result })
    }

    pub(crate) fn match_hir_type(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_type(type_id, substitutions, item_stack, false) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    match substitutions.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().same_shape(actual),
                        Entry::Vacant(entry) => {
                            entry.insert(actual.clone());
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_hir_type(*element, actual, substitutions, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_hir_type(field.ty, &actual_field.ty, substitutions, item_stack)
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_hir_type(parameter, actual_parameter, substitutions, item_stack)
                    && self.match_hir_type(result, actual_result, substitutions, item_stack)
            }
            TypeKind::Apply { callee, arguments } => self.match_hir_type_application(
                callee,
                &arguments,
                actual,
                substitutions,
                item_stack,
            ),
            TypeKind::RecordTransform { .. } => false,
        }
    }

    pub(crate) fn match_hir_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        substitutions: &mut HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        let arguments = arguments.iter().copied().collect::<Vec<_>>();
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                let GateType::List(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                let GateType::Map { key, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], key, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                let GateType::Set(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                let GateType::Option(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                let GateType::Result { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                let GateType::Validation { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                let GateType::Signal(element) = actual else {
                    return false;
                };
                arguments.len() == 1
                    && self.match_hir_type(arguments[0], element, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                let GateType::Task { error, value } = actual else {
                    return false;
                };
                arguments.len() == 2
                    && self.match_hir_type(arguments[0], error, substitutions, item_stack)
                    && self.match_hir_type(arguments[1], value, substitutions, item_stack)
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => match actual {
                GateType::Domain {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                GateType::OpaqueItem {
                    item,
                    arguments: actual_arguments,
                    ..
                } if *item == *item_id && arguments.len() == actual_arguments.len() => arguments
                    .iter()
                    .zip(actual_arguments.iter())
                    .all(|(argument, actual)| {
                        self.match_hir_type(*argument, actual, substitutions, item_stack)
                    }),
                _ => false,
            },
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => match actual {
                GateType::OpaqueImport {
                    import,
                    arguments: actual_arguments,
                    ..
                } if *import == *import_id && arguments.len() == actual_arguments.len() => {
                    arguments
                        .iter()
                        .zip(actual_arguments.iter())
                        .all(|(argument, actual)| {
                            self.match_hir_type(*argument, actual, substitutions, item_stack)
                        })
                }
                _ => false,
            },
            ResolutionState::Unresolved
            | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_)) => false,
        }
    }

    pub(crate) fn lower_domain_member_annotation(
        &mut self,
        resolution: DomainMemberResolution,
        substitutions: &HashMap<TypeParameterId, GateType>,
    ) -> Option<GateType> {
        let annotation = self.domain_member_annotation(resolution)?;
        let mut item_stack = Vec::new();
        self.lower_type(annotation, substitutions, &mut item_stack, false)
    }

    pub(crate) fn domain_member_annotation(
        &self,
        resolution: DomainMemberResolution,
    ) -> Option<TypeId> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        domain
            .members
            .get(resolution.member_index)
            .filter(|member| member.kind == DomainMemberKind::Method)
            .map(|member| member.annotation)
    }

    pub(crate) fn domain_member_label(&self, resolution: DomainMemberResolution) -> Option<String> {
        let Item::Domain(domain) = &self.module.items()[resolution.domain] else {
            return None;
        };
        let member = domain.members.get(resolution.member_index)?;
        Some(format!("{}.{}", domain.name.text(), member.name.text()))
    }

    pub(crate) fn lower_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => self.lower_type_reference(
                reference,
                substitutions,
                item_stack,
                allow_open_type_parameters,
            ),
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_type(
                        *element,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_type(
                            field.ty,
                            substitutions,
                            item_stack,
                            allow_open_type_parameters,
                        )?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => self.lower_record_row_transform(
                transform,
                *source,
                substitutions,
                item_stack,
                allow_open_type_parameters,
            ),
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_type(
                    *parameter,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
                result: Box::new(self.lower_type(
                    *result,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        substitutions,
                        item_stack,
                        allow_open_type_parameters,
                    )?);
                }
                self.lower_type_application(
                    *callee,
                    &lowered_arguments,
                    substitutions,
                    item_stack,
                    allow_open_type_parameters,
                )
            }
        }
    }

    pub(crate) fn lower_record_row_transform(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let source = self.lower_type(
            source,
            substitutions,
            item_stack,
            allow_open_type_parameters,
        )?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn apply_record_row_transform(
        &self,
        transform: &crate::RecordRowTransform,
        source: &GateType,
    ) -> Option<GateType> {
        let GateType::Record(fields) = source else {
            return None;
        };
        let field_index = fields
            .iter()
            .enumerate()
            .map(|(index, field)| (field.name.as_str(), index))
            .collect::<HashMap<_, _>>();
        match transform {
            crate::RecordRowTransform::Pick(labels) => labels
                .iter()
                .map(|label| fields.get(*field_index.get(label.text())?).cloned())
                .collect::<Option<Vec<_>>>()
                .map(GateType::Record),
            crate::RecordRowTransform::Omit(labels) => {
                let omitted = labels
                    .iter()
                    .map(|label| field_index.get(label.text()).copied())
                    .collect::<Option<HashSet<_>>>()?;
                Some(GateType::Record(
                    fields
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !omitted.contains(index))
                        .map(|(_, field)| field.clone())
                        .collect(),
                ))
            }
            crate::RecordRowTransform::Optional(labels)
            | crate::RecordRowTransform::Defaulted(labels) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        if labels.iter().any(|label| label.text() == field.name) {
                            GateRecordField {
                                name: field.name.clone(),
                                ty: match &field.ty {
                                    GateType::Option(_) => field.ty.clone(),
                                    other => GateType::Option(Box::new(other.clone())),
                                },
                            }
                        } else {
                            field.clone()
                        }
                    })
                    .collect(),
            )),
            crate::RecordRowTransform::Required(labels) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        if labels.iter().any(|label| label.text() == field.name) {
                            GateRecordField {
                                name: field.name.clone(),
                                ty: match &field.ty {
                                    GateType::Option(inner) => inner.as_ref().clone(),
                                    other => other.clone(),
                                },
                            }
                        } else {
                            field.clone()
                        }
                    })
                    .collect(),
            )),
            crate::RecordRowTransform::Rename(renames) => {
                let renamed = renames
                    .iter()
                    .map(|rename| Some((field_index.get(rename.from.text()).copied()?, rename)))
                    .collect::<Option<HashMap<_, _>>>()?;
                let mut result = Vec::with_capacity(fields.len());
                let mut seen = HashSet::with_capacity(fields.len());
                for (index, field) in fields.iter().enumerate() {
                    let name = renamed
                        .get(&index)
                        .map(|rename| rename.to.text().to_owned())
                        .unwrap_or_else(|| field.name.clone());
                    if !seen.insert(name.clone()) {
                        return None;
                    }
                    result.push(GateRecordField {
                        name,
                        ty: field.ty.clone(),
                    });
                }
                Some(GateType::Record(result))
            }
        }
    }

    pub(crate) fn lower_type_reference(
        &mut self,
        reference: &TypeReference,
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => None,
            ResolutionState::Resolved(TypeResolution::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            )) => Some(GateType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::Builtin(_)) => None,
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned().or_else(|| {
                    allow_open_type_parameters.then(|| GateType::TypeParameter {
                        parameter: *parameter,
                        name: self.module.type_parameters()[*parameter]
                            .name
                            .text()
                            .to_owned(),
                    })
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, allow_open_type_parameters)
            }
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                let name = self.module.imports()[*import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.opaque_import_type(*import_id, name, Vec::new()))
            }
        }
    }

    pub(crate) fn lower_type_application(
        &mut self,
        callee: TypeId,
        arguments: &[GateType],
        substitutions: &HashMap<TypeParameterId, GateType>,
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                Some(GateType::List(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map)) => {
                Some(GateType::Map {
                    key: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set)) => {
                Some(GateType::Set(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option)) => {
                Some(GateType::Option(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result)) => {
                Some(GateType::Result {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation)) => {
                Some(GateType::Validation {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                Some(GateType::Signal(Box::new(arguments.first()?.clone())))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Task)) => {
                Some(GateType::Task {
                    error: Box::new(arguments.first()?.clone()),
                    value: Box::new(arguments.get(1)?.clone()),
                })
            }
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, arguments, item_stack, allow_open_type_parameters)
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                substitutions.get(parameter).cloned()
            }
            ResolutionState::Resolved(TypeResolution::Import(import_id)) => {
                let name = self.module.imports()[*import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.opaque_import_type(*import_id, name, arguments.to_vec()))
            }
            ResolutionState::Resolved(TypeResolution::Builtin(
                BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes,
            ))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn lower_type_item(
        &mut self,
        item_id: ItemId,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
        allow_open_type_parameters: bool,
    ) -> Option<GateType> {
        let item = &self.module.items()[item_id];
        let name = item_type_name(item);
        if item_stack.contains(&item_id) {
            return Some(GateType::OpaqueItem {
                item: item_id,
                name,
                arguments: arguments.to_vec(),
            });
        }
        item_stack.push(item_id);
        let lowered = match item {
            Item::Type(item) => {
                if item.parameters.len() != arguments.len() {
                    None
                } else {
                    match &item.body {
                        crate::hir::TypeItemBody::Alias(alias) => {
                            let substitutions = item
                                .parameters
                                .iter()
                                .copied()
                                .zip(arguments.iter().cloned())
                                .collect::<HashMap<_, _>>();
                            self.lower_type(
                                *alias,
                                &substitutions,
                                item_stack,
                                allow_open_type_parameters,
                            )
                        }
                        crate::hir::TypeItemBody::Sum(_) => Some(GateType::OpaqueItem {
                            item: item_id,
                            name: item.name.text().to_owned(),
                            arguments: arguments.to_vec(),
                        }),
                    }
                }
            }
            Item::Domain(item) => Some(GateType::Domain {
                item: item_id,
                name: item.name.text().to_owned(),
                arguments: arguments.to_vec(),
            }),
            Item::Class(_)
            | Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => None,
        };
        let popped = item_stack.pop();
        debug_assert_eq!(popped, Some(item_id));
        lowered
    }

    pub(crate) fn lower_poly_type(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => {
                self.lower_poly_type_reference(reference, bindings, item_stack)
            }
            TypeKind::Tuple(elements) => {
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    lowered.push(self.lower_poly_type(*element, bindings, item_stack)?);
                }
                Some(GateType::Tuple(lowered))
            }
            TypeKind::Record(fields) => {
                let mut lowered = Vec::with_capacity(fields.len());
                for field in fields {
                    lowered.push(GateRecordField {
                        name: field.label.text().to_owned(),
                        ty: self.lower_poly_type(field.ty, bindings, item_stack)?,
                    });
                }
                Some(GateType::Record(lowered))
            }
            TypeKind::RecordTransform { transform, source } => {
                self.lower_poly_record_row_transform(transform, *source, bindings, item_stack)
            }
            TypeKind::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(self.lower_poly_type(*parameter, bindings, item_stack)?),
                result: Box::new(self.lower_poly_type(*result, bindings, item_stack)?),
            }),
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        self.apply_type_constructor(binding.head, &all_arguments, item_stack)
                    }
                    _ => {
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        (lowered_arguments.len() == arity)
                            .then(|| {
                                self.apply_type_constructor(head, &lowered_arguments, item_stack)
                            })
                            .flatten()
                    }
                }
            }
        }
    }

    pub(crate) fn lower_poly_record_row_transform(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let source = self.lower_poly_type(source, bindings, item_stack)?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn lower_poly_record_row_transform_partially(
        &mut self,
        transform: &crate::RecordRowTransform,
        source: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        let source = self.lower_poly_type_partially(source, bindings, item_stack)?;
        self.apply_record_row_transform(transform, &source)
    }

    pub(crate) fn lower_poly_type_reference(
        &mut self,
        reference: &TypeReference,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                match bindings.get(parameter)? {
                    TypeBinding::Type(ty) => Some(ty.clone()),
                    TypeBinding::Constructor(binding) => {
                        self.apply_type_constructor(binding.head, &binding.arguments, item_stack)
                    }
                }
            }
            ResolutionState::Resolved(TypeResolution::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            )) => Some(GateType::Primitive(*builtin)),
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                self.lower_type_item(*item_id, &[], item_stack, false)
            }
            ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Resolved(TypeResolution::Builtin(_))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn instantiate_poly_type_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<TypeBinding> {
        let mut item_stack = Vec::new();
        if let Some(ty) = self.lower_poly_type(type_id, bindings, &mut item_stack) {
            return Some(TypeBinding::Type(ty));
        }
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    bindings.get(parameter).cloned()
                }
                _ => self
                    .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                    .map(TypeBinding::Constructor),
            },
            TypeKind::Apply { .. } => self
                .partial_poly_type_constructor_binding(type_id, bindings, &mut item_stack)
                .map(TypeBinding::Constructor),
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn partial_poly_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        bindings: &PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                        return None;
                    };
                    Some(binding.clone())
                }
                _ => {
                    let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                    (arity > 0).then_some(TypeConstructorBinding {
                        head,
                        arguments: Vec::new(),
                    })
                }
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                        let TypeBinding::Constructor(binding) = bindings.get(parameter)? else {
                            return None;
                        };
                        let mut all_arguments =
                            Vec::with_capacity(binding.arguments.len() + arguments.len());
                        all_arguments.extend(binding.arguments.iter().cloned());
                        for argument in arguments.iter() {
                            all_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        let arity = type_constructor_arity(binding.head, self.module);
                        (all_arguments.len() < arity).then_some(TypeConstructorBinding {
                            head: binding.head,
                            arguments: all_arguments,
                        })
                    }
                    _ => {
                        let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                        if arguments.len() >= arity {
                            return None;
                        }
                        let mut lowered_arguments = Vec::with_capacity(arguments.len());
                        for argument in arguments.iter() {
                            lowered_arguments
                                .push(self.lower_poly_type(*argument, bindings, item_stack)?);
                        }
                        Some(TypeConstructorBinding {
                            head,
                            arguments: lowered_arguments,
                        })
                    }
                }
            }
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn match_poly_hir_type_inner(
        &mut self,
        type_id: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        if let Some(lowered) = self.lower_poly_type(type_id, bindings, item_stack) {
            return lowered.same_shape(actual);
        }
        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let candidate = TypeBinding::Type(actual.clone());
                    match bindings.entry(*parameter) {
                        Entry::Occupied(entry) => entry.get().matches(&candidate),
                        Entry::Vacant(entry) => {
                            entry.insert(candidate);
                            true
                        }
                    }
                }
                _ => false,
            },
            TypeKind::Tuple(elements) => {
                let GateType::Tuple(actual_elements) = actual else {
                    return false;
                };
                elements.len() == actual_elements.len()
                    && elements
                        .iter()
                        .zip(actual_elements.iter())
                        .all(|(element, actual)| {
                            self.match_poly_hir_type_inner(*element, actual, bindings, item_stack)
                        })
            }
            TypeKind::Record(fields) => {
                let GateType::Record(actual_fields) = actual else {
                    return false;
                };
                fields.len() == actual_fields.len()
                    && fields.iter().all(|field| {
                        let Some(actual_field) = actual_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                        else {
                            return false;
                        };
                        self.match_poly_hir_type_inner(
                            field.ty,
                            &actual_field.ty,
                            bindings,
                            item_stack,
                        )
                    })
            }
            TypeKind::Arrow { parameter, result } => {
                let GateType::Arrow {
                    parameter: actual_parameter,
                    result: actual_result,
                } = actual
                else {
                    return false;
                };
                self.match_poly_hir_type_inner(parameter, actual_parameter, bindings, item_stack)
                    && self.match_poly_hir_type_inner(result, actual_result, bindings, item_stack)
            }
            TypeKind::Apply { callee, arguments } => {
                self.match_poly_type_application(callee, &arguments, actual, bindings, item_stack)
            }
            TypeKind::RecordTransform { .. } => false,
        }
    }

    pub(crate) fn match_poly_type_application(
        &mut self,
        callee: TypeId,
        arguments: &crate::NonEmpty<TypeId>,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
        item_stack: &mut Vec<ItemId>,
    ) -> bool {
        let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
            return false;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                let Some((head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                let pattern_arguments = arguments.iter().copied().collect::<Vec<_>>();
                if actual_arguments.len() < pattern_arguments.len() {
                    return false;
                }
                let prefix_count = actual_arguments.len() - pattern_arguments.len();
                let candidate = TypeBinding::Constructor(TypeConstructorBinding {
                    head,
                    arguments: actual_arguments[..prefix_count].to_vec(),
                });
                match bindings.entry(*parameter) {
                    Entry::Occupied(entry) if !entry.get().matches(&candidate) => return false,
                    Entry::Occupied(_) => {}
                    Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                }
                pattern_arguments
                    .iter()
                    .zip(actual_arguments[prefix_count..].iter())
                    .all(|(argument, actual_argument)| {
                        self.match_poly_hir_type_inner(
                            *argument,
                            actual_argument,
                            bindings,
                            item_stack,
                        )
                    })
            }
            _ => {
                let Some((expected_head, _)) = self.type_constructor_head_and_arity(callee) else {
                    return false;
                };
                let Some((actual_head, actual_arguments)) = actual.constructor_view() else {
                    return false;
                };
                let heads_match = expected_head == actual_head
                    || self.constructor_heads_same_name(&expected_head, actual);
                heads_match
                    && actual_arguments.len() >= arguments.len()
                    && arguments.iter().zip(actual_arguments.iter()).all(
                        |(argument, actual_argument)| {
                            self.match_poly_hir_type_inner(
                                *argument,
                                actual_argument,
                                bindings,
                                item_stack,
                            )
                        },
                    )
            }
        }
    }

    pub(crate) fn partial_type_constructor_binding(
        &mut self,
        type_id: TypeId,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<TypeConstructorBinding> {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(_) => {
                let (head, arity) = self.type_constructor_head_and_arity(type_id)?;
                (arity > 0).then_some(TypeConstructorBinding {
                    head,
                    arguments: Vec::new(),
                })
            }
            TypeKind::Apply { callee, arguments } => {
                let (head, arity) = self.type_constructor_head_and_arity(*callee)?;
                if arguments.len() >= arity {
                    return None;
                }
                let mut lowered_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter() {
                    lowered_arguments.push(self.lower_type(
                        *argument,
                        &HashMap::new(),
                        item_stack,
                        false,
                    )?);
                }
                Some(TypeConstructorBinding {
                    head,
                    arguments: lowered_arguments,
                })
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::Arrow { .. } => None,
            TypeKind::RecordTransform { .. } => None,
        }
    }

    pub(crate) fn type_constructor_head_and_arity(
        &self,
        type_id: TypeId,
    ) -> Option<(TypeConstructorHead, usize)> {
        let TypeKind::Name(reference) = &self.module.types()[type_id].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TypeResolution::Builtin(builtin)) => Some((
                TypeConstructorHead::Builtin(*builtin),
                builtin_type_arity(*builtin),
            )),
            ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                let arity = match &self.module.items()[*item_id] {
                    Item::Type(item) => item.parameters.len(),
                    Item::Domain(item) => item.parameters.len(),
                    _ => return None,
                };
                Some((TypeConstructorHead::Item(*item_id), arity))
            }
            ResolutionState::Resolved(TypeResolution::TypeParameter(_))
            | ResolutionState::Resolved(TypeResolution::Import(_))
            | ResolutionState::Unresolved => None,
        }
    }

    pub(crate) fn apply_type_constructor(
        &mut self,
        head: TypeConstructorHead,
        arguments: &[GateType],
        item_stack: &mut Vec<ItemId>,
    ) -> Option<GateType> {
        match head {
            TypeConstructorHead::Builtin(builtin) => {
                self.apply_builtin_type_constructor(builtin, arguments)
            }
            TypeConstructorHead::Item(item_id) => {
                self.lower_type_item(item_id, arguments, item_stack, false)
            }
            TypeConstructorHead::Import(import_id) => {
                let name = self.module.imports()[import_id]
                    .local_name
                    .text()
                    .to_owned();
                Some(self.opaque_import_type(import_id, name, arguments.to_vec()))
            }
        }
    }

    /// Check whether an expected TypeConstructorHead matches an actual GateType
    /// by canonical name, handling cross-variant cases (Item vs Import).
    fn constructor_heads_same_name(
        &self,
        expected: &TypeConstructorHead,
        actual: &GateType,
    ) -> bool {
        let expected_name = match expected {
            TypeConstructorHead::Item(item_id) => {
                Some(item_type_name(&self.module.items()[*item_id]))
            }
            TypeConstructorHead::Import(import_id) => self
                .module
                .imports()
                .get(*import_id)
                .map(|imp| imp.imported_name.text().to_owned()),
            TypeConstructorHead::Builtin(_) => None,
        };
        let actual_name = actual.named_type_parts().map(|(n, _)| n);
        match (expected_name, actual_name) {
            (Some(en), Some(an)) => en == an,
            _ => false,
        }
    }

    pub(crate) fn apply_builtin_type_constructor(
        &self,
        builtin: BuiltinType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        if arguments.len() != builtin_type_arity(builtin) {
            return None;
        }
        match builtin {
            BuiltinType::Int
            | BuiltinType::Float
            | BuiltinType::Decimal
            | BuiltinType::BigInt
            | BuiltinType::Bool
            | BuiltinType::Text
            | BuiltinType::Unit
            | BuiltinType::Bytes => Some(GateType::Primitive(builtin)),
            BuiltinType::List => Some(GateType::List(Box::new(arguments.first()?.clone()))),
            BuiltinType::Map => Some(GateType::Map {
                key: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Set => Some(GateType::Set(Box::new(arguments.first()?.clone()))),
            BuiltinType::Option => Some(GateType::Option(Box::new(arguments.first()?.clone()))),
            BuiltinType::Result => Some(GateType::Result {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Validation => Some(GateType::Validation {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
            BuiltinType::Signal => Some(GateType::Signal(Box::new(arguments.first()?.clone()))),
            BuiltinType::Task => Some(GateType::Task {
                error: Box::new(arguments.first()?.clone()),
                value: Box::new(arguments.get(1)?.clone()),
            }),
        }
    }

    pub(crate) fn infer_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> GateExprInfo {
        let expr = self.module.exprs()[expr_id].clone();
        let info = match expr.kind {
            ExprKind::Name(reference) => self.infer_name(&reference, env),
            ExprKind::Integer(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Int)),
                ..GateExprInfo::default()
            },
            ExprKind::Float(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Float)),
                ..GateExprInfo::default()
            },
            ExprKind::Decimal(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Decimal)),
                ..GateExprInfo::default()
            },
            ExprKind::BigInt(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::BigInt)),
                ..GateExprInfo::default()
            },
            ExprKind::SuffixedInteger(literal) => GateExprInfo {
                ty: match literal.resolution.as_ref() {
                    ResolutionState::Resolved(resolution) => {
                        let domain = &self.module.items()[resolution.domain];
                        Some(GateType::Domain {
                            item: resolution.domain,
                            name: item_type_name(domain),
                            arguments: Vec::new(),
                        })
                    }
                    ResolutionState::Unresolved => None,
                },
                ..GateExprInfo::default()
            },
            ExprKind::Text(text) => {
                let mut info = GateExprInfo {
                    ty: Some(GateType::Primitive(BuiltinType::Text)),
                    ..GateExprInfo::default()
                };
                for segment in text.segments {
                    if let TextSegment::Interpolation(interpolation) = segment {
                        info.merge(self.infer_expr(interpolation.expr, env, ambient));
                    }
                }
                info
            }
            ExprKind::Regex(_) => GateExprInfo {
                ty: Some(GateType::Primitive(BuiltinType::Text)),
                ..GateExprInfo::default()
            },
            ExprKind::Tuple(elements) => {
                let mut info = GateExprInfo::default();
                let mut lowered = Vec::with_capacity(elements.len());
                for element in elements.iter() {
                    let child = self.infer_expr(*element, env, ambient);
                    if let Some(ty) = child.actual() {
                        lowered.push(ty);
                    }
                    info.merge(child);
                }
                if lowered.len() == elements.len() {
                    info.set_actual(SourceOptionActualType::Tuple(lowered));
                }
                info
            }
            ExprKind::List(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut element_gate_type = None::<GateType>;
                let mut consistent = true;
                for element in &elements {
                    let child = self.infer_expr(*element, env, ambient);
                    if consistent {
                        if let Some(child_ty) = child.actual_gate_type().or(child.ty.clone()) {
                            element_gate_type = match element_gate_type.take() {
                                None => Some(child_ty),
                                Some(current) => {
                                    if current.same_shape(&child_ty) {
                                        Some(current)
                                    } else {
                                        consistent = false;
                                        None
                                    }
                                }
                            };
                        }
                    }
                    if consistent {
                        if let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    } else {
                        let _ = child.actual();
                    }
                    info.merge(child);
                }
                if consistent {
                    if let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::List(Box::new(element_type)));
                        if info.ty.is_none() {
                            if let Some(element_gate_type) = element_gate_type {
                                info.ty = Some(GateType::List(Box::new(element_gate_type)));
                            }
                        }
                    } else if let Some(element_gate_type) = element_gate_type {
                        info.ty = Some(GateType::List(Box::new(element_gate_type)));
                    }
                }
                info
            }
            ExprKind::Map(map) => {
                let mut info = GateExprInfo::default();
                let mut key_type = None::<SourceOptionActualType>;
                let mut value_type = None::<SourceOptionActualType>;
                let mut keys_consistent = true;
                let mut values_consistent = true;
                for entry in &map.entries {
                    let key = self.infer_expr(entry.key, env, ambient);
                    if keys_consistent {
                        if let Some(child_ty) = key.actual() {
                            key_type = match key_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        keys_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(key);

                    let value = self.infer_expr(entry.value, env, ambient);
                    if values_consistent {
                        if let Some(child_ty) = value.actual() {
                            value_type = match value_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        values_consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(value);
                }
                if keys_consistent && values_consistent {
                    if let (Some(key), Some(value)) = (key_type, value_type) {
                        info.set_actual(SourceOptionActualType::Map {
                            key: Box::new(key),
                            value: Box::new(value),
                        });
                    }
                }
                info
            }
            ExprKind::Set(elements) => {
                let mut info = GateExprInfo::default();
                let mut element_type = None::<SourceOptionActualType>;
                let mut consistent = true;
                for element in elements {
                    let child = self.infer_expr(element, env, ambient);
                    if consistent {
                        if let Some(child_ty) = child.actual() {
                            element_type = match element_type.take() {
                                None => Some(child_ty),
                                Some(current) => match current.unify(&child_ty) {
                                    Some(unified) => Some(unified),
                                    None => {
                                        consistent = false;
                                        None
                                    }
                                },
                            };
                        }
                    }
                    info.merge(child);
                }
                if consistent {
                    if let Some(element_type) = element_type {
                        info.set_actual(SourceOptionActualType::Set(Box::new(element_type)));
                    }
                }
                info
            }
            ExprKind::Record(record) => {
                let mut info = GateExprInfo::default();
                let field_count = record.fields.len();
                let mut fields = Vec::with_capacity(field_count);
                for field in record.fields {
                    let child = self.infer_expr(field.value, env, ambient);
                    if let Some(ty) = child.actual() {
                        fields.push(SourceOptionActualRecordField {
                            name: field.label.text().to_owned(),
                            ty,
                        });
                    }
                    info.merge(child);
                }
                if fields.len() == field_count {
                    info.set_actual(SourceOptionActualType::Record(fields));
                }
                info
            }
            ExprKind::Projection { base, path } => {
                let mut info = GateExprInfo::default();
                let subject = match base {
                    crate::hir::ProjectionBase::Ambient => ambient.cloned(),
                    crate::hir::ProjectionBase::Expr(base) => {
                        let base_info = self.infer_expr(base, env, ambient);
                        let ty = base_info.ty.clone();
                        info.merge(base_info);
                        ty
                    }
                };
                if let Some(subject) = subject {
                    match self.project_type(&subject, &path) {
                        Ok(projected) => info.ty = Some(projected),
                        Err(issue) => info.issues.push(issue),
                    }
                } else {
                    info.issues.push(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(&path),
                        subject: "unknown subject".to_owned(),
                    });
                }
                info
            }
            ExprKind::AmbientSubject => {
                let mut info = GateExprInfo::default();
                if let Some(ambient) = ambient.cloned() {
                    info.ty = Some(ambient);
                } else {
                    info.issues
                        .push(GateIssue::AmbientSubjectOutsidePipe { span: expr.span });
                }
                info
            }
            ExprKind::Apply { callee, arguments } => {
                let mut same_module_function_item = None;
                if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind {
                    if let Some(info) = self
                        .infer_builtin_constructor_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) =
                        self.infer_domain_member_apply(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) =
                        self.infer_class_member_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) = self.infer_same_module_constructor_apply_expr(
                        reference, &arguments, env, ambient,
                    ) {
                        return self.finalize_expr_info(info);
                    }
                    if let ResolutionState::Resolved(TermResolution::Item(item_id)) =
                        reference.resolution.as_ref()
                        && let Item::Function(function) = &self.module.items()[*item_id]
                        && supports_same_module_function_inference(function)
                    {
                        same_module_function_item = Some(*item_id);
                    }
                    if let Some(info) = self
                        .infer_polymorphic_function_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                    if let Some(info) =
                        self.infer_import_function_apply_expr(reference, &arguments, env, ambient)
                    {
                        return self.finalize_expr_info(info);
                    }
                }
                let mut info = self.infer_expr(callee, env, ambient);
                let mut current = info.ty.clone();
                for argument in arguments.iter() {
                    // Extract the expected parameter type from the current Arrow type.
                    // This lets constructors like `None` / `Some` / `Ok` resolve when
                    // the callee's parameter type is known, mirroring the import path.
                    let (param_ty, fallback_result) = match &current {
                        Some(GateType::Arrow { parameter, result }) => (
                            Some(parameter.as_ref().clone()),
                            Some(result.as_ref().clone()),
                        ),
                        _ => (None, None),
                    };
                    let argument_info =
                        self.infer_expr(*argument, env, param_ty.as_ref().or(ambient));
                    // If inference returns no type, fall back to the parameter type so the
                    // Arrow chain can still be advanced (same strategy as import path).
                    let argument_ty = argument_info
                        .actual_gate_type()
                        .or(argument_info.ty.clone())
                        .or_else(|| param_ty.clone());
                    info.merge(argument_info);
                    current = match (current.as_ref(), argument_ty.as_ref()) {
                        (Some(callee_ty), Some(argument_ty)) => self
                            .apply_function(callee_ty, argument_ty)
                            .or(fallback_result),
                        _ => fallback_result,
                    };
                }
                if let Some(item_id) = same_module_function_item
                    && let Some(argument_types) = arguments
                        .iter()
                        .map(|argument| {
                            let argument_info = self.infer_expr(*argument, env, None);
                            argument_info.actual_gate_type().or(argument_info.ty)
                        })
                        .collect::<Option<Vec<_>>>()
                    && argument_types
                        .iter()
                        .all(|argument_ty| !argument_ty.has_type_params())
                    && current
                        .as_ref()
                        .is_none_or(|result_ty| !result_ty.has_type_params())
                {
                    self.record_function_call_evidence(FunctionCallEvidence {
                        item_id,
                        argument_types,
                        result_type: current.clone(),
                    });
                }
                info.ty = current;
                info
            }
            ExprKind::Unary { operator, expr } => {
                let mut info = self.infer_expr(expr, env, ambient);
                info.ty = match (operator, info.ty.as_ref()) {
                    (crate::hir::UnaryOperator::Not, Some(ty)) if ty.is_bool() => {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let mut info = self.infer_expr(left, env, ambient);
                let left_ty = info.ty.clone();
                let right_info = self.infer_expr(right, env, ambient);
                let right_ty = right_info.ty.clone();
                info.merge(right_info);
                info.ty = if let (Some(left), Some(right)) = (left_ty.as_ref(), right_ty.as_ref()) {
                    match select_domain_binary_operator(self.module, self, operator, left, right) {
                        Ok(maybe_matched) => maybe_matched.map(|matched| matched.result_type),
                        Err(candidates) => {
                            // Multiple domain operator implementations match: emit an ambiguity
                            // diagnostic and leave the result type unknown so downstream checking
                            // can continue without cascading false errors.
                            info.issues.push(GateIssue::AmbiguousDomainOperator {
                                span: expr.span,
                                operator: binary_operator_text(operator).to_owned(),
                                candidates: candidates
                                    .into_iter()
                                    .map(|c| {
                                        format!("{}.{}", c.callee.domain_name, c.callee.member_name)
                                    })
                                    .collect(),
                            });
                            None
                        }
                    }
                } else {
                    None
                };
                if info.ty.is_some() {
                    return self.finalize_expr_info(info);
                }
                info.ty = match (left_ty.as_ref(), right_ty.as_ref(), operator) {
                    (Some(left), Some(right), crate::hir::BinaryOperator::And)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Or)
                        if left.is_bool() && right.is_bool() =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::GreaterThan)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::LessThan)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::GreaterThanOrEqual)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::LessThanOrEqual)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Add)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Subtract)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Multiply)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Divide)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::Modulo)
                        if is_numeric_gate_type(left) && left.same_shape(right) =>
                    {
                        Some(left.clone())
                    }
                    (Some(left), Some(right), crate::hir::BinaryOperator::Equals)
                    | (Some(left), Some(right), crate::hir::BinaryOperator::NotEquals)
                        if left.same_shape(right) =>
                    {
                        info.constraints
                            .push(TypeConstraint::eq(expr.span, left.clone()));
                        Some(GateType::Primitive(BuiltinType::Bool))
                    }
                    _ => None,
                };
                info
            }
            ExprKind::Pipe(pipe) => self.infer_pipe_expr(&pipe, env),
            ExprKind::Cluster(cluster) => self.infer_cluster_expr(cluster, env),
            ExprKind::PatchApply { target, patch } => {
                let mut info = self.infer_expr(target, env, ambient);
                info.actual = info
                    .actual
                    .clone()
                    .or_else(|| info.ty.as_ref().map(SourceOptionActualType::from_gate_type));
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            info.merge(self.infer_expr(*expr, env, ambient));
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            info.merge(self.infer_expr(expr, env, ambient));
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                info
            }
            ExprKind::PatchLiteral(patch) => {
                let mut info = GateExprInfo::default();
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            info.merge(self.infer_expr(*expr, env, ambient));
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            info.merge(self.infer_expr(expr, env, ambient));
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                info
            }
            ExprKind::Markup(_) => GateExprInfo::default(),
        };
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_name(
        &mut self,
        reference: &TermReference,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        match reference.resolution.as_ref() {
            ResolutionState::Unresolved => GateExprInfo::default(),
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                let ty = env.locals.get(binding).cloned();
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                let constructor_ty = self.infer_same_module_constructor_name_type(reference);
                let ty = constructor_ty
                    .clone()
                    .or_else(|| self.item_value_type(*item_id));
                let actual = constructor_ty
                    .as_ref()
                    .map(SourceOptionActualType::from_gate_type)
                    .or_else(|| self.item_value_actual(*item_id));
                GateExprInfo {
                    actual,
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::Import(import_id)) => {
                let ty = self.import_value_type(*import_id);
                GateExprInfo {
                    contains_signal: ty.as_ref().is_some_and(GateType::is_signal),
                    ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::DomainMember(_)) => GateExprInfo {
                ty: self.infer_domain_member_name_type(reference),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_)) => GateExprInfo {
                issues: vec![GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                }],
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => {
                GateExprInfo::default()
            }
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(candidates)) => {
                // Without a known expected type, try to provide a type hint if all
                // candidates share the same type shape (e.g. all are `List A -> Bool`
                // with different concrete `A`s — uncommon but helpful for error messages).
                // Disambiguation with actual expected type happens in runtime_reference_for_name.
                let types: Vec<_> = candidates
                    .iter()
                    .filter_map(|id| self.import_value_type(*id))
                    .collect();
                let common_ty = if types.len() == candidates.len()
                    && types.windows(2).all(|w| w[0].same_shape(&w[1]))
                {
                    types.into_iter().next()
                } else {
                    None
                };
                GateExprInfo {
                    contains_signal: common_ty.as_ref().is_some_and(GateType::is_signal),
                    ty: common_ty,
                    ..GateExprInfo::default()
                }
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(value)) => GateExprInfo {
                ty: Some(self.intrinsic_value_type(value.clone())),
                ..GateExprInfo::default()
            },
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                let (ty, actual) = match builtin {
                    crate::hir::BuiltinTerm::True | crate::hir::BuiltinTerm::False => {
                        (Some(GateType::Primitive(BuiltinType::Bool)), None)
                    }
                    crate::hir::BuiltinTerm::None => (
                        None,
                        Some(SourceOptionActualType::Option(Box::new(
                            SourceOptionActualType::Hole,
                        ))),
                    ),
                    crate::hir::BuiltinTerm::Some
                    | crate::hir::BuiltinTerm::Ok
                    | crate::hir::BuiltinTerm::Err
                    | crate::hir::BuiltinTerm::Valid
                    | crate::hir::BuiltinTerm::Invalid => (None, None),
                };
                GateExprInfo {
                    actual,
                    ty,
                    ..GateExprInfo::default()
                }
            }
        }
    }

    pub(crate) fn same_module_constructor(
        &self,
        reference: &TermReference,
    ) -> Option<(ItemId, String, Vec<TypeParameterId>, Vec<TypeVariantField>)> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Type(item) = &self.module.items()[*item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let variant_name = reference.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        Some((
            *item_id,
            item.name.text().to_owned(),
            item.parameters.clone(),
            variant.fields.clone(),
        ))
    }

    pub(crate) fn infer_builtin_constructor_actual(
        &self,
        builtin: BuiltinTerm,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        match (builtin, arguments) {
            (BuiltinTerm::True | BuiltinTerm::False, []) => {
                Some(SourceOptionActualType::Primitive(BuiltinType::Bool))
            }
            (BuiltinTerm::None, []) => Some(SourceOptionActualType::Option(Box::new(
                SourceOptionActualType::Hole,
            ))),
            (BuiltinTerm::Some, [argument]) => {
                Some(SourceOptionActualType::Option(Box::new(argument.clone())))
            }
            (BuiltinTerm::Ok, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Err, [argument]) => Some(SourceOptionActualType::Result {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            (BuiltinTerm::Valid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(SourceOptionActualType::Hole),
                value: Box::new(argument.clone()),
            }),
            (BuiltinTerm::Invalid, [argument]) => Some(SourceOptionActualType::Validation {
                error: Box::new(argument.clone()),
                value: Box::new(SourceOptionActualType::Hole),
            }),
            _ => None,
        }
    }

    pub(crate) fn infer_builtin_constructor_actual_from_reference(
        &self,
        reference: &TermReference,
        arguments: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        self.infer_builtin_constructor_actual(*builtin, arguments)
    }

    pub(crate) fn infer_builtin_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Builtin(builtin)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let mut info = GateExprInfo::default();
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            info.merge(argument_info);
            let Some(argument_actual) = argument_actual else {
                return Some(info);
            };
            argument_actuals.push(argument_actual);
        }
        let actual = self.infer_builtin_constructor_actual(*builtin, &argument_actuals)?;
        info.set_actual(actual);
        Some(info)
    }

    pub(crate) fn infer_same_module_constructor_name_type(
        &mut self,
        reference: &TermReference,
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if !parameters.is_empty() {
            return None;
        }
        let substitutions = HashMap::new();
        let field_types = fields
            .into_iter()
            .map(|field| self.lower_hir_type(field.ty, &substitutions))
            .collect::<Option<Vec<_>>>()?;
        let mut ty = GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments: Vec::new(),
        };
        for field_ty in field_types.into_iter().rev() {
            ty = GateType::Arrow {
                parameter: Box::new(field_ty),
                result: Box::new(ty),
            };
        }
        Some(ty)
    }

    pub(crate) fn infer_same_module_constructor_apply(
        &mut self,
        reference: &TermReference,
        argument_types: &[GateType],
    ) -> Option<GateType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_types.len() {
            return None;
        }
        let mut substitutions = HashMap::new();
        for (field, actual) in fields.iter().zip(argument_types.iter()) {
            let mut item_stack = Vec::new();
            if !self.match_hir_type(field.ty, actual, &mut substitutions, &mut item_stack) {
                return None;
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| substitutions.get(parameter).cloned())
            .collect::<Option<Vec<_>>>()?;
        Some(GateType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    pub(crate) fn infer_same_module_constructor_apply_actual(
        &mut self,
        reference: &TermReference,
        argument_actuals: &[SourceOptionActualType],
    ) -> Option<SourceOptionActualType> {
        let (item_id, item_name, parameters, fields) = self.same_module_constructor(reference)?;
        if fields.len() != argument_actuals.len() {
            return None;
        }
        let validator = Validator {
            module: self.module,
            mode: ValidationMode::RequireResolvedNames,
            diagnostics: Vec::new(),
            kind_item_cache: HashMap::new(),
            kind_item_stack: HashSet::new(),
        };
        let mut substitutions = HashMap::<TypeParameterId, SourceOptionActualType>::new();
        for (field, actual) in fields.iter().zip(argument_actuals.iter()) {
            match validator.source_option_hir_type_matches_actual_type_inner(
                field.ty,
                actual,
                &mut substitutions,
            ) {
                Some(true) => {}
                Some(false) | None => return None,
            }
        }
        let arguments = parameters
            .iter()
            .map(|parameter| {
                substitutions
                    .get(parameter)
                    .cloned()
                    .unwrap_or(SourceOptionActualType::Hole)
            })
            .collect();
        Some(SourceOptionActualType::OpaqueItem {
            item: item_id,
            name: item_name,
            arguments,
        })
    }

    pub(crate) fn infer_same_module_constructor_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.same_module_constructor(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        let mut argument_actuals = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            let argument_actual = argument_info.actual();
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
            argument_actuals.push(argument_actual);
        }
        if let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() {
            info.ty = self.infer_same_module_constructor_apply(reference, &argument_types);
        }
        if info.ty.is_none() {
            let Some(argument_actuals) = argument_actuals.into_iter().collect::<Option<Vec<_>>>()
            else {
                return Some(info);
            };
            if let Some(actual) =
                self.infer_same_module_constructor_apply_actual(reference, &argument_actuals)
            {
                info.set_actual(actual);
            }
        }
        Some(info)
    }

    pub(crate) fn match_function_signature(
        &mut self,
        function: &crate::hir::FunctionItem,
        argument_types: &[GateType],
        expected_result: Option<&GateType>,
    ) -> Option<(Vec<GateType>, GateType)> {
        if function.parameters.len() < argument_types.len() || function.annotation.is_none() {
            return None;
        }
        let mut bindings = PolyTypeBindings::new();
        let mut instantiated_parameters = Vec::with_capacity(argument_types.len());
        for (parameter, actual) in function.parameters.iter().zip(argument_types.iter()) {
            let annotation = parameter.annotation?;
            if let Some(lowered) = self.lower_annotation(annotation) {
                if !lowered.same_shape(actual) {
                    return None;
                }
                instantiated_parameters.push(lowered);
                continue;
            }
            if !self.match_poly_hir_type(annotation, actual, &mut bindings) {
                return None;
            }
            instantiated_parameters.push(self.instantiate_poly_hir_type(annotation, &bindings)?);
        }
        let result_annotation = function.annotation?;
        if function.parameters.len() == argument_types.len() {
            // Full application: check expected result and return concrete result type.
            if let Some(expected) = expected_result {
                if let Some(lowered) = self.lower_annotation(result_annotation) {
                    if !lowered.same_shape(expected) {
                        return None;
                    }
                } else if !self.match_poly_hir_type(result_annotation, expected, &mut bindings) {
                    return None;
                }
            }
            let result = self
                .lower_annotation(result_annotation)
                .or_else(|| self.instantiate_poly_hir_type(result_annotation, &bindings))?;
            Some((instantiated_parameters, result))
        } else {
            // Partial application: compute curried result type from the remaining parameters
            // and the declared return type, instantiating any bound type parameters.
            let remaining_params = &function.parameters[argument_types.len()..];
            let remaining_types = remaining_params
                .iter()
                .map(|p| {
                    p.annotation.and_then(|ann| {
                        self.lower_annotation(ann)
                            .or_else(|| self.instantiate_poly_hir_type_partially(ann, &bindings))
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            let result_ty = self.lower_annotation(result_annotation).or_else(|| {
                self.instantiate_poly_hir_type_partially(result_annotation, &bindings)
            })?;
            let curried = remaining_types
                .into_iter()
                .rev()
                .fold(result_ty, |acc, param| GateType::Arrow {
                    parameter: Box::new(param),
                    result: Box::new(acc),
                });
            Some((instantiated_parameters, curried))
        }
    }

    pub(crate) fn function_signature(
        &self,
        ty: &GateType,
        arity: usize,
    ) -> Option<(Vec<GateType>, GateType)> {
        let mut parameters = Vec::with_capacity(arity);
        let mut current = ty;
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameters.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some((parameters, current.clone()))
    }

    pub(crate) fn flatten_apply_expr(&self, expr_id: ExprId) -> (ExprId, Vec<ExprId>) {
        let mut callee = expr_id;
        let mut segments = Vec::new();
        while let ExprKind::Apply {
            callee: next_callee,
            arguments: next_arguments,
        } = &self.module.exprs()[callee].kind
        {
            segments.push(next_arguments.iter().copied().collect::<Vec<_>>());
            callee = *next_callee;
        }
        let mut arguments = Vec::new();
        for segment in segments.into_iter().rev() {
            arguments.extend(segment);
        }
        (callee, arguments)
    }

    pub(crate) fn match_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<()> {
        if let Some(lowered) = self.lower_annotation(annotation) {
            lowered.same_shape(actual).then_some(())
        } else {
            self.match_poly_hir_type(annotation, actual, bindings)
                .then_some(())
        }
    }

    pub(crate) fn match_pipe_argument_parameter_annotation(
        &mut self,
        annotation: TypeId,
        actual: &GateType,
        bindings: &mut PolyTypeBindings,
    ) -> Option<bool> {
        if self
            .match_function_parameter_annotation(annotation, actual, bindings)
            .is_some()
        {
            return Some(false);
        }
        let GateType::Signal(payload) = actual else {
            return None;
        };
        self.match_function_parameter_annotation(annotation, payload, bindings)
            .map(|_| true)
    }

    pub(crate) fn instantiate_function_parameter_annotation(
        &mut self,
        annotation: TypeId,
        bindings: &PolyTypeBindings,
    ) -> Option<GateType> {
        self.lower_annotation(annotation)
            .or_else(|| self.instantiate_poly_hir_type(annotation, bindings))
            .or_else(|| self.instantiate_poly_hir_type_partially(annotation, bindings))
    }

    pub(crate) fn match_pipe_function_signature(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let (callee_expr, explicit_arguments) = self.flatten_apply_expr(expr_id);
        self.match_pipe_function_signature_parts(
            callee_expr,
            explicit_arguments,
            env,
            ambient,
            expected_result,
        )
    }

    pub(crate) fn match_pipe_function_signature_parts(
        &mut self,
        callee_expr: ExprId,
        explicit_arguments: Vec<ExprId>,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind else {
            return None;
        };
        if let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        {
            let Item::Function(function) = &self.module.items()[*item_id] else {
                return None;
            };
            if function.parameters.len() != explicit_arguments.len() + 1 {
                return None;
            }
            if function.annotation.is_none()
                || function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.annotation.is_none())
            {
                return self.match_pipe_unannotated_function_signature(
                    callee_expr,
                    &explicit_arguments,
                    function,
                    env,
                    ambient,
                    expected_result,
                );
            }

            let mut bindings = PolyTypeBindings::new();
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
                let annotation = parameter.annotation?;
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                let Some(argument_ty) = argument_info.actual_gate_type().or(argument_info.ty)
                else {
                    signal_payload_arguments.push(false);
                    continue;
                };
                let Some(reads_signal_payload) = self.match_pipe_argument_parameter_annotation(
                    annotation,
                    &argument_ty,
                    &mut bindings,
                ) else {
                    return None;
                };
                signal_payload_arguments.push(reads_signal_payload);
            }

            let ambient_parameter = function
                .parameters
                .last()
                .expect("checked pipe arity above");
            let ambient_annotation = ambient_parameter.annotation?;
            self.match_function_parameter_annotation(ambient_annotation, ambient, &mut bindings)?;

            let result_annotation = function.annotation?;
            if let Some(expected) = expected_result {
                self.match_function_parameter_annotation(
                    result_annotation,
                    expected,
                    &mut bindings,
                )?;
            }

            let mut parameter_types = Vec::with_capacity(function.parameters.len());
            for parameter in &function.parameters {
                let annotation = parameter.annotation?;
                parameter_types
                    .push(self.instantiate_function_parameter_annotation(annotation, &bindings)?);
            }
            let result_type =
                self.instantiate_function_parameter_annotation(result_annotation, &bindings)?;

            return Some(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments,
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }

        // Handle Import and AmbiguousHoistedImports: try to find a unique import candidate
        // by walking its curried Arrow type through explicit args then the ambient subject.
        // This covers hoisted prelude functions like `list.map`, `option.map`, etc.
        let import_candidates: Option<Vec<ImportId>> = match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Import(id)) => Some(vec![*id]),
            ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(ids)) => {
                Some(ids.iter().copied().collect())
            }
            _ => None,
        };
        if let Some(candidates) = import_candidates {
            return self.match_import_pipe_function_signature(
                callee_expr,
                candidates,
                explicit_arguments,
                env,
                ambient,
                expected_result,
            );
        }

        let explicit_argument_types = explicit_arguments
            .iter()
            .map(|argument| {
                let argument_info = self.infer_expr(*argument, env, Some(ambient));
                argument_info.actual_gate_type().or(argument_info.ty)
            })
            .collect::<Vec<_>>();
        if let Some(mut full_argument_types) = explicit_argument_types
            .iter()
            .cloned()
            .collect::<Option<Vec<_>>>()
        {
            full_argument_types.push(ambient.clone());
            if let DomainMemberSelection::Unique(matched) =
                self.select_class_member_call(reference, &full_argument_types, expected_result)?
            {
                return Some(PipeFunctionSignatureMatch {
                    callee_expr,
                    explicit_arguments,
                    signal_payload_arguments: vec![
                        false;
                        matched.parameters.len().saturating_sub(1)
                    ],
                    parameter_types: matched.parameters,
                    result_type: matched.result,
                });
            }
        }
        let candidates = self.class_member_candidates(reference)?;
        let mut matches = Vec::new();
        for candidate in candidates {
            let (_, member_annotation, _) = self.class_member_signature(candidate)?;
            let mut bindings = PolyTypeBindings::new();
            let mut current = member_annotation;
            let mut parameter_type_ids = Vec::with_capacity(explicit_arguments.len() + 1);
            let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
            for argument_ty in explicit_argument_types.iter().cloned() {
                let TypeKind::Arrow { parameter, result } =
                    self.module.types()[current].kind.clone()
                else {
                    continue;
                };
                if let Some(argument_ty) = argument_ty.as_ref() {
                    if self.match_poly_hir_type(parameter, argument_ty, &mut bindings) {
                        signal_payload_arguments.push(false);
                    } else if let GateType::Signal(payload) = argument_ty {
                        if self.match_poly_hir_type(parameter, payload, &mut bindings) {
                            signal_payload_arguments.push(true);
                        } else {
                            parameter_type_ids.clear();
                            signal_payload_arguments.clear();
                            break;
                        }
                    } else {
                        parameter_type_ids.clear();
                        signal_payload_arguments.clear();
                        break;
                    }
                } else {
                    signal_payload_arguments.push(false);
                }
                parameter_type_ids.push(parameter);
                current = result;
            }
            let TypeKind::Arrow { parameter, result } = self.module.types()[current].kind.clone()
            else {
                continue;
            };
            if !self.match_poly_hir_type(parameter, ambient, &mut bindings) {
                continue;
            }
            parameter_type_ids.push(parameter);
            current = result;
            if parameter_type_ids.len() != explicit_arguments.len() + 1 {
                continue;
            }
            if let Some(expected) = expected_result
                && !self.match_poly_hir_type(current, expected, &mut bindings)
            {
                continue;
            }
            let Some(parameter_types) = parameter_type_ids
                .into_iter()
                .map(|parameter| self.instantiate_poly_hir_type_partially(parameter, &bindings))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Some(result_type) = self.instantiate_poly_hir_type_partially(current, &bindings)
            else {
                continue;
            };
            if let Some(expected) = expected_result
                && !result_type.same_shape(expected)
            {
                continue;
            }
            let explicit_arguments_match = explicit_arguments
                .iter()
                .zip(parameter_types.iter().take(explicit_arguments.len()))
                .zip(signal_payload_arguments.iter())
                .all(|((argument, expected_parameter), reads_signal_payload)| {
                    let argument_info = self.infer_expr(*argument, env, Some(ambient));
                    let arg_ty = argument_info
                        .actual_gate_type()
                        .or(argument_info.ty.clone());
                    // If we have no type information for the argument, we can't prove it
                    // doesn't match — accept it and let downstream lowering verify.
                    arg_ty.is_none()
                        || arg_ty.as_ref().is_some_and(|actual| {
                            actual.same_shape(expected_parameter)
                                || (*reads_signal_payload
                                    && matches!(
                                        actual,
                                        GateType::Signal(payload)
                                            if payload.same_shape(expected_parameter)
                                    ))
                        })
                        || expression_matches(self.module, *argument, env, expected_parameter)
                });
            if !explicit_arguments_match {
                continue;
            }
            matches.push(PipeFunctionSignatureMatch {
                callee_expr,
                explicit_arguments: explicit_arguments.clone(),
                signal_payload_arguments,
                parameter_types,
                result_type,
            });
        }
        if matches.len() != 1 {
            return None;
        }
        matches.pop()
    }

    /// Match a pipe stage body against import-backed functions (single `Import` or
    /// `AmbiguousHoistedImports`). Walks each candidate's curried Arrow type through
    /// the explicit argument types and then the ambient subject type. Returns the unique
    /// `PipeFunctionSignatureMatch` when exactly one candidate matches.
    fn match_import_pipe_function_signature(
        &mut self,
        callee_expr: ExprId,
        import_candidates: Vec<ImportId>,
        explicit_arguments: Vec<ExprId>,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let explicit_arg_types: Vec<Option<GateType>> = explicit_arguments
            .iter()
            .map(|arg| {
                let info = self.infer_expr(*arg, env, Some(ambient));
                info.actual_gate_type().or(info.ty)
            })
            .collect();

        let mut result_matches: Vec<(Vec<GateType>, GateType)> = Vec::new();

        for import_id in import_candidates {
            let Some(import_ty) = self.import_value_type_with_ambient(import_id) else {
                continue;
            };
            if !matches!(import_ty, GateType::Arrow { .. }) {
                continue;
            }

            let mut current = import_ty;
            let mut param_types: Vec<GateType> = Vec::new();
            let mut ok = true;

            for arg_ty_opt in &explicit_arg_types {
                match arg_ty_opt {
                    Some(arg_ty) => {
                        let Some(next) = self.apply_function(&current, arg_ty) else {
                            ok = false;
                            break;
                        };
                        // Record the concrete parameter type (substituting any TypeParams
                        // with the concrete argument type).
                        let concrete_param = match &current {
                            GateType::Arrow { parameter, .. } => {
                                if parameter.has_type_params() {
                                    arg_ty.clone()
                                } else {
                                    *parameter.clone()
                                }
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        param_types.push(concrete_param);
                        current = next;
                    }
                    None => {
                        // Unknown arg type — advance past this Arrow position.
                        let GateType::Arrow { parameter, result } = current else {
                            ok = false;
                            break;
                        };
                        param_types.push(*parameter);
                        current = *result;
                    }
                }
            }

            if !ok {
                continue;
            }

            let Some(result_ty) = self.apply_function(&current, ambient) else {
                continue;
            };

            if let Some(expected) = expected_result {
                if !result_ty.same_shape(expected) {
                    continue;
                }
            }

            let concrete_ambient_param = match current {
                GateType::Arrow { parameter, .. } => {
                    if parameter.has_type_params() {
                        ambient.clone()
                    } else {
                        *parameter
                    }
                }
                _ => continue,
            };
            param_types.push(concrete_ambient_param);
            result_matches.push((param_types, result_ty));
        }

        if result_matches.len() != 1 {
            return None;
        }

        let (parameter_types, result_type) = result_matches.pop().unwrap();
        let n_explicit = explicit_arguments.len();
        Some(PipeFunctionSignatureMatch {
            callee_expr,
            explicit_arguments,
            signal_payload_arguments: vec![false; n_explicit],
            parameter_types,
            result_type,
        })
    }

    pub(crate) fn match_pipe_unannotated_function_signature(
        &mut self,
        callee_expr: ExprId,
        explicit_arguments: &[ExprId],
        function: &crate::hir::FunctionItem,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<PipeFunctionSignatureMatch> {
        let mut bindings = PolyTypeBindings::new();
        let mut signal_payload_arguments = Vec::with_capacity(explicit_arguments.len());
        let mut explicit_argument_types = Vec::with_capacity(explicit_arguments.len());

        for (argument, parameter) in explicit_arguments.iter().zip(function.parameters.iter()) {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_ty = argument_info.actual_gate_type().or(argument_info.ty);
            if let Some(annotation) = parameter.annotation {
                if let Some(argument_ty) = argument_ty.as_ref() {
                    let reads_signal_payload = self.match_pipe_argument_parameter_annotation(
                        annotation,
                        argument_ty,
                        &mut bindings,
                    )?;
                    signal_payload_arguments.push(reads_signal_payload);
                } else {
                    signal_payload_arguments.push(false);
                }
            } else {
                signal_payload_arguments.push(false);
            }
            explicit_argument_types.push(argument_ty);
        }

        let ambient_parameter = function
            .parameters
            .last()
            .expect("checked pipe arity above");
        if let Some(annotation) = ambient_parameter.annotation {
            self.match_function_parameter_annotation(annotation, ambient, &mut bindings)?;
        }

        if let Some(result_annotation) = function.annotation
            && let Some(expected) = expected_result
        {
            self.match_function_parameter_annotation(result_annotation, expected, &mut bindings)?;
        }

        let mut parameter_types = Vec::with_capacity(function.parameters.len());
        for (index, parameter) in function.parameters.iter().enumerate() {
            let parameter_ty = if let Some(annotation) = parameter.annotation {
                self.instantiate_function_parameter_annotation(annotation, &bindings)?
            } else if index < explicit_argument_types.len() {
                explicit_argument_types[index].clone()?
            } else {
                ambient.clone()
            };
            parameter_types.push(parameter_ty);
        }

        let mut function_env = GateExprEnv::default();
        for (parameter, parameter_ty) in function.parameters.iter().zip(parameter_types.iter()) {
            function_env
                .locals
                .insert(parameter.binding, parameter_ty.clone());
        }

        let result_type = if let Some(result_annotation) = function.annotation {
            self.instantiate_function_parameter_annotation(result_annotation, &bindings)?
        } else {
            let body_info = self.infer_expr(function.body, &function_env, None);
            body_info.actual_gate_type().or(body_info.ty)?
        };
        if let Some(expected) = expected_result
            && !result_type.same_shape(expected)
        {
            return None;
        }

        if let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind
            && let ResolutionState::Resolved(TermResolution::Item(item_id)) =
                reference.resolution.as_ref()
            && let Item::Function(function_item) = &self.module.items()[*item_id]
            && supports_same_module_function_inference(function_item)
        {
            self.record_function_signature_evidence(FunctionSignatureEvidence {
                item_id: *item_id,
                parameter_types: parameter_types.clone(),
                result_type: result_type.clone(),
            });
        }

        Some(PipeFunctionSignatureMatch {
            callee_expr,
            explicit_arguments: explicit_arguments.to_vec(),
            signal_payload_arguments,
            parameter_types,
            result_type,
        })
    }

    pub(crate) fn infer_polymorphic_function_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return None;
        };
        if function.type_parameters.is_empty() {
            return None;
        }
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let Some((_, result)) = self.match_function_signature(function, &argument_types, None) {
            info.ty = Some(result);
        }
        Some(info)
    }

    /// Infer the result type of applying an imported function to explicit arguments.
    ///
    /// When calling an imported function like `withSelectedThreadId None`, the generic
    /// `infer_expr` path cannot determine `None`'s type without the Arrow parameter context.
    /// This function uses the import's Arrow chain to provide expected types for each argument,
    /// falling back to the Arrow parameter type when argument inference returns `None` (which is
    /// safe because the HIR type-checker has already validated the call).
    pub(crate) fn infer_import_function_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        _ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let ResolutionState::Resolved(TermResolution::Import(import_id)) =
            reference.resolution.as_ref()
        else {
            return None;
        };
        let import_ty = self.import_value_type(*import_id)?;
        // Only handle Arrow types (function applications)
        if !matches!(import_ty, GateType::Arrow { .. }) {
            return None;
        }
        let mut info = GateExprInfo::default();
        let mut current = import_ty;
        for argument in arguments.iter() {
            // Extract parameter type from current Arrow for use as context
            let (param, fallback_result) = match &current {
                GateType::Arrow { parameter, result } => {
                    (parameter.as_ref().clone(), result.as_ref().clone())
                }
                _ => return Some(info),
            };
            // Infer argument using the Arrow parameter as context
            let arg_info = self.infer_expr(*argument, env, Some(&param));
            // Use inferred type, falling back to the parameter type when unknown.
            // This is safe because HIR type-checking has already validated the call.
            let arg_ty = arg_info
                .actual_gate_type()
                .or(arg_info.ty.clone())
                .unwrap_or_else(|| param.clone());
            info.merge(arg_info);
            // Advance the Arrow chain by applying this argument
            current = self
                .apply_function(&current, &arg_ty)
                .unwrap_or(fallback_result);
        }
        if info.issues.is_empty() {
            info.ty = Some(current);
        }
        Some(info)
    }

    pub(crate) fn infer_class_member_apply_expr(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.class_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        if let DomainMemberSelection::Unique(matched) =
            self.select_class_member_call(reference, &argument_types, None)?
        {
            info.ty = Some(matched.result);
        }
        Some(info)
    }

    pub(crate) fn infer_domain_member_apply(
        &mut self,
        reference: &TermReference,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        self.domain_member_candidates(reference)?;
        let mut info = GateExprInfo::default();
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments.iter() {
            let argument_info = self.infer_expr(*argument, env, ambient);
            argument_types.push(argument_info.ty.clone());
            info.merge(argument_info);
        }
        let Some(argument_types) = argument_types.into_iter().collect::<Option<Vec<_>>>() else {
            return Some(info);
        };
        match self.select_domain_member_call(reference, &argument_types, None)? {
            DomainMemberSelection::Unique(matched) => {
                info.ty = Some(matched.result);
            }
            DomainMemberSelection::Ambiguous => {
                info.issues.push(GateIssue::AmbiguousDomainMember {
                    span: reference.span(),
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: self
                        .domain_member_candidate_labels(reference)
                        .unwrap_or_default(),
                });
            }
            DomainMemberSelection::NoMatch => {}
        }
        Some(info)
    }

    pub(crate) fn infer_pipe_body_inference(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> PipeBodyInference {
        let ambient = subject.gate_payload().clone();
        let mut info = self.infer_expr(expr_id, env, Some(&ambient));
        let mut transform_mode = PipeTransformMode::Replace;
        if let Some(function_body) = self.infer_function_pipe_body(expr_id, env, &ambient, None) {
            info = function_body;
            transform_mode = PipeTransformMode::Apply;
        } else if let Some(GateType::Arrow { parameter, result }) = info.ty.clone() {
            if parameter.same_shape(&ambient) {
                info.ty = Some(*result);
                transform_mode = PipeTransformMode::Apply;
            } else {
                info.issues.push(GateIssue::InvalidPipeStageInput {
                    span: self.module.exprs()[expr_id].span,
                    stage: "pipe",
                    expected: ambient.to_string(),
                    actual: parameter.to_string(),
                });
                info.ty = None;
            }
        } else if info.ty.is_none() && info.issues.is_empty() {
            // Fallback: try to infer the result type for import-backed pipe stages
            // (e.g. `|> map f` where `map` is an ambiguous hoisted import).  The
            // earlier paths only handle same-module functions and class members; this
            // path covers the import / hoisted-import case by walking the Arrow chain
            // of each candidate import and returning the unique matching result.
            if let Some(result_ty) = self.infer_import_apply_pipe_result(expr_id, env, &ambient) {
                info.ty = Some(result_ty);
                transform_mode = PipeTransformMode::Apply;
            }
        }
        PipeBodyInference {
            info,
            transform_mode,
        }
    }

    /// Try to infer the result type of a pipe stage that is an imported function
    /// applied to explicit arguments, with the pipe subject as the last argument.
    /// Handles both a single resolved import and `AmbiguousHoistedImports`,
    /// returning the unique result when exactly one candidate successfully type-checks.
    fn infer_import_apply_pipe_result(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
    ) -> Option<GateType> {
        let (callee_expr, explicit_arguments) = self.flatten_apply_expr(expr_id);

        // Collect candidate import IDs from the callee resolution (owned Vec to free the borrow).
        let candidates: Vec<ImportId> = {
            let ExprKind::Name(reference) = &self.module.exprs()[callee_expr].kind else {
                return None;
            };
            match reference.resolution.as_ref() {
                ResolutionState::Resolved(TermResolution::Import(id)) => vec![*id],
                ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(ids)) => {
                    ids.iter().copied().collect()
                }
                _ => return None,
            }
        };

        // Infer types for explicit arguments (needs &mut self, safe because candidates is owned).
        let explicit_arg_types: Vec<Option<GateType>> = explicit_arguments
            .iter()
            .map(|arg| {
                let arg_info = self.infer_expr(*arg, env, None);
                arg_info.actual_gate_type().or(arg_info.ty)
            })
            .collect();

        // For each candidate: walk the Arrow chain through explicit args, then apply ambient.
        let mut results = Vec::new();
        for import_id in candidates {
            let import_ty = match self.import_value_type_with_ambient(import_id) {
                Some(ty) if matches!(ty, GateType::Arrow { .. }) => ty,
                _ => continue,
            };
            let mut current = import_ty;
            let mut ok = true;
            for arg_ty_opt in &explicit_arg_types {
                match arg_ty_opt {
                    Some(arg_ty) => match self.apply_function(&current, arg_ty) {
                        Some(next) => current = next,
                        None => {
                            ok = false;
                            break;
                        }
                    },
                    None => {
                        // Unknown arg type — advance past the Arrow parameter.
                        match current {
                            GateType::Arrow { result, .. } => current = *result,
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            if let Some(result_ty) = self.apply_function(&current, ambient) {
                results.push(result_ty);
            }
        }

        if results.len() == 1 {
            results.pop()
        } else {
            None
        }
    }

    pub(crate) fn infer_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let inference = self.infer_pipe_body_inference(expr_id, env, subject);
        self.finalize_expr_info(inference.info)
    }

    pub(crate) fn infer_tap_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        info.ty = Some(subject.clone());
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_accumulate_stage_info(
        &mut self,
        seed_expr: ExprId,
        step_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let seed_info = self.infer_expr(seed_expr, env, None);
        let seed_ty = seed_info.ty.clone();
        info.merge(seed_info);
        let Some(seed_ty) = seed_ty else {
            return self.finalize_expr_info(info);
        };

        let step_info = self.infer_expr(step_expr, env, Some(input_payload.as_ref()));
        let step_ty = step_info.actual_gate_type().or(step_info.ty.clone());
        info.merge(step_info);
        let Some(step_ty) = step_ty else {
            return self.finalize_expr_info(info);
        };

        let Some((parameters, result_ty)) = self.function_signature(&step_ty, 2) else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: format!("{} -> {} -> {}", input_payload.as_ref(), seed_ty, seed_ty),
                actual: step_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        };
        if !parameters[0].same_shape(input_payload.as_ref())
            || !parameters[1].same_shape(&seed_ty)
            || !result_ty.same_shape(&seed_ty)
        {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[step_expr].span,
                stage: "+|>",
                expected: format!("{} -> {} -> {}", input_payload.as_ref(), seed_ty, seed_ty),
                actual: step_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(Box::new(seed_ty)));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_previous_stage_info(
        &mut self,
        seed_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[seed_expr].span,
                stage: "~|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let seed_info = self.infer_expr(seed_expr, env, None);
        let seed_ty = seed_info.actual_gate_type().or(seed_info.ty.clone());
        info.merge(seed_info);
        let Some(seed_ty) = seed_ty else {
            return self.finalize_expr_info(info);
        };
        if !seed_ty.same_shape(input_payload.as_ref()) {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[seed_expr].span,
                stage: "~|>",
                expected: input_payload.as_ref().to_string(),
                actual: seed_ty.to_string(),
            });
            return self.finalize_expr_info(info);
        }

        info.ty = Some(GateType::Signal(Box::new(seed_ty)));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_diff_stage_info(
        &mut self,
        diff_expr: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let GateType::Signal(input_payload) = subject else {
            info.issues.push(GateIssue::InvalidPipeStageInput {
                span: self.module.exprs()[diff_expr].span,
                stage: "-|>",
                expected: "Signal _".to_owned(),
                actual: subject.to_string(),
            });
            return self.finalize_expr_info(info);
        };

        let stage_info = self.infer_expr(diff_expr, env, None);
        let stage_ty = stage_info.actual_gate_type().or(stage_info.ty.clone());
        info.merge(stage_info);
        let Some(stage_ty) = stage_ty else {
            return self.finalize_expr_info(info);
        };

        if let Some((parameters, result_ty)) = self.function_signature(&stage_ty, 2) {
            if !parameters[0].same_shape(input_payload.as_ref())
                || !parameters[1].same_shape(input_payload.as_ref())
                || result_ty.is_signal()
            {
                info.issues.push(GateIssue::InvalidPipeStageInput {
                    span: self.module.exprs()[diff_expr].span,
                    stage: "-|>",
                    expected: format!(
                        "{} -> {} -> _",
                        input_payload.as_ref(),
                        input_payload.as_ref()
                    ),
                    actual: stage_ty.to_string(),
                });
                return self.finalize_expr_info(info);
            }
            info.ty = Some(GateType::Signal(Box::new(result_ty.clone())));
            return self.finalize_expr_info(info);
        }

        if stage_ty.same_shape(input_payload.as_ref()) && is_numeric_gate_type(input_payload) {
            info.ty = Some(GateType::Signal(Box::new(stage_ty)));
            return self.finalize_expr_info(info);
        }

        info.issues.push(GateIssue::InvalidPipeStageInput {
            span: self.module.exprs()[diff_expr].span,
            stage: "-|>",
            expected: format!(
                "{} -> {} -> _  or seeded {}",
                input_payload.as_ref(),
                input_payload.as_ref(),
                input_payload.as_ref()
            ),
            actual: stage_ty.to_string(),
        });
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_truthy_falsy_branch(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        payload_subject: Option<&GateType>,
    ) -> GateExprInfo {
        match payload_subject {
            Some(subject) => self.infer_pipe_body(expr_id, env, subject),
            None => self.infer_expr(expr_id, env, None),
        }
    }

    pub(crate) fn infer_truthy_falsy_pair(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let subject_plan = self.truthy_falsy_subject_plan(subject)?;
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
        );
        if !truthy.issues.is_empty() {
            return None;
        }
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
        );
        if !falsy.issues.is_empty() {
            return None;
        }
        let truthy_ty = truthy.actual()?;
        let falsy_ty = falsy.actual()?;
        self.apply_truthy_falsy_result_actual(subject, truthy_ty.unify(&falsy_ty)?)
            .to_gate_type()
    }

    pub(crate) fn infer_truthy_falsy_pair_info(
        &mut self,
        pair: &TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(subject_plan) = self.truthy_falsy_subject_plan(subject) else {
            return GateExprInfo::default();
        };
        let mut info = GateExprInfo::default();
        let truthy = self.infer_truthy_falsy_branch(
            pair.truthy_expr,
            env,
            subject_plan.truthy_payload.as_ref(),
        );
        let truthy_ty = truthy.actual();
        info.merge(truthy);
        let falsy = self.infer_truthy_falsy_branch(
            pair.falsy_expr,
            env,
            subject_plan.falsy_payload.as_ref(),
        );
        let falsy_ty = falsy.actual();
        info.merge(falsy);
        if info.issues.is_empty() {
            if let (Some(truthy_ty), Some(falsy_ty)) = (truthy_ty, falsy_ty) {
                if let Some(branch_ty) = truthy_ty.unify(&falsy_ty) {
                    info.set_actual(self.apply_truthy_falsy_result_actual(subject, branch_ty));
                }
            }
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_function_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected_result: Option<&GateType>,
    ) -> Option<GateExprInfo> {
        let plan = self.match_pipe_function_signature(expr_id, env, ambient, expected_result)?;
        let mut info = GateExprInfo::default();
        for ((argument, expected), reads_signal_payload) in plan
            .explicit_arguments
            .iter()
            .zip(
                plan.parameter_types
                    .iter()
                    .take(plan.explicit_arguments.len()),
            )
            .zip(plan.signal_payload_arguments.iter())
        {
            let argument_info = self.infer_expr(*argument, env, Some(ambient));
            let argument_actual = argument_info.actual_gate_type();
            let argument_annot = argument_info.ty.clone();
            // Prefer actual type; also keep the annotated type as a fallback for
            // generic functions whose actual inference produces Hole placeholders
            // in place of type parameters (e.g. `lengthStep : Int -> A -> Int`).
            let argument_ty = argument_actual.or(argument_annot.clone());
            info.merge(argument_info);
            let matches_expected = argument_ty.as_ref().is_some_and(|actual| {
                actual.same_shape(expected)
                    || (*reads_signal_payload
                        && matches!(
                            actual,
                            GateType::Signal(payload) if payload.same_shape(expected)
                        ))
            }) || argument_annot
                .as_ref()
                .is_some_and(|ty| ty.same_shape(expected))
                || expression_matches(self.module, *argument, env, expected);
            if !matches_expected {
                return Some(info);
            }
        }
        if info.issues.is_empty() {
            info.ty = Some(plan.result_type);
        }
        Some(info)
    }

    pub(crate) fn infer_gate_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_gate_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_fanout_map_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanout_map_stage_info(expr_id, env, subject).ty
    }

    /// Infer only the result type for a joined fanout segment, without building
    /// filter plans or join plans.  Used by `validate_gate_pipe` to advance the
    /// subject type past a `*|> … <|*` segment without re-running the full
    /// `elaborate_fanout_segment` pass that `validate_fanout_semantics` already
    /// performed (PA-H2).
    pub(crate) fn infer_fanout_segment_result_type(
        &mut self,
        segment: &crate::PipeFanoutSegment<'_>,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        let carrier = self.fanout_carrier(subject)?;
        let element_subject = subject.fanout_element().cloned()?;
        let mapped_element_type = self
            .infer_pipe_body(segment.map_expr(), env, &element_subject)
            .ty?;
        let mapped_collection_type = self.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Map, carrier),
            mapped_element_type,
        );
        if let Some(join_expr) = segment.join_expr() {
            let join_value_type = self
                .infer_pipe_body(join_expr, env, &mapped_collection_type)
                .ty?;
            Some(self.apply_fanout_plan(
                FanoutPlanner::plan(FanoutStageKind::Join, carrier),
                join_value_type,
            ))
        } else {
            Some(mapped_collection_type)
        }
    }

    pub(crate) fn infer_fanin_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_fanin_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_transform_stage(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> Option<GateType> {
        self.infer_transform_stage_info(expr_id, env, subject).ty
    }

    pub(crate) fn infer_transform_stage_mode(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> PipeTransformMode {
        self.infer_pipe_body_inference(expr_id, env, subject)
            .transform_mode
    }

    pub(crate) fn infer_transform_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let PipeBodyInference {
            mut info,
            transform_mode: _,
        } = self.infer_pipe_body_inference(expr_id, env, subject);
        info.ty = info.ty.map(|body_ty| match subject {
            GateType::Signal(_) => GateType::Signal(Box::new(body_ty)),
            _ => body_ty,
        });
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_gate_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        let is_valid = info.issues.is_empty()
            && !info.contains_signal
            && !info.ty.as_ref().is_some_and(GateType::is_signal)
            && info.ty.as_ref().is_some_and(GateType::is_bool);
        info.ty = is_valid
            .then(|| self.apply_gate_plan(GatePlanner::plan(subject.gate_carrier()), subject));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_fanout_map_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let Some(element) = subject.fanout_element() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, element);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Map, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_fanin_stage_info(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let Some(carrier) = subject.fanout_carrier() else {
            return GateExprInfo::default();
        };
        let mut info = self.infer_pipe_body(expr_id, env, subject);
        if info.issues.is_empty() {
            info.ty = info.ty.map(|body_ty| {
                self.apply_fanout_plan(FanoutPlanner::plan(FanoutStageKind::Join, carrier), body_ty)
            });
        } else {
            info.ty = None;
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_case_stage_run_info(
        &mut self,
        case_stages: &[&crate::hir::PipeStage],
        env: &GateExprEnv,
        subject: &GateType,
    ) -> GateExprInfo {
        let mut info = GateExprInfo::default();
        let mut branch_result = None::<SourceOptionActualType>;
        let branch_subject = subject.gate_payload().clone();
        for stage in case_stages {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                continue;
            };
            let mut branch_env = env.clone();
            branch_env
                .locals
                .extend(self.case_pattern_bindings(*pattern, &branch_subject).locals);
            let branch = self.infer_pipe_body(*body, &branch_env, &branch_subject);
            let branch_ty = branch.actual();
            info.merge(branch);
            let Some(branch_ty) = branch_ty else {
                branch_result = None;
                continue;
            };
            match branch_result.as_ref() {
                None => branch_result = Some(branch_ty),
                Some(current) => {
                    let Some(unified) = current.unify(&branch_ty) else {
                        info.issues.push(GateIssue::CaseBranchTypeMismatch {
                            span: stage.span,
                            expected: current.to_string(),
                            actual: branch_ty.to_string(),
                        });
                        branch_result = None;
                        break;
                    };
                    branch_result = Some(unified);
                }
            }
        }
        if info.issues.is_empty() {
            if let Some(branch_result) = branch_result {
                info.set_actual(match subject.gate_carrier() {
                    GateCarrier::Ordinary => branch_result,
                    GateCarrier::Signal => SourceOptionActualType::Signal(Box::new(branch_result)),
                });
            }
        }
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_cluster_expr(
        &mut self,
        cluster_id: ClusterId,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        let Some(cluster) = self.module.clusters().get(cluster_id).cloned() else {
            return GateExprInfo::default();
        };
        let spine = cluster.normalized_spine();
        let mut info = GateExprInfo::default();
        let mut cluster_kind = None::<ApplicativeClusterKind>;
        let mut payloads = Vec::new();

        for member in spine.apply_arguments() {
            let member_info = self.infer_expr(member, env, None);
            let member_ty = member_info.actual();
            info.merge(member_info);
            let Some(member_ty) = member_ty else {
                return self.finalize_expr_info(info);
            };
            let Some((member_kind, payload)) =
                ApplicativeClusterKind::from_member_actual(&member_ty)
            else {
                info.issues
                    .push(GateIssue::UnsupportedApplicativeClusterMember {
                        span: self.module.exprs()[member].span,
                        actual: member_ty.to_string(),
                    });
                return self.finalize_expr_info(info);
            };
            match cluster_kind.as_ref() {
                None => {
                    cluster_kind = Some(member_kind);
                    payloads.push(payload);
                }
                Some(expected) => {
                    let Some(unified) = expected.unify(&member_kind) else {
                        info.issues.push(GateIssue::ApplicativeClusterMismatch {
                            span: self.module.exprs()[member].span,
                            expected: expected.surface(),
                            actual: member_kind.surface(),
                        });
                        return self.finalize_expr_info(info);
                    };
                    cluster_kind = Some(unified);
                    payloads.push(payload);
                }
            }
        }

        let Some(cluster_kind) = cluster_kind else {
            return self.finalize_expr_info(info);
        };
        let payload_result = match spine.pure_head() {
            ApplicativeSpineHead::TupleConstructor(_) => SourceOptionActualType::Tuple(payloads),
            ApplicativeSpineHead::Expr(finalizer) => {
                let finalizer_info = self.infer_expr(finalizer, env, None);
                let finalizer_ty = finalizer_info.ty.clone();
                let finalizer_had_issues = !finalizer_info.issues.is_empty();
                info.merge(finalizer_info);

                let closed_payloads = payloads
                    .iter()
                    .map(SourceOptionActualType::to_gate_type)
                    .collect::<Option<Vec<_>>>();
                let applied_from_type = finalizer_ty
                    .as_ref()
                    .zip(closed_payloads.as_ref())
                    .and_then(|(ty, payloads)| self.apply_function_chain(ty, payloads));
                let applied_from_constructor = match &self.module.exprs()[finalizer].kind {
                    ExprKind::Name(reference) => {
                        let from_builtin = self
                            .infer_builtin_constructor_actual_from_reference(reference, &payloads);
                        let from_same_module =
                            self.infer_same_module_constructor_apply_actual(reference, &payloads);
                        from_builtin.or(from_same_module).or_else(|| {
                            closed_payloads.as_ref().and_then(|payloads| {
                                self.infer_same_module_constructor_apply(reference, payloads)
                                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                            })
                        })
                    }
                    _ => None,
                };

                match applied_from_type
                    .map(|result| SourceOptionActualType::from_gate_type(&result))
                    .or(applied_from_constructor)
                {
                    Some(result) => result,
                    None => {
                        if !finalizer_had_issues {
                            info.issues.push(GateIssue::InvalidClusterFinalizer {
                                span: self.module.exprs()[finalizer].span,
                                expected_inputs: payloads.iter().map(ToString::to_string).collect(),
                                actual: finalizer_ty
                                    .map(|ty| ty.to_string())
                                    .unwrap_or_else(|| "unknown type".to_owned()),
                            });
                        }
                        return self.finalize_expr_info(info);
                    }
                }
            }
        };
        info.set_actual(cluster_kind.wrap_actual(payload_result));
        self.finalize_expr_info(info)
    }

    pub(crate) fn infer_pipe_expr(
        &mut self,
        pipe: &crate::hir::PipeExpr,
        env: &GateExprEnv,
    ) -> GateExprInfo {
        let stages = pipe.stages.iter().collect::<Vec<_>>();
        let mut info = self.infer_expr(pipe.head, env, None);
        let mut current = info.ty.clone();
        let mut pipe_env = env.clone();
        let mut stage_index = 0usize;
        while stage_index < stages.len() {
            let stage = stages[stage_index];
            let Some(subject) = current.clone() else {
                break;
            };
            let stage_info = match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    stage_index += 1;
                    let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                    self.infer_transform_stage_info(*expr, &stage_env, &subject)
                }
                PipeStageKind::Tap { expr } => {
                    stage_index += 1;
                    let stage_env = pipe_stage_expr_env(&pipe_env, stage, &subject);
                    self.infer_tap_stage_info(*expr, &stage_env, &subject)
                }
                PipeStageKind::Gate { expr } => {
                    stage_index += 1;
                    self.infer_gate_stage_info(*expr, &pipe_env, &subject)
                }
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        stage_index = segment.next_stage_index();
                        match crate::fanout_elaboration::elaborate_fanout_segment(
                            self.module,
                            &segment,
                            Some(&subject),
                            &pipe_env,
                            self,
                        ) {
                            crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                let mut info = GateExprInfo::default();
                                info.ty = Some(plan.result_type);
                                info
                            }
                            crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => {
                                GateExprInfo::default()
                            }
                        }
                    } else {
                        stage_index += 1;
                        self.infer_fanout_map_stage_info(*expr, &pipe_env, &subject)
                    }
                }
                PipeStageKind::FanIn { expr } => {
                    stage_index += 1;
                    self.infer_fanin_stage_info(*expr, &pipe_env, &subject)
                }
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&stages, stage_index) else {
                        break;
                    };
                    stage_index = pair.next_index;
                    self.infer_truthy_falsy_pair_info(&pair, &pipe_env, &subject)
                }
                PipeStageKind::Case { .. } => {
                    let case_start = stage_index;
                    while stage_index < stages.len()
                        && matches!(stages[stage_index].kind, PipeStageKind::Case { .. })
                    {
                        stage_index += 1;
                    }
                    self.infer_case_stage_run_info(
                        &stages[case_start..stage_index],
                        &pipe_env,
                        &subject,
                    )
                }
                PipeStageKind::Accumulate { seed, step } => {
                    stage_index += 1;
                    self.infer_accumulate_stage_info(*seed, *step, &pipe_env, &subject)
                }
                PipeStageKind::Previous { expr } => {
                    stage_index += 1;
                    self.infer_previous_stage_info(*expr, &pipe_env, &subject)
                }
                PipeStageKind::Diff { expr } => {
                    stage_index += 1;
                    self.infer_diff_stage_info(*expr, &pipe_env, &subject)
                }
                PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. } => {
                    stage_index += 1;
                    GateExprInfo::default()
                }
                PipeStageKind::Validate { expr } => {
                    stage_index += 1;
                    self.infer_transform_stage_info(*expr, &pipe_env, &subject)
                }
            };
            if let Some(result_subject) = stage_info.ty.as_ref() {
                extend_pipe_env_with_stage_memos(&mut pipe_env, stage, &subject, result_subject);
            }
            current = stage_info.ty.clone();
            info.merge(stage_info);
        }
        info.ty = current;
        info
    }

    pub(crate) fn project_type(
        &mut self,
        subject: &GateType,
        path: &NamePath,
    ) -> Result<GateType, GateIssue> {
        let mut current = subject.clone();
        for segment in path.segments().iter() {
            current = self
                .project_type_step(&current, segment, path)?
                .result()
                .clone();
        }
        Ok(current)
    }

    pub(crate) fn project_type_step(
        &mut self,
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
    ) -> Result<GateProjectionStep, GateIssue> {
        match subject {
            GateType::Record(fields) => {
                self.project_record_field_step(fields, false, subject, segment, path)
            }
            GateType::Signal(payload) => match payload.as_ref() {
                GateType::Record(fields) => {
                    self.project_record_field_step(fields, true, subject, segment, path)
                }
                _ => Err(GateIssue::InvalidProjection {
                    span: path.span(),
                    path: name_path_text(path),
                    subject: subject.to_string(),
                }),
            },
            GateType::Domain {
                item, arguments, ..
            } => self.project_domain_member_step(*item, arguments, subject, segment, path),
            GateType::OpaqueImport { import, .. } => {
                // Guard against the sentinel value (u32::MAX) used when a named type was not
                // found in the current module's imports — `get` returns None safely.
                let Some(import_binding) = self.module.imports().get(*import) else {
                    return Err(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(path),
                        subject: subject.to_string(),
                    });
                };
                // Dispatch on the import metadata.
                match &import_binding.metadata {
                    ImportBindingMetadata::TypeConstructor {
                        fields: Some(fields),
                        ..
                    } => {
                        // Imported record type: resolve the named field.
                        let import_fields = fields
                            .iter()
                            .map(|f| GateRecordField {
                                name: f.name.to_string(),
                                ty: self.lower_import_value_type(&f.ty),
                            })
                            .collect::<Vec<_>>();
                        self.project_record_field_step(
                            &import_fields,
                            false,
                            subject,
                            segment,
                            path,
                        )
                    }
                    ImportBindingMetadata::Domain {
                        carrier: Some(carrier),
                        ..
                    } => {
                        // Imported domain type: `.unwrap` / `.value` / `.carrier` return the carrier type.
                        if segment.text() == "value" || segment.text() == "carrier" {
                            let carrier_ty = self.lower_import_value_type(carrier);
                            Ok(GateProjectionStep::RecordField { result: carrier_ty })
                        } else {
                            Err(GateIssue::InvalidProjection {
                                span: path.span(),
                                path: name_path_text(path),
                                subject: subject.to_string(),
                            })
                        }
                    }
                    _ => Err(GateIssue::InvalidProjection {
                        span: path.span(),
                        path: name_path_text(path),
                        subject: subject.to_string(),
                    }),
                }
            }
            _ => Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
        }
    }

    pub(crate) fn project_record_field_step(
        &self,
        fields: &[GateRecordField],
        wrap_signal: bool,
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
    ) -> Result<GateProjectionStep, GateIssue> {
        let Some(field) = fields.iter().find(|field| field.name == segment.text()) else {
            return Err(GateIssue::UnknownField {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            });
        };
        let result = if wrap_signal {
            GateType::Signal(Box::new(field.ty.clone()))
        } else {
            field.ty.clone()
        };
        Ok(GateProjectionStep::RecordField { result })
    }

    pub(crate) fn project_domain_member_step(
        &mut self,
        domain_item: ItemId,
        domain_arguments: &[GateType],
        subject: &GateType,
        segment: &Name,
        path: &NamePath,
    ) -> Result<GateProjectionStep, GateIssue> {
        let Item::Domain(domain) = &self.module.items()[domain_item] else {
            return Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            });
        };

        // Built-in `.carrier` accessor: returns the underlying carrier type.
        if segment.text() == "carrier" {
            let carrier_ty = self.lower_open_annotation(domain.carrier);
            if let Some(mut carrier_ty) = carrier_ty {
                let substitutions: HashMap<_, _> = domain
                    .parameters
                    .iter()
                    .copied()
                    .zip(domain_arguments.iter().cloned())
                    .collect();
                if !substitutions.is_empty() {
                    carrier_ty = carrier_ty.substitute_type_parameters(&substitutions);
                }
                let handle = DomainMemberHandle {
                    domain: domain_item,
                    domain_name: domain.name.text().into(),
                    member_name: "carrier".into(),
                    member_index: usize::MAX,
                };
                return Ok(GateProjectionStep::DomainMember {
                    handle,
                    result: carrier_ty,
                });
            }
        }

        let substitutions = domain
            .parameters
            .iter()
            .copied()
            .zip(domain_arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut matches = Vec::new();
        let mut found_named_member = false;
        for (member_index, member) in domain.members.iter().enumerate() {
            if member.kind != DomainMemberKind::Method || member.name.text() != segment.text() {
                continue;
            }
            found_named_member = true;
            let resolution = DomainMemberResolution {
                domain: domain_item,
                member_index,
            };
            let Some(annotation) = self.lower_domain_member_annotation(resolution, &substitutions)
            else {
                continue;
            };
            let Some((parameters, result)) = self.function_signature(&annotation, 1) else {
                continue;
            };
            let Some(parameter) = parameters.first() else {
                continue;
            };
            if !parameter.same_shape(subject) {
                continue;
            }
            let Some(handle) = self.module.domain_member_handle(resolution) else {
                continue;
            };
            matches.push((handle, result));
        }

        match matches.len() {
            1 => {
                let (handle, result) = matches
                    .pop()
                    .expect("exactly one domain projection match should be available");
                Ok(GateProjectionStep::DomainMember { handle, result })
            }
            0 if found_named_member => Err(GateIssue::InvalidProjection {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
            0 => Err(GateIssue::UnknownField {
                span: path.span(),
                path: name_path_text(path),
                subject: subject.to_string(),
            }),
            _ => Err(GateIssue::AmbiguousDomainMember {
                span: path.span(),
                name: segment.text().to_owned(),
                candidates: matches
                    .into_iter()
                    .map(|(handle, _)| format!("{}.{}", handle.domain_name, handle.member_name))
                    .collect(),
            }),
        }
    }

    pub(crate) fn apply_function(
        &self,
        callee: &GateType,
        argument: &GateType,
    ) -> Option<GateType> {
        let GateType::Arrow { parameter, result } = callee else {
            return None;
        };
        if parameter.same_shape(argument) {
            return Some(result.as_ref().clone());
        }
        // Polymorphic application: if the parameter is an open type variable, substitute it in
        // the result to produce a concrete return type without requiring exact structural equality.
        if let GateType::TypeParameter {
            parameter: param_id,
            ..
        } = parameter.as_ref()
        {
            return Some(result.substitute_type_parameter(*param_id, argument));
        }
        // Structural unification: the parameter may be a compound type containing TypeParameter
        // nodes (e.g. OpaqueImport(NEL, [TypeParam(A)])). Match structurally against the argument,
        // collect bindings, and substitute them into the result.
        if parameter.has_type_params() {
            let mut bindings = HashMap::new();
            if argument.unify_type_params(parameter, &mut bindings) && !bindings.is_empty() {
                return Some(result.substitute_type_parameters(&bindings));
            }
        }
        None
    }

    pub(crate) fn apply_function_chain(
        &self,
        callee: &GateType,
        arguments: &[GateType],
    ) -> Option<GateType> {
        let mut current = callee.clone();
        for argument in arguments {
            current = self.apply_function(&current, argument)?;
        }
        Some(current)
    }
}

pub(crate) fn is_numeric_gate_type(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(
            BuiltinType::Int | BuiltinType::Float | BuiltinType::Decimal | BuiltinType::BigInt
        )
    )
}

pub(crate) fn truthy_falsy_pair_stages<'a>(
    stages: &[&'a crate::hir::PipeStage],
    index: usize,
) -> Option<TruthyFalsyPairStages<'a>> {
    let first = *stages.get(index)?;
    let second = *stages.get(index + 1)?;
    match (&first.kind, &second.kind) {
        (
            PipeStageKind::Truthy { expr: truthy_expr },
            PipeStageKind::Falsy { expr: falsy_expr },
        ) => Some(TruthyFalsyPairStages {
            truthy_index: index,
            truthy_stage: first,
            truthy_expr: *truthy_expr,
            falsy_index: index + 1,
            falsy_stage: second,
            falsy_expr: *falsy_expr,
            next_index: index + 2,
        }),
        (
            PipeStageKind::Falsy { expr: falsy_expr },
            PipeStageKind::Truthy { expr: truthy_expr },
        ) => Some(TruthyFalsyPairStages {
            truthy_index: index + 1,
            truthy_stage: second,
            truthy_expr: *truthy_expr,
            falsy_index: index,
            falsy_stage: first,
            falsy_expr: *falsy_expr,
            next_index: index + 2,
        }),
        _ => None,
    }
}

pub(crate) fn name_path_text(path: &NamePath) -> String {
    format!(
        ".{}",
        path.segments()
            .iter()
            .map(|segment| segment.text())
            .collect::<Vec<_>>()
            .join(".")
    )
}

pub(crate) fn case_constructor_key(reference: &TermReference) -> Option<CaseConstructorKey> {
    match reference.resolution.as_ref() {
        ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
            Some(CaseConstructorKey::Builtin(*builtin))
        }
        ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            Some(CaseConstructorKey::SameModuleVariant {
                item: *item_id,
                name: reference.path.segments().iter().last()?.text().to_owned(),
            })
        }
        ResolutionState::Resolved(TermResolution::Import(import_id)) => {
            Some(CaseConstructorKey::ImportedVariant {
                import: *import_id,
                name: reference.path.segments().iter().last()?.text().to_owned(),
            })
        }
        ResolutionState::Unresolved
        | ResolutionState::Resolved(TermResolution::Local(_))
        | ResolutionState::Resolved(TermResolution::DomainMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
        | ResolutionState::Resolved(TermResolution::ClassMember(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
        | ResolutionState::Resolved(TermResolution::AmbiguousHoistedImports(_))
        | ResolutionState::Resolved(TermResolution::IntrinsicValue(_)) => None,
    }
}

pub(crate) fn missing_case_list(missing: &[CaseConstructorShape]) -> String {
    missing
        .iter()
        .map(|constructor| format!("`{}`", constructor.display))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn missing_case_label(missing: &[CaseConstructorShape]) -> String {
    let cases = missing_case_list(missing);
    if missing.len() == 1 {
        format!("add a case for {cases}, or use `_` to make the catch-all explicit")
    } else {
        format!("add cases for {cases}, or use `_` to make the catch-all explicit")
    }
}

pub(crate) fn custom_source_wakeup_kind(
    wakeup: CustomSourceRecurrenceWakeup,
) -> RecurrenceWakeupKind {
    match wakeup {
        CustomSourceRecurrenceWakeup::Timer => RecurrenceWakeupKind::Timer,
        CustomSourceRecurrenceWakeup::Backoff => RecurrenceWakeupKind::Backoff,
        CustomSourceRecurrenceWakeup::SourceEvent => RecurrenceWakeupKind::SourceEvent,
        CustomSourceRecurrenceWakeup::ProviderDefinedTrigger => {
            RecurrenceWakeupKind::ProviderDefinedTrigger
        }
    }
}

pub(crate) fn is_db_changed_trigger_projection(module: &Module, expr: ExprId) -> bool {
    matches!(
        &module.exprs()[expr].kind,
        ExprKind::Projection {
            base: ProjectionBase::Expr(_),
            path,
        } if path.segments().len() == 1 && path.segments().first().text() == "changed"
    )
}

pub(crate) fn type_argument_phrase(count: usize) -> String {
    format!("{count} type argument{}", if count == 1 { "" } else { "s" })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionTypeCheck {
    Match,
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionGenericConstructorRootCheck {
    Match(SourceOptionActualType),
    Mismatch(SourceOptionTypeMismatch),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionTypeMismatch {
    pub(crate) span: SourceSpan,
    pub(crate) actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSourceOptionValue {
    pub(crate) field: crate::hir::RecordExprField,
    pub(crate) expected_surface: String,
    pub(crate) expected: SourceOptionExpectedType,
}

pub(crate) fn custom_source_contract_expected(
    module: &Module,
    annotation: TypeId,
    typing: &mut GateTypeContext<'_>,
) -> Option<(SourceOptionExpectedType, String)> {
    let expected = custom_source_contract_expected_type(module, annotation)?;
    let surface = typing.lower_annotation(annotation)?.to_string();
    Some((expected, surface))
}

pub(crate) fn custom_source_contract_expected_type(
    module: &Module,
    annotation: TypeId,
) -> Option<SourceOptionExpectedType> {
    SourceOptionExpectedType::from_hir_type(
        module,
        annotation,
        &HashMap::new(),
        SourceOptionTypeSurface::Contract,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionExpectedType {
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionExpectedRecordField>),
    List(Box<Self>),
    Map { key: Box<Self>, value: Box<Self> },
    Set(Box<Self>),
    Signal(Box<Self>),
    Option(Box<Self>),
    Result { error: Box<Self>, value: Box<Self> },
    Validation { error: Box<Self>, value: Box<Self> },
    Named(SourceOptionNamedType),
    ContractParameter(SourceTypeParameter),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionExpectedRecordField {
    pub(crate) name: String,
    pub(crate) ty: SourceOptionExpectedType,
}

/// Local proof type that keeps builtin container holes explicit until later
/// ordinary-expression or source-option evidence refines them into closed `GateType`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionActualType {
    Hole,
    Primitive(BuiltinType),
    Tuple(Vec<Self>),
    Record(Vec<SourceOptionActualRecordField>),
    Arrow {
        parameter: Box<Self>,
        result: Box<Self>,
    },
    List(Box<Self>),
    Map {
        key: Box<Self>,
        value: Box<Self>,
    },
    Set(Box<Self>),
    Option(Box<Self>),
    Result {
        error: Box<Self>,
        value: Box<Self>,
    },
    Validation {
        error: Box<Self>,
        value: Box<Self>,
    },
    Signal(Box<Self>),
    Task {
        error: Box<Self>,
        value: Box<Self>,
    },
    Domain {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
    OpaqueItem {
        item: ItemId,
        name: String,
        arguments: Vec<Self>,
    },
    OpaqueImport {
        import: ImportId,
        name: String,
        arguments: Vec<Self>,
        definition: Option<Box<ImportTypeDefinition>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionActualRecordField {
    pub(crate) name: String,
    pub(crate) ty: SourceOptionActualType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionTypeSurface {
    Contract,
    Expression,
}

impl SourceOptionExpectedType {
    pub(crate) fn from_resolved(module: &Module, ty: &ResolvedSourceContractType) -> Option<Self> {
        match ty {
            ResolvedSourceContractType::Builtin(
                builtin @ (BuiltinType::Int
                | BuiltinType::Float
                | BuiltinType::Decimal
                | BuiltinType::BigInt
                | BuiltinType::Bool
                | BuiltinType::Text
                | BuiltinType::Unit
                | BuiltinType::Bytes),
            ) => Some(Self::Primitive(*builtin)),
            ResolvedSourceContractType::Builtin(_) => None,
            ResolvedSourceContractType::ContractParameter(parameter) => {
                Some(Self::ContractParameter(*parameter))
            }
            ResolvedSourceContractType::Item(item) => Some(Self::Named(
                SourceOptionNamedType::from_item(module, *item, Vec::new())?,
            )),
            ResolvedSourceContractType::Apply { callee, arguments } => match callee {
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::List) => Some(Self::List(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Map) => Some(Self::Map {
                    key: Box::new(Self::from_resolved(module, arguments.first()?)?),
                    value: Box::new(Self::from_resolved(module, arguments.get(1)?)?),
                }),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Set) => Some(Self::Set(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(BuiltinType::Signal) => Some(Self::Signal(
                    Box::new(Self::from_resolved(module, arguments.first()?)?),
                )),
                ResolvedSourceTypeConstructor::Builtin(_) => None,
                ResolvedSourceTypeConstructor::Item(item) => {
                    let arguments = arguments
                        .iter()
                        .map(|argument| Self::from_resolved(module, argument))
                        .collect::<Option<Vec<_>>>()?;
                    Some(Self::Named(SourceOptionNamedType::from_item(
                        module, *item, arguments,
                    )?))
                }
            },
        }
    }

    pub(crate) fn from_hir_type(
        module: &Module,
        ty: TypeId,
        substitutions: &HashMap<TypeParameterId, SourceOptionExpectedType>,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match &module.types()[ty].kind {
            TypeKind::RecordTransform { .. } => None,
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Builtin(
                    builtin @ (BuiltinType::Int
                    | BuiltinType::Float
                    | BuiltinType::Decimal
                    | BuiltinType::BigInt
                    | BuiltinType::Bool
                    | BuiltinType::Text
                    | BuiltinType::Unit
                    | BuiltinType::Bytes),
                )) => Some(Self::Primitive(*builtin)),
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    substitutions.get(parameter).cloned()
                }
                ResolutionState::Resolved(TypeResolution::Item(item)) => Some(Self::Named(
                    SourceOptionNamedType::from_item(module, *item, Vec::new())?,
                )),
                ResolutionState::Resolved(TypeResolution::Builtin(_))
                | ResolutionState::Resolved(TypeResolution::Import(_))
                | ResolutionState::Unresolved => None,
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &module.types()[*callee].kind else {
                    return None;
                };
                match reference.resolution.as_ref() {
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List)) => {
                        Some(Self::List(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Map))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Map {
                            key: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Set))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Set(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Signal)) => {
                        Some(Self::Signal(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Option(Box::new(Self::from_hir_type(
                            module,
                            *arguments.first(),
                            substitutions,
                            surface,
                        )?)))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Result))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Result {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Validation))
                        if surface == SourceOptionTypeSurface::Expression =>
                    {
                        Some(Self::Validation {
                            error: Box::new(Self::from_hir_type(
                                module,
                                *arguments.first(),
                                substitutions,
                                surface,
                            )?),
                            value: Box::new(Self::from_hir_type(
                                module,
                                *arguments.iter().nth(1)?,
                                substitutions,
                                surface,
                            )?),
                        })
                    }
                    ResolutionState::Resolved(TypeResolution::Item(item)) => {
                        let arguments = arguments
                            .iter()
                            .map(|argument| {
                                Self::from_hir_type(module, *argument, substitutions, surface)
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some(Self::Named(SourceOptionNamedType::from_item(
                            module, *item, arguments,
                        )?))
                    }
                    ResolutionState::Resolved(TypeResolution::Builtin(_))
                    | ResolutionState::Resolved(TypeResolution::TypeParameter(_))
                    | ResolutionState::Resolved(TypeResolution::Import(_))
                    | ResolutionState::Unresolved => None,
                }
            }
            TypeKind::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .copied()
                    .map(|element| Self::from_hir_type(module, element, substitutions, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.label.text().to_owned(),
                            ty: Self::from_hir_type(module, field.ty, substitutions, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            TypeKind::Arrow { .. } => None,
        }
    }

    pub(crate) fn from_gate_type(
        module: &Module,
        ty: &GateType,
        surface: SourceOptionTypeSurface,
    ) -> Option<Self> {
        match ty {
            GateType::Primitive(builtin) => Some(Self::Primitive(*builtin)),
            GateType::TypeParameter { .. } => None,
            GateType::Tuple(elements) => Some(Self::Tuple(
                elements
                    .iter()
                    .map(|element| Self::from_gate_type(module, element, surface))
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::Record(fields) => Some(Self::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(SourceOptionExpectedRecordField {
                            name: field.name.clone(),
                            ty: Self::from_gate_type(module, &field.ty, surface)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            GateType::List(element) => Some(Self::List(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Map { key, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Map {
                    key: Box::new(Self::from_gate_type(module, key, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Set(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Set(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Signal(element) => Some(Self::Signal(Box::new(Self::from_gate_type(
                module, element, surface,
            )?))),
            GateType::Option(element) if surface == SourceOptionTypeSurface::Expression => Some(
                Self::Option(Box::new(Self::from_gate_type(module, element, surface)?)),
            ),
            GateType::Result { error, value } if surface == SourceOptionTypeSurface::Expression => {
                Some(Self::Result {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Validation { error, value }
                if surface == SourceOptionTypeSurface::Expression =>
            {
                Some(Self::Validation {
                    error: Box::new(Self::from_gate_type(module, error, surface)?),
                    value: Box::new(Self::from_gate_type(module, value, surface)?),
                })
            }
            GateType::Domain {
                item, arguments, ..
            }
            | GateType::OpaqueItem {
                item, arguments, ..
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| Self::from_gate_type(module, argument, surface))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Named(SourceOptionNamedType::from_item(
                    module, *item, arguments,
                )?))
            }
            GateType::Arrow { .. }
            | GateType::Map { .. }
            | GateType::Set(_)
            | GateType::Option(_)
            | GateType::Result { .. }
            | GateType::Validation { .. }
            | GateType::Task { .. }
            | GateType::OpaqueImport { .. } => None,
        }
    }

    pub(crate) fn is_signal_contract(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn matches_named_item(&self, item: ItemId) -> bool {
        matches!(self, Self::Named(named) if named.item == item)
    }

    pub(crate) fn as_named(&self) -> Option<&SourceOptionNamedType> {
        let Self::Named(named) = self else {
            return None;
        };
        Some(named)
    }
}

impl SourceOptionActualType {
    pub(crate) fn is_signal(&self) -> bool {
        matches!(self, Self::Signal(_))
    }

    pub(crate) fn from_gate_type(ty: &GateType) -> Self {
        match ty {
            GateType::Primitive(builtin) => Self::Primitive(*builtin),
            GateType::TypeParameter { .. } => Self::Hole,
            GateType::Tuple(elements) => {
                Self::Tuple(elements.iter().map(Self::from_gate_type).collect())
            }
            GateType::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|field| SourceOptionActualRecordField {
                        name: field.name.clone(),
                        ty: Self::from_gate_type(&field.ty),
                    })
                    .collect(),
            ),
            GateType::Arrow { parameter, result } => Self::Arrow {
                parameter: Box::new(Self::from_gate_type(parameter)),
                result: Box::new(Self::from_gate_type(result)),
            },
            GateType::List(element) => Self::List(Box::new(Self::from_gate_type(element))),
            GateType::Map { key, value } => Self::Map {
                key: Box::new(Self::from_gate_type(key)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Set(element) => Self::Set(Box::new(Self::from_gate_type(element))),
            GateType::Option(element) => Self::Option(Box::new(Self::from_gate_type(element))),
            GateType::Result { error, value } => Self::Result {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Validation { error, value } => Self::Validation {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Signal(element) => Self::Signal(Box::new(Self::from_gate_type(element))),
            GateType::Task { error, value } => Self::Task {
                error: Box::new(Self::from_gate_type(error)),
                value: Box::new(Self::from_gate_type(value)),
            },
            GateType::Domain {
                item,
                name,
                arguments,
            } => Self::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
            GateType::OpaqueItem {
                item,
                name,
                arguments,
            } => Self::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
            },
            GateType::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Self::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments.iter().map(Self::from_gate_type).collect(),
                definition: definition.clone(),
            },
        }
    }

    pub(crate) fn to_gate_type(&self) -> Option<GateType> {
        match self {
            Self::Hole => None,
            Self::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
            Self::Tuple(elements) => Some(GateType::Tuple(
                elements
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Record(fields) => Some(GateType::Record(
                fields
                    .iter()
                    .map(|field| {
                        Some(GateRecordField {
                            name: field.name.clone(),
                            ty: field.ty.to_gate_type()?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            )),
            Self::Arrow { parameter, result } => Some(GateType::Arrow {
                parameter: Box::new(parameter.to_gate_type()?),
                result: Box::new(result.to_gate_type()?),
            }),
            Self::List(element) => Some(GateType::List(Box::new(element.to_gate_type()?))),
            Self::Map { key, value } => Some(GateType::Map {
                key: Box::new(key.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Set(element) => Some(GateType::Set(Box::new(element.to_gate_type()?))),
            Self::Option(element) => Some(GateType::Option(Box::new(element.to_gate_type()?))),
            Self::Result { error, value } => Some(GateType::Result {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Validation { error, value } => Some(GateType::Validation {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Signal(element) => Some(GateType::Signal(Box::new(element.to_gate_type()?))),
            Self::Task { error, value } => Some(GateType::Task {
                error: Box::new(error.to_gate_type()?),
                value: Box::new(value.to_gate_type()?),
            }),
            Self::Domain {
                item,
                name,
                arguments,
            } => Some(GateType::Domain {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
            Self::OpaqueItem {
                item,
                name,
                arguments,
            } => Some(GateType::OpaqueItem {
                item: *item,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
            }),
            Self::OpaqueImport {
                import,
                name,
                arguments,
                definition,
            } => Some(GateType::OpaqueImport {
                import: *import,
                name: name.clone(),
                arguments: arguments
                    .iter()
                    .map(Self::to_gate_type)
                    .collect::<Option<Vec<_>>>()?,
                definition: definition.clone(),
            }),
        }
    }

    pub(crate) fn unify(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Hole, actual) | (actual, Self::Hole) => Some(actual.clone()),
            (Self::Primitive(left), Self::Primitive(right)) if left == right => {
                Some(Self::Primitive(*left))
            }
            (Self::Tuple(left), Self::Tuple(right)) if left.len() == right.len() => {
                Some(Self::Tuple(
                    left.iter()
                        .zip(right)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                ))
            }
            (Self::Record(left), Self::Record(right)) if left.len() == right.len() => {
                let right_fields = right
                    .iter()
                    .map(|field| (field.name.as_str(), field))
                    .collect::<HashMap<_, _>>();
                let mut fields = Vec::with_capacity(left.len());
                for left in left {
                    let right = right_fields.get(left.name.as_str())?;
                    fields.push(SourceOptionActualRecordField {
                        name: left.name.clone(),
                        ty: left.ty.unify(&right.ty)?,
                    });
                }
                Some(Self::Record(fields))
            }
            (
                Self::Arrow {
                    parameter: left_parameter,
                    result: left_result,
                },
                Self::Arrow {
                    parameter: right_parameter,
                    result: right_result,
                },
            ) => Some(Self::Arrow {
                parameter: Box::new(left_parameter.unify(right_parameter)?),
                result: Box::new(left_result.unify(right_result)?),
            }),
            (Self::List(left), Self::List(right)) => Some(Self::List(Box::new(left.unify(right)?))),
            (
                Self::Map {
                    key: left_key,
                    value: left_value,
                },
                Self::Map {
                    key: right_key,
                    value: right_value,
                },
            ) => Some(Self::Map {
                key: Box::new(left_key.unify(right_key)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Set(left), Self::Set(right)) => Some(Self::Set(Box::new(left.unify(right)?))),
            (Self::Option(left), Self::Option(right)) => {
                Some(Self::Option(Box::new(left.unify(right)?)))
            }
            (
                Self::Result {
                    error: left_error,
                    value: left_value,
                },
                Self::Result {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Result {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Validation {
                    error: left_error,
                    value: left_value,
                },
                Self::Validation {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Validation {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (Self::Signal(left), Self::Signal(right)) => {
                Some(Self::Signal(Box::new(left.unify(right)?)))
            }
            (
                Self::Task {
                    error: left_error,
                    value: left_value,
                },
                Self::Task {
                    error: right_error,
                    value: right_value,
                },
            ) => Some(Self::Task {
                error: Box::new(left_error.unify(right_error)?),
                value: Box::new(left_value.unify(right_value)?),
            }),
            (
                Self::Domain {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::Domain {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::Domain {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            (
                Self::OpaqueItem {
                    item: left_item,
                    name,
                    arguments: left_arguments,
                },
                Self::OpaqueItem {
                    item: right_item,
                    arguments: right_arguments,
                    ..
                },
            ) if left_item == right_item && left_arguments.len() == right_arguments.len() => {
                Some(Self::OpaqueItem {
                    item: *left_item,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                })
            }
            (
                Self::OpaqueImport {
                    import: left_import,
                    name,
                    arguments: left_arguments,
                    definition,
                },
                Self::OpaqueImport {
                    import: right_import,
                    arguments: right_arguments,
                    ..
                },
            ) if left_import == right_import && left_arguments.len() == right_arguments.len() => {
                Some(Self::OpaqueImport {
                    import: *left_import,
                    name: name.clone(),
                    arguments: left_arguments
                        .iter()
                        .zip(right_arguments)
                        .map(|(left, right)| left.unify(right))
                        .collect::<Option<Vec<_>>>()?,
                    definition: definition.clone(),
                })
            }
            _ => None,
        }
    }
}

impl fmt::Display for SourceOptionActualType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hole => write!(f, "_"),
            Self::Primitive(builtin) => write!(f, "{}", builtin_type_name(*builtin)),
            Self::Tuple(elements) => {
                write!(f, "(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{element}")?;
                }
                write!(f, ")")
            }
            Self::Record(fields) => {
                write!(f, "{{ ")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field.name, field.ty)?;
                }
                write!(f, " }}")
            }
            Self::Arrow { parameter, result } => write!(f, "{parameter} -> {result}"),
            Self::List(element) => write!(f, "List {element}"),
            Self::Map { key, value } => write!(f, "Map {key} {value}"),
            Self::Set(element) => write!(f, "Set {element}"),
            Self::Option(element) => write!(f, "Option {element}"),
            Self::Result { error, value } => write!(f, "Result {error} {value}"),
            Self::Validation { error, value } => write!(f, "Validation {error} {value}"),
            Self::Signal(element) => write!(f, "Signal {element}"),
            Self::Task { error, value } => write!(f, "Task {error} {value}"),
            Self::Domain {
                name, arguments, ..
            }
            | Self::OpaqueItem {
                name, arguments, ..
            }
            | Self::OpaqueImport {
                name, arguments, ..
            } => {
                if arguments.is_empty() {
                    write!(f, "{name}")
                } else {
                    write!(f, "{name}")?;
                    for argument in arguments {
                        write!(f, " {argument}")?;
                    }
                    Ok(())
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionNamedType {
    pub(crate) item: ItemId,
    pub(crate) name: String,
    pub(crate) kind: SourceOptionNamedKind,
    pub(crate) arguments: Vec<SourceOptionExpectedType>,
}

impl SourceOptionNamedType {
    pub(crate) fn from_item(
        module: &Module,
        item: ItemId,
        arguments: Vec<SourceOptionExpectedType>,
    ) -> Option<Self> {
        let item_ref = &module.items()[item];
        let kind = match item_ref {
            Item::Domain(_) => SourceOptionNamedKind::Domain,
            Item::Type(_) => SourceOptionNamedKind::Type,
            Item::Value(_)
            | Item::Function(_)
            | Item::Signal(_)
            | Item::Class(_)
            | Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_)
            | Item::Hoist(_) => return None,
        };
        Some(Self {
            item,
            name: item_type_name(item_ref),
            kind,
            arguments,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceOptionNamedKind {
    Domain,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptionConstructorActual {
    pub(crate) parent_item: ItemId,
    pub(crate) parent_name: String,
    pub(crate) constructor_name: String,
    pub(crate) field_types: Vec<TypeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceOptionTypeBindings {
    pub(crate) parameters: HashMap<SourceTypeParameter, SourceOptionActualType>,
}

impl SourceOptionTypeBindings {
    pub(crate) fn parameter(
        &self,
        parameter: SourceTypeParameter,
    ) -> Option<&SourceOptionActualType> {
        self.parameters.get(&parameter)
    }

    pub(crate) fn parameter_gate_type(&self, parameter: SourceTypeParameter) -> Option<GateType> {
        self.parameter(parameter)?.to_gate_type()
    }

    pub(crate) fn bind_or_match_actual(
        &mut self,
        parameter: SourceTypeParameter,
        actual: &SourceOptionActualType,
    ) -> bool {
        match self.parameters.entry(parameter) {
            Entry::Occupied(mut entry) => {
                let Some(unified) = entry.get().unify(actual) else {
                    return false;
                };
                entry.insert(unified);
                true
            }
            Entry::Vacant(entry) => {
                entry.insert(actual.clone());
                true
            }
        }
    }
}

pub(crate) fn source_option_contract_parameters(
    expected: &SourceOptionExpectedType,
) -> Vec<SourceTypeParameter> {
    pub(crate) fn collect(
        expected: &SourceOptionExpectedType,
        parameters: &mut Vec<SourceTypeParameter>,
    ) {
        match expected {
            SourceOptionExpectedType::Primitive(_) => {}
            SourceOptionExpectedType::Tuple(elements) => {
                for element in elements {
                    collect(element, parameters);
                }
            }
            SourceOptionExpectedType::Record(fields) => {
                for field in fields {
                    collect(&field.ty, parameters);
                }
            }
            SourceOptionExpectedType::List(element)
            | SourceOptionExpectedType::Set(element)
            | SourceOptionExpectedType::Signal(element)
            | SourceOptionExpectedType::Option(element) => collect(element, parameters),
            SourceOptionExpectedType::Map { key, value }
            | SourceOptionExpectedType::Result { error: key, value }
            | SourceOptionExpectedType::Validation { error: key, value } => {
                collect(key, parameters);
                collect(value, parameters);
            }
            SourceOptionExpectedType::Named(named) => {
                for argument in &named.arguments {
                    collect(argument, parameters);
                }
            }
            SourceOptionExpectedType::ContractParameter(parameter) => {
                if !parameters.contains(parameter) {
                    parameters.push(*parameter);
                }
            }
        }
    }

    let mut parameters = Vec::new();
    collect(expected, &mut parameters);
    parameters
}

pub(crate) fn source_option_unresolved_contract_parameters(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Vec<SourceTypeParameter> {
    source_option_contract_parameters(expected)
        .into_iter()
        .filter(|parameter| bindings.parameter_gate_type(*parameter).is_none())
        .collect()
}

pub(crate) fn source_option_contract_parameter_phrase(
    parameters: &[SourceTypeParameter],
) -> String {
    let quoted = parameters
        .iter()
        .map(|parameter| format!("`{parameter}`"))
        .collect::<Vec<_>>();
    match quoted.as_slice() {
        [] => "contract parameters".to_owned(),
        [single] => format!("contract parameter {single}"),
        [left, right] => format!("contract parameters {left} and {right}"),
        _ => format!(
            "contract parameters {}, and {}",
            quoted[..quoted.len() - 1].join(", "),
            quoted
                .last()
                .expect("non-empty parameter list should keep a tail"),
        ),
    }
}

pub(crate) fn source_option_expected_to_gate_type(
    expected: &SourceOptionExpectedType,
    bindings: &SourceOptionTypeBindings,
) -> Option<GateType> {
    match expected {
        SourceOptionExpectedType::Primitive(builtin) => Some(GateType::Primitive(*builtin)),
        SourceOptionExpectedType::Tuple(elements) => Some(GateType::Tuple(
            elements
                .iter()
                .map(|element| source_option_expected_to_gate_type(element, bindings))
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::Record(fields) => Some(GateType::Record(
            fields
                .iter()
                .map(|field| {
                    Some(GateRecordField {
                        name: field.name.clone(),
                        ty: source_option_expected_to_gate_type(&field.ty, bindings)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        SourceOptionExpectedType::List(element) => Some(GateType::List(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Map { key, value } => Some(GateType::Map {
            key: Box::new(source_option_expected_to_gate_type(key, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Set(element) => Some(GateType::Set(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Signal(element) => Some(GateType::Signal(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Option(element) => Some(GateType::Option(Box::new(
            source_option_expected_to_gate_type(element, bindings)?,
        ))),
        SourceOptionExpectedType::Result { error, value } => Some(GateType::Result {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Validation { error, value } => Some(GateType::Validation {
            error: Box::new(source_option_expected_to_gate_type(error, bindings)?),
            value: Box::new(source_option_expected_to_gate_type(value, bindings)?),
        }),
        SourceOptionExpectedType::Named(named) => {
            let arguments = named
                .arguments
                .iter()
                .map(|argument| source_option_expected_to_gate_type(argument, bindings))
                .collect::<Option<Vec<_>>>()?;
            Some(match named.kind {
                SourceOptionNamedKind::Domain => GateType::Domain {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
                SourceOptionNamedKind::Type => GateType::OpaqueItem {
                    item: named.item,
                    name: named.name.clone(),
                    arguments,
                },
            })
        }
        SourceOptionExpectedType::ContractParameter(parameter) => {
            bindings.parameter_gate_type(*parameter)
        }
    }
}

pub(crate) fn source_option_expected_matches_actual_type(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if !expected.is_signal_contract() {
        if let SourceOptionActualType::Signal(inner) = actual {
            return source_option_expected_matches_actual_type_inner(expected, inner, bindings);
        }
    }

    source_option_expected_matches_actual_type_inner(expected, actual, bindings)
}

pub(crate) fn source_option_expected_matches_actual_type_inner(
    expected: &SourceOptionExpectedType,
    actual: &SourceOptionActualType,
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    match (expected, actual) {
        (SourceOptionExpectedType::ContractParameter(parameter), _) => {
            bindings.bind_or_match_actual(*parameter, actual)
        }
        (SourceOptionExpectedType::Primitive(_), SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Primitive(expected),
            SourceOptionActualType::Primitive(actual),
        ) => expected == actual,
        (SourceOptionExpectedType::Tuple(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Tuple(expected), SourceOptionActualType::Tuple(actual)) => {
            source_option_expected_args_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Record(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Record(expected), SourceOptionActualType::Record(actual)) => {
            source_option_expected_record_fields_match(expected, actual, bindings)
        }
        (SourceOptionExpectedType::List(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::List(expected), SourceOptionActualType::List(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Map { .. }, SourceOptionActualType::Hole) => true,
        (
            SourceOptionExpectedType::Map { key, value },
            SourceOptionActualType::Map {
                key: actual_key,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(key, actual_key, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Set(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Set(expected), SourceOptionActualType::Set(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Signal(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Signal(expected), SourceOptionActualType::Signal(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Option(_), SourceOptionActualType::Hole) => true,
        (SourceOptionExpectedType::Option(expected), SourceOptionActualType::Option(actual)) => {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        }
        (SourceOptionExpectedType::Result { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Result { error, value },
            SourceOptionActualType::Result {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Validation { error, value }, SourceOptionActualType::Hole) => {
            let _ = (error, value);
            true
        }
        (
            SourceOptionExpectedType::Validation { error, value },
            SourceOptionActualType::Validation {
                error: actual_error,
                value: actual_value,
            },
        ) => {
            source_option_expected_matches_actual_type(error, actual_error, bindings)
                && source_option_expected_matches_actual_type(value, actual_value, bindings)
        }
        (SourceOptionExpectedType::Named(expected), SourceOptionActualType::Hole) => {
            let _ = expected;
            true
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::Domain {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Domain && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        (
            SourceOptionExpectedType::Named(expected),
            SourceOptionActualType::OpaqueItem {
                item, arguments, ..
            },
        ) if expected.kind == SourceOptionNamedKind::Type && expected.item == *item => {
            source_option_expected_args_match(&expected.arguments, arguments, bindings)
        }
        _ => false,
    }
}

pub(crate) fn source_option_expected_args_match(
    expected: &[SourceOptionExpectedType],
    actual: &[SourceOptionActualType],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(expected, actual)| {
            source_option_expected_matches_actual_type(expected, actual, bindings)
        })
}

pub(crate) fn source_option_expected_record_fields_match(
    expected: &[SourceOptionExpectedRecordField],
    actual: &[SourceOptionActualRecordField],
    bindings: &mut SourceOptionTypeBindings,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    let actual_fields = actual
        .iter()
        .map(|field| (field.name.as_str(), &field.ty))
        .collect::<HashMap<_, _>>();
    expected.iter().all(|field| {
        actual_fields
            .get(field.name.as_str())
            .is_some_and(|actual| {
                source_option_expected_matches_actual_type(&field.ty, actual, bindings)
            })
    })
}
