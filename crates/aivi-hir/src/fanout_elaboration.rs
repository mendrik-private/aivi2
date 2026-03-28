use aivi_base::SourceSpan;
use aivi_typing::{FanoutCarrier, FanoutPlanner, FanoutStageKind};

use crate::{
    ExprId, Item, ItemId, Module, PipeExpr, PipeStageKind,
    gate_elaboration::{
        GateElaborationBlocker, GateRuntimeExpr, GateRuntimeUnsupportedKind,
        lower_gate_pipe_body_runtime_expr,
    },
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, PipeSubjectStepOutcome,
        PipeSubjectWalker, truthy_falsy_pair_stages, walk_expr_tree,
    },
};

/// Focused fan-out plans derived from resolved HIR.
///
/// This is intentionally narrower than a future full typed-core IR. It exposes the RFC §11.5
/// carrier split in a typed, presentation-free form: the report records the current collection
/// subject, the element subject used inside `*|>`, the mapped collection result, and the optional
/// `<|*` reduction that follows immediately after that map stage. When the current local inference
/// cannot justify a segment, the report records explicit blockers instead of guessing a core plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FanoutElaborationReport {
    segments: Vec<FanoutSegmentElaboration>,
}

impl FanoutElaborationReport {
    pub fn new(segments: Vec<FanoutSegmentElaboration>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[FanoutSegmentElaboration] {
        &self.segments
    }

    pub fn into_segments(self) -> Vec<FanoutSegmentElaboration> {
        self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutSegmentElaboration {
    pub owner: ItemId,
    pub pipe_expr: ExprId,
    pub map_stage_index: usize,
    pub map_stage_span: SourceSpan,
    pub map_expr: ExprId,
    pub outcome: FanoutSegmentOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FanoutSegmentOutcome {
    Planned(FanoutSegmentPlan),
    Blocked(BlockedFanoutSegment),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutSegmentPlan {
    pub carrier: FanoutCarrier,
    pub input_subject: GateType,
    pub element_subject: GateType,
    pub mapped_element_type: GateType,
    pub mapped_collection_type: GateType,
    pub runtime_map: GateRuntimeExpr,
    pub filters: Vec<FanoutFilterPlan>,
    pub join: Option<FanoutJoinPlan>,
    pub result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutFilterPlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub predicate: ExprId,
    pub input_subject: GateType,
    pub runtime_predicate: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutJoinPlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub expr: ExprId,
    pub input_subject: GateType,
    pub collection_subject: GateType,
    pub runtime_expr: GateRuntimeExpr,
    pub result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedFanoutSegment {
    pub subject: Option<GateType>,
    pub blockers: Vec<FanoutElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FanoutElaborationBlocker {
    UnknownSubjectType,
    SubjectNotCollection {
        found: GateType,
    },
    MapInvalidProjection {
        path: String,
        subject: String,
    },
    MapUnknownField {
        path: String,
        subject: String,
    },
    UnknownMapBodyType,
    FilterStage {
        stage_index: usize,
        stage_span: SourceSpan,
        blocker: FanoutFilterBlocker,
    },
    JoinInvalidProjection {
        path: String,
        subject: String,
    },
    JoinUnknownField {
        path: String,
        subject: String,
    },
    UnknownJoinBodyType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FanoutFilterBlocker {
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

pub fn elaborate_fanouts(module: &Module) -> FanoutElaborationReport {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    let module = &module;
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut typing = GateTypeContext::new(module);

    for (owner, item) in items {
        match item {
            Item::Value(item) => collect_fanout_segments(
                module,
                owner,
                item.body,
                &GateExprEnv::default(),
                &mut typing,
                &mut segments,
            ),
            Item::Function(item) => {
                let env = fanout_env_for_function(&item, &mut typing);
                collect_fanout_segments(module, owner, item.body, &env, &mut typing, &mut segments);
            }
            Item::Signal(item) => {
                if let Some(body) = item.body {
                    collect_fanout_segments(
                        module,
                        owner,
                        body,
                        &GateExprEnv::default(),
                        &mut typing,
                        &mut segments,
                    );
                }
            }
            Item::Instance(item) => {
                for member in item.members {
                    collect_fanout_segments(
                        module,
                        owner,
                        member.body,
                        &GateExprEnv::default(),
                        &mut typing,
                        &mut segments,
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

    FanoutElaborationReport::new(segments)
}

fn collect_fanout_segments(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    segments: &mut Vec<FanoutSegmentElaboration>,
) {
    walk_expr_tree(module, root, |pipe_expr, expr, _| {
        if let crate::ExprKind::Pipe(pipe) = &expr.kind {
            collect_fanout_pipe(module, owner, pipe_expr, pipe, env, typing, segments);
        }
    });
}

fn collect_fanout_pipe(
    module: &Module,
    owner: ItemId,
    pipe_expr: ExprId,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    segments: &mut Vec<FanoutSegmentElaboration>,
) {
    // Collect the stages snapshot once for truthy_falsy_pair_stages lookups.
    let all_stages = pipe.stages.iter().collect::<Vec<_>>();
    PipeSubjectWalker::new(pipe, env, typing).walk(
        typing,
        |stage_index, stage, current, current_env, typing| {
            match &stage.kind {
                PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                    new_subject: current
                        .and_then(|s| typing.infer_gate_stage(*expr, current_env, s)),
                    advance_by: 1,
                },
                PipeStageKind::Map { expr } => {
                    let segment = pipe
                        .fanout_segment(stage_index)
                        .expect("map stages should expose a fan-out segment");
                    let outcome =
                        elaborate_fanout_segment(module, &segment, current, current_env, typing);
                    segments.push(FanoutSegmentElaboration {
                        owner,
                        pipe_expr,
                        map_stage_index: stage_index,
                        map_stage_span: stage.span,
                        map_expr: *expr,
                        outcome: outcome.clone(),
                    });
                    let advance = segment.next_stage_index().saturating_sub(stage_index);
                    PipeSubjectStepOutcome::Continue {
                        new_subject: match outcome {
                            FanoutSegmentOutcome::Planned(plan) => Some(plan.result_type),
                            FanoutSegmentOutcome::Blocked(_) => None,
                        },
                        advance_by: advance.max(1),
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
                    let advance = pair.next_index.saturating_sub(stage_index);
                    PipeSubjectStepOutcome::Continue {
                        new_subject: current
                            .and_then(|s| typing.infer_truthy_falsy_pair(&pair, current_env, s)),
                        advance_by: advance.max(1),
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
                // Transform and Tap are handled by the walker itself.
                PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                    unreachable!("PipeSubjectWalker handles Transform and Tap internally")
                }
            }
        },
    );
}

pub(crate) fn elaborate_fanout_segment(
    module: &Module,
    segment: &crate::PipeFanoutSegment<'_>,
    subject: Option<&GateType>,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
) -> FanoutSegmentOutcome {
    let Some(subject) = subject else {
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: None,
            blockers: vec![FanoutElaborationBlocker::UnknownSubjectType],
        });
    };
    let Some(carrier) = typing.fanout_carrier(subject) else {
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: Some(subject.clone()),
            blockers: vec![FanoutElaborationBlocker::SubjectNotCollection {
                found: subject.clone(),
            }],
        });
    };
    let Some(element_subject) = subject.fanout_element().cloned() else {
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: Some(subject.clone()),
            blockers: vec![FanoutElaborationBlocker::SubjectNotCollection {
                found: subject.clone(),
            }],
        });
    };

    let map_info = typing.infer_pipe_body(segment.map_expr(), env, &element_subject);
    let mut blockers = map_info
        .issues
        .into_iter()
        .map(blocker_for_map_issue)
        .collect::<Vec<_>>();
    let Some(mapped_element_type) = map_info.ty else {
        if blockers.is_empty() {
            blockers.push(FanoutElaborationBlocker::UnknownMapBodyType);
        }
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: Some(subject.clone()),
            blockers,
        });
    };
    if !blockers.is_empty() {
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: Some(subject.clone()),
            blockers,
        });
    }
    let runtime_map = match lower_gate_pipe_body_runtime_expr(
        module,
        segment.map_expr(),
        env,
        &element_subject,
        typing,
    ) {
        Ok(runtime_map) => runtime_map,
        Err(blocker) => {
            return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
                subject: Some(subject.clone()),
                blockers: vec![blocker_for_map_runtime_blocker(blocker)],
            });
        }
    };

    let mut filters = Vec::new();
    for (offset, stage) in segment.filter_stages().enumerate() {
        let PipeStageKind::Gate { expr } = stage.kind else {
            unreachable!("validated fan-out filters must use `?|>`");
        };
        let stage_index = segment.map_stage_index() + 1 + offset;
        match lower_fanout_filter_predicate(module, expr, env, &mapped_element_type, typing) {
            Ok(runtime_predicate) => {
                filters.push(FanoutFilterPlan {
                    stage_index,
                    stage_span: stage.span,
                    predicate: expr,
                    input_subject: mapped_element_type.clone(),
                    runtime_predicate,
                });
            }
            Err(blocker) => {
                blockers.push(FanoutElaborationBlocker::FilterStage {
                    stage_index,
                    stage_span: stage.span,
                    blocker,
                });
            }
        }
    }
    if !blockers.is_empty() {
        return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
            subject: Some(subject.clone()),
            blockers,
        });
    }

    let mapped_collection_type = typing.apply_fanout_plan(
        FanoutPlanner::plan(FanoutStageKind::Map, carrier),
        mapped_element_type.clone(),
    );
    let mut result_type = mapped_collection_type.clone();
    let mut join = None;

    if let Some((stage_index, stage)) = segment.join_stage_index().zip(segment.join_stage()) {
        let join_expr = segment
            .join_expr()
            .expect("join stage index implies a join expression");
        let join_info = typing.infer_pipe_body(join_expr, env, &mapped_collection_type);
        let mut join_blockers = join_info
            .issues
            .into_iter()
            .map(blocker_for_join_issue)
            .collect::<Vec<_>>();
        let Some(join_value_type) = join_info.ty else {
            if join_blockers.is_empty() {
                join_blockers.push(FanoutElaborationBlocker::UnknownJoinBodyType);
            }
            return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
                subject: Some(subject.clone()),
                blockers: join_blockers,
            });
        };
        if !join_blockers.is_empty() {
            return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
                subject: Some(subject.clone()),
                blockers: join_blockers,
            });
        }
        let collection_subject = mapped_collection_type.gate_payload().clone();
        let runtime_expr = match lower_gate_pipe_body_runtime_expr(
            module,
            join_expr,
            env,
            &collection_subject,
            typing,
        ) {
            Ok(runtime_expr) => runtime_expr,
            Err(blocker) => {
                return FanoutSegmentOutcome::Blocked(BlockedFanoutSegment {
                    subject: Some(subject.clone()),
                    blockers: vec![blocker_for_join_runtime_blocker(blocker)],
                });
            }
        };

        result_type = typing.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Join, carrier),
            join_value_type,
        );
        join = Some(FanoutJoinPlan {
            stage_index,
            stage_span: stage.span,
            expr: join_expr,
            input_subject: mapped_collection_type.clone(),
            collection_subject,
            runtime_expr,
            result_type: result_type.clone(),
        });
    }

    FanoutSegmentOutcome::Planned(FanoutSegmentPlan {
        carrier,
        input_subject: subject.clone(),
        element_subject,
        mapped_element_type,
        mapped_collection_type,
        runtime_map,
        filters,
        join,
        result_type,
    })
}

fn lower_fanout_filter_predicate(
    module: &Module,
    predicate: ExprId,
    env: &GateExprEnv,
    subject: &GateType,
    typing: &mut GateTypeContext<'_>,
) -> Result<GateRuntimeExpr, FanoutFilterBlocker> {
    let predicate_span = module.exprs()[predicate].span;
    let predicate_info = typing.infer_pipe_body(predicate, env, subject);
    if let Some(issue) = predicate_info.issues.into_iter().next() {
        return Err(fanout_filter_issue_blocker(issue, predicate_span));
    }
    if predicate_info.contains_signal || predicate_info.ty.as_ref().is_some_and(GateType::is_signal)
    {
        return Err(FanoutFilterBlocker::ImpureExpr);
    }
    let Some(predicate_ty) = predicate_info.ty else {
        return Err(FanoutFilterBlocker::UnknownExprType {
            span: predicate_span,
        });
    };
    if !predicate_ty.is_bool() {
        return Err(FanoutFilterBlocker::PredicateNotBool {
            found: predicate_ty,
        });
    }
    lower_gate_pipe_body_runtime_expr(module, predicate, env, subject, typing)
        .map_err(fanout_filter_blocker)
}

fn blocker_for_map_issue(issue: GateIssue) -> FanoutElaborationBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            FanoutElaborationBlocker::MapInvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            FanoutElaborationBlocker::MapUnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { .. }
        | GateIssue::AmbientSubjectOutsidePipe { .. }
        | GateIssue::AmbiguousDomainOperator { .. }
        | GateIssue::InvalidPipeStageInput { .. }
        | GateIssue::UnsupportedApplicativeClusterMember { .. }
        | GateIssue::ApplicativeClusterMismatch { .. }
        | GateIssue::InvalidClusterFinalizer { .. }
        | GateIssue::CaseBranchTypeMismatch { .. } => FanoutElaborationBlocker::UnknownMapBodyType,
    }
}

fn blocker_for_join_issue(issue: GateIssue) -> FanoutElaborationBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            FanoutElaborationBlocker::JoinInvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            FanoutElaborationBlocker::JoinUnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { .. }
        | GateIssue::AmbientSubjectOutsidePipe { .. }
        | GateIssue::AmbiguousDomainOperator { .. }
        | GateIssue::InvalidPipeStageInput { .. }
        | GateIssue::UnsupportedApplicativeClusterMember { .. }
        | GateIssue::ApplicativeClusterMismatch { .. }
        | GateIssue::InvalidClusterFinalizer { .. }
        | GateIssue::CaseBranchTypeMismatch { .. } => FanoutElaborationBlocker::UnknownJoinBodyType,
    }
}

fn blocker_for_map_runtime_blocker(blocker: GateElaborationBlocker) -> FanoutElaborationBlocker {
    match blocker {
        GateElaborationBlocker::InvalidProjection { path, subject } => {
            FanoutElaborationBlocker::MapInvalidProjection { path, subject }
        }
        GateElaborationBlocker::UnknownField { path, subject } => {
            FanoutElaborationBlocker::MapUnknownField { path, subject }
        }
        GateElaborationBlocker::UnknownSubjectType
        | GateElaborationBlocker::UnknownPredicateType
        | GateElaborationBlocker::ImpurePredicate
        | GateElaborationBlocker::PredicateNotBool { .. }
        | GateElaborationBlocker::UnknownRuntimeExprType { .. }
        | GateElaborationBlocker::UnsupportedRuntimeExpr { .. } => {
            FanoutElaborationBlocker::UnknownMapBodyType
        }
    }
}

fn blocker_for_join_runtime_blocker(blocker: GateElaborationBlocker) -> FanoutElaborationBlocker {
    match blocker {
        GateElaborationBlocker::InvalidProjection { path, subject } => {
            FanoutElaborationBlocker::JoinInvalidProjection { path, subject }
        }
        GateElaborationBlocker::UnknownField { path, subject } => {
            FanoutElaborationBlocker::JoinUnknownField { path, subject }
        }
        GateElaborationBlocker::UnknownSubjectType
        | GateElaborationBlocker::UnknownPredicateType
        | GateElaborationBlocker::ImpurePredicate
        | GateElaborationBlocker::PredicateNotBool { .. }
        | GateElaborationBlocker::UnknownRuntimeExprType { .. }
        | GateElaborationBlocker::UnsupportedRuntimeExpr { .. } => {
            FanoutElaborationBlocker::UnknownJoinBodyType
        }
    }
}

fn fanout_filter_issue_blocker(issue: GateIssue, span: SourceSpan) -> FanoutFilterBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            FanoutFilterBlocker::InvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            FanoutFilterBlocker::UnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { .. }
        | GateIssue::AmbientSubjectOutsidePipe { .. }
        | GateIssue::AmbiguousDomainOperator { .. }
        | GateIssue::InvalidPipeStageInput { .. }
        | GateIssue::UnsupportedApplicativeClusterMember { .. }
        | GateIssue::ApplicativeClusterMismatch { .. }
        | GateIssue::InvalidClusterFinalizer { .. }
        | GateIssue::CaseBranchTypeMismatch { .. } => FanoutFilterBlocker::UnknownExprType { span },
    }
}

fn fanout_filter_blocker(blocker: GateElaborationBlocker) -> FanoutFilterBlocker {
    match blocker {
        GateElaborationBlocker::InvalidProjection { path, subject } => {
            FanoutFilterBlocker::InvalidProjection { path, subject }
        }
        GateElaborationBlocker::UnknownField { path, subject } => {
            FanoutFilterBlocker::UnknownField { path, subject }
        }
        GateElaborationBlocker::ImpurePredicate => FanoutFilterBlocker::ImpureExpr,
        GateElaborationBlocker::PredicateNotBool { found } => {
            FanoutFilterBlocker::PredicateNotBool { found }
        }
        GateElaborationBlocker::UnknownRuntimeExprType { span } => {
            FanoutFilterBlocker::UnknownExprType { span }
        }
        GateElaborationBlocker::UnsupportedRuntimeExpr { span, kind } => {
            FanoutFilterBlocker::UnsupportedExpr { span, kind }
        }
        GateElaborationBlocker::UnknownSubjectType
        | GateElaborationBlocker::UnknownPredicateType => {
            unreachable!("fan-out filter lowering always has an explicit subject")
        }
    }
}

fn fanout_env_for_function(
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

    use aivi_base::{DiagnosticCode, SourceDatabase};
    use aivi_syntax::parse_module;
    use aivi_typing::FanoutCarrier;

    use super::{FanoutElaborationBlocker, FanoutSegmentOutcome, elaborate_fanouts};
    use crate::{
        BuiltinType, GateRuntimeExprKind, GateRuntimeProjectionBase, GateType, Item,
        ValidationMode, lower_module,
    };

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
    fn elaborates_valid_fanout_fixture_into_ordinary_and_signal_plans() {
        let lowered = lower_fixture("milestone-2/valid/pipe-fanout-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "fanout fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_fanouts(lowered.module());
        assert_eq!(
            report.segments().len(),
            4,
            "expected ordinary and signal map/join segments"
        );

        let ordinary_map = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "emails")
            .expect("expected map-only fanout segment for emails");
        match &ordinary_map.outcome {
            FanoutSegmentOutcome::Planned(plan) => {
                assert_eq!(plan.carrier, FanoutCarrier::Ordinary);
                assert!(
                    plan.filters.is_empty(),
                    "plain `*|>` should not invent fan-out filters"
                );
                assert!(matches!(&plan.input_subject, GateType::List(_)));
                assert_eq!(
                    plan.element_subject,
                    GateType::Record(vec![
                        crate::GateRecordField {
                            name: "active".into(),
                            ty: GateType::Primitive(BuiltinType::Bool),
                        },
                        crate::GateRecordField {
                            name: "email".into(),
                            ty: GateType::Primitive(BuiltinType::Text),
                        },
                    ])
                );
                assert_eq!(
                    plan.mapped_element_type,
                    GateType::Primitive(BuiltinType::Text)
                );
                assert_eq!(
                    plan.mapped_collection_type,
                    GateType::List(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
                match &plan.runtime_map.kind {
                    GateRuntimeExprKind::Projection { base, path } => {
                        assert_eq!(base, &GateRuntimeProjectionBase::AmbientSubject);
                        assert_eq!(
                            path.segments().iter().next().map(|segment| segment.text()),
                            Some("email")
                        );
                    }
                    other => panic!("expected retained map projection, found {other:?}"),
                }
                assert_eq!(plan.result_type, plan.mapped_collection_type);
                assert!(plan.join.is_none(), "plain `*|>` should not invent a join");
            }
            other => panic!("expected planned ordinary fanout segment, found {other:?}"),
        }

        let ordinary_join = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "joinedEmails")
            .expect("expected joined fanout segment for joinedEmails");
        match &ordinary_join.outcome {
            FanoutSegmentOutcome::Planned(plan) => {
                assert!(
                    plan.filters.is_empty(),
                    "simple joined fan-out should not invent filters"
                );
                let join = plan
                    .join
                    .as_ref()
                    .expect("joined fanout should record `<|*`");
                assert_eq!(
                    join.collection_subject,
                    GateType::List(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
                assert_eq!(plan.runtime_map.ty, GateType::Primitive(BuiltinType::Text));
                assert_eq!(join.runtime_expr.ty, GateType::Primitive(BuiltinType::Text));
                assert_eq!(join.result_type, GateType::Primitive(BuiltinType::Text));
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned ordinary join segment, found {other:?}"),
        }

        let signal_map = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "liveEmails")
            .expect("expected signal map fanout segment");
        match &signal_map.outcome {
            FanoutSegmentOutcome::Planned(plan) => {
                assert_eq!(plan.carrier, FanoutCarrier::Signal);
                assert!(plan.filters.is_empty());
                assert_eq!(
                    plan.mapped_collection_type,
                    GateType::Signal(Box::new(GateType::List(Box::new(GateType::Primitive(
                        BuiltinType::Text,
                    )))))
                );
                assert_eq!(plan.runtime_map.ty, GateType::Primitive(BuiltinType::Text));
                assert_eq!(plan.result_type, plan.mapped_collection_type);
            }
            other => panic!("expected planned signal fanout segment, found {other:?}"),
        }

        let signal_join = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "liveJoinedEmails")
            .expect("expected signal joined fanout segment");
        match &signal_join.outcome {
            FanoutSegmentOutcome::Planned(plan) => {
                assert!(plan.filters.is_empty());
                let join = plan.join.as_ref().expect("signal join should record `<|*`");
                assert_eq!(
                    join.collection_subject,
                    GateType::List(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
                assert_eq!(
                    join.input_subject,
                    GateType::Signal(Box::new(GateType::List(Box::new(GateType::Primitive(
                        BuiltinType::Text,
                    )))))
                );
                assert_eq!(
                    join.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
                assert_eq!(join.runtime_expr.ty, GateType::Primitive(BuiltinType::Text));
                assert_eq!(plan.result_type, join.result_type);
            }
            other => panic!("expected planned signal join segment, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_joined_fanout_filters_before_join() {
        let lowered = lower_text(
            "fanout-filter-before-join.aivi",
            r#"
type User = {
    email: Text
}

fun keepText:Bool email:Text =>
    True

fun joinEmails:Text items:List Text =>
    "joined"

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
            "fan-out filter example should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_fanouts(lowered.module());
        let joined = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "joinedEmails")
            .expect("expected joined fan-out segment with a filter");

        match &joined.outcome {
            FanoutSegmentOutcome::Planned(plan) => {
                assert_eq!(plan.filters.len(), 1);
                assert_eq!(
                    plan.filters[0].input_subject,
                    GateType::Primitive(BuiltinType::Text)
                );
                let join = plan
                    .join
                    .as_ref()
                    .expect("joined fan-out should record `<|*`");
                assert_eq!(
                    join.collection_subject,
                    GateType::List(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned joined fan-out segment, found {other:?}"),
        }
    }

    #[test]
    fn blocks_non_list_fanout_subjects() {
        let lowered = lower_fixture("milestone-2/invalid/fanout-non-list-subject/main.aivi");
        let report = elaborate_fanouts(lowered.module());
        let blocked = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "emails")
            .expect("expected blocked fanout segment");

        match &blocked.outcome {
            FanoutSegmentOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    FanoutElaborationBlocker::SubjectNotCollection {
                        found: GateType::Record(_)
                    }
                )));
            }
            other => panic!("expected blocked fanout segment, found {other:?}"),
        }
    }

    #[test]
    fn blocks_invalid_fanin_body_projections() {
        let lowered = lower_fixture("milestone-2/invalid/fanin-invalid-projection/main.aivi");
        let report = elaborate_fanouts(lowered.module());
        let blocked = report
            .segments()
            .iter()
            .find(|segment| item_name(lowered.module(), segment.owner) == "joinedEmails")
            .expect("expected blocked fanout join segment");

        match &blocked.outcome {
            FanoutSegmentOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    FanoutElaborationBlocker::JoinInvalidProjection { subject, .. }
                    if subject == "List Text"
                )));
            }
            other => panic!("expected blocked fanout join segment, found {other:?}"),
        }
    }

    #[test]
    fn resolved_validation_reports_fanout_diagnostics() {
        let lowered = lower_fixture("milestone-2/invalid/fanout-non-list-subject/main.aivi");
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::new("hir", "fanout-subject-not-list"))
        }));

        let lowered = lower_fixture("milestone-2/invalid/fanin-invalid-projection/main.aivi");
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code == Some(DiagnosticCode::new("hir", "invalid-fanin-projection"))
        }));
    }
}
