use aivi_base::SourceSpan;

use crate::{
    BuiltinTerm, ExprId, ExprKind, Item, ItemId, Module, PipeExpr, PipeStageKind,
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, PipeSubjectStepOutcome,
        PipeSubjectWalker, TruthyFalsyPairStages, gate_env_for_function, truthy_falsy_pair_stages,
        walk_expr_tree,
    },
};

/// Focused truthy/falsy branch plans derived from resolved HIR.
///
/// This is intentionally narrower than a future full typed-core case IR. It proves only the RFC
/// v1 canonical builtin carriers that the current resolved-HIR layer can identify honestly:
/// `Bool`, `Option A`, `Result E A`, and `Validation E A`, plus exactly one pointwise outer
/// `Signal (...)` around those same carriers. Each successful elaboration records the chosen
/// builtin constructor pair plus the branch-local payload subject (when one exists) and the
/// unified stage result type. If the current layer cannot justify the carrier or either branch,
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
            Item::Value(item) => collect_truthy_falsy_stages(
                module,
                owner,
                item.body,
                &GateExprEnv::default(),
                &mut typing,
                &mut stages,
            ),
            Item::Function(item) => {
                let env = gate_env_for_function(&item, &mut typing);
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
    truthy_falsy_stages: &mut Vec<TruthyFalsyStageElaboration>,
) {
    let all_stages = pipe.stages.iter().collect::<Vec<_>>();
    PipeSubjectWalker::new(pipe, env, typing).walk(
        typing,
        |stage_index, stage, current, typing| match &stage.kind {
            PipeStageKind::Gate { expr } => PipeSubjectStepOutcome::Continue {
                new_subject: current.and_then(|s| typing.infer_gate_stage(*expr, env, s)),
                advance_by: 1,
            },
            PipeStageKind::Map { expr } => PipeSubjectStepOutcome::Continue {
                new_subject: current.and_then(|s| typing.infer_fanout_map_stage(*expr, env, s)),
                advance_by: 1,
            },
            PipeStageKind::FanIn { expr } => PipeSubjectStepOutcome::Continue {
                new_subject: current.and_then(|s| typing.infer_fanin_stage(*expr, env, s)),
                advance_by: 1,
            },
            PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                let Some(pair) = truthy_falsy_pair_stages(&all_stages, stage_index) else {
                    return PipeSubjectStepOutcome::Continue {
                        new_subject: None,
                        advance_by: 1,
                    };
                };
                let outcome = elaborate_truthy_falsy_pair(&pair, current, env, typing);
                truthy_falsy_stages.push(TruthyFalsyStageElaboration {
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
                let advance = pair.next_index.saturating_sub(stage_index).max(1);
                PipeSubjectStepOutcome::Continue {
                    new_subject: match outcome {
                        TruthyFalsyStageOutcome::Planned(plan) => Some(plan.result_type),
                        TruthyFalsyStageOutcome::Blocked(_) => None,
                    },
                    advance_by: advance,
                }
            }
            PipeStageKind::Case { .. }
            | PipeStageKind::Apply { .. }
            | PipeStageKind::RecurStart { .. }
            | PipeStageKind::RecurStep { .. } => PipeSubjectStepOutcome::Continue {
                new_subject: None,
                advance_by: 1,
            },
            PipeStageKind::Transform { .. } | PipeStageKind::Tap { .. } => {
                unreachable!("PipeSubjectWalker handles Transform and Tap internally")
            }
        },
    );
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

    // Collect UnknownBranchType for both branches before early-returning so
    // that a user who breaks both branches sees both errors at once (PA-M2).
    if truthy_ty.is_none() {
        blockers.push(TruthyFalsyElaborationBlocker::UnknownBranchType {
            branch: TruthyFalsyBranchKind::Truthy,
        });
    }
    if falsy_ty.is_none() {
        blockers.push(TruthyFalsyElaborationBlocker::UnknownBranchType {
            branch: TruthyFalsyBranchKind::Falsy,
        });
    }
    let (Some(truthy_result_type), Some(falsy_result_type)) = (truthy_ty, falsy_ty) else {
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

    let stage_result_type =
        typing.apply_truthy_falsy_result_type(subject, truthy_result_type.clone());
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
        result_type: stage_result_type,
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
        GateIssue::AmbiguousDomainMember { .. }
        | GateIssue::InvalidPipeStageInput { .. }
        | GateIssue::UnsupportedApplicativeClusterMember { .. }
        | GateIssue::ApplicativeClusterMismatch { .. }
        | GateIssue::InvalidClusterFinalizer { .. }
        | GateIssue::CaseBranchTypeMismatch { .. } => {
            TruthyFalsyElaborationBlocker::UnknownBranchType { branch }
        }
    }
}

// truthy_falsy_env_for_function is now the shared crate::validate::gate_env_for_function (PA-I2).

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
    fn elaborates_signal_lifted_truthy_falsy_fixture_into_pointwise_branch_plans() {
        let lowered =
            lower_fixture("milestone-2/valid/pipe-truthy-falsy-signal-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "signal truthy/falsy fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let validation = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            validation.is_ok(),
            "signal truthy/falsy fixture should validate cleanly: {:?}",
            validation.diagnostics()
        );

        let report = elaborate_truthy_falsy(lowered.module());
        assert_eq!(
            report.stages().len(),
            5,
            "expected every signal-lifted truthy/falsy pair to elaborate"
        );

        let ready = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "readyText")
            .expect("expected readyText truthy/falsy pair");
        match &ready.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert_eq!(
                    plan.input_subject,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Bool)))
                );
                assert_eq!(plan.truthy.constructor, BuiltinTerm::True);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::False);
                assert!(plan.truthy.payload_subject.is_none());
                assert!(plan.falsy.payload_subject.is_none());
                assert_eq!(
                    plan.truthy.result_type,
                    GateType::Primitive(BuiltinType::Text)
                );
                assert_eq!(
                    plan.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected planned signal bool truthy/falsy pair, found {other:?}"),
        }

        let greeting = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "greeting")
            .expect("expected greeting truthy/falsy pair");
        match &greeting.outcome {
            TruthyFalsyStageOutcome::Planned(plan) => {
                assert!(matches!(
                    &plan.input_subject,
                    GateType::Signal(inner) if matches!(inner.as_ref(), GateType::Option(_))
                ));
                let truthy_payload = plan
                    .truthy
                    .payload_subject
                    .as_ref()
                    .expect("`Some` branch should expose the payload subject");
                assert!(matches!(truthy_payload, GateType::Record(_)));
                assert!(plan.falsy.payload_subject.is_none());
                assert_eq!(plan.truthy.constructor, BuiltinTerm::Some);
                assert_eq!(plan.falsy.constructor, BuiltinTerm::None);
                assert_eq!(
                    plan.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected planned signal option truthy/falsy pair, found {other:?}"),
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
                assert_eq!(
                    plan.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected planned reversed signal truthy/falsy pair, found {other:?}"),
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
                assert_eq!(
                    plan.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => panic!("expected planned signal result truthy/falsy pair, found {other:?}"),
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
                assert_eq!(
                    plan.result_type,
                    GateType::Signal(Box::new(GateType::Primitive(BuiltinType::Text)))
                );
            }
            other => {
                panic!("expected planned signal validation truthy/falsy pair, found {other:?}")
            }
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
    fn blocks_signal_truthy_falsy_misuse() {
        let lowered = lower_fixture("milestone-2/invalid/truthy-falsy-signal-misuse/main.aivi");
        let report = elaborate_truthy_falsy(lowered.module());

        let bad_projection = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "badProjection")
            .expect("expected blocked signal truthy/falsy projection pair");
        match &bad_projection.outcome {
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
            other => {
                panic!("expected blocked signal truthy/falsy projection pair, found {other:?}")
            }
        }

        let bad_subject = report
            .stages()
            .iter()
            .find(|stage| item_name(lowered.module(), stage.owner) == "badSubject")
            .expect("expected blocked signal truthy/falsy subject pair");
        match &bad_subject.outcome {
            TruthyFalsyStageOutcome::Blocked(stage) => {
                assert!(stage.blockers.iter().any(|blocker| matches!(
                    blocker,
                    TruthyFalsyElaborationBlocker::UnsupportedSubject {
                        found: GateType::Signal(inner)
                    } if matches!(inner.as_ref(), GateType::Record(_))
                )));
            }
            other => {
                panic!("expected blocked signal truthy/falsy subject pair, found {other:?}")
            }
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

        let lowered = lower_fixture("milestone-2/invalid/truthy-falsy-signal-misuse/main.aivi");
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
        assert!(report.diagnostics().iter().any(|diagnostic| {
            diagnostic.code
                == Some(DiagnosticCode::new(
                    "hir",
                    "invalid-truthy-falsy-projection",
                ))
        }));
    }
}
