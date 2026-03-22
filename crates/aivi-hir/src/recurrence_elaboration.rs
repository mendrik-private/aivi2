use aivi_base::SourceSpan;
use aivi_typing::{
    BuiltinSourceWakeupCause, CustomSourceRecurrenceWakeupContext, NonSourceWakeupCause,
    RecurrencePlan, RecurrencePlanner, RecurrenceTargetEvidence, RecurrenceWakeupPlan,
    RecurrenceWakeupPlanner, SourceRecurrenceWakeupContext, builtin_source_option_wakeup_cause,
};

use crate::{
    CustomSourceRecurrenceWakeup, DecoratorId, DecoratorPayload, ExprId, ExprKind, Item, ItemId,
    Module, PipeExpr, PipeStageKind, SignalItem, SourceDecorator, SourceProviderRef,
    gate_elaboration::{
        GateElaborationBlocker, GateRuntimeExpr, GateRuntimeUnsupportedKind,
        lower_gate_pipe_body_runtime_expr, lower_gate_runtime_expr,
    },
    validate::{GateExprEnv, GateType, GateTypeContext, walk_expr_tree},
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
/// RFC names those roles separately, but the full runtime/source-lifecycle story that will consume
/// this handoff is still being built.
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
pub struct RecurrenceNonSourceWakeupBinding {
    pub cause: NonSourceWakeupCause,
    pub witness: ExprId,
    pub runtime_witness: RecurrenceRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceNodePlan {
    pub target: RecurrencePlan,
    pub wakeup: RecurrenceWakeupPlan,
    pub start: RecurrenceStagePlan,
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
    StepStage {
        stage_index: usize,
        stage_span: SourceSpan,
        blocker: RecurrenceRuntimeStageBlocker,
    },
    NonSourceWakeupWitness(RecurrenceRuntimeStageBlocker),
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
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();
    let mut typing = GateTypeContext::new(module);

    for (owner, item) in items {
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
                let env = recurrence_env_for_function(&item, &mut typing);
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
    let mut step_plans = Vec::new();
    if let Some(input_subject) = input_subject.as_ref() {
        match lower_gate_pipe_body_runtime_expr(
            module,
            suffix.start_expr(),
            env,
            input_subject,
            typing,
        ) {
            Ok(runtime_expr) => {
                let mut current_subject = runtime_expr.ty.clone();
                start_plan = Some(RecurrenceStagePlan {
                    stage_index: prefix_stage_count,
                    stage_span: suffix.start_stage().span,
                    expr: suffix.start_expr(),
                    input_subject: input_subject.clone(),
                    result_subject: current_subject.clone(),
                    runtime_expr,
                });

                for (offset, stage) in suffix.step_stages().enumerate() {
                    let PipeStageKind::RecurStep { expr } = stage.kind else {
                        unreachable!("validated recurrence suffix steps must use `<|@`");
                    };
                    let stage_index = prefix_stage_count + 1 + offset;
                    match lower_gate_pipe_body_runtime_expr(
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

    if blockers.is_empty() {
        RecurrenceNodeOutcome::Planned(RecurrenceNodePlan {
            target: target.expect("planned recurrence nodes should have a target"),
            wakeup: wakeup.expect("planned recurrence nodes should have a wakeup"),
            start: start_plan.expect("planned recurrence nodes should have a start stage"),
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

fn infer_recurrence_input_subject(
    pipe: &PipeExpr,
    prefix_stage_count: usize,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
) -> Option<GateType> {
    let mut current = typing.infer_expr(pipe.head, env, None).ty?;
    for stage in pipe.stages.iter().take(prefix_stage_count) {
        match &stage.kind {
            PipeStageKind::Transform { expr } => {
                current = typing.infer_transform_stage(*expr, env, &current)?;
            }
            PipeStageKind::Tap { expr } => {
                let _ = typing.infer_pipe_body(*expr, env, &current);
            }
            PipeStageKind::Gate { expr } => {
                current = typing.infer_gate_stage(*expr, env, &current)?;
            }
            PipeStageKind::Map { expr } => {
                current = typing.infer_fanout_map_stage(*expr, env, &current)?;
            }
            PipeStageKind::FanIn { expr } => {
                current = typing.infer_fanin_stage(*expr, env, &current)?;
            }
            PipeStageKind::Case { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::Truthy { .. }
            | PipeStageKind::Falsy { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. } => return None,
        }
    }
    Some(current)
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
        if metadata.is_some_and(|metadata| metadata.is_reactive) {
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
    if metadata.is_some_and(|metadata| metadata.is_reactive) {
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

fn recurrence_env_for_function(
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
            unreachable!("runtime expression lowering should not emit subject-only gate blockers")
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
    use aivi_typing::{
        BuiltinSourceWakeupCause, CustomSourceWakeupCause, RecurrenceTarget, RecurrenceWakeupKind,
    };

    use super::{RecurrenceElaborationBlocker, RecurrenceNodeOutcome, elaborate_recurrences};
    use crate::{GateRuntimeExprKind, GateType, Item, lower_module};

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
    fn elaborates_nonsource_recurrences_into_scheduler_node_plans() {
        let lowered = lower_fixture("milestone-2/valid/pipe-recurrence-nonsource-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "non-source recurrence fixture should lower cleanly: {:?}",
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
    fn elaborates_builtin_and_custom_source_wakeups() {
        let lowered = lower_fixture("milestone-2/valid/pipe-recurrence-suffix/main.aivi");
        assert!(
            !lowered.has_errors(),
            "built-in source recurrence fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_recurrences(lowered.module());
        let built_in = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "retried")
            .expect("expected built-in source recurrence node");
        match &built_in.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::Timer);
                assert_eq!(
                    plan.wakeup.evidence().builtin_source_cause(),
                    Some(BuiltinSourceWakeupCause::ProviderTimer)
                );
                assert!(
                    plan.non_source_wakeup.is_none(),
                    "source-backed wakeups should not invent a non-source witness"
                );
            }
            other => panic!("expected planned built-in source recurrence node, found {other:?}"),
        }

        let lowered = lower_fixture("milestone-2/valid/custom-source-recurrence-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "custom source recurrence fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_recurrences(lowered.module());
        let custom = report
            .nodes()
            .iter()
            .find(|node| item_name(lowered.module(), node.owner) == "updates")
            .expect("expected custom source recurrence node");
        match &custom.outcome {
            RecurrenceNodeOutcome::Planned(plan) => {
                assert_eq!(plan.wakeup.kind(), RecurrenceWakeupKind::SourceEvent);
                assert_eq!(
                    plan.wakeup.evidence().custom_source_cause(),
                    Some(CustomSourceWakeupCause::ReactiveInputs)
                );
            }
            other => panic!("expected planned custom source recurrence node, found {other:?}"),
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
    literal s : Int -> Duration

fun keep #value:Int =>
    value

fun asText #value:Int =>
    "oops"

@recur.timer 5s
sig broken : Signal Int =
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
}
