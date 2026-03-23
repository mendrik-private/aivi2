use aivi_base::SourceSpan;

use crate::{
    BuiltinTerm, ExprId, ExprKind, Item, ItemId, Module, PipeExpr, PipeStageKind,
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, TruthyFalsyPairStages,
        truthy_falsy_pair_stages, walk_expr_tree,
    },
};

/// Focused truthy/falsy branch plans derived from resolved HIR.
///
/// This is intentionally narrower than a future full typed-core case IR. It proves only the RFC
/// v1 canonical builtin carriers that the current resolved-HIR layer can identify honestly:
/// `Bool`, `Option A`, `Result E A`, and `Validation E A`. Each successful elaboration records the
/// chosen builtin constructor pair plus the branch-local payload subject (when one exists) and the
/// unified branch result type. If the current layer cannot justify the carrier or either branch,
/// the report records explicit blockers instead of guessing a case lowering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TruthyFalsyElaborationReport {
    stages: Vec<TruthyFalsyStageElaboration>,
}

impl TruthyFalsyElaborationReport {
    pub fn new(stages: Vec<TruthyFalsyStageElaboration>) -> Self {
        Self { stages }
    }

    pub fn stages(&self) -> &[TruthyFalsyStageElaboration] {
        &self.stages
    }

    pub fn into_stages(self) -> Vec<TruthyFalsyStageElaboration> {
        self.stages
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruthyFalsyStageElaboration {
    pub owner: ItemId,
    pub pipe_expr: ExprId,
    pub truthy_stage_index: usize,
    pub truthy_stage_span: SourceSpan,
    pub truthy_expr: ExprId,
    pub falsy_stage_index: usize,
    pub falsy_stage_span: SourceSpan,
    pub falsy_expr: ExprId,
    pub outcome: TruthyFalsyStageOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TruthyFalsyStageOutcome {
    Planned(TruthyFalsyStagePlan),
    Blocked(BlockedTruthyFalsyStage),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TruthyFalsyBranchKind {
    Truthy,
    Falsy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruthyFalsyBranchPlan {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub expr: ExprId,
    pub constructor: BuiltinTerm,
    pub payload_subject: Option<GateType>,
    pub result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruthyFalsyStagePlan {
    pub input_subject: GateType,
    pub truthy: TruthyFalsyBranchPlan,
    pub falsy: TruthyFalsyBranchPlan,
    pub result_type: GateType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedTruthyFalsyStage {
    pub subject: Option<GateType>,
    pub blockers: Vec<TruthyFalsyElaborationBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TruthyFalsyElaborationBlocker {
    UnknownSubjectType,
    UnsupportedSubject {
        found: GateType,
    },
    InvalidProjection {
        branch: TruthyFalsyBranchKind,
        path: String,
        subject: String,
    },
    UnknownField {
        branch: TruthyFalsyBranchKind,
        path: String,
        subject: String,
    },
    UnknownBranchType {
        branch: TruthyFalsyBranchKind,
    },
    BranchTypeMismatch {
        truthy: GateType,
        falsy: GateType,
    },
}

pub fn elaborate_truthy_falsy(module: &Module) -> TruthyFalsyElaborationReport {
    let items = module
        .items()
        .iter()
        .map(|(item_id, item)| (item_id, item.clone()))
        .collect::<Vec<_>>();
    let mut stages = Vec::new();
    let mut typing = GateTypeContext::new(module);

    for (owner, item) in items {
        match item {
            Item::Value(item) => collect_truthy_falsy_stages(
                module,
                owner,
                item.body,
                &GateExprEnv::default(),
                &mut typing,
                &mut stages,
            ),
            Item::Function(item) => {
                let env = truthy_falsy_env_for_function(&item, &mut typing);
                collect_truthy_falsy_stages(
                    module,
                    owner,
                    item.body,
                    &env,
                    &mut typing,
                    &mut stages,
                );
            }
            Item::Signal(item) => {
                if let Some(body) = item.body {
                    collect_truthy_falsy_stages(
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
                    collect_truthy_falsy_stages(
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

    TruthyFalsyElaborationReport::new(stages)
}

fn collect_truthy_falsy_stages(
    module: &Module,
    owner: ItemId,
    root: ExprId,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    stages: &mut Vec<TruthyFalsyStageElaboration>,
) {
    walk_expr_tree(module, root, |pipe_expr, expr, _| {
        if let ExprKind::Pipe(pipe) = &expr.kind {
            collect_truthy_falsy_pipe(owner, pipe_expr, pipe, env, typing, stages);
        }
    });
}

fn collect_truthy_falsy_pipe(
    owner: ItemId,
    pipe_expr: ExprId,
    pipe: &PipeExpr,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
    stages: &mut Vec<TruthyFalsyStageElaboration>,
) {
    let stage_refs = pipe.stages.iter().collect::<Vec<_>>();
    let mut current = typing.infer_expr(pipe.head, env, None).ty;
    let mut stage_index = 0usize;
    while stage_index < stage_refs.len() {
        let stage = stage_refs[stage_index];
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
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_fanout_map_stage(*expr, env, subject));
                stage_index += 1;
            }
            PipeStageKind::FanIn { expr } => {
                current = current
                    .as_ref()
                    .and_then(|subject| typing.infer_fanin_stage(*expr, env, subject));
                stage_index += 1;
            }
            PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                let Some(pair) = truthy_falsy_pair_stages(&stage_refs, stage_index) else {
                    current = None;
                    stage_index += 1;
                    continue;
                };
                let outcome = elaborate_truthy_falsy_pair(&pair, current.as_ref(), env, typing);
                stages.push(TruthyFalsyStageElaboration {
                    owner,
                    pipe_expr,
                    truthy_stage_index: pair.truthy_index,
                    truthy_stage_span: pair.truthy_stage.span,
                    truthy_expr: pair.truthy_expr,
                    falsy_stage_index: pair.falsy_index,
                    falsy_stage_span: pair.falsy_stage.span,
                    falsy_expr: pair.falsy_expr,
                    outcome: outcome.clone(),
                });
                current = match outcome {
                    TruthyFalsyStageOutcome::Planned(plan) => Some(plan.result_type),
                    TruthyFalsyStageOutcome::Blocked(_) => None,
                };
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

fn elaborate_truthy_falsy_pair(
    pair: &TruthyFalsyPairStages<'_>,
    subject: Option<&GateType>,
    env: &GateExprEnv,
    typing: &mut GateTypeContext<'_>,
) -> TruthyFalsyStageOutcome {
    let Some(subject) = subject else {
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: None,
            blockers: vec![TruthyFalsyElaborationBlocker::UnknownSubjectType],
        });
    };
    let Some(subject_plan) = typing.truthy_falsy_subject_plan(subject) else {
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: Some(subject.clone()),
            blockers: vec![TruthyFalsyElaborationBlocker::UnsupportedSubject {
                found: subject.clone(),
            }],
        });
    };

    let truthy_info = typing.infer_truthy_falsy_branch(
        pair.truthy_expr,
        env,
        subject_plan.truthy_payload.as_ref(),
    );
    let falsy_info =
        typing.infer_truthy_falsy_branch(pair.falsy_expr, env, subject_plan.falsy_payload.as_ref());
    let truthy_ty = truthy_info.ty.clone();
    let falsy_ty = falsy_info.ty.clone();
    let mut blockers = truthy_info
        .issues
        .into_iter()
        .map(|issue| blocker_for_branch_issue(TruthyFalsyBranchKind::Truthy, issue))
        .collect::<Vec<_>>();
    blockers.extend(
        falsy_info
            .issues
            .into_iter()
            .map(|issue| blocker_for_branch_issue(TruthyFalsyBranchKind::Falsy, issue)),
    );

    let Some(truthy_result_type) = truthy_ty else {
        blockers.push(TruthyFalsyElaborationBlocker::UnknownBranchType {
            branch: TruthyFalsyBranchKind::Truthy,
        });
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: Some(subject.clone()),
            blockers,
        });
    };
    let Some(falsy_result_type) = falsy_ty else {
        blockers.push(TruthyFalsyElaborationBlocker::UnknownBranchType {
            branch: TruthyFalsyBranchKind::Falsy,
        });
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: Some(subject.clone()),
            blockers,
        });
    };
    if !truthy_result_type.same_shape(&falsy_result_type) {
        blockers.push(TruthyFalsyElaborationBlocker::BranchTypeMismatch {
            truthy: truthy_result_type,
            falsy: falsy_result_type,
        });
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: Some(subject.clone()),
            blockers,
        });
    }
    if !blockers.is_empty() {
        return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
            subject: Some(subject.clone()),
            blockers,
        });
    }

    TruthyFalsyStageOutcome::Planned(TruthyFalsyStagePlan {
        input_subject: subject.clone(),
        truthy: TruthyFalsyBranchPlan {
            stage_index: pair.truthy_index,
            stage_span: pair.truthy_stage.span,
            expr: pair.truthy_expr,
            constructor: subject_plan.truthy_constructor,
            payload_subject: subject_plan.truthy_payload.clone(),
            result_type: truthy_result_type.clone(),
        },
        falsy: TruthyFalsyBranchPlan {
            stage_index: pair.falsy_index,
            stage_span: pair.falsy_stage.span,
            expr: pair.falsy_expr,
            constructor: subject_plan.falsy_constructor,
            payload_subject: subject_plan.falsy_payload.clone(),
            result_type: falsy_result_type,
        },
        result_type: truthy_result_type,
    })
}

fn blocker_for_branch_issue(
    branch: TruthyFalsyBranchKind,
    issue: GateIssue,
) -> TruthyFalsyElaborationBlocker {
    match issue {
        GateIssue::InvalidProjection { path, subject, .. } => {
            TruthyFalsyElaborationBlocker::InvalidProjection {
                branch,
                path,
                subject,
            }
        }
        GateIssue::UnknownField { path, subject, .. } => {
            TruthyFalsyElaborationBlocker::UnknownField {
                branch,
                path,
                subject,
            }
        }
        GateIssue::AmbiguousDomainMember { .. } => {
            TruthyFalsyElaborationBlocker::UnknownBranchType { branch }
        }
    }
}

fn truthy_falsy_env_for_function(
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

    use super::{
        TruthyFalsyBranchKind, TruthyFalsyElaborationBlocker, TruthyFalsyStageOutcome,
        elaborate_truthy_falsy,
    };
    use crate::{BuiltinTerm, BuiltinType, GateType, Item, ValidationMode, lower_module};

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
    fn elaborates_valid_truthy_falsy_fixture_into_canonical_branch_plans() {
        let lowered = lower_fixture("milestone-2/valid/pipe-truthy-falsy-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "truthy/falsy fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_truthy_falsy(lowered.module());
        assert_eq!(
            report.stages().len(),
            5,
            "expected every valid truthy/falsy pair to elaborate"
        );

        let ready = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "readyText")
            .expect("expected readyText truthy/falsy pair");
        match &ready.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert_eq!(plan.input_subject, GateType::Primitive(BuiltinType::Bool));
                assert_eq!(plan.truthy.constructor, BuiltinTerm::True);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::False);
                assert!(plan.truthy.payload_subject.is_none());
                assert!(plan.falsy.payload_subject.is_none());
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned bool truthy/falsy pair, found {other:?}"),
        }

        let greeting = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "greeting")
            .expect("expected greeting truthy/falsy pair");
        match &greeting.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                let truthy_payload = plan
                    .truthy
                    .payload_subject
                    .as_ref()
                    .expect("`Some` branch should expose the payload subject");
                assert!(matches!(truthy_payload, GateType::Record(_)));
                assert!(plan.falsy.payload_subject.is_none());
                assert_eq!(plan.truthy.constructor, BuiltinTerm::Some);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::None);
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned option truthy/falsy pair, found {other:?}"),
        }

        let reversed = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "maybeDisplay")
            .expect("expected reversed truthy/falsy pair");
        match &reversed.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert_eq!(
                    reversed.truthy_stage_index, 1,
                    "truthy stage index should preserve reversed pair order"
                );
                assert_eq!(
                    reversed.falsy_stage_index, 0,
                    "falsy stage index should preserve reversed pair order"
                );
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned reversed truthy/falsy pair, found {other:?}"),
        }

        let rendered = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "rendered")
            .expect("expected result truthy/falsy pair");
        match &rendered.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert_eq!(plan.truthy.constructor, BuiltinTerm::Ok);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::Err);
                assert!(plan.truthy.payload_subject.is_some());
                assert!(plan.falsy.payload_subject.is_some());
                assert_eq!(plan.result_type, GateType::Primitive(BuiltinType::Text));
            }
            other => panic!("expected planned result truthy/falsy pair, found {other:?}"),
        }

        let summary = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "summary")
            .expect("expected validation truthy/falsy pair");
        match &summary.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert_eq!(plan.truthy.constructor, BuiltinTerm::Valid);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::Invalid);
            }
            other => panic!("expected planned validation truthy/falsy pair, found {other:?}"),
        }
    }

    #[test]
    fn blocks_noncanonical_truthy_falsy_subjects() {
        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-noncanonical-subject/main.aivi");
        let report = elaborate_truthy_falsy(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "message")
            .expect("expected blocked truthy/falsy pair");

        match &blocked.outcome {
            TruthyFalsyStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    TruthyFalsyElaborationBlocker::UnsupportedSubject {
                        found: GateType::Record(_)
                    }
                )));
            }
            other => panic!("expected blocked truthy/falsy pair, found {other:?}"),
        }
    }

    #[test]
    fn blocks_mismatched_truthy_falsy_branch_types() {
        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-branch-type-mismatch/main.aivi");
        let report = elaborate_truthy_falsy(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "branch")
            .expect("expected blocked truthy/falsy pair");

        match &blocked.outcome {
            TruthyFalsyStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    TruthyFalsyElaborationBlocker::BranchTypeMismatch {
                        truthy: GateType::Primitive(BuiltinType::Text),
                        falsy: GateType::Primitive(BuiltinType::Int),
                    }
                )));
            }
            other => panic!("expected blocked truthy/falsy pair, found {other:?}"),
        }
    }

    #[test]
    fn blocks_payloadless_truthy_branch_projections() {
        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-payloadless-projection/main.aivi");
        let report = elaborate_truthy_falsy(lowered.module());
        let blocked = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "branch")
            .expect("expected blocked truthy/falsy projection pair");

        match &blocked.outcome {
            TruthyFalsyStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    TruthyFalsyElaborationBlocker::InvalidProjection {
                        branch: TruthyFalsyBranchKind::Truthy,
                        subject,
                        ..
                    } if subject == "unknown subject"
                )));
            }
            other => panic!("expected blocked truthy/falsy projection pair, found {other:?}"),
        }
    }

    #[test]
    fn resolved_validation_reports_truthy_falsy_diagnostics() {
        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-noncanonical-subject/main.aivi");
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code
                == Some(DiagnosticCode::new(
                    "hir",
                    "truthy-falsy-subject-not-canonical",
                ))
        }));

        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-branch-type-mismatch/main.aivi");
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code
                == Some(DiagnosticCode::new(
                    "hir",
                    "truthy-falsy-branch-type-mismatch",
                ))
        }));

        let lowered =
            lower_fixture("milestone-2/invalid/truthy-falsy-payloadless-projection/main.aivi");
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code
                == Some(DiagnosticCode::new(
                    "hir",
                    "invalid-truthy-falsy-projection",
                ))
        }));
    }
}
