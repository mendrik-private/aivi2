use aivi_base::SourceSpan;
use aivi_typing::{
    BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext, NonSourceWakeupCause,
    RecurrencePlan, RecurrencePlanner, RecurrenceTargetEvidence, RecurrenceWakeupEvidence,
    RecurrenceWakeupPlan, RecurrenceWakeupPlanner, SourceRecurrenceWakeupContext,
    builtin_source_option_wakeup_cause,
};

use crate::{
    CustomSourceRecurrenceWakeup, DecoratorId, DecoratorPayload, ExprId, ExprKind, Item, ItemId,
    Module, PipeExpr, PipeStageKind, SignalItem, SourceDecorator, SourceMetadata,
    SourceProviderRef,
    gate_elaboration::{
        GateElaborationBlocker, GateRuntimeExpr, GateRuntimeUnsupportedKind,
        lower_gate_pipe_body_runtime_expr, lower_gate_pipe_body_runtime_expr_allow_signal_reads,
        lower_gate_pipe_function_apply_runtime_expr_allow_signal_reads, lower_gate_runtime_expr,
    },
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, PipeSubjectStepOutcome,
        PipeSubjectWalker, gate_env_for_function, truthy_falsy_pair_stages, walk_expr_tree,
    },
};

/// Focused scheduler-node plans derived from validated recurrence suffixes.
///
/// This is intentionally narrower than a future runtime-aware/backend IR. It keeps each validated
/// trailing `@|> ... <|@ ...` suffix as one scheduler-owned node handoff with:
/// - the closed recurrence target family (`Signal`, `Task`, or future source helper),
/// - the canonical explicit wakeup proof selected by `aivi-typing`,
/// - the typed `@|>` start stage and ordered `<|@` step stages as runtime-ready expression trees,
/// - and explicit blockers when the current local inference/runtime-expression subset cannot yet
///   justify lowering.
///
/// The report deliberately keeps the `@|>` start stage distinct from the `<|@` step stages. The
/// RFC names those roles separately, and later runtime/backend IR can consume that handoff
/// directly without collapsing the distinction here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecurrenceElaborationReport {
    nodes: Vec<RecurrenceNodeElaboration>,
}

impl RecurrenceElaborationReport {
    pub fn new(nodes: Vec<RecurrenceNodeElaboration>) -> Self {
        Self { nodes }
    }

    pub fn nodes(&self) -> &[RecurrenceNodeElaboration] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<RecurrenceNodeElaboration> {
        self.nodes
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceNodeElaboration {
    pub owner: ItemId,
    pub pipe_expr: ExprId,
    pub start_stage_index: usize,
    pub start_stage_span: SourceSpan,
    pub outcome: RecurrenceNodeOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecurrenceNodeOutcome {
    Planned(RecurrenceNodePlan),
    Blocked(BlockedRecurrenceNode),
}

pub type RecurrenceRuntimeExpr = GateRuntimeExpr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceStagePlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub expr: ExprId,
    pub input_subject: GateType,
    pub result_subject: GateType,
    pub runtime_expr: RecurrenceRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceGuardPlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate: ExprId,
    pub input_subject: GateType,
    pub runtime_predicate: RecurrenceRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceNonSourceWakeupBinding {
    pub cause: NonSourceWakeupCause,
    pub witness: ExprId,
    pub runtime_witness: RecurrenceRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceNodePlan {
    pub target: RecurrencePlan,
    pub wakeup: RecurrenceWakeupPlan,
    pub wakeup_signal: Option<ItemId>,
    pub seed: RecurrenceRuntimeExpr,
    pub start: RecurrenceStagePlan,
    pub guards: Vec<RecurrenceGuardPlan>,
    pub steps: Vec<RecurrenceStagePlan>,
    pub non_source_wakeup: Option<RecurrenceNonSourceWakeupBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedRecurrenceNode {
    pub target: Option<RecurrencePlan>,
    pub wakeup: Option<RecurrenceWakeupPlan>,
    pub input_subject: Option<GateType>,
    pub blockers: Vec<RecurrenceElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecurrenceElaborationBlocker {
    UnknownTarget,
    UnsupportedTarget {
        ty: GateType,
    },
    MissingWakeup,
    UnknownInputSubjectType,
    StartStage {
        stage_span: SourceSpan,
        blocker: RecurrenceRuntimeStageBlocker,
    },
    GuardStage {
        stage_index: usize,
        stage_span: SourceSpan,
        blocker: RecurrenceRuntimeStageBlocker,
    },
    StepStage {
        stage_index: usize,
        stage_span: SourceSpan,
        blocker: RecurrenceRuntimeStageBlocker,
    },
    NonSourceWakeupWitness(RecurrenceRuntimeStageBlocker),
    SeedExpressionBlocked(RecurrenceRuntimeStageBlocker),
    StepChainDoesNotClose {
        expected: GateType,
        found: GateType,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecurrenceRuntimeStageBlocker {
    InvalidProjection {
        path: String,
        subject: String,
    },
    UnknownField {
        path: String,
        subject: String,
    },
    ImpureExpr,
    UnknownExprType {
        span: SourceSpan,
    },
    PredicateNotBool {
        found: GateType,
    },
    UnsupportedExpr {
        span: SourceSpan,
        kind: GateRuntimeUnsupportedKind,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LocalRecurrenceTargetHint {
    Evidence(RecurrenceTargetEvidence),
    UnsupportedType { ty: GateType },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalRecurrenceWakeupHint {
    BuiltinSource(SourceRecurrenceWakeupContext),
    CustomSource(CustomSourceRecurrenceWakeupContext),
    NonSource {
        cause: NonSourceWakeupCause,
        witness: ExprId,
    },
}

pub fn elaborate_recurrences(module: &Module) -> RecurrenceElaborationReport {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    let module = &module;
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();
    let mut typing = GateTypeContext::new(module);

    for (owner, item) in items {
        if module.ambient_items().contains(&owner) {
            continue;
        }
        match item {
            Item::Value(item) => {
                let target = item.annotation.and_then(|annotation| {
                    recurrence_target_hint_for_annotation(&mut typing, annotation)
                });
                let wakeup = recurrence_wakeup_hint_for_decorators(module, &item.header.decorators);
                collect_recurrence_nodes(
                    module,
                    owner,
                    item.body,
                    &GateExprEnv::default(),
                    target,
                    wakeup,
                    &mut typing,
                    &mut nodes,
                );
            }
            Item::Function(item) => {
                let target = item.annotation.and_then(|annotation| {
                    recurrence_target_hint_for_annotation(&mut typing, annotation)
                });
                let wakeup = recurrence_wakeup_hint_for_decorators(module, &item.header.decorators);
                let env = gate_env_for_function(&item, &mut typing);
                collect_recurrence_nodes(
                    module,
                    owner,
                    item.body,
                    &env,
                    target,
                    wakeup,
                    &mut typing,
                    &mut nodes,
                );
            }
            Item::Signal(item) => {
                if let Some(body) = item.body {
                    let wakeup = recurrence_wakeup_hint_for_signal(module, &item);
                    collect_recurrence_nodes(
                        module,
                        owner,
                        body,
                        &GateExprEnv::default(),
                        Some(LocalRecurrenceTargetHint::Evidence(
                            RecurrenceTargetEvidence::SignalItemBody,
                        )),
                        wakeup,
                        &mut typing,
                        &mut nodes,
                    );
                    collect_scan_node(
                        module,
                        owner,
                        body,
                        &GateExprEnv::default(),
                        &mut typing,
                        &mut nodes,
                    );
                }
            }
            Item::Instance(item) => {
                for member in item.members {
                    let target = member.annotation.and_then(|annotation| {
                        recurrence_target_hint_for_annotation(&mut typing, annotation)
                    });
                    collect_recurrence_nodes(
                        module,
                        owner,
                        member.body,
                        &GateExprEnv::default(),
                        target,
                        None,
                        &mut typing,
                        &mut nodes,
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

    RecurrenceElaborationReport::new(nodes)
}

fn collect_recurrence_nodes(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    root_target: Option<LocalRecurrenceTargetHint>,
    root_wakeup: Option<LocalRecurrenceWakeupHint>,
    typing: &mut GateTypeContext<'_>,
    nodes: &mut Vec<RecurrenceNodeElaboration>,
) {
    walk_expr_tree(module, root, |pipe_expr, expr, is_root| {
        let ExprKind::Pipe(pipe) = &expr.kind else {
            return;
        };
        let Ok(Some(suffix)) = pipe.recurrence_suffix() else {
            return;
        };
        let outcome = elaborate_recurrence_pipe(
            module,
            pipe,
            suffix.prefix_stage_count(),
            env,
            if is_root { root_target.as_ref() } else { None },
            if is_root { root_wakeup.as_ref() } else { None },
            typing,
        );
        nodes.push(RecurrenceNodeElaboration {
            owner,
            pipe_expr,
            start_stage_index: suffix.prefix_stage_count(),
            start_stage_span: suffix.start_stage().span,
            outcome,
        });
    });
}

#[derive(Clone, Copy)]
struct ScanStage {
    stage_index: usize,
    stage_span: SourceSpan,
    stage_expr: ExprId,
    seed_expr: ExprId,
    step_expr: ExprId,
}

fn collect_scan_node(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    nodes: &mut Vec<RecurrenceNodeElaboration>,
) {
    let ExprKind::Pipe(pipe) = &module.exprs()[root].kind else {
        return;
    };
    if matches!(pipe.recurrence_suffix(), Ok(Some(_))) {
        return;
    }
    let Some(scan) = scan_stage(module, pipe) else {
        return;
    };
    nodes.push(RecurrenceNodeElaboration {
        owner,
        pipe_expr: root,
        start_stage_index: scan.stage_index,
        start_stage_span: scan.stage_span,
        outcome: elaborate_scan_pipe(module, pipe, &scan, env, typing),
    });
}

fn scan_stage(module: &Module, pipe: &PipeExpr) -> Option<ScanStage> {
    if pipe.stages.len() != 1 {
        return None;
    }
    let stage = pipe.stages.first();
    let PipeStageKind::Transform { expr } = stage.kind else {
        return None;
    };
    let (seed_expr, step_expr) = parse_scan_expr(module, expr)?;
    Some(ScanStage {
        stage_index: 0,
        stage_span: stage.span,
        stage_expr: expr,
        seed_expr,
        step_expr,
    })
}

fn parse_scan_expr(module: &Module, expr_id: ExprId) -> Option<(ExprId, ExprId)> {
    let (callee_expr, arguments) = flatten_apply_expr(module, expr_id);
    if arguments.len() != 2 || !is_ambient_scan_name(module, callee_expr) {
        return None;
    }
    Some((arguments[0], arguments[1]))
}

fn flatten_apply_expr(module: &Module, root: ExprId) -> (ExprId, Vec<ExprId>) {
    let mut callee = root;
    let mut segments = Vec::new();
    while let ExprKind::Apply {
        callee: next,
        arguments: next_arguments,
    } = &module.exprs()[callee].kind
    {
        segments.push(next_arguments.iter().copied().collect::<Vec<_>>());
        callee = *next;
    }
    let mut arguments = Vec::new();
    for segment in segments.into_iter().rev() {
        arguments.extend(segment);
    }
    (callee, arguments)
}

fn is_ambient_scan_name(module: &Module, expr_id: ExprId) -> bool {
    let ExprKind::Name(reference) = &module.exprs()[expr_id].kind else {
        return false;
    };
    let crate::ResolutionState::Resolved(crate::TermResolution::Item(item_id)) =
        reference.resolution.as_ref()
    else {
        return false;
    };
    module.ambient_items().contains(item_id)
        && matches!(
            &module.items()[*item_id],
            Item::Function(function) if function.name.text() == "scan"
        )
}

fn signal_item_reference(module: &Module, expr_id: ExprId) -> Option<ItemId> {
    let ExprKind::Name(reference) = &module.exprs()[expr_id].kind else {
        return None;
    };
    let crate::ResolutionState::Resolved(crate::TermResolution::Item(item_id)) =
        reference.resolution.as_ref()
    else {
        return None;
    };
    matches!(module.items().get(*item_id), Some(Item::Signal(_))).then_some(*item_id)
}

fn elaborate_scan_pipe(
    module: &Module,
    pipe: &PipeExpr,
    scan: &ScanStage,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
) -> RecurrenceNodeOutcome {
    let target = Some(
        RecurrencePlanner::plan(Some(RecurrenceTargetEvidence::SignalItemBody))
            .expect("signal scan nodes should always plan to signal recurrence"),
    );
    let wakeup_signal = signal_item_reference(module, pipe.head);
    let wakeup = wakeup_signal
        .map(|_| RecurrenceWakeupPlan::from_evidence(RecurrenceWakeupEvidence::SignalDependency));

    let mut blockers = Vec::new();
    if wakeup.is_none() {
        blockers.push(RecurrenceElaborationBlocker::MissingWakeup);
    }

    let seed = match lower_gate_runtime_expr(module, scan.seed_expr, env, None, typing) {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(RecurrenceElaborationBlocker::SeedExpressionBlocked(
                recurrence_runtime_stage_blocker(blocker),
            ));
            None
        }
    };

    let mut start = None;
    if let Some(seed) = seed.as_ref() {
        match lower_gate_pipe_function_apply_runtime_expr_allow_signal_reads(
            module,
            scan.stage_span,
            scan.step_expr,
            vec![pipe.head],
            env,
            &seed.ty,
            Some(&seed.ty),
            typing,
        ) {
            Ok(runtime_expr) => {
                let result_subject = runtime_expr.ty.clone();
                if !result_subject.same_shape(&seed.ty) {
                    blockers.push(RecurrenceElaborationBlocker::StepChainDoesNotClose {
                        expected: seed.ty.clone(),
                        found: result_subject.clone(),
                    });
                }
                start = Some(RecurrenceStagePlan {
                    stage_index: scan.stage_index,
                    stage_span: scan.stage_span,
                    expr: scan.stage_expr,
                    input_subject: seed.ty.clone(),
                    result_subject,
                    runtime_expr,
                });
            }
            Err(blocker) => blockers.push(RecurrenceElaborationBlocker::StartStage {
                stage_span: scan.stage_span,
                blocker: recurrence_runtime_stage_blocker(blocker),
            }),
        }
    }

    if blockers.is_empty() {
        let start = start.expect("planned scan nodes should have a start");
        RecurrenceNodeOutcome::Planned(RecurrenceNodePlan {
            target: target.expect("planned scan nodes should have a target"),
            wakeup: wakeup.expect("planned scan nodes should have a wakeup"),
            wakeup_signal,
            seed: seed.expect("planned scan nodes should have a seed"),
            start: start.clone(),
            guards: Vec::new(),
            steps: Vec::new(),
            non_source_wakeup: None,
        })
    } else {
        RecurrenceNodeOutcome::Blocked(BlockedRecurrenceNode {
            target,
            wakeup,
            input_subject: seed.as_ref().map(|expr| expr.ty.clone()),
            blockers,
        })
    }
}

fn elaborate_recurrence_pipe(
    module: &Module,
    pipe: &PipeExpr,
    prefix_stage_count: usize,
    env: &GateExprEnv,
    target_hint: Option<&LocalRecurrenceTargetHint>,
    wakeup_hint: Option<&LocalRecurrenceWakeupHint>,
    typing: &mut GateTypeContext<'_>,
) -> RecurrenceNodeOutcome {
    let suffix = pipe
        .recurrence_suffix()
        .expect("recurrence elaboration expects structurally valid recurrence suffixes")
        .expect("recurrence elaboration should only be called for pipes with a recurrence suffix");

    let mut blockers = Vec::new();

    let target = match target_hint {
        Some(LocalRecurrenceTargetHint::Evidence(evidence)) => Some(
            RecurrencePlanner::plan(Some(*evidence))
                .expect("explicit recurrence target evidence should always plan"),
        ),
        Some(LocalRecurrenceTargetHint::UnsupportedType { ty }) => {
            blockers.push(RecurrenceElaborationBlocker::UnsupportedTarget { ty: ty.clone() });
            None
        }
        None => {
            blockers.push(RecurrenceElaborationBlocker::UnknownTarget);
            None
        }
    };

    let wakeup = match wakeup_hint {
        Some(LocalRecurrenceWakeupHint::BuiltinSource(context)) => {
            match RecurrenceWakeupPlanner::plan_source(*context) {
                Ok(plan) => Some(plan),
                Err(_) => {
                    blockers.push(RecurrenceElaborationBlocker::MissingWakeup);
                    None
                }
            }
        }
        Some(LocalRecurrenceWakeupHint::CustomSource(context)) => {
            match RecurrenceWakeupPlanner::plan_custom_source(*context) {
                Ok(plan) => Some(plan),
                Err(_) => {
                    blockers.push(RecurrenceElaborationBlocker::MissingWakeup);
                    None
                }
            }
        }
        Some(LocalRecurrenceWakeupHint::NonSource { cause, .. }) => Some(
            RecurrenceWakeupPlanner::plan_non_source(*cause)
                .expect("explicit non-source recurrence wakeup witnesses should always plan"),
        ),
        None => {
            blockers.push(RecurrenceElaborationBlocker::MissingWakeup);
            None
        }
    };

    let input_subject = infer_recurrence_input_subject(pipe, prefix_stage_count, env, typing);
    if input_subject.is_none() {
        blockers.push(RecurrenceElaborationBlocker::UnknownInputSubjectType);
    }

    let mut start_plan = None;
    let mut guard_plans = Vec::new();
    let mut step_plans = Vec::new();
    if let Some(input_subject) = input_subject.as_ref() {
        match lower_gate_pipe_body_runtime_expr_allow_signal_reads(
            module,
            suffix.start_expr(),
            env,
            input_subject,
            typing,
        ) {
            Ok(runtime_expr) => {
                let start_subject = runtime_expr.ty.clone();
                start_plan = Some(RecurrenceStagePlan {
                    stage_index: prefix_stage_count,
                    stage_span: suffix.start_stage().span,
                    expr: suffix.start_expr(),
                    input_subject: input_subject.clone(),
                    result_subject: start_subject.clone(),
                    runtime_expr,
                });

                for (offset, stage) in suffix.guard_stages().enumerate() {
                    let PipeStageKind::Gate { expr } = stage.kind else {
                        unreachable!("validated recurrence guards must use `?|>`");
                    };
                    let stage_index = prefix_stage_count + 1 + offset;
                    match lower_recurrence_guard_predicate(
                        module,
                        expr,
                        env,
                        &start_subject,
                        typing,
                    ) {
                        Ok(runtime_predicate) => {
                            guard_plans.push(RecurrenceGuardPlan {
                                stage_index,
                                stage_span: stage.span,
                                predicate: expr,
                                input_subject: start_subject.clone(),
                                runtime_predicate,
                            });
                        }
                        Err(blocker) => {
                            blockers.push(RecurrenceElaborationBlocker::GuardStage {
                                stage_index,
                                stage_span: stage.span,
                                blocker,
                            });
                        }
                    }
                }

                if blockers.is_empty() {
                    let mut current_subject = start_subject.clone();
                    for (offset, stage) in suffix.step_stages().enumerate() {
                        let PipeStageKind::RecurStep { expr } = stage.kind else {
                            unreachable!("validated recurrence suffix steps must use `<|@`");
                        };
                        let stage_index =
                            prefix_stage_count + 1 + suffix.guard_stage_count() + offset;
                        match lower_gate_pipe_body_runtime_expr_allow_signal_reads(
                            module,
                            expr,
                            env,
                            &current_subject,
                            typing,
                        ) {
                            Ok(runtime_expr) => {
                                let result_subject = runtime_expr.ty.clone();
                                step_plans.push(RecurrenceStagePlan {
                                    stage_index,
                                    stage_span: stage.span,
                                    expr,
                                    input_subject: current_subject.clone(),
                                    result_subject: result_subject.clone(),
                                    runtime_expr,
                                });
                                current_subject = result_subject;
                            }
                            Err(blocker) => {
                                blockers.push(RecurrenceElaborationBlocker::StepStage {
                                    stage_index,
                                    stage_span: stage.span,
                                    blocker: recurrence_runtime_stage_blocker(blocker),
                                });
                                break;
                            }
                        }
                    }

                    if blockers.is_empty()
                        && !current_subject
                            .same_shape(&start_plan.as_ref().expect("start exists").result_subject)
                    {
                        blockers.push(RecurrenceElaborationBlocker::StepChainDoesNotClose {
                            expected: start_plan
                                .as_ref()
                                .expect("start exists")
                                .result_subject
                                .clone(),
                            found: current_subject,
                        });
                    }
                }
            }
            Err(blocker) => blockers.push(RecurrenceElaborationBlocker::StartStage {
                stage_span: suffix.start_stage().span,
                blocker: recurrence_runtime_stage_blocker(blocker),
            }),
        }
    }

    let non_source_wakeup = match wakeup_hint {
        Some(LocalRecurrenceWakeupHint::NonSource { cause, witness }) => {
            match lower_gate_runtime_expr(module, *witness, env, None, typing) {
                Ok(runtime_witness) => Some(RecurrenceNonSourceWakeupBinding {
                    cause: *cause,
                    witness: *witness,
                    runtime_witness,
                }),
                Err(blocker) => {
                    blockers.push(RecurrenceElaborationBlocker::NonSourceWakeupWitness(
                        recurrence_runtime_stage_blocker(blocker),
                    ));
                    None
                }
            }
        }
        Some(LocalRecurrenceWakeupHint::BuiltinSource(_))
        | Some(LocalRecurrenceWakeupHint::CustomSource(_))
        | None => None,
    };

    // Elaborate the pipe head as the seed expression (no ambient subject).
    let seed_result = lower_gate_runtime_expr(module, pipe.head, env, None, typing);
    let seed = match seed_result {
        Ok(expr) => Some(expr),
        Err(blocker) => {
            blockers.push(RecurrenceElaborationBlocker::SeedExpressionBlocked(
                recurrence_runtime_stage_blocker(blocker),
            ));
            None
        }
    };

    if blockers.is_empty() {
        RecurrenceNodeOutcome::Planned(RecurrenceNodePlan {
            target: target.expect("planned recurrence nodes should have a target"),
            wakeup: wakeup.expect("planned recurrence nodes should have a wakeup"),
            wakeup_signal: None,
            seed: seed.expect("planned recurrence nodes should have a seed"),
            start: start_plan.expect("planned recurrence nodes should have a start stage"),
            guards: guard_plans,
            steps: step_plans,
            non_source_wakeup,
        })
    } else {
        RecurrenceNodeOutcome::Blocked(BlockedRecurrenceNode {
            target,
            wakeup,
            input_subject,
            blockers,
        })
    }
}

fn lower_recurrence_guard_predicate(
    module: &Module,
    predicate: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) -> Result<RecurrenceRuntimeExpr, RecurrenceRuntimeStageBlocker> {
    let predicate_span = module.exprs()[predicate].span;
    let predicate_info = typing.infer_pipe_body(predicate, env, subject);
    if let Some(issue) = predicate_info.issues.into_iter().next() {
        return Err(recurrence_guard_issue_blocker(issue, predicate_span));
    }
    if predicate_info.contains_signal || predicate_info.ty.as_ref().is_some_and(GateType::is_signal)
    {
        return Err(RecurrenceRuntimeStageBlocker::ImpureExpr);
    }
    let Some(predicate_ty) = predicate_info.ty else {
        return Err(RecurrenceRuntimeStageBlocker::UnknownExprType {
            span: predicate_span,
        });
    };
    if !predicate_ty.is_bool() {
        return Err(RecurrenceRuntimeStageBlocker::PredicateNotBool {
            found: predicate_ty,
        });
    }
    lower_gate_pipe_body_runtime_expr(module, predicate, env, subject, typing)
        .map_err(recurrence_runtime_stage_blocker)
}

fn recurrence_guard_issue_blocker(
    issue: GateIssue,
    span: SourceSpan,
) -> RecurrenceRuntimeStageBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            RecurrenceRuntimeStageBlocker::InvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            RecurrenceRuntimeStageBlocker::UnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { .. }
        | GateIssue::AmbientSubjectOutsidePipe { .. }
        | GateIssue::AmbiguousDomainOperator { .. }
        | GateIssue::InvalidPipeStageInput { .. }
        | GateIssue::UnsupportedApplicativeClusterMember { .. }
        | GateIssue::ApplicativeClusterMismatch { .. }
        | GateIssue::InvalidClusterFinalizer { .. }
        | GateIssue::CaseBranchTypeMismatch { .. } => {
            RecurrenceRuntimeStageBlocker::UnknownExprType { span }
        }
    }
}

fn infer_recurrence_input_subject(
    pipe: &PipeExpr,
    prefix_stage_count: usize,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
) -> Option<GateType> {
    // Collect the limited stage slice once so the truthy/falsy pair helper can
    // index into it by position.  The walker is constructed with the same limit
    // so it only iterates over these prefix stages (PA-M1).
    let all_stages = pipe
        .stages
        .iter()
        .take(prefix_stage_count)
        .collect::<Vec<_>>();
    PipeSubjectWalker::new_with_limit(pipe, env, typing, prefix_stage_count).walk(
        typing,
        |stage_index, stage, current, current_env, typing| match &stage.kind {
            PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                new_subject: current
                    .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                advance_by: 1,
            },
            PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                new_subject: current
                    .and_then(|s| typing.infer_fanout_map_stage(*expr, current_env, s)),
                advance_by: 1,
            },
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
                let new_subject = current
                    .and_then(|s| typing.infer_truthy_falsy_pair(&pair, current_env, s));
                let advance = pair.next_index.saturating_sub(stage_index).max(1);
                PipeSubjectStepOutcome::Continue {
                    new_subject,
                    advance_by: advance,
                }
            }
            // Recurrence boundary stages and unhandled case/apply stages should
            // never appear within the prefix (the caller computes prefix_stage_count
            // to exclude them), but stop cleanly if one is encountered.
            PipeStageKind::Case { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. }
            | PipeStageKind::Validate { .. }
            | PipeStageKind::Previous { .. }
            | PipeStageKind::Diff { .. }
            | PipeStageKind::Accumulate { .. } => PipeSubjectStepOutcome::Stop,
            // Transform and Tap are handled by PipeSubjectWalker before the
            // callback is invoked; they can never reach this arm.
            PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                unreachable!("Transform/Tap are consumed by PipeSubjectWalker before the callback")
            }
        },
    )
}

fn recurrence_target_hint_for_annotation(
    typing: &mut GateTypeContext<'_>,
    annotation: crate::TypeId,
) -> Option<LocalRecurrenceTargetHint> {
    let ty = typing.lower_annotation(annotation)?;
    Some(match ty {
        GateType::Signal(_) => {
            LocalRecurrenceTargetHint::Evidence(RecurrenceTargetEvidence::ExplicitSignalAnnotation)
        }
        GateType::Task { .. } => {
            LocalRecurrenceTargetHint::Evidence(RecurrenceTargetEvidence::ExplicitTaskAnnotation)
        }
        _ => LocalRecurrenceTargetHint::UnsupportedType { ty },
    })
}

fn recurrence_wakeup_hint_for_decorators(
    module: &Module,
    decorators: &[DecoratorId],
) -> Option<LocalRecurrenceWakeupHint> {
    decorators.iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        let DecoratorPayload::RecurrenceWakeup(ref wakeup) = decorator.payload else {
            return None;
        };
        Some(LocalRecurrenceWakeupHint::NonSource {
            cause: match wakeup.kind {
                crate::RecurrenceWakeupDecoratorKind::Timer => NonSourceWakeupCause::ExplicitTimer,
                crate::RecurrenceWakeupDecoratorKind::Backoff => {
                    NonSourceWakeupCause::ExplicitBackoff
                }
            },
            witness: wakeup.witness,
        })
    })
}

fn recurrence_wakeup_hint_for_signal(
    module: &Module,
    item: &SignalItem,
) -> Option<LocalRecurrenceWakeupHint> {
    let Some(source) = signal_source_decorator(module, item) else {
        return recurrence_wakeup_hint_for_decorators(module, &item.header.decorators);
    };
    let provider = source.provider.as_ref()?;
    let metadata = item.source_metadata.as_ref();
    let provider_ref = metadata
        .map(|metadata| metadata.provider.clone())
        .unwrap_or_else(|| SourceProviderRef::from_path(Some(provider)));
    let Some(provider) = provider_ref.builtin() else {
        let mut context = CustomSourceRecurrenceWakeupContext::new();
        if metadata.is_some_and(SourceMetadata::has_reactive_wakeup_inputs) {
            context = context.with_reactive_inputs();
        }
        if let Some(wakeup) = metadata
            .and_then(|metadata| metadata.custom_contract.clone())
            .and_then(|contract| contract.recurrence_wakeup)
        {
            context = context.with_declared_wakeup(custom_source_wakeup_kind(wakeup));
        }
        return Some(LocalRecurrenceWakeupHint::CustomSource(context));
    };

    let mut context = SourceRecurrenceWakeupContext::new(provider);
    if metadata.is_some_and(SourceMetadata::has_reactive_wakeup_inputs) {
        context = context.with_reactive_inputs();
    }
    let contract = provider.contract();
    if let Some(options) = source.options {
        if let ExprKind::Record(record) = &module.exprs()[options].kind {
            for field in &record.fields {
                let Some(cause) = contract
                    .wakeup_option(field.label.text())
                    .map(|option| builtin_source_option_wakeup_cause(option.cause()))
                else {
                    continue;
                };
                context = match cause {
                    BuiltinSourceWakeupCause::RetryPolicy => context.with_retry_policy(),
                    BuiltinSourceWakeupCause::PollingPolicy => context.with_polling_policy(),
                    BuiltinSourceWakeupCause::TriggerSignal => context.with_signal_trigger(),
                    BuiltinSourceWakeupCause::ProviderTimer
                    | BuiltinSourceWakeupCause::ReactiveInputs
                    | BuiltinSourceWakeupCause::ProviderDefinedTrigger => context,
                };
            }
        }
    }
    Some(LocalRecurrenceWakeupHint::BuiltinSource(context))
}

fn signal_source_decorator<'a>(
    module: &'a Module,
    item: &SignalItem,
) -> Option<&'a SourceDecorator> {
    item.header.decorators.iter().find_map(|decorator_id| {
        let decorator = module.decorators().get(*decorator_id)?;
        match &decorator.payload {
            DecoratorPayload::Source(source) => Some(source),
            _ => None,
        }
    })
}

// recurrence_env_for_function is now the shared crate::validate::gate_env_for_function (PA-I2).

fn recurrence_runtime_stage_blocker(
    blocker: GateElaborationBlocker,
) -> RecurrenceRuntimeStageBlocker {
    match blocker {
        GateElaborationBlocker::InvalidProjection { path, subject } => {
            RecurrenceRuntimeStageBlocker::InvalidProjection { path, subject }
        }
        GateElaborationBlocker::UnknownField { path, subject } => {
            RecurrenceRuntimeStageBlocker::UnknownField { path, subject }
        }
        GateElaborationBlocker::ImpurePredicate => RecurrenceRuntimeStageBlocker::ImpureExpr,
        GateElaborationBlocker::UnknownRuntimeExprType { span } => {
            RecurrenceRuntimeStageBlocker::UnknownExprType { span }
        }
        GateElaborationBlocker::PredicateNotBool { found } => {
            RecurrenceRuntimeStageBlocker::PredicateNotBool { found }
        }
        GateElaborationBlocker::UnsupportedRuntimeExpr { span, kind } => {
            RecurrenceRuntimeStageBlocker::UnsupportedExpr { span, kind }
        }
        GateElaborationBlocker::UnknownSubjectType
        | GateElaborationBlocker::UnknownPredicateType => {
            // These variants are emitted by `elaborate_gate_stage` when the
            // subject type is not yet known — a subject-level concern, not a
            // runtime-expression concern.  `lower_gate_pipe_body_runtime_expr`
            // is only called after the subject type is confirmed (it takes
            // `subject: &GateType`, not `Option<&GateType>`), so these arms
            // are unreachable today.
            //
            // If `GateElaborationBlocker` is ever split into
            // `GateSubjectBlocker` + `GateRuntimeBlocker` (PA-I1), this match
            // arm should be removed and `recurrence_runtime_stage_blocker`
            // should accept only `GateRuntimeBlocker`, making the invariant
            // compiler-enforced rather than a runtime assertion.
            unreachable!(
                "lower_gate_pipe_body_runtime_expr takes a concrete subject and cannot emit \
                 subject-only gate blockers; this arm is evidence the type should be split (PA-I1)"
            )
        }
    }
}

fn custom_source_wakeup_kind(
    wakeup: CustomSourceRecurrenceWakeup,
) -> aivi_typing::RecurrenceWakeupKind {
    match wakeup {
        CustomSourceRecurrenceWakeup::Timer => aivi_typing::RecurrenceWakeupKind::Timer,
        CustomSourceRecurrenceWakeup::Backoff => aivi_typing::RecurrenceWakeupKind::Backoff,
        CustomSourceRecurrenceWakeup::SourceEvent => aivi_typing::RecurrenceWakeupKind::SourceEvent,
        CustomSourceRecurrenceWakeup::ProviderDefinedTrigger => {
            aivi_typing::RecurrenceWakeupKind::ProviderDefinedTrigger
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;
    use aivi_typing::{RecurrenceTarget, RecurrenceWakeupEvidence, RecurrenceWakeupKind};

    use super::{RecurrenceElaborationBlocker, RecurrenceNodeOutcome, elaborate_recurrences};
    use crate::{GateRuntimeExprKind, GateRuntimeProjectionBase, GateType, Item, lower_module};

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
            Item::SourceProviderContract(_)
            | Item::Instance(_)
            | Item::Use(_)
            | Item::Export(_) => "<anonymous>",
        }
    }

    #[test]
    fn elaborates_explicit_recurrences_into_scheduler_node_plans() {
        let lowered = lower_fixture("milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi");
        assert!(
            !lowered.has_errors(),
            "explicit recurrence fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_recurrences(lowered.module());
        assert_eq!(
            report.nodes().len(),
            2,
            "expected one signal and one task recurrence scheduler node"
        );

        let signal = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "polled")
            .expect("expected recurrence plan for polled");
        match &signal.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.target.target(), RecurrenceTarget::Signal);
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::Timer);
                assert_eq!(plan.start.stage_index, 0);
                assert!(plan.guards.is_empty());
                assert_eq!(plan.steps.len(), 1);
                assert_eq!(
                    plan.start.result_subject,
                    GateType::Primitive(crate::BuiltinType::Int)
                );
                let witness = plan
                    .non_source_wakeup
                    .as_ref()
                    .expect("signal recurrence should carry a non-source wakeup witness");
                assert_eq!(
                    witness.cause,
                    aivi_typing::NonSourceWakeupCause::ExplicitTimer
                );
                assert!(matches!(
                    witness.runtime_witness.kind,
                    GateRuntimeExprKind::SuffixedInteger(_)
                ));
            }
            other => panic!("expected planned signal recurrence node, found {other:?}"),
        }

        let task = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "retried")
            .expect("expected recurrence plan for retried");
        match &task.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.target.target(), RecurrenceTarget::Task);
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::Backoff);
                assert!(plan.guards.is_empty());
                let witness = plan
                    .non_source_wakeup
                    .as_ref()
                    .expect("task recurrence should carry a non-source wakeup witness");
                assert_eq!(
                    witness.cause,
                    aivi_typing::NonSourceWakeupCause::ExplicitBackoff
                );
                assert!(matches!(
                    witness.runtime_witness.kind,
                    GateRuntimeExprKind::SuffixedInteger(_)
                ));
            }
            other => panic!("expected planned task recurrence node, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_scan_signal_wakeups() {
        let lowered = lower_fixture("milestone-2/valid/pipe-scan-signal-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "timer scan fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_recurrences(lowered.module());
        let built_in = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "retried")
            .expect("expected scan node for retried");
        match &built_in.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::SourceEvent);
                assert!(plan.guards.is_empty());
                assert!(matches!(
                    plan.wakeup.evidence(),
                    RecurrenceWakeupEvidence::SignalDependency
                ));
                assert_eq!(
                    item_name(
                        lowered.module(),
                        plan.wakeup_signal
                            .expect("scan should preserve its upstream wakeup signal"),
                    ),
                    "tick"
                );
                assert!(
                    plan.non_source_wakeup.is_none(),
                    "scan wakeups should not invent a non-source witness"
                );
            }
            other => panic!("expected planned timer scan node, found {other:?}"),
        }

        let lowered = lower_fixture("milestone-2/valid/custom-source-recurrence-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "custom source scan fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_recurrences(lowered.module());
        let custom = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "updates")
            .expect("expected custom source scan node");
        match &custom.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::SourceEvent);
                assert!(plan.guards.is_empty());
                assert!(matches!(
                    plan.wakeup.evidence(),
                    RecurrenceWakeupEvidence::SignalDependency
                ));
                assert_eq!(
                    item_name(
                        lowered.module(),
                        plan.wakeup_signal
                            .expect("scan should preserve its upstream wakeup signal"),
                    ),
                    "updateEvents"
                );
            }
            other => panic!("expected planned custom source scan node, found {other:?}"),
        }
    }

    #[test]
    fn active_when_bodyless_sources_still_feed_scan_signals() {
        let lowered = lower_text(
            "scan_active_when_source.aivi",
            r#"
fun step:Int n:Int current:Int =>
    n

signal enabled = True

@source http.get "/users" with {
    activeWhen: enabled
}
signal userEvents : Signal Int

signal gated : Signal Int =
    userEvents
     |> scan 0 step
"#,
        );
        assert!(
            !lowered.has_errors(),
            "activeWhen scan example should lower cleanly before elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_recurrences(lowered.module());
        let gated = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "gated")
            .expect("expected recurrence plan for gated");
        match &gated.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::SourceEvent);
                assert!(plan.guards.is_empty());
                assert!(matches!(
                    plan.wakeup.evidence(),
                    RecurrenceWakeupEvidence::SignalDependency
                ));
                assert_eq!(
                    item_name(
                        lowered.module(),
                        plan.wakeup_signal
                            .expect("scan should preserve its upstream wakeup signal"),
                    ),
                    "userEvents"
                );
            }
            other => panic!("expected planned activeWhen scan node, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_recurrence_guards_before_steps() {
        let lowered = lower_text(
            "recurrence-guard.aivi",
            r#"
domain Duration over Int
    literal sec : Int -> Duration

type Cursor = {
    hasNext: Bool
}

fun keep:Cursor cursor:Cursor =>
    cursor

value seed:Cursor = { hasNext: True }

@recur.timer 1sec
signal cursor : Signal Cursor =
    seed
     @|> keep
     ?|> .hasNext
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "recurrence guard example should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_recurrences(lowered.module());
        let guarded = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "cursor")
            .expect("expected recurrence plan for cursor");

        match &guarded.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.guards.len(), 1);
                assert_eq!(plan.guards[0].input_subject, plan.start.result_subject);
                match &plan.guards[0].runtime_predicate.kind {
                    GateRuntimeExprKind::Projection { base, path } => {
                        assert_eq!(base, &GateRuntimeProjectionBase::AmbientSubject);
                        assert_eq!(path.to_string(), "hasNext");
                    }
                    other => panic!("expected ambient projection guard predicate, found {other:?}"),
                }
            }
            other => panic!("expected planned guarded recurrence node, found {other:?}"),
        }
    }

    #[test]
    fn blocks_missing_recurrence_wakeups() {
        let lowered = lower_fixture("milestone-2/invalid/missing-recurrence-wakeup/main.aivi");
        let report = elaborate_recurrences(lowered.module());
        let blocked = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "broken")
            .expect("expected blocked recurrence node");

        match &blocked.outcome {
            RecurrenceNodeOutcome::Blocked(node) => {
                assert!(
                    node.blockers
                        .contains(&RecurrenceElaborationBlocker::MissingWakeup),
                    "expected missing wakeup blocker, found {:?}",
                    node.blockers
                );
            }
            other => panic!("expected blocked recurrence node, found {other:?}"),
        }
    }

    #[test]
    fn blocks_recurrence_step_chains_that_do_not_close() {
        let lowered = lower_text(
            "recurrence-step-chain-mismatch.aivi",
            r#"
domain Duration over Int
    literal sec : Int -> Duration

fun keep n:Int =>
    n

fun asText n:Int =>
    "oops"

@recur.timer 5sec
signal broken : Signal Int =
    0
     @|> keep
     <|@ asText
"#,
        );
        assert!(
            !lowered.has_errors(),
            "closure-mismatch recurrence example should lower cleanly before elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_recurrences(lowered.module());
        let blocked = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "broken")
            .expect("expected blocked recurrence node");

        match &blocked.outcome {
            RecurrenceNodeOutcome::Blocked(node) => {
                assert!(node.blockers.iter().any(|blocker| matches!(
                    blocker,
                    RecurrenceElaborationBlocker::StepChainDoesNotClose { expected, found }
                    if *expected == GateType::Primitive(crate::BuiltinType::Int)
                        && *found == GateType::Primitive(crate::BuiltinType::Text)
                )));
            }
            other => panic!("expected blocked recurrence node, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_recurrence_start_and_steps_that_read_signals() {
        let lowered = lower_text(
            "recurrence-signal-reads-in-update-stages.aivi",
            r#"
domain Duration over Int
    literal sec : Int -> Duration

fun advance:Int pressed:Bool n:Int =>
    pressed
     T|> n + 1
     F|> n

fun belowLimit:Bool n:Int =>
    n < 10

signal ready : Signal Bool = True

@recur.timer 5sec
signal counter : Signal Int =
    0
     @|> advance ready
     ?|> belowLimit
     <|@ advance ready
"#,
        );
        assert!(
            !lowered.has_errors(),
            "signal-reading recurrence update example should lower cleanly before elaboration: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_recurrences(lowered.module());
        let counter = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "counter")
            .expect("expected recurrence plan for counter");

        match &counter.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(
                    plan.start.result_subject,
                    GateType::Primitive(crate::BuiltinType::Int)
                );
                assert_eq!(plan.guards.len(), 1);
                assert_eq!(plan.steps.len(), 1);
                assert_eq!(
                    plan.start.result_subject,
                    GateType::Primitive(crate::BuiltinType::Int)
                );
            }
            other => panic!("expected planned recurrence node, found {other:?}"),
        }
    }
}
