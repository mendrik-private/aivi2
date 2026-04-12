use aivi_base::SourceSpan;

use crate::{
    BuiltinType, ExprId, ExprKind, Item, ItemId, Module, PipeExpr, PipeStageKind,
    gate_elaboration::{GateElaborationBlocker, lower_gate_runtime_expr},
    validate::{
        GateExprEnv, GateType, GateTypeContext, PipeSubjectStepOutcome, PipeSubjectWalker,
        gate_env_for_function, walk_expr_tree,
    },
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemporalElaborationReport {
    stages: Vec<TemporalStageElaboration>,
}

impl TemporalElaborationReport {
    pub fn new(stages: Vec<TemporalStageElaboration>) -> Self {
        Self { stages }
    }

    pub fn stages(&self) -> &[TemporalStageElaboration] {
        &self.stages
    }

    pub fn into_stages(self) -> Vec<TemporalStageElaboration> {
        self.stages
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalStageElaboration {
    pub owner: ItemId,
    pub pipe_expr: ExprId,
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub outcome: TemporalStageOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalStageOutcome {
    Previous(PreviousStagePlan),
    Diff(DiffStagePlan),
    Delay(DelayStagePlan),
    Burst(BurstStagePlan),
    Blocked(BlockedTemporalStage),
}

impl TemporalStageOutcome {
    fn result_subject(&self) -> Option<&GateType> {
        match self {
            Self::Previous(plan) => Some(&plan.result_subject),
            Self::Diff(plan) => Some(&plan.result_subject),
            Self::Delay(plan) => Some(&plan.result_subject),
            Self::Burst(plan) => Some(&plan.result_subject),
            Self::Blocked(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousStagePlan {
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub seed_expr: crate::GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffStagePlan {
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub mode: DiffStageMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelayStagePlan {
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub duration_expr: crate::GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BurstStagePlan {
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub every_expr: crate::GateRuntimeExpr,
    pub count_expr: crate::GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffStageMode {
    Function { diff_expr: crate::GateRuntimeExpr },
    Seed { seed_expr: crate::GateRuntimeExpr },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedTemporalStage {
    pub subject: Option<GateType>,
    pub blockers: Vec<TemporalElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalElaborationBlocker {
    UnknownSubjectType,
    InvalidSubjectType { found: GateType },
    StageReadsSignals,
    UnknownStageExprType { span: SourceSpan },
    SeedTypeMismatch { expected: GateType, found: GateType },
    UnsupportedSeededDiffSubject { found: GateType },
    DiffFunctionShapeMismatch { expected: String, found: GateType },
    SignalResultNotSupported { found: GateType },
    InvalidDurationType { found: GateType },
    InvalidBurstCountType { found: GateType },
    RuntimeExprBlocked(GateElaborationBlocker),
}

pub fn elaborate_temporal_stages(module: &Module) -> TemporalElaborationReport {
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
            Item::Value(item) => collect_temporal_stages(
                module,
                owner,
                item.body,
                &GateExprEnv::default(),
                &mut typing,
                &mut stages,
            ),
            Item::Function(item) => {
                let env = gate_env_for_function(&item, &mut typing);
                collect_temporal_stages(module, owner, item.body, &env, &mut typing, &mut stages);
            }
            Item::Signal(item) => {
                if let Some(body) = item.body {
                    collect_temporal_stages(
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
                    collect_temporal_stages(
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
            | Item::Export(_)
            | Item::Hoist(_) => {}
        }
    }

    TemporalElaborationReport::new(stages)
}

fn collect_temporal_stages(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    stages: &mut Vec<TemporalStageElaboration>,
) {
    walk_expr_tree(module, root, |pipe_expr, expr, _| {
        if let ExprKind::Pipe(pipe) = &expr.kind {
            collect_temporal_pipe(module, owner, pipe_expr, pipe, env, typing, stages);
        }
    });
}

fn collect_temporal_pipe(
    module: &Module,
    owner: ItemId,
    pipe_expr: ExprId,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    temporal_stages: &mut Vec<TemporalStageElaboration>,
) {
    let recurrence_start_index = pipe
        .recurrence_suffix()
        .ok()
        .flatten()
        .map(|suffix| suffix.prefix_stage_count());
    PipeSubjectWalker::new(pipe, env, typing).walk(
        typing,
        |stage, current, current_env, typing| {
            if recurrence_start_index.is_some_and(|start| stage.start_stage_index() >= start) {
                return PipeSubjectStepOutcome::Stop;
            }
            match stage {
                crate::PipeSubjectStage::Single { stage, stage_index } => match &stage.kind {
                    PipeStageKind::Previous { expr } => {
                        let outcome = elaborate_previous_stage(
                            module,
                            stage.span,
                            *expr,
                            current_env,
                            current,
                            typing,
                        );
                        temporal_stages.push(TemporalStageElaboration {
                            owner,
                            pipe_expr,
                            stage_index: *stage_index,
                            stage_span: stage.span,
                            outcome: outcome.clone(),
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject: outcome.result_subject().cloned(),
                        }
                    }
                    PipeStageKind::Diff { expr } => {
                        let outcome = elaborate_diff_stage(
                            module,
                            stage.span,
                            *expr,
                            current_env,
                            current,
                            typing,
                        );
                        temporal_stages.push(TemporalStageElaboration {
                            owner,
                            pipe_expr,
                            stage_index: *stage_index,
                            stage_span: stage.span,
                            outcome: outcome.clone(),
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject: outcome.result_subject().cloned(),
                        }
                    }
                    PipeStageKind::Delay { duration } => {
                        let outcome = elaborate_delay_stage(
                            module,
                            stage.span,
                            *duration,
                            current_env,
                            current,
                            typing,
                        );
                        temporal_stages.push(TemporalStageElaboration {
                            owner,
                            pipe_expr,
                            stage_index: *stage_index,
                            stage_span: stage.span,
                            outcome: outcome.clone(),
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject: outcome.result_subject().cloned(),
                        }
                    }
                    PipeStageKind::Burst { every, count } => {
                        let outcome = elaborate_burst_stage(
                            module,
                            stage.span,
                            *every,
                            *count,
                            current_env,
                            current,
                            typing,
                        );
                        temporal_stages.push(TemporalStageElaboration {
                            owner,
                            pipe_expr,
                            stage_index: *stage_index,
                            stage_span: stage.span,
                            outcome: outcome.clone(),
                        });
                        PipeSubjectStepOutcome::Continue {
                            new_subject: outcome.result_subject().cloned(),
                        }
                    }
                    PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_fanin_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    },
                    PipeStageKind::Accumulate { seed, step } => PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .map(|s| {
                                typing.infer_accumulate_stage_info(*seed, *step, current_env, s)
                            })
                            .and_then(|info| info.ty),
                    },
                    PipeStageKind::Case { .. }
                    | PipeStageKind::Apply { .. }
                    | PipeStageKind::RecurStart { .. }
                    | PipeStageKind::RecurStep { .. }
                    | PipeStageKind::Validate { .. } => {
                        PipeSubjectStepOutcome::Continue { new_subject: None }
                    }
                    PipeStageKind::Truthy { .. }
                    | PipeStageKind::Falsy { .. }
                    | PipeStageKind::Transform { .. }
                    | PipeStageKind::Tap { .. } => {
                        unreachable!(
                            "subject walker groups truthy/falsy pairs and consumes transform/tap"
                        )
                    }
                },
                crate::PipeSubjectStage::FanoutSegment(segment) => {
                    let outcome = crate::fanout_elaboration::elaborate_fanout_segment(
                        module,
                        segment,
                        current,
                        current_env,
                        typing,
                    );
                    PipeSubjectStepOutcome::Continue {
                        new_subject: match outcome {
                            crate::fanout_elaboration::FanoutSegmentOutcome::Planned(plan) => {
                                Some(plan.result_type)
                            }
                            crate::fanout_elaboration::FanoutSegmentOutcome::Blocked(_) => None,
                        },
                    }
                }
                crate::PipeSubjectStage::TruthyFalsyPair(pair) => {
                    PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_truthy_falsy_pair(pair, current_env, s)),
                    }
                }
                crate::PipeSubjectStage::CaseRun(_) => {
                    PipeSubjectStepOutcome::Continue { new_subject: None }
                }
            }
        },
    );
}

fn elaborate_previous_stage(
    module: &Module,
    stage_span: SourceSpan,
    seed_expr: ExprId,
    env: &GateExprEnv,
    current: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> TemporalStageOutcome {
    let Some(subject) = current.cloned() else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: None,
            blockers: vec![TemporalElaborationBlocker::UnknownSubjectType],
        });
    };
    let GateType::Signal(payload) = &subject else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers: vec![TemporalElaborationBlocker::InvalidSubjectType { found: subject }],
        });
    };

    let mut blockers = Vec::new();
    let seed_info = typing.infer_expr(seed_expr, env, None);
    if seed_info.contains_signal {
        blockers.push(TemporalElaborationBlocker::StageReadsSignals);
    }
    let seed_ty = seed_info.actual_gate_type().or(seed_info.ty.clone());
    let Some(seed_ty) = seed_ty else {
        blockers.push(TemporalElaborationBlocker::UnknownStageExprType { span: stage_span });
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    };
    if !seed_ty.same_shape(payload.as_ref()) {
        blockers.push(TemporalElaborationBlocker::SeedTypeMismatch {
            expected: payload.as_ref().clone(),
            found: seed_ty,
        });
    }
    let seed_expr = match lower_gate_runtime_expr(module, seed_expr, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(TemporalElaborationBlocker::RuntimeExprBlocked(blocker));
            None
        }
    };
    if !blockers.is_empty() {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    }

    TemporalStageOutcome::Previous(PreviousStagePlan {
        input_subject: subject.clone(),
        result_subject: subject,
        seed_expr: seed_expr.expect("planned previous stage should have a seed expression"),
    })
}

fn elaborate_diff_stage(
    module: &Module,
    stage_span: SourceSpan,
    expr_id: ExprId,
    env: &GateExprEnv,
    current: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> TemporalStageOutcome {
    let Some(subject) = current.cloned() else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: None,
            blockers: vec![TemporalElaborationBlocker::UnknownSubjectType],
        });
    };
    let GateType::Signal(payload) = &subject else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers: vec![TemporalElaborationBlocker::InvalidSubjectType { found: subject }],
        });
    };

    let mut blockers = Vec::new();
    let info = typing.infer_expr(expr_id, env, None);
    if info.contains_signal {
        blockers.push(TemporalElaborationBlocker::StageReadsSignals);
    }
    let stage_ty = info.actual_gate_type().or(info.ty.clone());
    let Some(stage_ty) = stage_ty else {
        blockers.push(TemporalElaborationBlocker::UnknownStageExprType { span: stage_span });
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    };

    let mode = if let Some((parameters, result_ty)) = typing.function_signature(&stage_ty, 2) {
        if !parameters[0].same_shape(payload.as_ref())
            || !parameters[1].same_shape(payload.as_ref())
        {
            blockers.push(TemporalElaborationBlocker::DiffFunctionShapeMismatch {
                expected: format!("{} -> {} -> _", payload.as_ref(), payload.as_ref()),
                found: stage_ty.clone(),
            });
            None
        } else if result_ty.is_signal() {
            blockers.push(TemporalElaborationBlocker::SignalResultNotSupported {
                found: result_ty.clone(),
            });
            None
        } else {
            Some((DiffStageModeDiscriminant::Function, result_ty.clone()))
        }
    } else if stage_ty.same_shape(payload.as_ref()) {
        if is_numeric_payload(payload.as_ref()) {
            Some((DiffStageModeDiscriminant::Seed, payload.as_ref().clone()))
        } else {
            blockers.push(TemporalElaborationBlocker::UnsupportedSeededDiffSubject {
                found: payload.as_ref().clone(),
            });
            None
        }
    } else {
        blockers.push(TemporalElaborationBlocker::DiffFunctionShapeMismatch {
            expected: format!(
                "{} -> {} -> _  or seeded {}",
                payload.as_ref(),
                payload.as_ref(),
                payload.as_ref()
            ),
            found: stage_ty.clone(),
        });
        None
    };

    let runtime_expr = match lower_gate_runtime_expr(module, expr_id, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(TemporalElaborationBlocker::RuntimeExprBlocked(blocker));
            None
        }
    };
    if !blockers.is_empty() {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    }

    let (discriminant, result_payload) = mode.expect("planned diff stage should have a mode");
    let runtime_expr = runtime_expr.expect("planned diff stage should lower its runtime expr");
    let result_subject = GateType::Signal(Box::new(result_payload));
    let mode = match discriminant {
        DiffStageModeDiscriminant::Function => DiffStageMode::Function {
            diff_expr: runtime_expr,
        },
        DiffStageModeDiscriminant::Seed => DiffStageMode::Seed {
            seed_expr: runtime_expr,
        },
    };
    TemporalStageOutcome::Diff(DiffStagePlan {
        input_subject: subject,
        result_subject,
        mode,
    })
}

fn elaborate_delay_stage(
    module: &Module,
    stage_span: SourceSpan,
    duration_expr_id: ExprId,
    env: &GateExprEnv,
    current: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> TemporalStageOutcome {
    let Some(subject) = current.cloned() else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: None,
            blockers: vec![TemporalElaborationBlocker::UnknownSubjectType],
        });
    };
    let GateType::Signal(_) = &subject else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers: vec![TemporalElaborationBlocker::InvalidSubjectType { found: subject }],
        });
    };

    let mut blockers = Vec::new();
    let duration_info = typing.infer_expr(duration_expr_id, env, None);
    if duration_info.contains_signal {
        blockers.push(TemporalElaborationBlocker::StageReadsSignals);
    }
    let duration_ty = duration_info
        .actual_gate_type()
        .or(duration_info.ty.clone());
    let Some(duration_ty) = duration_ty else {
        blockers.push(TemporalElaborationBlocker::UnknownStageExprType { span: stage_span });
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    };
    if !is_duration_stage_type(&duration_ty) {
        blockers.push(TemporalElaborationBlocker::InvalidDurationType { found: duration_ty });
    }
    let duration_expr = match lower_gate_runtime_expr(module, duration_expr_id, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(TemporalElaborationBlocker::RuntimeExprBlocked(blocker));
            None
        }
    };
    if !blockers.is_empty() {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers,
        });
    }

    TemporalStageOutcome::Delay(DelayStagePlan {
        input_subject: subject.clone(),
        result_subject: subject,
        duration_expr: duration_expr.expect("planned delay stage should lower its runtime expr"),
    })
}

fn elaborate_burst_stage(
    module: &Module,
    stage_span: SourceSpan,
    every_expr_id: ExprId,
    count_expr_id: ExprId,
    env: &GateExprEnv,
    current: Option<&GateType>,
    typing: &mut GateTypeContext<'_>,
) -> TemporalStageOutcome {
    let Some(subject) = current.cloned() else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: None,
            blockers: vec![TemporalElaborationBlocker::UnknownSubjectType],
        });
    };
    let GateType::Signal(_) = &subject else {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers: vec![TemporalElaborationBlocker::InvalidSubjectType { found: subject }],
        });
    };

    let mut blockers = Vec::new();
    let every_info = typing.infer_expr(every_expr_id, env, None);
    if every_info.contains_signal {
        blockers.push(TemporalElaborationBlocker::StageReadsSignals);
    }
    let every_ty = every_info.actual_gate_type().or(every_info.ty.clone());
    let Some(every_ty) = every_ty else {
        blockers.push(TemporalElaborationBlocker::UnknownStageExprType { span: stage_span });
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    };
    if !is_duration_stage_type(&every_ty) {
        blockers.push(TemporalElaborationBlocker::InvalidDurationType { found: every_ty });
    }

    let count_info = typing.infer_expr(count_expr_id, env, None);
    if count_info.contains_signal {
        blockers.push(TemporalElaborationBlocker::StageReadsSignals);
    }
    let count_ty = count_info.actual_gate_type().or(count_info.ty.clone());
    let Some(count_ty) = count_ty else {
        blockers.push(TemporalElaborationBlocker::UnknownStageExprType { span: stage_span });
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject),
            blockers,
        });
    };
    if !is_burst_count_stage_type(&count_ty) {
        blockers.push(TemporalElaborationBlocker::InvalidBurstCountType { found: count_ty });
    }

    let every_expr = match lower_gate_runtime_expr(module, every_expr_id, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(TemporalElaborationBlocker::RuntimeExprBlocked(blocker));
            None
        }
    };
    let count_expr = match lower_gate_runtime_expr(module, count_expr_id, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(TemporalElaborationBlocker::RuntimeExprBlocked(blocker));
            None
        }
    };
    if !blockers.is_empty() {
        return TemporalStageOutcome::Blocked(BlockedTemporalStage {
            subject: Some(subject.clone()),
            blockers,
        });
    }

    TemporalStageOutcome::Burst(BurstStagePlan {
        input_subject: subject.clone(),
        result_subject: subject,
        every_expr: every_expr.expect("planned burst stage should lower its interval expr"),
        count_expr: count_expr.expect("planned burst stage should lower its count expr"),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffStageModeDiscriminant {
    Function,
    Seed,
}

fn is_numeric_payload(ty: &GateType) -> bool {
    matches!(
        ty,
        GateType::Primitive(
            BuiltinType::Int | BuiltinType::Float | BuiltinType::Decimal | BuiltinType::BigInt
        )
    )
}

fn is_duration_stage_type(ty: &GateType) -> bool {
    ty.has_named_type("Duration")
}

fn is_burst_count_stage_type(ty: &GateType) -> bool {
    matches!(ty, GateType::Primitive(BuiltinType::Int)) || ty.has_named_type("Retry")
}
