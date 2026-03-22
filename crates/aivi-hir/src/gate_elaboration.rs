use aivi_base::SourceSpan;
use aivi_typing::{GatePlanner, GateResultKind};

use crate::{
    BinaryOperator, BindingId, BuiltinTerm, ExprId, ExprKind, IntegerLiteral, Item, ItemId, Module,
    Name, NamePath, PipeExpr, PipeStageKind, ProjectionBase, SuffixedIntegerLiteral,
    TermResolution, TextFragment, TextSegment, UnaryOperator,
    validate::{GateExprEnv, GateIssue, GateType, GateTypeContext, walk_expr_tree},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateRuntimeUnsupportedKind {
    RegexLiteral,
    ApplicativeCluster,
    Markup,
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
    SuffixedInteger(SuffixedIntegerLiteral),
    Text(GateRuntimeTextLiteral),
    Tuple(Vec<GateRuntimeExpr>),
    List(Vec<GateRuntimeExpr>),
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
    Builtin(BuiltinTerm),
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
pub struct GateRuntimePipeExpr {
    pub head: Box<GateRuntimeExpr>,
    pub stages: Vec<GateRuntimePipeStage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRuntimePipeStage {
    pub span: SourceSpan,
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub kind: GateRuntimePipeStageKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRuntimePipeStageKind {
    Transform {
        expr: GateRuntimeExpr,
    },
    Tap {
        expr: GateRuntimeExpr,
    },
    Gate {
        predicate: GateRuntimeExpr,
        emits_negative_update: bool,
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
            Item::Type(_) | Item::Class(_) | Item::Domain(_) | Item::Use(_) | Item::Export(_) => {}
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
    stages: &mut Vec<GateStageElaboration>,
) {
    let mut current = typing.infer_expr(pipe.head, env, None).ty;
    for (stage_index, stage) in pipe.stages.iter().enumerate() {
        match &stage.kind {
            PipeStageKind::Transform { expr } => {
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_transform_stage(*expr, env, subject));
            }
            PipeStageKind::Tap { expr } => {
                if let Some(subject) = current.clone() {
                    let _ = typing.infer_pipe_body(*expr, env, &subject);
                    current = Some(subject);
                }
            }
            PipeStageKind::Gate { expr } => {
                let outcome = elaborate_gate_stage(module, *expr, env, current.as_ref(), typing);
                stages.push(GateStageElaboration {
                    owner,
                    pipe_expr,
                    stage_index,
                    stage_span: stage.span,
                    predicate: *expr,
                    outcome: outcome.clone(),
                });
                current = match outcome {
                    GateStageOutcome::Ordinary(stage) => Some(stage.result_type),
                    GateStageOutcome::SignalFilter(stage) => Some(stage.result_type),
                    GateStageOutcome::Blocked(_) => None,
                };
            }
            PipeStageKind::Case { .. }
            | PipeStageKind::Map { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::FanIn { .. }
            | PipeStageKind::Truthy { .. }
            | PipeStageKind::Falsy { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. } => {
                current = None;
            }
        }
    }
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

fn lower_gate_pipe_body_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let ambient = subject.gate_payload().clone();
    let mut lowered = lower_gate_runtime_expr(module, expr_id, env, Some(&ambient), typing)?;
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

fn lower_gate_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, GateElaborationBlocker> {
    let (expr, ty) = inferred_runtime_expr(module, expr_id, env, ambient, typing)?;
    let kind = match expr.kind {
        ExprKind::Name(reference) => {
            GateRuntimeExprKind::Reference(runtime_reference_for_name(expr.span, &reference)?)
        }
        ExprKind::Integer(literal) => GateRuntimeExprKind::Integer(literal),
        ExprKind::SuffixedInteger(literal) => GateRuntimeExprKind::SuffixedInteger(literal),
        ExprKind::Text(text) => GateRuntimeExprKind::Text(lower_runtime_text_literal(
            module, &text, env, ambient, typing,
        )?),
        ExprKind::Regex(_) => {
            return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                span: expr.span,
                kind: GateRuntimeUnsupportedKind::RegexLiteral,
            });
        }
        ExprKind::Tuple(elements) => GateRuntimeExprKind::Tuple(
            elements
                .iter()
                .map(|element| lower_gate_runtime_expr(module, *element, env, ambient, typing))
                .collect::<Result<_, _>>()?,
        ),
        ExprKind::List(elements) => GateRuntimeExprKind::List(
            elements
                .iter()
                .map(|element| lower_gate_runtime_expr(module, *element, env, ambient, typing))
                .collect::<Result<_, _>>()?,
        ),
        ExprKind::Record(record) => GateRuntimeExprKind::Record(
            record
                .fields
                .into_iter()
                .map(|field| lower_runtime_record_field(module, field, env, ambient, typing))
                .collect::<Result<_, _>>()?,
        ),
        ExprKind::Projection { base, path } => {
            let base = match base {
                ProjectionBase::Ambient => GateRuntimeProjectionBase::AmbientSubject,
                ProjectionBase::Expr(base) => GateRuntimeProjectionBase::Expr(Box::new(
                    lower_gate_runtime_expr(module, base, env, ambient, typing)?,
                )),
            };
            GateRuntimeExprKind::Projection { base, path }
        }
        ExprKind::Apply { callee, arguments } => GateRuntimeExprKind::Apply {
            callee: Box::new(lower_gate_runtime_expr(
                module, callee, env, ambient, typing,
            )?),
            arguments: arguments
                .iter()
                .map(|argument| lower_gate_runtime_expr(module, *argument, env, ambient, typing))
                .collect::<Result<_, _>>()?,
        },
        ExprKind::Unary { operator, expr } => GateRuntimeExprKind::Unary {
            operator,
            expr: Box::new(lower_gate_runtime_expr(module, expr, env, ambient, typing)?),
        },
        ExprKind::Binary {
            left,
            operator,
            right,
        } => GateRuntimeExprKind::Binary {
            left: Box::new(lower_gate_runtime_expr(module, left, env, ambient, typing)?),
            operator,
            right: Box::new(lower_gate_runtime_expr(
                module, right, env, ambient, typing,
            )?),
        },
        ExprKind::Pipe(pipe) => GateRuntimeExprKind::Pipe(lower_runtime_pipe_expr(
            module, &pipe, env, ambient, typing,
        )?),
        ExprKind::Cluster(_) => {
            return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                span: expr.span,
                kind: GateRuntimeUnsupportedKind::ApplicativeCluster,
            });
        }
        ExprKind::Markup(_) => {
            return Err(GateElaborationBlocker::UnsupportedRuntimeExpr {
                span: expr.span,
                kind: GateRuntimeUnsupportedKind::Markup,
            });
        }
    };
    Ok(GateRuntimeExpr {
        span: expr.span,
        ty,
        kind,
    })
}

fn lower_runtime_text_literal(
    module: &Module,
    text: &crate::TextLiteral,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeTextLiteral, GateElaborationBlocker> {
    let mut segments = Vec::with_capacity(text.segments.len());
    for segment in &text.segments {
        let lowered = match segment {
            TextSegment::Text(fragment) => GateRuntimeTextSegment::Fragment(fragment.clone()),
            TextSegment::Interpolation(interpolation) => {
                GateRuntimeTextSegment::Interpolation(Box::new(lower_gate_runtime_expr(
                    module,
                    interpolation.expr,
                    env,
                    ambient,
                    typing,
                )?))
            }
        };
        segments.push(lowered);
    }
    Ok(GateRuntimeTextLiteral { segments })
}

fn lower_runtime_record_field(
    module: &Module,
    field: crate::RecordExprField,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeRecordField, GateElaborationBlocker> {
    Ok(GateRuntimeRecordField {
        label: field.label,
        value: lower_gate_runtime_expr(module, field.value, env, ambient, typing)?,
    })
}

fn lower_runtime_pipe_expr(
    module: &Module,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    ambient: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimePipeExpr, GateElaborationBlocker> {
    let head = lower_gate_runtime_expr(module, pipe.head, env, ambient, typing)?;
    let mut current = head.ty.clone();
    let mut stages = Vec::with_capacity(pipe.stages.len());
    for stage in pipe.stages.iter() {
        let input_subject = current.clone();
        let (kind, result_subject) = match &stage.kind {
            PipeStageKind::Transform { expr } => {
                let body = lower_gate_pipe_body_runtime_expr(module, *expr, env, &current, typing)?;
                let result_subject = typing
                    .infer_transform_stage(*expr, env, &current)
                    .ok_or(GateElaborationBlocker::UnknownRuntimeExprType { span: stage.span })?;
                (
                    GateRuntimePipeStageKind::Transform { expr: body },
                    result_subject,
                )
            }
            PipeStageKind::Tap { expr } => (
                GateRuntimePipeStageKind::Tap {
                    expr: lower_gate_pipe_body_runtime_expr(module, *expr, env, &current, typing)?,
                },
                current.clone(),
            ),
            PipeStageKind::Gate { expr } => {
                let predicate =
                    lower_gate_pipe_body_runtime_expr(module, *expr, env, &current, typing)?;
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
            PipeStageKind::Map { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Map,
                ));
            }
            PipeStageKind::Apply { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::Apply,
                ));
            }
            PipeStageKind::FanIn { .. } => {
                return Err(unsupported_runtime_pipe_stage(
                    stage.span,
                    GateRuntimeUnsupportedPipeStageKind::FanIn,
                ));
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
        };
        stages.push(GateRuntimePipeStage {
            span: stage.span,
            input_subject,
            result_subject: result_subject.clone(),
            kind,
        });
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
) -> Result<(crate::Expr, GateType), GateElaborationBlocker> {
    let expr = module.exprs()[expr_id].clone();
    let info = typing.infer_expr(expr_id, env, ambient);
    if info.contains_signal || info.ty.as_ref().is_some_and(GateType::is_signal) {
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
    span: SourceSpan,
    reference: &crate::TermReference,
) -> Result<GateRuntimeReference, GateElaborationBlocker> {
    match reference.resolution.as_ref() {
        crate::ResolutionState::Resolved(TermResolution::Local(binding)) => {
            Ok(GateRuntimeReference::Local(*binding))
        }
        crate::ResolutionState::Resolved(TermResolution::Item(item_id)) => {
            Ok(GateRuntimeReference::Item(*item_id))
        }
        crate::ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
            Ok(GateRuntimeReference::Builtin(*builtin))
        }
        crate::ResolutionState::Resolved(TermResolution::Import(_))
        | crate::ResolutionState::Unresolved => {
            Err(GateElaborationBlocker::UnknownRuntimeExprType { span })
        }
    }
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
    }
}

fn gate_env_for_function(
    item: &crate::FunctionItem,
    typing: &mut GateTypeContext<'_>,
) -> GateExprEnv {
    let mut env = GateExprEnv::default();
    for parameter in &item.parameters {
        let Some(annotation) = parameter.annotation else {
            continue;
        };
        if let Some(ty) = typing.lower_annotation(annotation) {
            env.locals.insert(parameter.binding, ty);
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{
        GateCoreExprKind, GateElaborationBlocker, GateRuntimeExprKind, GateRuntimeProjectionBase,
        GateRuntimeReference, GateStageOutcome, elaborate_gates,
    };
    use crate::{BuiltinType, GateType, Item, lower_module};

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
    }

    fn lower_text(path: &str, text: &str) -> crate::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn lower_fixture(path: &str) -> crate::LoweringResult {
        let text =
            fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
        lower_text(path, &text)
    }

    fn item_name(module: &crate::Module, item_id: crate::ItemId) -> &str {
        match &module.items()[item_id] {
            Item::Type(item) => item.name.text(),
            Item::Value(item) => item.name.text(),
            Item::Function(item) => item.name.text(),
            Item::Signal(item) => item.name.text(),
            Item::Class(item) => item.name.text(),
            Item::Domain(item) => item.name.text(),
            Item::Instance(_) | Item::Use(_) | Item::Export(_) => "<anonymous>",
        }
    }

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

fun isEligible:Bool #user:User =>
    .active and .age > 18

sig users:Signal User = { active: True, age: 21 }

sig eligibleUsers:Signal User =
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

sig users:Signal User = { active: True, age: 21 }

sig activeUsers:Signal User =
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
}
