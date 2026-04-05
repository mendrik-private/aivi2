use aivi_base::SourceSpan;

use crate::{
    BuiltinType, ExprId, ExprKind, Item, ItemId, Module, PipeExpr, PipeStageKind,
    gate_elaboration::{GateElaborationBlocker, lower_gate_runtime_expr},
    validate::{
        GateExprEnv, GateType, GateTypeContext, PipeSubjectStepOutcome, PipeSubjectWalker,
        gate_env_for_function, truthy_falsy_pair_stages, walk_expr_tree,
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
    Blocked(BlockedTemporalStage),
}

impl TemporalStageOutcome {
    fn result_subject(&self) -> Option<&GateType> {
        match self {
            Self::Previous(plan) => Some(&plan.result_subject),
            Self::Diff(plan) => Some(&plan.result_subject),
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
    let all_stages = pipe.stages.iter().collect::<Vec<_>>();
    PipeSubjectWalker::new(pipe, env, typing).walk(
        typing,
        |stage_index, stage, current, current_env, typing| {
            if recurrence_start_index.is_some_and(|start| stage_index >= start) {
                return PipeSubjectStepOutcome::Stop;
            }
            match &stage.kind {
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
                        stage_index,
                        stage_span: stage.span,
                        outcome: outcome.clone(),
                    });
                    PipeSubjectStepOutcome::Continue {
                        new_subject: outcome.result_subject().cloned(),
                        advance_by: 1,
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
                        stage_index,
                        stage_span: stage.span,
                        outcome: outcome.clone(),
                    });
                    PipeSubjectStepOutcome::Continue {
                        new_subject: outcome.result_subject().cloned(),
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
                PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Accumulate { seed, step } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .map(|s| typing.infer_accumulate_stage_info(*seed, *step, current_env, s))
                        .and_then(|info| info.ty),
                    advance_by: 1,
                },
                PipeStageKind::Case { .. }
                | PipeStageKind::Apply { .. }
                | PipeStageKind::RecurStart { .. }
                | PipeStageKind::RecurStep { .. }
                | PipeStageKind::Validate { .. } => PipeSubjectStepOutcome::Continue {
                    new_subject: None,
                    advance_by: 1,
                },
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!("transform and tap stages are consumed by PipeSubjectWalker")
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
