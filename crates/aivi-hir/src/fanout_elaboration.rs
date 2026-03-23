use aivi_base::SourceSpan;
use aivi_typing::{FanoutCarrier, FanoutPlanner, FanoutStageKind};

use crate::{
    ExprId, Item, ItemId, Module, PipeExpr, PipeStageKind,
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, truthy_falsy_pair_stages, walk_expr_tree,
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
    pub join: Option<FanoutJoinPlan>,
    pub result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanoutJoinPlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub expr: ExprId,
    pub input_subject: GateType,
    pub collection_subject: GateType,
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
    SubjectNotCollection { found: GateType },
    MapInvalidProjection { path: String, subject: String },
    MapUnknownField { path: String, subject: String },
    UnknownMapBodyType,
    JoinInvalidProjection { path: String, subject: String },
    JoinUnknownField { path: String, subject: String },
    UnknownJoinBodyType,
}

pub fn elaborate_fanouts(module: &Module) -> FanoutElaborationReport {
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
            collect_fanout_pipe(owner, pipe_expr, pipe, env, typing, segments);
        }
    });
}

fn collect_fanout_pipe(
    owner: ItemId,
    pipe_expr: ExprId,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    segments: &mut Vec<FanoutSegmentElaboration>,
) {
    let stages = pipe.stages.iter().collect::<Vec<_>>();
    let mut current = typing.infer_expr(pipe.head, env, None).ty;
    let mut stage_index = 0usize;
    while stage_index < stages.len() {
        let stage = stages[stage_index];
        match &stage.kind {
            PipeStageKind::Transform { expr } => {
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_transform_stage(*expr, env, subject));
                stage_index += 1;
            }
            PipeStageKind::Tap { expr } => {
                if let Some(subject) = current.clone() {
                    let _ = typing.infer_pipe_body(*expr, env, &subject);
                    current = Some(subject);
                }
                stage_index += 1;
            }
            PipeStageKind::Gate { expr } => {
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_gate_stage(*expr, env, subject));
                stage_index += 1;
            }
            PipeStageKind::Map { expr } => {
                let join_stage = stages
                    .get(stage_index + 1)
                    .and_then(|candidate| match &candidate.kind {
                        PipeStageKind::FanIn { expr } => Some((stage_index + 1, *candidate, *expr)),
                        _ => None,
                    });
                let outcome =
                    elaborate_fanout_segment(*expr, current.as_ref(), join_stage, env, typing);
                segments.push(FanoutSegmentElaboration {
                    owner,
                    pipe_expr,
                    map_stage_index: stage_index,
                    map_stage_span: stage.span,
                    map_expr: *expr,
                    outcome: outcome.clone(),
                });
                current = match outcome {
                    FanoutSegmentOutcome::Planned(plan) => Some(plan.result_type),
                    FanoutSegmentOutcome::Blocked(_) => None,
                };
                stage_index += if join_stage.is_some() { 2 } else { 1 };
            }
            PipeStageKind::FanIn { expr } => {
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_fanin_stage(*expr, env, subject));
                stage_index += 1;
            }
            PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                let Some(pair) = truthy_falsy_pair_stages(&stages, stage_index) else {
                    current = None;
                    stage_index += 1;
                    continue;
                };
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_truthy_falsy_pair(&pair, env, subject));
                stage_index = pair.next_index;
            }
            PipeStageKind::Case { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. } => {
                current = None;
                stage_index += 1;
            }
        }
    }
}

fn elaborate_fanout_segment(
    map_expr: ExprId,
    subject: Option<&GateType>,
    join_stage: Option<(usize, &crate::PipeStage, ExprId)>,
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

    let map_info = typing.infer_pipe_body(map_expr, env, &element_subject);
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

    let mapped_collection_type = typing.apply_fanout_plan(
        FanoutPlanner::plan(FanoutStageKind::Map, carrier),
        mapped_element_type.clone(),
    );
    let mut result_type = mapped_collection_type.clone();
    let mut join = None;

    if let Some((stage_index, stage, join_expr)) = join_stage {
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

        result_type = typing.apply_fanout_plan(
            FanoutPlanner::plan(FanoutStageKind::Join, carrier),
            join_value_type,
        );
        join = Some(FanoutJoinPlan {
            stage_index,
            stage_span: stage.span,
            expr: join_expr,
            input_subject: mapped_collection_type.clone(),
            collection_subject: mapped_collection_type.gate_payload().clone(),
            result_type: result_type.clone(),
        });
    }

    FanoutSegmentOutcome::Planned(FanoutSegmentPlan {
        carrier,
        input_subject: subject.clone(),
        element_subject,
        mapped_element_type,
        mapped_collection_type,
        join,
        result_type,
    })
}

fn blocker_for_map_issue(issue: GateIssue) -> FanoutElaborationBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            FanoutElaborationBlocker::MapInvalidProjection { path, subject }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            FanoutElaborationBlocker::MapUnknownField { path, subject }
        }
        GateIssue::AmbiguousDomainMember { .. } => FanoutElaborationBlocker::UnknownMapBodyType,
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
        GateIssue::AmbiguousDomainMember { .. } => FanoutElaborationBlocker::UnknownJoinBodyType,
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
    use crate::{BuiltinType, GateType, Item, ValidationMode, lower_module};

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
                let join = plan
                    .join
                    .as_ref()
                    .expect("joined fanout should record `<|*`");
                assert_eq!(
                    join.collection_subject,
                    GateType::List(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
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
                assert_eq!(
                    plan.mapped_collection_type,
                    GateType::Signal(Box::new(GateType::List(Box::new(GateType::Primitive(
                        BuiltinType::Text,
                    )))))
                );
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
                assert_eq!(plan.result_type, join.result_type);
            }
            other => panic!("expected planned signal join segment, found {other:?}"),
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
