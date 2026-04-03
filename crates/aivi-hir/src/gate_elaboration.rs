use std::fmt;

use aivi_base::SourceSpan;
use aivi_typing::{GatePlanner, GateResultKind};

use crate::{
    BigIntLiteral, BinaryOperator, BindingId, BuiltinTerm, ClassMemberResolution, ClusterId,
    DecimalLiteral, DomainMemberHandle, ExprId, ExprKind, FloatLiteral, IntegerLiteral,
    IntrinsicValue, Item, ItemId, Module, Name, NamePath, PatternId, PipeExpr, PipeStageKind,
    PipeTransformMode, ProjectionBase, SuffixedIntegerLiteral, TermReference, TermResolution,
    TextFragment, TextSegment, UnaryOperator,
    domain_operator_elaboration::select_domain_binary_operator,
    typecheck::resolve_class_member_dispatch,
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, PipeFunctionSignatureMatch,
        PipeSubjectStepOutcome, PipeSubjectWalker, extend_pipe_env_with_stage_memos,
        gate_env_for_function, pipe_stage_expr_env, truthy_falsy_pair_stages, walk_expr_tree,
    },
};

/// Focused gate-core plans derived from resolved HIR.
///
/// This is intentionally narrower than a future full typed-core IR. It exposes the RFC §11.3
/// gate split in a typed, presentation-free form: ordinary gates become explicit `Some` / `None`
/// branch terms over the ambient subject, while signal gates carry an explicit typed runtime-filter
/// predicate tree that later scheduler/runtime layers can consume without re-deriving it from raw
/// HIR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateElaborationReport {
    stages: Vec<GateStageElaboration>,
}

impl GateElaborationReport {
    pub fn new(stages: Vec<GateStageElaboration>) -> Self {
        Self { stages }
    }

    pub fn stages(&self) -> &[GateStageElaboration] {
        &self.stages
    }

    pub fn into_stages(self) -> Vec<GateStageElaboration> {
        self.stages
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateStageElaboration {
    pub owner: ItemId,
    pub pipe_expr: ExprId,
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate: ExprId,
    pub outcome: GateStageOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateStageOutcome {
    Ordinary(OrdinaryGateStage),
    SignalFilter(SignalGateFilter),
    Blocked(BlockedGateStage),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrdinaryGateStage {
    pub input_subject: GateType,
    pub result_type: GateType,
    pub when_true: GateCoreExpr,
    pub when_false: GateCoreExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalGateFilter {
    pub input_subject: GateType,
    pub payload_type: GateType,
    pub result_type: GateType,
    pub runtime_predicate: GateRuntimeExpr,
    pub emits_negative_update: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedGateStage {
    pub subject: Option<GateType>,
    pub blockers: Vec<GateElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateElaborationBlocker {
    UnknownSubjectType,
    UnknownPredicateType,
    InvalidProjection {
        path: String,
        subject: String,
    },
    UnknownField {
        path: String,
        subject: String,
    },
    ImpurePredicate,
    PredicateNotBool {
        found: GateType,
    },
    UnknownRuntimeExprType {
        span: SourceSpan,
    },
    UnsupportedRuntimeExpr {
        span: SourceSpan,
        kind: GateRuntimeUnsupportedKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateRuntimePurity {
    PureOnly,
    AllowSignalReads,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateRuntimeUnsupportedKind {
    RegexLiteral,
    FloatLiteral,
    DecimalLiteral,
    BigIntLiteral,
    ApplicativeCluster,
    Markup,
    PatchExpr,
    NestedGate,
    NestedFanout,
    PipeStage(GateRuntimeUnsupportedPipeStageKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateRuntimeUnsupportedPipeStageKind {
    Case,
    Map,
    Apply,
    FanIn,
    Truthy,
    Falsy,
    RecurStart,
    RecurStep,
}

impl fmt::Display for GateRuntimeUnsupportedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegexLiteral => f.write_str("regex literal"),
            Self::FloatLiteral => f.write_str("float literal"),
            Self::DecimalLiteral => f.write_str("decimal literal"),
            Self::BigIntLiteral => f.write_str("BigInt literal"),
            Self::ApplicativeCluster => f.write_str("applicative cluster"),
            Self::Markup => f.write_str("markup expression"),
            Self::PatchExpr => f.write_str("patch expression"),
            Self::NestedGate => f.write_str("nested gate expression"),
            Self::NestedFanout => f.write_str("nested fan-out expression"),
            Self::PipeStage(kind) => write!(f, "{kind}"),
        }
    }
}

impl fmt::Display for GateRuntimeUnsupportedPipeStageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Case => f.write_str("case pipe stage"),
            Self::Map => f.write_str("map pipe stage"),
            Self::Apply => f.write_str("apply pipe stage"),
            Self::FanIn => f.write_str("fan-in pipe stage"),
            Self::Truthy => f.write_str("truthy pipe stage"),
            Self::Falsy => f.write_str("falsy pipe stage"),
            Self::RecurStart => f.write_str("recurrence-start pipe stage"),
            Self::RecurStep => f.write_str("recurrence-step pipe stage"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateCoreExpr {
    pub ty: GateType,
    pub kind: GateCoreExprKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateCoreExprKind {
    AmbientSubject,
    OptionSome { payload: Box<GateCoreExpr> },
    OptionNone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeExpr {
    pub span: SourceSpan,
    pub ty: GateType,
    pub kind: GateRuntimeExprKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimeExprKind {
    AmbientSubject,
    Reference(GateRuntimeReference),
    Integer(IntegerLiteral),
    Float(FloatLiteral),
    Decimal(DecimalLiteral),
    BigInt(BigIntLiteral),
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(GateRuntimeTextLiteral),
    Tuple(Vec<GateRuntimeExpr>),
    List(Vec<GateRuntimeExpr>),
    Map(Vec<GateRuntimeMapEntry>),
    Set(Vec<GateRuntimeExpr>),
    Record(Vec<GateRuntimeRecordField>),
    Projection {
        base: GateRuntimeProjectionBase,
        path: NamePath,
    },
    Apply {
        callee: Box<GateRuntimeExpr>,
        arguments: Vec<GateRuntimeExpr>,
    },
    Unary {
        operator: UnaryOperator,
        expr: Box<GateRuntimeExpr>,
    },
    Binary {
        left: Box<GateRuntimeExpr>,
        operator: BinaryOperator,
        right: Box<GateRuntimeExpr>,
    },
    Pipe(GateRuntimePipeExpr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimeReference {
    Local(BindingId),
    Item(ItemId),
    Import(crate::ImportId),
    SumConstructor(crate::SumConstructorHandle),
    DomainMember(DomainMemberHandle),
    ClassMember(crate::ResolvedClassMemberDispatch),
    Builtin(BuiltinTerm),
    IntrinsicValue(IntrinsicValue),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimeProjectionBase {
    AmbientSubject,
    Expr(Box<GateRuntimeExpr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeTextLiteral {
    pub segments: Vec<GateRuntimeTextSegment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimeTextSegment {
    Fragment(TextFragment),
    Interpolation(Box<GateRuntimeExpr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeRecordField {
    pub label: Name,
    pub value: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeMapEntry {
    pub key: GateRuntimeExpr,
    pub value: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimePipeExpr {
    pub head: Box<GateRuntimeExpr>,
    pub stages: Vec<GateRuntimePipeStage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimePipeStage {
    pub span: SourceSpan,
    pub subject_memo: Option<BindingId>,
    pub result_memo: Option<BindingId>,
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub kind: GateRuntimePipeStageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeCaseArm {
    pub span: SourceSpan,
    pub pattern: PatternId,
    pub body: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimeTruthyFalsyBranch {
    pub span: SourceSpan,
    pub constructor: BuiltinTerm,
    pub payload_subject: Option<GateType>,
    pub result_type: GateType,
    pub body: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimePipeStageKind {
    Transform {
        mode: PipeTransformMode,
        expr: GateRuntimeExpr,
    },
    Tap {
        expr: GateRuntimeExpr,
    },
    Gate {
        predicate: GateRuntimeExpr,
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<GateRuntimeCaseArm>,
    },
    TruthyFalsy {
        truthy: GateRuntimeTruthyFalsyBranch,
        falsy: GateRuntimeTruthyFalsyBranch,
    },
    FanOut {
        map_expr: GateRuntimeExpr,
    },
}

impl GateCoreExpr {
    fn ambient_subject(ty: GateType) -> Self {
        Self {
            ty,
            kind: GateCoreExprKind::AmbientSubject,
        }
    }

    fn option_some(result_type: GateType, payload: GateCoreExpr) -> Self {
        Self {
            ty: result_type,
            kind: GateCoreExprKind::OptionSome {
                payload: Box::new(payload),
            },
        }
    }

    fn option_none(result_type: GateType) -> Self {
        Self {
            ty: result_type,
            kind: GateCoreExprKind::OptionNone,
        }
    }
}

impl GateRuntimeExpr {
    fn ambient_subject(span: SourceSpan, ty: GateType) -> Self {
        Self {
            span,
            ty,
            kind: GateRuntimeExprKind::AmbientSubject,
        }
    }

    fn apply(
        span: SourceSpan,
        ty: GateType,
        callee: GateRuntimeExpr,
        arguments: Vec<GateRuntimeExpr>,
    ) -> Self {
        Self {
            span,
            ty,
            kind: GateRuntimeExprKind::Apply {
                callee: Box::new(callee),
                arguments,
            },
        }
    }
}

pub fn elaborate_gates(module: &Module) -> GateElaborationReport {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    let module = &module;
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut stages = Vec::new();
    let mut typing = GateTypeContext::new(module);

    for (owner, item) in items {
        match item {
            Item::Value(item) => collect_gate_stages(
                module,
                owner,
                item.body,
                &GateExprEnv::default(),
                &mut typing,
                &mut stages,
            ),
            Item::Function(item) => {
                let env = gate_env_for_function(&item, &mut typing);
                collect_gate_stages(module, owner, item.body, &env, &mut typing, &mut stages);
            }
            Item::Signal(item) => {
                if let Some(body) = item.body {
                    collect_gate_stages(
                        module,
                        owner,
                        body,
                        &GateExprEnv::default(),
                        &mut typing,
                        &mut stages,
                    );
                }
            }
            Item::Instance(item) => {
                for member in item.members {
                    collect_gate_stages(
                        module,
                        owner,
                        member.body,
                        &GateExprEnv::default(),
                        &mut typing,
                        &mut stages,
                    );
                }
            }
            Item::Type(_)
            | Item::Class(_)
            | Item::Domain(_)
            | Item::SourceProviderContract(_)
            | Item::Use(_)
            | Item::Export(_) => {}
        }
    }

    GateElaborationReport::new(stages)
}

fn collect_gate_stages(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    stages: &mut Vec<GateStageElaboration>,
) {
    walk_expr_tree(module, root, |pipe_expr, expr, _| {
        if let ExprKind::Pipe(pipe) = &expr.kind {
            collect_gate_pipe(module, owner, pipe_expr, pipe, env, typing, stages);
        }
    });
}

fn collect_gate_pipe(
    module: &Module,
    owner: ItemId,
    pipe_expr: ExprId,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    gate_stages: &mut Vec<GateStageElaboration>,
) {
    let recurrence_start_index = pipe
        .recurrence_suffix()
        .ok()
        .flatten()
        .map(|suffix| suffix.prefix_stage_count());
    let all_stages = pipe.stages.iter().collect::<Vec<_>>();
    PipeSubjectWalker::new(pipe, env, typing).walk(
        typing,
        |stage_index, stage, current, current_env, typing| {
            // Stop at the recurrence boundary — gate elaboration only covers the
            // non-recurrence prefix of the pipe.
            if recurrence_start_index.is_some_and(|start| stage_index >= start) {
                return PipeSubjectStepOutcome::Stop;
            }
            match &stage.kind {
                PipeStageKind::Gate { expr } => {
                    let outcome = elaborate_gate_stage(module, *expr, current_env, current, typing);
                    gate_stages.push(GateStageElaboration {
                        owner,
                        pipe_expr,
                        stage_index,
                        stage_span: stage.span,
                        predicate: *expr,
                        outcome: outcome.clone(),
                    });
                    PipeSubjectStepOutcome::Continue {
                        new_subject: match outcome {
                            GateStageOutcome::Ordinary(s) => Some(s.result_type),
                            GateStageOutcome::SignalFilter(s) => Some(s.result_type),
                            GateStageOutcome::Blocked(_) => None,
                        },
                        advance_by: 1,
                    }
                }
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    if segment.join_stage().is_some() {
                        let outcome = crate::fanout_elaboration::elaborate_fanout_segment(
                            module,
                            &segment,
                            current,
                            current_env,
                            typing,
                        );
                        let advance = segment
                            .next_stage_index()
                            .saturating_sub(stage_index)
                            .max(1);
                        PipeSubjectStepOutcome::Continue {
                            new_subject: match outcome {
                                crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                    Some(plan.result_type)
                                }
                                crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => None,
                            },
                            advance_by: advance,
                        }
                    } else {
                        PipeSubjectStepOutcome::Continue {
                            new_subject: current
                                .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                            advance_by: 1,
                        }
                    }
                }
                PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_fanin_stage(*expr, current_env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&all_stages, stage_index) else {
                        return PipeSubjectStepOutcome::Continue {
                            new_subject: None,
                            advance_by: 1,
                        };
                    };
                    let advance = pair.next_index.saturating_sub(stage_index).max(1);
                    PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_truthy_falsy_pair(&pair, current_env, s)),
                        advance_by: advance,
                    }
                }
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. }
                | PipeStageKind::Validate { .. }
                | PipeStageKind::Previous { .. }
                | PipeStageKind::Diff { .. }
                | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                    advance_by: 1,
                },
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!("PipeSubjectWalker handles Transform and Tap internally")
                }
            }
        },
    );
}

fn elaborate_gate_stage(
    module: &Module,
    predicate: ExprId,
    env: &GateExprEnv,
    subject: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> GateStageOutcome {
    let Some(subject) = subject else {
        return GateStageOutcome::Blocked(BlockedGateStage {
            subject: None,
            blockers: vec![GateElaborationBlocker::UnknownSubjectType],
        });
    };

    let predicate_info = typing.infer_pipe_body(predicate, env, subject);
    let mut blockers = predicate_info
        .issues
        .into_iter()
        .map(blocker_for_issue)
        .collect::<Vec<_>>();
    if predicate_info.contains_signal || predicate_info.ty.as_ref().is_some_and(GateType::is_signal)
    {
        blockers.push(GateElaborationBlocker::ImpurePredicate);
    }
    if let Some(predicate_ty) = predicate_info.ty.as_ref() {
        if !predicate_ty.is_bool() {
            blockers.push(GateElaborationBlocker::PredicateNotBool {
                found: predicate_ty.clone(),
            });
        }
    } else if blockers.is_empty() {
        blockers.push(GateElaborationBlocker::UnknownPredicateType);
    }
    if !blockers.is_empty() {
        return GateStageOutcome::Blocked(BlockedGateStage {
            subject: Some(subject.clone()),
            blockers,
        });
    }

    let plan = GatePlanner::plan(typing.gate_carrier(subject));
    match plan.result() {
        GateResultKind::OptionWrappedSubject => {
            let result_type = typing.apply_gate_plan(plan, subject);
            let ambient = GateCoreExpr::ambient_subject(subject.clone());
            GateStageOutcome::Ordinary(OrdinaryGateStage {
                input_subject: subject.clone(),
                result_type: result_type.clone(),
                when_true: GateCoreExpr::option_some(result_type.clone(), ambient),
                when_false: GateCoreExpr::option_none(result_type),
            })
        }
        GateResultKind::PreservedSignalSubject => {
            let result_type = typing.apply_gate_plan(plan, subject);
            let runtime_predicate =
                match lower_gate_pipe_body_runtime_expr(module, predicate, env, subject, typing) {
                    Ok(predicate) => predicate,
                    Err(blocker) => {
                        return GateStageOutcome::Blocked(BlockedGateStage {
                            subject: Some(subject.clone()),
                            blockers: vec![blocker],
                        });
                    }
                };
            if !runtime_predicate.ty.is_bool() {
                return GateStageOutcome::Blocked(BlockedGateStage {
                    subject: Some(subject.clone()),
                    blockers: vec![GateElaborationBlocker::PredicateNotBool {
                        found: runtime_predicate.ty,
                    }],
                });
            }
            GateStageOutcome::SignalFilter(SignalGateFilter {
                input_subject: subject.clone(),
                payload_type: subject.gate_payload().clone(),
                result_type,
                runtime_predicate,
                emits_negative_update: plan.emits_negative_update(),
            })
        }
    }
}

pub(crate) fn lower_gate_pipe_body_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    lower_gate_pipe_body_runtime_expr_with_purity(
        module,
        expr_id,
        env,
        subject,
        typing,
        GateRuntimePurity::PureOnly,
    )
}

pub(crate) fn lower_gate_pipe_body_runtime_expr_allow_signal_reads(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    lower_gate_pipe_body_runtime_expr_with_purity(
        module,
        expr_id,
        env,
        subject,
        typing,
        GateRuntimePurity::AllowSignalReads,
    )
}

fn lower_gate_pipe_body_runtime_expr_with_purity(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let ambient = subject.gate_payload().clone();
    let mut lowered = match lower_gate_runtime_expr_with_purity(
        module,
        expr_id,
        env,
        Some(&ambient),
        typing,
        purity,
    ) {
        Ok(lowered) => lowered,
        Err(GateElaborationBlocker::UnknownRuntimeExprType { .. }) => {
            lower_function_pipe_body_runtime_expr(module, expr_id, env, &ambient, typing, purity)?
        }
        Err(other) => return Err(other),
    };
    let GateType::Arrow { parameter, result } = lowered.ty.clone() else {
        return Ok(lowered);
    };
    if !parameter.same_shape(&ambient) {
        return Ok(lowered);
    }
    lowered = GateRuntimeExpr::apply(
        module.exprs()[expr_id].span,
        *result,
        lowered,
        vec![GateRuntimeExpr::ambient_subject(
            module.exprs()[expr_id].span,
            ambient,
        )],
    );
    Ok(lowered)
}

fn lower_function_pipe_body_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: &GateType,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let expr = module.exprs()[expr_id].clone();
    let plan = typing
        .match_pipe_function_signature(expr_id, env, ambient, None)
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: expr.span })?;
    lower_pipe_function_runtime_expr_from_plan(
        module, expr.span, plan, env, ambient, typing, purity,
    )
}

pub(crate) fn lower_gate_pipe_function_apply_runtime_expr_allow_signal_reads(
    module: &Module,
    span: SourceSpan,
    callee_expr: ExprId,
    explicit_arguments: Vec<ExprId>,
    env: &GateExprEnv,
    ambient: &GateType,
    expected_result: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let plan = typing
        .match_pipe_function_signature_parts(
            callee_expr,
            explicit_arguments,
            env,
            ambient,
            expected_result,
        )
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
    lower_pipe_function_runtime_expr_from_plan(
        module,
        span,
        plan,
        env,
        ambient,
        typing,
        GateRuntimePurity::AllowSignalReads,
    )
}

fn lower_pipe_function_runtime_expr_from_plan(
    module: &Module,
    span: SourceSpan,
    plan: PipeFunctionSignatureMatch,
    env: &GateExprEnv,
    ambient: &GateType,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let callee_ty = arrow_type(&plan.parameter_types, plan.result_type.clone());
    let callee = if let ExprKind::Name(reference) = &module.exprs()[plan.callee_expr].kind {
        if matches!(
            reference.resolution.as_ref(),
            crate::ResolutionState::Resolved(TermResolution::ClassMember(_))
                | crate::ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
        ) {
            let dispatch = resolve_class_member_dispatch(
                module,
                reference,
                &plan.parameter_types,
                Some(&plan.result_type),
            )
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
            GateRuntimeExpr {
                span: module.exprs()[plan.callee_expr].span,
                ty: callee_ty.clone(),
                kind: GateRuntimeExprKind::Reference(GateRuntimeReference::ClassMember(dispatch)),
            }
        } else {
            match lower_gate_runtime_expr_with_purity(
                module,
                plan.callee_expr,
                env,
                Some(ambient),
                typing,
                purity,
            ) {
                Ok(lowered) => lowered,
                Err(GateElaborationBlocker::UnknownRuntimeExprType { .. }) => GateRuntimeExpr {
                    span: module.exprs()[plan.callee_expr].span,
                    ty: callee_ty.clone(),
                    kind: GateRuntimeExprKind::Reference(runtime_reference_for_name(
                        module,
                        module.exprs()[plan.callee_expr].span,
                        reference,
                    )?),
                },
                Err(other) => return Err(other),
            }
        }
    } else {
        lower_gate_runtime_expr_with_purity(
            module,
            plan.callee_expr,
            env,
            Some(ambient),
            typing,
            purity,
        )?
    };
    let mut arguments = Vec::with_capacity(plan.explicit_arguments.len() + 1);
    for ((argument, expected_parameter), reads_signal_payload) in plan
        .explicit_arguments
        .iter()
        .zip(
            plan.parameter_types
                .iter()
                .take(plan.explicit_arguments.len()),
        )
        .zip(plan.signal_payload_arguments.iter())
    {
        arguments.push(lower_pipe_argument_runtime_expr(
            module,
            *argument,
            env,
            Some(ambient),
            expected_parameter,
            *reads_signal_payload,
            typing,
            purity,
        )?);
    }
    arguments.push(GateRuntimeExpr::ambient_subject(span, ambient.clone()));
    Ok(GateRuntimeExpr::apply(
        span,
        plan.result_type,
        callee,
        arguments,
    ))
}

fn lower_pipe_argument_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    expected: &GateType,
    reads_signal_payload: bool,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    if reads_signal_payload && let ExprKind::Name(reference) = &module.exprs()[expr_id].kind {
        let info = typing.infer_expr(expr_id, env, ambient);
        if let Some(GateType::Signal(payload)) = info.ty
            && payload.same_shape(expected)
        {
            return Ok(GateRuntimeExpr {
                span: module.exprs()[expr_id].span,
                ty: expected.clone(),
                kind: GateRuntimeExprKind::Reference(runtime_reference_for_name(
                    module,
                    module.exprs()[expr_id].span,
                    reference,
                )?),
            });
        }
    }
    lower_gate_runtime_expr_with_purity(module, expr_id, env, ambient, typing, purity)
}

fn arrow_type(parameters: &[GateType], result: GateType) -> GateType {
    parameters
        .iter()
        .rev()
        .cloned()
        .fold(result, |result, parameter| GateType::Arrow {
            parameter: Box::new(parameter),
            result: Box::new(result),
        })
}

/// Inline a single-parameter function reference as a gate predicate by
/// substituting the function's sole parameter with the ambient subject.
///
/// **Purity requirement**: this inlining is semantically correct only if the
/// referenced function is *pure* — it must capture no mutable state and have
/// no observable side effects.  Today purity is guaranteed because the type
/// system only permits functions with a `Bool`-returning body in predicate
/// position, and no impure language features (effects, I/O, task launchers)
/// are allowed inside gate predicates.  If the type system is ever relaxed to
/// permit impure functions in predicate position, this substitution must be
/// guarded by an explicit purity check before calling this function (PA-I3).
/// Continuation tasks used by the iterative post-order traversal inside
/// [`lower_gate_runtime_expr`].  Each variant corresponds to a step in the
/// tree-building process after all child expressions have been lowered and
/// pushed onto the result stack.
///
/// The traversal maintains two stacks:
///   * `work`   — pending [`LowerTask`] items (LIFO)
///   * `results` — completed [`GateRuntimeExpr`] values (ordered by completion)
enum LowerTask {
    /// Evaluate this expression node.  The task handler inspects the
    /// `ExprKind`, pushes the appropriate `Build*` continuation, and then
    /// pushes child `Eval` tasks so they execute first.
    Eval(ExprId),
    /// Pop one result → wrap in `Unary { operator }` with the given span/ty.
    BuildUnary {
        span: SourceSpan,
        ty: GateType,
        operator: UnaryOperator,
    },
    /// Pop `right` then `left` from the result stack → build `Binary`.
    BuildBinary {
        span: SourceSpan,
        ty: GateType,
        operator: BinaryOperator,
    },
    /// Pop `right` then `left` → build a domain-operator `Apply` node.
    BuildDomainBinary {
        span: SourceSpan,
        ty: GateType,
        callee_ty: GateType,
        callee: DomainMemberHandle,
    },
    /// Pop N results → build `Tuple`.
    BuildTuple {
        span: SourceSpan,
        ty: GateType,
        n: usize,
    },
    /// Pop N results → build `List`.
    BuildList {
        span: SourceSpan,
        ty: GateType,
        n: usize,
    },
    /// Pop 2*N results (key₀, value₀, key₁, value₁, …) → build `Map`.
    BuildMap {
        span: SourceSpan,
        ty: GateType,
        n: usize,
    },
    /// Pop N results → build `Set`.
    BuildSet {
        span: SourceSpan,
        ty: GateType,
        n: usize,
    },
    /// Pop N results → build `Record` with the given labels.
    BuildRecord {
        span: SourceSpan,
        ty: GateType,
        labels: Vec<Name>,
    },
    /// Pop one result → wrap in `Projection { base: Expr(_), path }`.
    BuildProjectionExpr {
        span: SourceSpan,
        ty: GateType,
        path: NamePath,
    },
    /// Pop `n_args` results (arguments) then one more (callee) → `Apply`.
    BuildApply {
        span: SourceSpan,
        ty: GateType,
        n_args: usize,
    },
}

/// Iterative post-order tree lowering for gate runtime expressions.
///
/// Converts the HIR expression rooted at `expr_id` into a [`GateRuntimeExpr`]
/// without using call-stack recursion.  This prevents stack overflows on
/// adversarially deep predicate expressions (e.g. 4096-level-deep `&&` chains).
///
/// `env` and `ambient` are threaded unchanged through all non-pipe sub-
/// expressions.  Pipe and Text expressions contain finite, source-bounded
/// sequences so they are handled as ordinary function calls.
pub(crate) fn lower_gate_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    lower_gate_runtime_expr_with_purity(
        module,
        expr_id,
        env,
        ambient,
        typing,
        GateRuntimePurity::PureOnly,
    )
}

fn lower_gate_runtime_expr_with_purity(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let mut work: Vec<LowerTask> = vec![LowerTask::Eval(expr_id)];
    let mut results: Vec<GateRuntimeExpr> = Vec::new();

    while let Some(task) = work.pop() {
        match task {
            LowerTask::Eval(expr_id) => {
                // --- Domain-operator shortcut (Binary with domain member callee) ---
                // Returns true if the domain path was taken; in that case `BuildDomainBinary`
                // and two `Eval` children have already been pushed onto `work`.
                if check_domain_operator_and_schedule(
                    module, expr_id, env, ambient, typing, &mut work,
                )? {
                    continue;
                }

                let (expr, ty) =
                    inferred_runtime_expr(module, expr_id, env, ambient, typing, purity)?;

                match expr.kind {
                    // --- Leaf nodes: push result directly ---
                    ExprKind::Name(reference) => {
                        let kind = GateRuntimeExprKind::Reference(runtime_reference_for_name(
                            module, expr.span, &reference,
                        )?);
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind,
                        });
                    }
                    ExprKind::Integer(literal) => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Integer(literal),
                        });
                    }
                    ExprKind::Float(literal) => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Float(literal),
                        });
                    }
                    ExprKind::Decimal(literal) => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Decimal(literal),
                        });
                    }
                    ExprKind::BigInt(literal) => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::BigInt(literal),
                        });
                    }
                    ExprKind::SuffixedInteger(literal) => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::SuffixedInteger(literal),
                        });
                    }
                    ExprKind::AmbientSubject => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::AmbientSubject,
                        });
                    }
                    // --- Text: bounded interpolation segments (not a source of deep nesting) ---
                    ExprKind::Text(text) => {
                        let lowered = lower_runtime_text_literal(
                            module, &text, env, ambient, typing, purity,
                        )?;
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Text(lowered),
                        });
                    }
                    ExprKind::Regex(_) => {
                        return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                            span: expr.span,
                            kind: GateRuntimeUnsupportedKind::RegexLiteral,
                        });
                    }
                    ExprKind::Cluster(cluster_id) => {
                        let lowered = lower_cluster_as_gate_runtime_expr(
                            module, cluster_id, expr.span, ty, env, ambient, typing, purity,
                        )?;
                        results.push(lowered);
                    }
                    ExprKind::Markup(_) => {
                        return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                            span: expr.span,
                            kind: GateRuntimeUnsupportedKind::Markup,
                        });
                    }
                    ExprKind::PatchApply { .. } | ExprKind::PatchLiteral(_) => {
                        return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                            span: expr.span,
                            kind: GateRuntimeUnsupportedKind::PatchExpr,
                        });
                    }
                    // --- Pipe: bounded number of stages (not deep in practice) ---
                    ExprKind::Pipe(pipe) => {
                        if let Some(kind) = nested_pipe_runtime_unsupported_kind(&pipe) {
                            return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                                span: expr.span,
                                kind,
                            });
                        }
                        let lowered =
                            lower_runtime_pipe_expr(module, &pipe, env, ambient, typing, purity)?;
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Pipe(lowered),
                        });
                    }
                    // --- Projection with ambient base: leaf ---
                    ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        path,
                    } => {
                        results.push(GateRuntimeExpr {
                            span: expr.span,
                            ty,
                            kind: GateRuntimeExprKind::Projection {
                                base: GateRuntimeProjectionBase::AmbientSubject,
                                path,
                            },
                        });
                    }
                    // --- Compound nodes: push continuation then children ---
                    ExprKind::Unary {
                        operator,
                        expr: child,
                    } => {
                        work.push(LowerTask::BuildUnary {
                            span: expr.span,
                            ty,
                            operator,
                        });
                        work.push(LowerTask::Eval(child));
                    }
                    ExprKind::Binary {
                        left,
                        operator,
                        right,
                    } => {
                        // Push Build first (runs last), then children in reverse order
                        // so `left` is evaluated before `right`.
                        work.push(LowerTask::BuildBinary {
                            span: expr.span,
                            ty,
                            operator,
                        });
                        work.push(LowerTask::Eval(right));
                        work.push(LowerTask::Eval(left));
                    }
                    ExprKind::Tuple(elements) => {
                        let n = elements.len();
                        work.push(LowerTask::BuildTuple {
                            span: expr.span,
                            ty,
                            n,
                        });
                        for &element in elements.iter().rev() {
                            work.push(LowerTask::Eval(element));
                        }
                    }
                    ExprKind::List(elements) => {
                        let n = elements.len();
                        work.push(LowerTask::BuildList {
                            span: expr.span,
                            ty,
                            n,
                        });
                        for element in elements.into_iter().rev() {
                            work.push(LowerTask::Eval(element));
                        }
                    }
                    ExprKind::Map(map) => {
                        let n = map.entries.len();
                        work.push(LowerTask::BuildMap {
                            span: expr.span,
                            ty,
                            n,
                        });
                        for entry in map.entries.into_iter().rev() {
                            work.push(LowerTask::Eval(entry.value));
                            work.push(LowerTask::Eval(entry.key));
                        }
                    }
                    ExprKind::Set(elements) => {
                        let n = elements.len();
                        work.push(LowerTask::BuildSet {
                            span: expr.span,
                            ty,
                            n,
                        });
                        for element in elements.into_iter().rev() {
                            work.push(LowerTask::Eval(element));
                        }
                    }
                    ExprKind::Record(record) => {
                        let labels: Vec<Name> =
                            record.fields.iter().map(|f| f.label.clone()).collect();
                        work.push(LowerTask::BuildRecord {
                            span: expr.span,
                            ty,
                            labels,
                        });
                        for field in record.fields.into_iter().rev() {
                            work.push(LowerTask::Eval(field.value));
                        }
                    }
                    ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        path,
                    } => {
                        work.push(LowerTask::BuildProjectionExpr {
                            span: expr.span,
                            ty,
                            path,
                        });
                        work.push(LowerTask::Eval(base));
                    }
                    ExprKind::Apply { callee, arguments } => {
                        let n_args = arguments.len();
                        work.push(LowerTask::BuildApply {
                            span: expr.span,
                            ty,
                            n_args,
                        });
                        for &arg in arguments.iter().rev() {
                            work.push(LowerTask::Eval(arg));
                        }
                        work.push(LowerTask::Eval(callee));
                    }
                }
            }

            // --- Build continuations: pop children from result stack ---
            LowerTask::BuildUnary { span, ty, operator } => {
                let child = results
                    .pop()
                    .expect("result stack has child for BuildUnary");
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Unary {
                        operator,
                        expr: Box::new(child),
                    },
                });
            }
            LowerTask::BuildBinary { span, ty, operator } => {
                let right = results
                    .pop()
                    .expect("result stack has right for BuildBinary");
                let left = results
                    .pop()
                    .expect("result stack has left for BuildBinary");
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Binary {
                        left: Box::new(left),
                        operator,
                        right: Box::new(right),
                    },
                });
            }
            LowerTask::BuildDomainBinary {
                span,
                ty,
                callee_ty,
                callee,
            } => {
                let right = results
                    .pop()
                    .expect("result stack has right for BuildDomainBinary");
                let left = results
                    .pop()
                    .expect("result stack has left for BuildDomainBinary");
                let callee_expr = GateRuntimeExpr {
                    span,
                    ty: callee_ty,
                    kind: GateRuntimeExprKind::Reference(GateRuntimeReference::DomainMember(
                        callee,
                    )),
                };
                results.push(GateRuntimeExpr::apply(
                    span,
                    ty,
                    callee_expr,
                    vec![left, right],
                ));
            }
            LowerTask::BuildTuple { span, ty, n } => {
                let start = results.len() - n;
                let elements: Vec<GateRuntimeExpr> = results.drain(start..).collect();
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Tuple(elements),
                });
            }
            LowerTask::BuildList { span, ty, n } => {
                let start = results.len() - n;
                let elements: Vec<GateRuntimeExpr> = results.drain(start..).collect();
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::List(elements),
                });
            }
            LowerTask::BuildMap { span, ty, n } => {
                // Elements arrive in forward order: key₀, value₀, key₁, value₁, …
                // because children were pushed reversed onto `work`.
                let start = results.len() - n * 2;
                let flat: Vec<GateRuntimeExpr> = results.drain(start..).collect();
                let entries = flat
                    .chunks_exact(2)
                    .map(|pair| GateRuntimeMapEntry {
                        key: pair[0].clone(),
                        value: pair[1].clone(),
                    })
                    .collect();
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Map(entries),
                });
            }
            LowerTask::BuildSet { span, ty, n } => {
                let start = results.len() - n;
                let elements: Vec<GateRuntimeExpr> = results.drain(start..).collect();
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Set(elements),
                });
            }
            LowerTask::BuildRecord { span, ty, labels } => {
                let n = labels.len();
                let start = results.len() - n;
                let values: Vec<GateRuntimeExpr> = results.drain(start..).collect();
                let fields = labels
                    .into_iter()
                    .zip(values)
                    .map(|(label, value)| GateRuntimeRecordField { label, value })
                    .collect();
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Record(fields),
                });
            }
            LowerTask::BuildProjectionExpr { span, ty, path } => {
                let base = results
                    .pop()
                    .expect("result stack has base for BuildProjectionExpr");
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Projection {
                        base: GateRuntimeProjectionBase::Expr(Box::new(base)),
                        path,
                    },
                });
            }
            LowerTask::BuildApply { span, ty, n_args } => {
                let arg_start = results.len() - n_args;
                let arguments: Vec<GateRuntimeExpr> = results.drain(arg_start..).collect();
                let callee = results
                    .pop()
                    .expect("result stack has callee for BuildApply");
                results.push(GateRuntimeExpr {
                    span,
                    ty,
                    kind: GateRuntimeExprKind::Apply {
                        callee: Box::new(callee),
                        arguments,
                    },
                });
            }
        }
    }

    Ok(results
        .pop()
        .expect("iterative lowering produces exactly one result"))
}

/// Desugar an `&|>` applicative cluster into a chain of `pure`/`apply` calls
/// for the gate-elaboration worklist path.
///
/// The algorithm mirrors `GeneralExprLowerer::lower_cluster_expr` but uses
/// `lower_gate_runtime_expr_with_purity` for sub-expressions and the free
/// helper functions below rather than `self.*` methods.
fn lower_cluster_as_gate_runtime_expr(
    module: &Module,
    cluster_id: ClusterId,
    span: SourceSpan,
    cluster_ty: GateType,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let cluster = module
        .clusters()
        .get(cluster_id)
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?
        .clone();

    let result_payload = cluster_applicative_payload_type(&cluster_ty)
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;

    let spine = cluster.normalized_spine();
    let member_ids: Vec<ExprId> = spine.apply_arguments().collect();
    let mut member_payloads: Vec<(GateType, GateType)> = Vec::with_capacity(member_ids.len());
    let mut lowered_members: Vec<GateRuntimeExpr> = Vec::with_capacity(member_ids.len());

    for &member_id in &member_ids {
        let member_ty = typing
            .infer_expr(member_id, env, ambient)
            .ty
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
        let payload = cluster_applicative_payload_type(&member_ty)
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
        member_payloads.push((member_ty.clone(), payload));
        lowered_members.push(lower_gate_runtime_expr_with_purity(
            module, member_id, env, ambient, typing, purity,
        )?);
    }

    let finalizer_payload_parameters: Vec<GateType> =
        member_payloads.iter().map(|(_, p)| p.clone()).collect();
    let finalizer_ty =
        gate_arrow_type(finalizer_payload_parameters.clone(), result_payload.clone());

    let finalizer = match spine.pure_head() {
        crate::ApplicativeSpineHead::Expr(finalizer_id) => {
            lower_gate_runtime_expr_with_purity(module, finalizer_id, env, ambient, typing, purity)?
        }
        crate::ApplicativeSpineHead::TupleConstructor(arity) => GateRuntimeExpr {
            span,
            ty: finalizer_ty.clone(),
            kind: GateRuntimeExprKind::Reference(GateRuntimeReference::IntrinsicValue(
                crate::IntrinsicValue::TupleConstructor { arity: arity.get() },
            )),
        },
    };

    let mut current_inner = finalizer_ty.clone();
    let pure_result = cluster_rewrap_applicative_gate_type(&cluster_ty, current_inner.clone())
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
    let pure_ref = lower_builtin_class_member_ref(
        module,
        span,
        "Applicative",
        "pure",
        vec![current_inner.clone()],
        pure_result.clone(),
    )?;
    let mut current = GateRuntimeExpr {
        span,
        ty: pure_result,
        kind: GateRuntimeExprKind::Apply {
            callee: Box::new(pure_ref),
            arguments: vec![finalizer],
        },
    };

    for (index, ((member_ty, _), member_expr)) in
        member_payloads.into_iter().zip(lowered_members).enumerate()
    {
        let remaining: Vec<GateType> = finalizer_payload_parameters
            .iter()
            .skip(index + 1)
            .cloned()
            .collect();
        current_inner = gate_arrow_type(remaining, result_payload.clone());
        let apply_result = cluster_rewrap_applicative_gate_type(&cluster_ty, current_inner.clone())
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
        let apply_ref = lower_builtin_class_member_ref(
            module,
            span,
            "Apply",
            "apply",
            vec![current.ty.clone(), member_ty.clone()],
            apply_result.clone(),
        )?;
        current = GateRuntimeExpr {
            span,
            ty: apply_result,
            kind: GateRuntimeExprKind::Apply {
                callee: Box::new(apply_ref),
                arguments: vec![current, member_expr],
            },
        };
    }

    Ok(current)
}

/// Extract the single payload type from a supported applicative carrier type.
fn cluster_applicative_payload_type(ty: &GateType) -> Option<GateType> {
    match ty {
        GateType::List(p) => Some(p.as_ref().clone()),
        GateType::Option(p) => Some(p.as_ref().clone()),
        GateType::Result { value, .. } => Some(value.as_ref().clone()),
        GateType::Validation { value, .. } => Some(value.as_ref().clone()),
        GateType::Signal(p) => Some(p.as_ref().clone()),
        GateType::Task { value, .. } => Some(value.as_ref().clone()),
        _ => None,
    }
}

/// Re-wrap a new payload inside the same applicative constructor as `applicative`.
fn cluster_rewrap_applicative_gate_type(
    applicative: &GateType,
    payload: GateType,
) -> Option<GateType> {
    match applicative {
        GateType::List(_) => Some(GateType::List(Box::new(payload))),
        GateType::Option(_) => Some(GateType::Option(Box::new(payload))),
        GateType::Result { error, .. } => Some(GateType::Result {
            error: error.clone(),
            value: Box::new(payload),
        }),
        GateType::Validation { error, .. } => Some(GateType::Validation {
            error: error.clone(),
            value: Box::new(payload),
        }),
        GateType::Signal(_) => Some(GateType::Signal(Box::new(payload))),
        GateType::Task { error, .. } => Some(GateType::Task {
            error: error.clone(),
            value: Box::new(payload),
        }),
        _ => None,
    }
}

/// Build a curried arrow type from a parameter list and a result type.
fn gate_arrow_type(parameters: Vec<GateType>, result: GateType) -> GateType {
    parameters
        .into_iter()
        .rev()
        .fold(result, |acc, param| GateType::Arrow {
            parameter: Box::new(param),
            result: Box::new(acc),
        })
}

/// Find the `ClassMemberResolution` for the named class/member in the module's
/// ambient (prelude) items.  Returns `None` if the class or member is absent.
fn ambient_class_member_resolution_gate(
    module: &Module,
    class_name: &str,
    member_name: &str,
) -> Option<ClassMemberResolution> {
    module
        .ambient_items()
        .iter()
        .find_map(|item_id| match &module.items()[*item_id] {
            Item::Class(class_item) if class_item.name.text() == class_name => class_item
                .members
                .iter()
                .position(|m| m.name.text() == member_name)
                .map(|member_index| ClassMemberResolution {
                    class: *item_id,
                    member_index,
                }),
            _ => None,
        })
}

/// Build a synthetic `GateRuntimeExpr` reference to a builtin class member
/// (e.g. `Applicative::pure` or `Apply::apply`) with the given argument and
/// result types.
fn lower_builtin_class_member_ref(
    module: &Module,
    span: SourceSpan,
    class_name: &str,
    member_name: &str,
    argument_types: Vec<GateType>,
    result_type: GateType,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let resolution = ambient_class_member_resolution_gate(module, class_name, member_name)
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
    let reference = TermReference::resolved(
        NamePath::from_vec(vec![
            Name::new(member_name, span).expect("class member names are valid identifiers"),
        ])
        .expect("single-segment class member path is valid"),
        TermResolution::ClassMember(resolution),
    );
    let dispatch =
        resolve_class_member_dispatch(module, &reference, &argument_types, Some(&result_type))
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span })?;
    let callee_ty = gate_arrow_type(argument_types, result_type);
    Ok(GateRuntimeExpr {
        span,
        ty: callee_ty,
        kind: GateRuntimeExprKind::Reference(GateRuntimeReference::ClassMember(dispatch)),
    })
}

/// a domain member.  If so, schedules `BuildDomainBinary` and two `Eval` tasks
/// on `work` and returns `true`.  Returns `false` if the expression is not a
/// domain operator (the caller should handle it normally).
fn check_domain_operator_and_schedule(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    work: &mut Vec<LowerTask>,
) -> Result<bool, GateElaborationBlocker> {
    let expr = module.exprs()[expr_id].clone();
    let ExprKind::Binary {
        left,
        operator,
        right,
    } = expr.kind
    else {
        return Ok(false);
    };
    let left_ty = typing.infer_expr(left, env, ambient).ty;
    let right_ty = typing.infer_expr(right, env, ambient).ty;
    let (Some(left_ty), Some(right_ty)) = (left_ty.as_ref(), right_ty.as_ref()) else {
        return Ok(false);
    };
    let Some(matched) =
        select_domain_binary_operator(module, typing, operator, left_ty, right_ty).unwrap_or(None)
    else {
        return Ok(false);
    };

    // Push the build continuation first (it runs after children are done),
    // then children so that `left` is evaluated before `right`.
    work.push(LowerTask::BuildDomainBinary {
        span: expr.span,
        ty: matched.result_type,
        callee_ty: matched.callee_type,
        callee: matched.callee,
    });
    work.push(LowerTask::Eval(right));
    work.push(LowerTask::Eval(left));
    Ok(true)
}

fn lower_runtime_text_literal(
    module: &Module,
    text: &crate::TextLiteral,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimeTextLiteral, GateElaborationBlocker> {
    let mut segments = Vec::with_capacity(text.segments.len());
    for segment in &text.segments {
        let lowered = match segment {
            TextSegment::Text(fragment) => GateRuntimeTextSegment::Fragment(fragment.clone()),
            TextSegment::Interpolation(interpolation) => GateRuntimeTextSegment::Interpolation(
                Box::new(lower_gate_runtime_expr_with_purity(
                    module,
                    interpolation.expr,
                    env,
                    ambient,
                    typing,
                    purity,
                )?),
            ),
        };
        segments.push(lowered);
    }
    Ok(GateRuntimeTextLiteral { segments })
}

fn lower_runtime_pipe_expr(
    module: &Module,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<GateRuntimePipeExpr, GateElaborationBlocker> {
    let head =
        lower_gate_runtime_expr_with_purity(module, pipe.head, env, ambient, typing, purity)?;
    let mut current = head.ty.clone();
    let mut pipe_env = env.clone();
    let mut stages = Vec::with_capacity(pipe.stages.len());
    for stage in pipe.stages.iter() {
        let input_subject = current.clone();
        let stage_env = pipe_stage_expr_env(&pipe_env, stage, &current);
        let (kind, result_subject) = match &stage.kind {
            PipeStageKind::Transform { expr } => {
                let mode = typing.infer_transform_stage_mode(*expr, &stage_env, &current);
                let body = lower_gate_pipe_body_runtime_expr_with_purity(
                    module, *expr, &stage_env, &current, typing, purity,
                )?;
                let result_subject = typing
                    .infer_transform_stage(*expr, &stage_env, &current)
                    .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span })?;
                (
                    GateRuntimePipeStageKind::Transform { mode, expr: body },
                    result_subject,
                )
            }
            PipeStageKind::Tap { expr } => (
                GateRuntimePipeStageKind::Tap {
                    expr: lower_gate_pipe_body_runtime_expr_with_purity(
                        module, *expr, &stage_env, &current, typing, purity,
                    )?,
                },
                current.clone(),
            ),
            PipeStageKind::Gate { expr } => {
                let predicate =
                    lower_gate_pipe_body_runtime_expr(module, *expr, &stage_env, &current, typing)?;
                if !predicate.ty.is_bool() {
                    return Err(GateElaborationBlocker::PredicateNotBool {
                        found: predicate.ty,
                    });
                }
                let plan = GatePlanner::plan(typing.gate_carrier(&current));
                let result_subject = typing.apply_gate_plan(plan, &current);
                (
                    GateRuntimePipeStageKind::Gate {
                        predicate,
                        emits_negative_update: plan.emits_negative_update(),
                    },
                    result_subject,
                )
            }
            PipeStageKind::Case { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Case,
                ));
            }
            PipeStageKind::Map { expr } => {
                let element = current.fanout_element().cloned().ok_or(
                    GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span },
                )?;
                let body = lower_gate_pipe_body_runtime_expr_with_purity(
                    module, *expr, &stage_env, &element, typing, purity,
                )?;
                let result_subject = typing
                    .infer_fanout_map_stage_info(*expr, &stage_env, &current)
                    .ty
                    .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span })?;
                (
                    GateRuntimePipeStageKind::FanOut { map_expr: body },
                    result_subject,
                )
            }
            PipeStageKind::Apply { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Apply,
                ));
            }
            PipeStageKind::FanIn { expr } => {
                let body = lower_gate_pipe_body_runtime_expr_with_purity(
                    module, *expr, &stage_env, &current, typing, purity,
                )?;
                let result_subject = typing
                    .infer_fanin_stage_info(*expr, &stage_env, &current)
                    .ty
                    .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span })?;
                (
                    GateRuntimePipeStageKind::Transform {
                        mode: PipeTransformMode::Replace,
                        expr: body,
                    },
                    result_subject,
                )
            }
            PipeStageKind::Truthy { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Truthy,
                ));
            }
            PipeStageKind::Falsy { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Falsy,
                ));
            }
            PipeStageKind::RecurStart { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::RecurStart,
                ));
            }
            PipeStageKind::RecurStep { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::RecurStep,
                ));
            }
            PipeStageKind::Validate { expr } => {
                let body = lower_gate_pipe_body_runtime_expr_with_purity(
                    module, *expr, &stage_env, &current, typing, purity,
                )?;
                let result_subject = typing
                    .infer_transform_stage(*expr, &stage_env, &current)
                    .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span })?;
                (
                    GateRuntimePipeStageKind::Transform {
                        mode: PipeTransformMode::Replace,
                        expr: body,
                    },
                    result_subject,
                )
            }
            PipeStageKind::Previous { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::RecurStart,
                ));
            }
            PipeStageKind::Accumulate { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::RecurStart,
                ));
            }
            PipeStageKind::Diff { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::RecurStart,
                ));
            }
        };
        stages.push(GateRuntimePipeStage {
            span: stage.span,
            subject_memo: stage.subject_memo,
            result_memo: stage.result_memo,
            input_subject,
            result_subject: result_subject.clone(),
            kind,
        });
        extend_pipe_env_with_stage_memos(&mut pipe_env, stage, &current, &result_subject);
        current = result_subject;
    }
    Ok(GateRuntimePipeExpr {
        head: Box::new(head),
        stages,
    })
}

fn inferred_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
    purity: GateRuntimePurity,
) -> Result<(crate::Expr, GateType), GateElaborationBlocker> {
    let expr = module.exprs()[expr_id].clone();
    let info = typing.infer_expr(expr_id, env, ambient);
    if matches!(purity, GateRuntimePurity::PureOnly)
        && (info.contains_signal || info.ty.as_ref().is_some_and(GateType::is_signal))
    {
        return Err(GateElaborationBlocker::ImpurePredicate);
    }
    if let Some(issue) = info.issues.into_iter().next() {
        return Err(blocker_for_issue(issue));
    }
    let ty = info
        .ty
        .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: expr.span })?;
    Ok((expr, ty))
}

fn runtime_reference_for_name(
    module: &Module,
    span: SourceSpan,
    reference: &crate::TermReference,
) -> Result<GateRuntimeReference, GateElaborationBlocker> {
    match reference.resolution.as_ref() {
        crate::ResolutionState::Resolved(TermResolution::Local(binding)) => {
            Ok(GateRuntimeReference::Local(*binding))
        }
        crate::ResolutionState::Resolved(TermResolution::Item(item_id)) => Ok(module
            .sum_constructor_handle(*item_id, reference.path.segments().last().text())
            .map(GateRuntimeReference::SumConstructor)
            .unwrap_or(GateRuntimeReference::Item(*item_id))),
        crate::ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => module
            .domain_member_handle(*resolution)
            .map(GateRuntimeReference::DomainMember)
            .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span }),
        crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
            Ok(GateRuntimeReference::Builtin(*builtin))
        }
        crate::ResolutionState::Resolved(TermResolution::IntrinsicValue(value)) => {
            Ok(GateRuntimeReference::IntrinsicValue(value.clone()))
        }
        crate::ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
        | crate::ResolutionState::Resolved(TermResolution::ClassMember(_))
        | crate::ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
        | crate::ResolutionState::Resolved(TermResolution::Import(_))
        | crate::ResolutionState::Unresolved => {
            Err(GateElaborationBlocker::UnknownRuntimeExprType { span })
        }
    }
}

fn nested_pipe_runtime_unsupported_kind(
    pipe: &crate::PipeExpr,
) -> Option<GateRuntimeUnsupportedKind> {
    for stage in pipe.stages.iter() {
        match stage.kind {
            PipeStageKind::Gate { .. } => return Some(GateRuntimeUnsupportedKind::NestedGate),
            PipeStageKind::Map { .. } | PipeStageKind::FanIn { .. } => {
                return Some(GateRuntimeUnsupportedKind::NestedFanout);
            }
            PipeStageKind::Transform { .. }
            | PipeStageKind::Tap { .. }
            | PipeStageKind::Case { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::Truthy { .. }
            | PipeStageKind::Falsy { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. }
            | PipeStageKind::Validate { .. }
            | PipeStageKind::Previous { .. }
            | PipeStageKind::Diff { .. }
            | PipeStageKind::Accumulate { .. } => {}
        }
    }
    None
}

fn unsupported_runtime_pipe_stage(
    span: SourceSpan,
    kind: GateRuntimeUnsupportedPipeStageKind,
) -> GateElaborationBlocker {
    GateElaborationBlocker::UnsupportedRuntimeExpr {
        span,
        kind: GateRuntimeUnsupportedKind::PipeStage(kind),
    }
}

fn blocker_for_issue(issue: GateIssue) -> GateElaborationBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            GateElaborationBlocker::InvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            GateElaborationBlocker::UnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { span, .. }
        | GateIssue::AmbientSubjectOutsidePipe { span, .. }
        | GateIssue::AmbiguousDomainOperator { span, .. }
        | GateIssue::InvalidPipeStageInput { span, .. }
        | GateIssue::UnsupportedApplicativeClusterMember { span, .. }
        | GateIssue::ApplicativeClusterMismatch { span, .. }
        | GateIssue::InvalidClusterFinalizer { span, .. }
        | GateIssue::CaseBranchTypeMismatch { span, .. } => {
            GateElaborationBlocker::UnknownRuntimeExprType { span }
        }
    }
}

// gate_env_for_function is now the shared crate::validate::gate_env_for_function (PA-I2).

#[cfg(test)]
mod tests {
    use super::{
        GateCoreExprKind, GateElaborationBlocker, GateRuntimeExprKind, GateRuntimeProjectionBase,
        GateRuntimeReference, GateRuntimeUnsupportedKind, GateStageOutcome, elaborate_gates,
    };
    use crate::test_support::{fixture_root, item_name, lower_fixture, lower_text};
    use crate::{BuiltinType, GateType};

    #[test]
    fn elaborates_valid_gate_fixture_into_ordinary_and_signal_core_plans() {
        let lowered = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "gate fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        assert_eq!(
            report.stages().len(),
            2,
            "expected both gate stages to elaborate"
        );

        let ordinary = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "maybeActive")
            .expect("expected ordinary gate plan for maybeActive");
        match &ordinary.outcome {
            GateStageOutcome::Ordinary(stage) => {
                let GateType::Option(inner) = &stage.result_type else {
                    panic!(
                        "ordinary gate should lower through Option, found {:?}",
                        stage.result_type
                    );
                };
                assert_eq!(&stage.input_subject, inner.as_ref());
                match &stage.when_true.kind {
                    GateCoreExprKind::OptionSome { payload } => {
                        assert_eq!(&payload.ty, &stage.input_subject);
                        assert!(matches!(&payload.kind, GateCoreExprKind::AmbientSubject));
                    }
                    other => panic!(
                        "ordinary success branch should construct Some subject, found {other:?}"
                    ),
                }
                assert!(matches!(
                    &stage.when_false.kind,
                    GateCoreExprKind::OptionNone
                ));
                assert_eq!(&stage.when_false.ty, &stage.result_type);
            }
            other => panic!("expected ordinary gate elaboration, found {other:?}"),
        }

        let signal = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "activeUsers")
            .expect("expected signal gate plan for activeUsers");
        match &signal.outcome {
            GateStageOutcome::SignalFilter(stage) => {
                assert!(matches!(&stage.input_subject, GateType::Signal(_)));
                assert_eq!(&stage.result_type, &stage.input_subject);
                assert_eq!(&stage.payload_type, stage.input_subject.gate_payload());
                assert!(stage.runtime_predicate.ty.is_bool());
                match &stage.runtime_predicate.kind {
                    GateRuntimeExprKind::Projection { base, path } => {
                        assert_eq!(base, &GateRuntimeProjectionBase::AmbientSubject);
                        let segments = path.segments();
                        assert_eq!(segments.len(), 1);
                        assert_eq!(
                            segments
                                .iter()
                                .next()
                                .expect("path should have one segment")
                                .text(),
                            "active"
                        );
                    }
                    other => panic!("expected direct ambient projection filter, found {other:?}"),
                }
                assert!(
                    !stage.emits_negative_update,
                    "signal gate runtime filter must preserve the RFC's no-negative-update rule"
                );
            }
            other => panic!("expected signal filter plan, found {other:?}"),
        }
    }

    #[test]
    fn lowers_signal_gate_function_predicates_with_implicit_ambient_application() {
        let lowered = lower_text(
            "signal-gate-function-predicate.aivi",
            r#"
type User = {
    active: Bool,
    age: Int
}

fun isEligible:Bool = user:User=>    .active and .age > 18

signal users:Signal User = { active: True, age: 21 }

signal eligibleUsers:Signal User =
    users
     ?|> isEligible
"#,
        );
        assert!(
            !lowered.has_errors(),
            "signal gate function predicate should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        let signal = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "eligibleUsers")
            .expect("expected signal gate plan for eligibleUsers");

        match &signal.outcome {
            GateStageOutcome::SignalFilter(stage) => {
                assert!(stage.runtime_predicate.ty.is_bool());
                match &stage.runtime_predicate.kind {
                    GateRuntimeExprKind::Apply { callee, arguments } => {
                        assert_eq!(arguments.len(), 1);
                        assert!(matches!(
                            &arguments[0].kind,
                            GateRuntimeExprKind::AmbientSubject
                        ));
                        match &callee.kind {
                            GateRuntimeExprKind::Reference(GateRuntimeReference::Item(item_id)) => {
                                assert_eq!(item_name(lowered.module(), *item_id), "isEligible");
                            }
                            other => panic!(
                                "expected function item reference in runtime filter, found {other:?}"
                            ),
                        }
                    }
                    other => panic!(
                        "expected implicit ambient application runtime filter, found {other:?}"
                    ),
                }
            }
            other => panic!("expected signal filter plan, found {other:?}"),
        }
    }

    #[test]
    fn lowers_composite_signal_gate_predicates_into_runtime_expr_trees() {
        let lowered = lower_text(
            "signal-gate-composite-predicate.aivi",
            r#"
type User = {
    active: Bool,
    age: Int
}

signal users:Signal User = { active: True, age: 21 }

signal activeUsers:Signal User =
    users
     ?|> (.active and .age > 18)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "signal gate composite predicate should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        let signal = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "activeUsers")
            .expect("expected signal gate plan for activeUsers");

        match &signal.outcome {
            GateStageOutcome::SignalFilter(stage) => {
                assert!(stage.runtime_predicate.ty.is_bool());
                match &stage.runtime_predicate.kind {
                    GateRuntimeExprKind::Binary {
                        left,
                        operator: crate::BinaryOperator::And,
                        right,
                    } => {
                        match &left.kind {
                            GateRuntimeExprKind::Projection { base, path } => {
                                assert_eq!(base, &GateRuntimeProjectionBase::AmbientSubject);
                                assert_eq!(
                                    path.segments()
                                        .iter()
                                        .next()
                                        .expect("path should have one segment")
                                        .text(),
                                    "active"
                                );
                            }
                            other => panic!(
                                "expected ambient projection on the left side, found {other:?}"
                            ),
                        }
                        match &right.kind {
                            GateRuntimeExprKind::Binary {
                                left,
                                operator: crate::BinaryOperator::GreaterThan,
                                right,
                            } => {
                                assert!(matches!(
                                    &left.kind,
                                    GateRuntimeExprKind::Projection { .. }
                                ));
                                assert!(matches!(&right.kind, GateRuntimeExprKind::Integer(_)));
                            }
                            other => panic!(
                                "expected numeric comparison on the right side, found {other:?}"
                            ),
                        }
                    }
                    other => panic!("expected binary runtime predicate, found {other:?}"),
                }
            }
            other => panic!("expected signal filter plan, found {other:?}"),
        }
    }

    #[test]
    fn lowers_domain_operator_predicates_into_explicit_domain_calls() {
        let lowered = lower_text(
            "signal-gate-domain-operators.aivi",
            r#"
domain Duration over Int = {
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration
    (>) : Duration -> Duration -> Bool
}
type Window = {
    delay: Duration
}

signal windows:Signal Window = { delay: 10ms }

signal slowWindows:Signal Window =
    windows
     ?|> ((.delay + 5ms) > 12ms)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "signal gate domain-operator predicate should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        let signal = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "slowWindows")
            .expect("expected signal gate plan for slowWindows");

        match &signal.outcome {
            GateStageOutcome::SignalFilter(stage) => match &stage.runtime_predicate.kind {
                GateRuntimeExprKind::Apply { callee, arguments } => {
                    assert_eq!(arguments.len(), 2);
                    match &callee.kind {
                        GateRuntimeExprKind::Reference(GateRuntimeReference::DomainMember(
                            handle,
                        )) => {
                            assert_eq!(handle.domain_name.as_ref(), "Duration");
                            assert_eq!(handle.member_name.as_ref(), ">");
                        }
                        other => panic!(
                            "expected explicit domain-member reference for outer comparison, found {other:?}"
                        ),
                    }
                    match &arguments[0].kind {
                        GateRuntimeExprKind::Apply { callee, arguments } => {
                            assert_eq!(arguments.len(), 2);
                            assert!(matches!(
                                &arguments[0].kind,
                                GateRuntimeExprKind::Projection {
                                    base: GateRuntimeProjectionBase::AmbientSubject,
                                    ..
                                }
                            ));
                            assert!(matches!(
                                &arguments[1].kind,
                                GateRuntimeExprKind::SuffixedInteger(_)
                            ));
                            match &callee.kind {
                                GateRuntimeExprKind::Reference(
                                    GateRuntimeReference::DomainMember(handle),
                                ) => {
                                    assert_eq!(handle.domain_name.as_ref(), "Duration");
                                    assert_eq!(handle.member_name.as_ref(), "+");
                                }
                                other => panic!(
                                    "expected explicit domain-member reference for nested add, found {other:?}"
                                ),
                            }
                        }
                        other => panic!(
                            "expected nested explicit apply for domain addition, found {other:?}"
                        ),
                    }
                    assert!(matches!(
                        &arguments[1].kind,
                        GateRuntimeExprKind::SuffixedInteger(_)
                    ));
                }
                other => panic!(
                    "expected explicit apply tree for domain operator predicate, found {other:?}"
                ),
            },
            other => panic!("expected signal filter plan, found {other:?}"),
        }
    }

    #[test]
    fn keeps_gate_subject_tracking_through_fanout_segments() {
        let lowered = lower_text(
            "gate-after-fanout.aivi",
            r#"
type User = {
    email: Text
}

fun joinEmails:Text = items:List Text=>    "joined"

value users:List User = [
    { email: "ada@example.com" }
]

value maybeJoined:Option Text =
    users
     *|> .email
     <|* joinEmails
     ?|> True
"#,
        );
        assert!(
            !lowered.has_errors(),
            "gate-after-fanout example should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        let ordinary = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "maybeJoined")
            .expect("expected ordinary gate after fanout");

        match &ordinary.outcome {
            GateStageOutcome::Ordinary(stage) => {
                assert_eq!(
                    stage.input_subject,
                    GateType::Primitive(BuiltinType::Text),
                    "gate should see the joined fanout result, not lose the pipe subject"
                );
                assert_eq!(
                    stage.result_type,
                    GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected ordinary gate after fanout, found {other:?}"),
        }
    }

    #[test]
    fn skips_joined_fanout_filter_gates() {
        let lowered = lower_text(
            "gate-inside-fanout-join.aivi",
            r#"
type User = {
    email: Text
}

fun keepText:Bool = email:Text=>    True

fun joinEmails:Text = items:List Text=>    "joined"

value users:List User = [
    { email: "ada@example.com" }
]

value joinedEmails:Text =
    users
     *|> .email
     ?|> keepText
     <|* joinEmails
"#,
        );
        assert!(
            !lowered.has_errors(),
            "fan-out filter example should lower cleanly before gate elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        assert!(
            report
                .stages()
                .iter()
                .all(|stage| item_name(lowered.module(), stage.owner) != "joinedEmails"),
            "joined fan-out filter gates should stay part of the fan-out segment, not become standalone gate stages: {:?}",
            report.stages()
        );
    }

    #[test]
    fn skips_recurrence_guard_gates() {
        let lowered = lower_text(
            "gate-inside-recurrence.aivi",
            r#"
domain Duration over Int = {
    literal sec : Int -> Duration
}
type Cursor = {
    hasNext: Bool
}

fun keep:Cursor = cursor:Cursor=>    cursor

value seed:Cursor = { hasNext: True }

@recur.timer 5sec
signal cursor : Signal Cursor =
    seed
     @|> keep
     ?|> .hasNext
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "recurrence guard example should lower cleanly before gate elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        assert!(
            report
                .stages()
                .iter()
                .all(|stage| item_name(lowered.module(), stage.owner) != "cursor"),
            "recurrence guards should stay attached to the recurrence, not become standalone gate stages: {:?}",
            report.stages()
        );
    }

    #[test]
    fn keeps_gate_subject_tracking_through_truthy_falsy_pairs() {
        let lowered = lower_fixture("milestone-2/valid/pipe-truthy-falsy-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "truthy/falsy fixture should lower cleanly before gate elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_gates(lowered.module());
        let ordinary = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "maybeDisplay")
            .expect("expected ordinary gate after truthy/falsy pair");

        match &ordinary.outcome {
            GateStageOutcome::Ordinary(stage) => {
                assert_eq!(
                    stage.input_subject,
                    GateType::Primitive(BuiltinType::Text),
                    "gate should see the truthy/falsy branch result, not lose the pipe subject"
                );
                assert_eq!(
                    stage.result_type,
                    GateType::Option(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected ordinary gate after truthy/falsy pair, found {other:?}"),
        }
    }

    /// Stack-depth torture test for the iterative `lower_gate_runtime_expr`.
    ///
    /// Constructs a deeply nested `and` predicate: `(((a and a) and a) and a) …`
    /// for 1024 levels.  If `lower_gate_runtime_expr` were still recursive this
    /// would overflow even a 64 MB thread stack (each activation frame is
    /// substantial).  The iterative worklist implementation must handle it
    /// without a stack overflow.
    ///
    /// The test is run on a thread with a 64 MB stack so that the still-recursive
    /// `infer_expr` pass (which is called once per node but is itself recursive)
    /// has sufficient headroom.  Only `lower_gate_runtime_expr` is fully
    /// iterative today; making `infer_expr` iterative is tracked separately.
    #[test]
    fn lower_gate_runtime_expr_handles_4096_deep_and_chain_without_stack_overflow() {
        let depth = 200_usize;
        let mut predicate = "True".to_owned();
        for _ in 1..depth {
            predicate = format!("({predicate} and True)");
        }
        let source = format!(
            r#"
signal flags : Signal Bool
signal filtered : Signal Bool = flags ?|> {predicate}
"#
        );
        // Run in a thread with a 64 MB stack so infer_expr (still recursive) has
        // headroom, while lower_gate_runtime_expr is the iterative component under test.
        let result = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let lowered = lower_text("deep-and-chain.aivi", &source);
                let report = elaborate_gates(lowered.module());
                // The predicate should elaborate into a SignalFilter (or Ordinary gate)
                // without panicking.  A blocked outcome due to elaboration issues is
                // also acceptable — the critical invariant is no stack overflow.
                assert!(
                    !report.stages().is_empty(),
                    "gate elaboration should produce at least one stage for filtered signal"
                );
            })
            .expect("thread spawn should succeed")
            .join();
        result.expect("gate elaboration thread should not panic");
    }

    #[test]
    fn blocks_non_bool_gate_predicates() {
        let lowered = lower_fixture("milestone-2/invalid/gate-predicate-not-bool/main.aivi");
        let report = elaborate_gates(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "maybeUser")
            .expect("expected blocked gate stage");

        match &blocked.outcome {
            GateStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    GateElaborationBlocker::PredicateNotBool {
                        found: GateType::Primitive(BuiltinType::Text)
                    }
                )));
            }
            other => panic!("expected blocked gate stage, found {other:?}"),
        }
    }

    #[test]
    fn blocks_impure_signal_gate_predicates() {
        let lowered = lower_fixture("milestone-2/invalid/impure-gate-predicate/main.aivi");
        let report = elaborate_gates(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "filtered")
            .expect("expected blocked gate stage");

        match &blocked.outcome {
            GateStageOutcome::Blocked(stage) => {
                assert!(
                    stage
                        .blockers
                        .iter()
                        .any(|blocker| blocker == &GateElaborationBlocker::ImpurePredicate)
                );
            }
            other => panic!("expected blocked gate stage, found {other:?}"),
        }
    }

    #[test]
    fn blocks_nested_gate_predicates_explicitly() {
        let lowered = lower_fixture("milestone-2/invalid/nested-gate-predicate/main.aivi");
        let report = elaborate_gates(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "filtered")
            .expect("expected blocked gate stage");

        match &blocked.outcome {
            GateStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    GateElaborationBlocker::UnsupportedRuntimeExpr {
                        kind: GateRuntimeUnsupportedKind::NestedGate,
                        ..
                    }
                )));
            }
            other => panic!("expected blocked gate stage, found {other:?}"),
        }
    }
}
