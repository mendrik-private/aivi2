use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use aivi_base::SourceSpan;
use aivi_typing::GatePlanner;

use crate::{
    BigIntLiteral, BindingId, BuiltinTerm, DecimalLiteral, ExprId, ExprKind, FloatLiteral,
    FunctionItem, FunctionParameter, GateRuntimeCaseArm, GateRuntimeExpr, GateRuntimeExprKind,
    GateRuntimePipeExpr, GateRuntimePipeStage, GateRuntimePipeStageKind, GateRuntimeProjectionBase,
    GateRuntimeRecordField, GateRuntimeReference, GateRuntimeTextLiteral, GateRuntimeTextSegment,
    GateRuntimeTruthyFalsyBranch, GateRuntimeUnsupportedKind, GateRuntimeUnsupportedPipeStageKind,
    InstanceItem, InstanceMember, Item, ItemId, Module, PipeExpr, PipeStageKind, ProjectionBase,
    ResolutionState, SignalItem, TermReference, TermResolution, TypeItemBody, TypeResolution,
    ValueItem,
    gate_elaboration::{GateElaborationBlocker, GateRuntimeMapEntry},
    typecheck::{expression_matches, resolve_class_member_dispatch},
    validate::{
        GateExprEnv, GateIssue, GateType, GateTypeContext, PolyTypeBindings,
        truthy_falsy_pair_stages,
    },
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeneralExprElaborationReport {
    items: Vec<GeneralExprItemElaboration>,
    instance_members: Vec<GeneralExprInstanceMemberElaboration>,
}

impl GeneralExprElaborationReport {
    pub fn new(
        items: Vec<GeneralExprItemElaboration>,
        instance_members: Vec<GeneralExprInstanceMemberElaboration>,
    ) -> Self {
        Self {
            items,
            instance_members,
        }
    }

    pub fn items(&self) -> &[GeneralExprItemElaboration] {
        &self.items
    }

    pub fn instance_members(&self) -> &[GeneralExprInstanceMemberElaboration] {
        &self.instance_members
    }

    pub fn into_parts(
        self,
    ) -> (
        Vec<GeneralExprItemElaboration>,
        Vec<GeneralExprInstanceMemberElaboration>,
    ) {
        (self.items, self.instance_members)
    }

    pub fn into_items(self) -> Vec<GeneralExprItemElaboration> {
        self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.instance_members.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralExprItemElaboration {
    pub owner: ItemId,
    pub body_expr: ExprId,
    pub parameters: Vec<GeneralExprParameter>,
    pub outcome: GeneralExprOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralExprInstanceMemberElaboration {
    pub instance_owner: ItemId,
    pub member_index: usize,
    pub body_expr: ExprId,
    pub parameters: Vec<GeneralExprParameter>,
    pub outcome: GeneralExprOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneralExprParameter {
    pub binding: BindingId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub ty: GateType,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkupRuntimeExprSites {
    sites: BTreeMap<ExprId, MarkupRuntimeExprSite>,
}

impl MarkupRuntimeExprSites {
    pub fn new(sites: BTreeMap<ExprId, MarkupRuntimeExprSite>) -> Self {
        Self { sites }
    }

    pub fn sites(&self) -> &BTreeMap<ExprId, MarkupRuntimeExprSite> {
        &self.sites
    }

    pub fn get(&self, expr: ExprId) -> Option<&MarkupRuntimeExprSite> {
        self.sites.get(&expr)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupRuntimeExprSite {
    pub expr: ExprId,
    pub span: SourceSpan,
    pub ty: GateType,
    pub parameters: Vec<GeneralExprParameter>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupRuntimeExprSiteError {
    RootNotMarkup {
        expr: ExprId,
        span: SourceSpan,
    },
    MissingMarkupNode {
        expr: ExprId,
        node: crate::MarkupNodeId,
    },
    MissingControlNode {
        expr: ExprId,
        node: crate::ControlNodeId,
    },
    UnknownExprType {
        expr: ExprId,
        span: SourceSpan,
    },
}

impl fmt::Display for MarkupRuntimeExprSiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootNotMarkup { expr, .. } => {
                write!(f, "expression {expr} is not a markup root")
            }
            Self::MissingMarkupNode { expr, node } => {
                write!(
                    f,
                    "markup root expression {expr} references missing markup node {node}"
                )
            }
            Self::MissingControlNode { expr, node } => {
                write!(
                    f,
                    "markup root expression {expr} references missing control node {node}"
                )
            }
            Self::UnknownExprType { expr, .. } => {
                write!(f, "markup runtime expression {expr} has no resolved type")
            }
        }
    }
}

impl std::error::Error for MarkupRuntimeExprSiteError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneralExprOutcome {
    Lowered(GateRuntimeExpr),
    Blocked(BlockedGeneralExpr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedGeneralExpr {
    pub blockers: Vec<GeneralExprBlocker>,
}

impl BlockedGeneralExpr {
    pub fn primary_span(&self) -> Option<SourceSpan> {
        self.blockers
            .iter()
            .map(GeneralExprBlocker::span)
            .find(|span| *span != SourceSpan::default())
            .or_else(|| self.blockers.first().map(GeneralExprBlocker::span))
    }

    pub fn requires_typed_core_error(&self) -> bool {
        self.blockers
            .iter()
            .any(GeneralExprBlocker::requires_typed_core_error)
    }
}

impl fmt::Display for BlockedGeneralExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some((first, rest)) = self.blockers.split_first() else {
            return f.write_str("blocked with no recorded general-expression diagnostics");
        };
        write!(f, "{first}")?;
        for blocker in rest {
            write!(f, "; {blocker}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneralExprBlocker {
    UnknownExprType {
        span: SourceSpan,
    },
    UnsupportedRuntimeExpr {
        span: SourceSpan,
        kind: GateRuntimeUnsupportedKind,
    },
    UnsupportedImportReference {
        span: SourceSpan,
    },
    InvalidProjection {
        span: SourceSpan,
        path: String,
        subject: String,
    },
    UnknownField {
        span: SourceSpan,
        path: String,
        subject: String,
    },
    AmbiguousDomainMember {
        span: SourceSpan,
        name: String,
        candidates: Vec<String>,
    },
    CaseBranchTypeMismatch {
        span: SourceSpan,
        expected: String,
        actual: String,
    },
    MissingParameterType {
        span: SourceSpan,
        name: Box<str>,
    },
    UnsupportedSignalCase {
        span: SourceSpan,
        subject: GateType,
    },
}

impl GeneralExprBlocker {
    pub fn span(&self) -> SourceSpan {
        match self {
            Self::UnknownExprType { span }
            | Self::UnsupportedRuntimeExpr { span, .. }
            | Self::UnsupportedImportReference { span }
            | Self::InvalidProjection { span, .. }
            | Self::UnknownField { span, .. }
            | Self::AmbiguousDomainMember { span, .. }
            | Self::CaseBranchTypeMismatch { span, .. }
            | Self::MissingParameterType { span, .. }
            | Self::UnsupportedSignalCase { span, .. } => *span,
        }
    }

    pub fn requires_typed_core_error(&self) -> bool {
        !matches!(
            self,
            Self::UnsupportedRuntimeExpr {
                kind: GateRuntimeUnsupportedKind::PipeStage(
                    GateRuntimeUnsupportedPipeStageKind::Map
                        | GateRuntimeUnsupportedPipeStageKind::FanIn
                        | GateRuntimeUnsupportedPipeStageKind::RecurStart
                        | GateRuntimeUnsupportedPipeStageKind::RecurStep
                ),
                ..
            }
        )
    }
}

impl fmt::Display for GeneralExprBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownExprType { .. } => {
                f.write_str("expression type could not be determined for typed-core general-expression lowering")
            }
            Self::UnsupportedRuntimeExpr { kind, .. } => {
                write!(f, "{kind} is not supported in typed-core general expressions")
            }
            Self::UnsupportedImportReference { .. } => {
                f.write_str("imported names are not supported in typed-core general expressions")
            }
            Self::InvalidProjection { path, subject, .. } => {
                write!(f, "projection `{path}` is not valid for `{subject}`")
            }
            Self::UnknownField { path, subject, .. } => {
                write!(f, "field `{path}` does not exist on `{subject}`")
            }
            Self::AmbiguousDomainMember {
                name, candidates, ..
            } => {
                if candidates.is_empty() {
                    write!(f, "domain member `{name}` is ambiguous in this context")
                } else {
                    write!(
                        f,
                        "domain member `{name}` is ambiguous in this context; candidates: {}",
                        candidates.join(", ")
                    )
                }
            }
            Self::CaseBranchTypeMismatch {
                expected, actual, ..
            } => write!(
                f,
                "case split branches must agree on one result type, found `{expected}` and `{actual}`"
            ),
            Self::MissingParameterType { name, .. } => write!(
                f,
                "function parameter `{name}` requires an explicit type annotation for typed-core general-expression lowering"
            ),
            Self::UnsupportedSignalCase { subject, .. } => write!(
                f,
                "case pipe stages over `{subject}` are not supported in typed-core general expressions"
            ),
        }
    }
}

pub fn elaborate_general_expressions(module: &Module) -> GeneralExprElaborationReport {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    GeneralExprElaborator::new(&module).build()
}

pub fn collect_markup_runtime_expr_sites(
    module: &Module,
    root: ExprId,
) -> Result<MarkupRuntimeExprSites, MarkupRuntimeExprSiteError> {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    GeneralExprElaborator::new(&module).collect_markup_runtime_expr_sites(root)
}

pub fn elaborate_runtime_expr_with_env(
    module: &Module,
    expr_id: ExprId,
    parameters: &[GeneralExprParameter],
    expected: Option<&GateType>,
) -> Result<GateRuntimeExpr, BlockedGeneralExpr> {
    let module = crate::typecheck::elaborate_default_record_fields(module);
    let env = gate_env_from_parameters(parameters);
    GeneralExprElaborator::new(&module)
        .lower_expr(expr_id, &env, None, expected)
        .map_err(|blockers| BlockedGeneralExpr { blockers })
}

pub(crate) fn elaborate_runtime_expr(
    module: &Module,
    expr_id: ExprId,
    expected: Option<&GateType>,
) -> Result<GateRuntimeExpr, BlockedGeneralExpr> {
    GeneralExprElaborator::new(module)
        .lower_expr(expr_id, &GateExprEnv::default(), None, expected)
        .map_err(|blockers| BlockedGeneralExpr { blockers })
}

fn signal_pipe_boundary_index(pipe: &GateRuntimePipeExpr) -> Option<usize> {
    pipe.stages.iter().position(|stage| {
        matches!(stage.kind, GateRuntimePipeStageKind::Gate { .. })
            && stage.input_subject.is_signal()
    })
}

/// Extract the pure prefix for a signal pipe whose remaining stages are scheduler-owned.
///
/// When the runtime cannot execute the whole pipe inline, the linked-runtime body kernel still
/// needs to produce the subject value that the first scheduler-owned stage consumes. That prefix
/// may be the original head expression itself (when the boundary is the first stage).
fn extract_signal_pipe_prefix_body(
    owner: ItemId,
    body_expr: ExprId,
    expr: GateRuntimeExpr,
) -> Option<GeneralExprItemElaboration> {
    let GateRuntimeExprKind::Pipe(pipe) = expr.kind else {
        return None;
    };
    let boundary = signal_pipe_boundary_index(&pipe)?;
    let lowered_body = if boundary == 0 {
        *pipe.head
    } else {
        let prefix_stages: Vec<GateRuntimePipeStage> = pipe.stages[..boundary].to_vec();
        let body_ty = prefix_stages.last()?.result_subject.clone();
        GateRuntimeExpr {
            span: expr.span,
            ty: body_ty,
            kind: GateRuntimeExprKind::Pipe(GateRuntimePipeExpr {
                head: pipe.head,
                stages: prefix_stages,
            }),
        }
    };
    Some(GeneralExprItemElaboration {
        owner,
        body_expr,
        parameters: Vec::new(),
        outcome: GeneralExprOutcome::Lowered(lowered_body),
    })
}

fn signal_pipe_body_runtime_supported(expr: &GateRuntimeExpr) -> bool {
    let GateRuntimeExprKind::Pipe(pipe) = &expr.kind else {
        return true;
    };
    pipe.stages.iter().all(|stage| match &stage.kind {
        GateRuntimePipeStageKind::Transform { .. }
        | GateRuntimePipeStageKind::Tap { .. }
        | GateRuntimePipeStageKind::Case { .. }
        | GateRuntimePipeStageKind::TruthyFalsy { .. } => true,
        GateRuntimePipeStageKind::Gate { .. } => !stage.input_subject.is_signal(),
    })
}

struct GeneralExprElaborator<'a> {
    module: &'a Module,
    typing: GateTypeContext<'a>,
}

impl<'a> GeneralExprElaborator<'a> {
    fn new(module: &'a Module) -> Self {
        Self {
            module,
            typing: GateTypeContext::new(module),
        }
    }

    fn build(mut self) -> GeneralExprElaborationReport {
        let mut items = Vec::new();
        let mut instance_members = Vec::new();
        for (item_id, item) in self.module.items().iter() {
            if self.module.ambient_items().contains(&item_id) {
                continue;
            }
            match item {
                Item::Value(value) => items.push(self.elaborate_value(item_id, value)),
                Item::Function(function) => items.push(self.elaborate_function(item_id, function)),
                Item::Signal(signal) => {
                    if let Some(item) = self.elaborate_signal(item_id, signal) {
                        items.push(item);
                    }
                }
                Item::Instance(instance) => {
                    instance_members.extend(self.elaborate_instance_members(item_id, instance));
                }
                Item::Type(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }
        GeneralExprElaborationReport::new(items, instance_members)
    }

    fn collect_markup_runtime_expr_sites(
        mut self,
        root: ExprId,
    ) -> Result<MarkupRuntimeExprSites, MarkupRuntimeExprSiteError> {
        let expr = &self.module.exprs()[root];
        let ExprKind::Markup(root_node) = expr.kind else {
            return Err(MarkupRuntimeExprSiteError::RootNotMarkup {
                expr: root,
                span: expr.span,
            });
        };

        enum Work {
            Markup {
                expr: ExprId,
                node: crate::MarkupNodeId,
                env: GateExprEnv,
            },
            Control {
                expr: ExprId,
                node: crate::ControlNodeId,
                env: GateExprEnv,
            },
        }

        let mut sites = BTreeMap::new();
        let mut work = vec![Work::Markup {
            expr: root,
            node: root_node,
            env: GateExprEnv::default(),
        }];
        while let Some(frame) = work.pop() {
            match frame {
                Work::Markup { expr, node, env } => {
                    let Some(node) = self.module.markup_nodes().get(node).cloned() else {
                        return Err(MarkupRuntimeExprSiteError::MissingMarkupNode { expr, node });
                    };
                    match node.kind {
                        crate::MarkupNodeKind::Element(element) => {
                            for child in element.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                            for attribute in element.attributes.into_iter().rev() {
                                match attribute.value {
                                    crate::MarkupAttributeValue::Expr(attribute_expr) => {
                                        self.record_markup_runtime_expr_site(
                                            attribute_expr,
                                            &env,
                                            &mut sites,
                                        )?;
                                    }
                                    crate::MarkupAttributeValue::Text(text) => {
                                        for segment in text.segments.into_iter().rev() {
                                            if let crate::TextSegment::Interpolation(
                                                interpolation,
                                            ) = segment
                                            {
                                                self.record_markup_runtime_expr_site(
                                                    interpolation.expr,
                                                    &env,
                                                    &mut sites,
                                                )?;
                                            }
                                        }
                                    }
                                    crate::MarkupAttributeValue::ImplicitTrue => {}
                                }
                            }
                        }
                        crate::MarkupNodeKind::Control(control) => {
                            work.push(Work::Control {
                                expr,
                                node: control,
                                env,
                            });
                        }
                    }
                }
                Work::Control { expr, node, env } => {
                    let Some(control) = self.module.control_nodes().get(node).cloned() else {
                        return Err(MarkupRuntimeExprSiteError::MissingControlNode { expr, node });
                    };
                    match control {
                        crate::ControlNode::Show(node) => {
                            self.record_markup_runtime_expr_site(node.when, &env, &mut sites)?;
                            if let Some(keep_mounted) = node.keep_mounted {
                                self.record_markup_runtime_expr_site(
                                    keep_mounted,
                                    &env,
                                    &mut sites,
                                )?;
                            }
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        crate::ControlNode::Each(node) => {
                            self.record_markup_runtime_expr_site(
                                node.collection,
                                &env,
                                &mut sites,
                            )?;
                            let child_env = self.each_child_env(&env, &node);
                            if let Some(key) = node.key {
                                self.record_markup_runtime_expr_site(key, &child_env, &mut sites)?;
                            }
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: child_env.clone(),
                                });
                            }
                            if let Some(empty) = node.empty {
                                work.push(Work::Control {
                                    expr,
                                    node: empty,
                                    env,
                                });
                            }
                        }
                        crate::ControlNode::Empty(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        crate::ControlNode::Match(node) => {
                            self.record_markup_runtime_expr_site(node.scrutinee, &env, &mut sites)?;
                            let subject = self
                                .typing
                                .infer_expr(node.scrutinee, &env, None)
                                .ty
                                .ok_or(MarkupRuntimeExprSiteError::UnknownExprType {
                                    expr: node.scrutinee,
                                    span: self.module.exprs()[node.scrutinee].span,
                                })?;
                            for case in node.cases.iter().rev() {
                                let case_env = self
                                    .module
                                    .control_nodes()
                                    .get(*case)
                                    .and_then(|case_node| match case_node {
                                        crate::ControlNode::Case(case_node) => Some(
                                            self.case_branch_env(&env, case_node.pattern, &subject),
                                        ),
                                        _ => None,
                                    })
                                    .unwrap_or_else(|| env.clone());
                                work.push(Work::Control {
                                    expr,
                                    node: *case,
                                    env: case_env,
                                });
                            }
                        }
                        crate::ControlNode::Case(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        crate::ControlNode::Fragment(node) => {
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: env.clone(),
                                });
                            }
                        }
                        crate::ControlNode::With(node) => {
                            self.record_markup_runtime_expr_site(node.value, &env, &mut sites)?;
                            let child_env = self.with_child_env(&env, &node);
                            for child in node.children.into_iter().rev() {
                                work.push(Work::Markup {
                                    expr,
                                    node: child,
                                    env: child_env.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(MarkupRuntimeExprSites::new(sites))
    }

    fn elaborate_value(&mut self, owner: ItemId, value: &ValueItem) -> GeneralExprItemElaboration {
        let expected = value
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        let outcome =
            match self.lower_expr(value.body, &GateExprEnv::default(), None, expected.as_ref()) {
                Ok(body) => GeneralExprOutcome::Lowered(body),
                Err(blockers) => GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
            };
        GeneralExprItemElaboration {
            owner,
            body_expr: value.body,
            parameters: Vec::new(),
            outcome,
        }
    }

    fn elaborate_function(
        &mut self,
        owner: ItemId,
        function: &FunctionItem,
    ) -> GeneralExprItemElaboration {
        let (parameters, env) = match self.lower_parameters(&function.parameters) {
            Ok(lowered) => lowered,
            Err(blockers) => {
                return GeneralExprItemElaboration {
                    owner,
                    body_expr: function.body,
                    parameters: Vec::new(),
                    outcome: GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
                };
            }
        };
        let expected = function
            .annotation
            .and_then(|annotation| self.typing.lower_open_annotation(annotation));
        let outcome = match self.lower_expr(function.body, &env, None, expected.as_ref()) {
            Ok(body) => GeneralExprOutcome::Lowered(body),
            Err(blockers) => GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
        };
        GeneralExprItemElaboration {
            owner,
            body_expr: function.body,
            parameters,
            outcome,
        }
    }

    fn elaborate_signal(
        &mut self,
        owner: ItemId,
        signal: &SignalItem,
    ) -> Option<GeneralExprItemElaboration> {
        let body = signal.body?;
        let pipe = match &self.module.exprs()[body].kind {
            ExprKind::Pipe(pipe) => Some(pipe),
            _ => None,
        };
        let expected = signal
            .annotation
            .and_then(|annotation| self.typing.lower_annotation(annotation));
        let lowered = match expected.as_ref() {
            Some(annotation @ GateType::Signal(_)) => {
                match self.lower_expr(body, &GateExprEnv::default(), None, Some(annotation)) {
                    Ok(lowered_body) => Ok(lowered_body),
                    Err(blockers) => match annotation {
                        GateType::Signal(payload) => self
                            .lower_expr(body, &GateExprEnv::default(), None, Some(payload.as_ref()))
                            .map_err(|_| blockers),
                        _ => unreachable!("signal elaboration only falls back from `Signal A`"),
                    },
                }
            }
            _ => self.lower_expr(body, &GateExprEnv::default(), None, expected.as_ref()),
        };
        match lowered {
            Ok(lowered_body)
                if pipe.is_none() || signal_pipe_body_runtime_supported(&lowered_body) =>
            {
                Some(GeneralExprItemElaboration {
                    owner,
                    body_expr: body,
                    parameters: Vec::new(),
                    outcome: GeneralExprOutcome::Lowered(lowered_body),
                })
            }
            Ok(lowered_body) => extract_signal_pipe_prefix_body(owner, body, lowered_body),
            Err(blockers) if pipe.is_some() => self
                .lower_signal_pipe_prefix(owner, body, pipe.expect("checked pipe presence"))
                .or_else(|| {
                    Some(GeneralExprItemElaboration {
                        owner,
                        body_expr: body,
                        parameters: Vec::new(),
                        outcome: GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
                    })
                }),
            Err(blockers) => Some(GeneralExprItemElaboration {
                owner,
                body_expr: body,
                parameters: Vec::new(),
                outcome: GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
            }),
        }
    }

    fn lower_signal_pipe_prefix(
        &mut self,
        owner: ItemId,
        body_expr: ExprId,
        pipe: &PipeExpr,
    ) -> Option<GeneralExprItemElaboration> {
        let prefix = self
            .lower_pipe_expr(
                pipe,
                &GateExprEnv::default(),
                None,
                None,
                PipeLoweringMode::PrefixBeforeSchedulerBoundary,
            )
            .ok()?;
        let lowered_body = if prefix.stages.is_empty() {
            *prefix.head
        } else {
            GateRuntimeExpr {
                span: self.module.exprs()[body_expr].span,
                ty: prefix
                    .stages
                    .last()
                    .expect("prefix stages should exist")
                    .result_subject
                    .clone(),
                kind: GateRuntimeExprKind::Pipe(prefix),
            }
        };
        Some(GeneralExprItemElaboration {
            owner,
            body_expr,
            parameters: Vec::new(),
            outcome: GeneralExprOutcome::Lowered(lowered_body),
        })
    }

    fn elaborate_instance_members(
        &mut self,
        owner: ItemId,
        instance: &InstanceItem,
    ) -> Vec<GeneralExprInstanceMemberElaboration> {
        let Some(class_item_id) = self.instance_class_item_id(instance) else {
            return Vec::new();
        };
        let Some(argument_bindings) = self.instance_argument_bindings(class_item_id, instance)
        else {
            return Vec::new();
        };
        let Item::Class(class_item) = &self.module.items()[class_item_id] else {
            return Vec::new();
        };
        let expected_members = class_item
            .members
            .iter()
            .map(|member| (member.name.text().to_owned(), member.annotation))
            .collect::<HashMap<_, _>>();
        instance
            .members
            .iter()
            .enumerate()
            .filter_map(|(member_index, member)| {
                let annotation = expected_members.get(member.name.text()).copied()?;
                let expected = self
                    .typing
                    .instantiate_poly_hir_type(annotation, &argument_bindings)?;
                Some(self.elaborate_instance_member(owner, member_index, member, &expected))
            })
            .collect()
    }

    fn elaborate_instance_member(
        &mut self,
        owner: ItemId,
        member_index: usize,
        member: &InstanceMember,
        expected: &GateType,
    ) -> GeneralExprInstanceMemberElaboration {
        let (parameters, env, result_ty) =
            match self.lower_instance_member_parameters(member, expected) {
                Ok(lowered) => lowered,
                Err(blockers) => {
                    return GeneralExprInstanceMemberElaboration {
                        instance_owner: owner,
                        member_index,
                        body_expr: member.body,
                        parameters: Vec::new(),
                        outcome: GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
                    };
                }
            };
        let outcome = match self.lower_expr(member.body, &env, None, Some(&result_ty)) {
            Ok(body) => GeneralExprOutcome::Lowered(body),
            Err(blockers) => GeneralExprOutcome::Blocked(BlockedGeneralExpr { blockers }),
        };
        GeneralExprInstanceMemberElaboration {
            instance_owner: owner,
            member_index,
            body_expr: member.body,
            parameters,
            outcome,
        }
    }

    fn lower_parameters(
        &mut self,
        parameters: &[FunctionParameter],
    ) -> Result<(Vec<GeneralExprParameter>, GateExprEnv), Vec<GeneralExprBlocker>> {
        let mut env = GateExprEnv::default();
        let mut lowered = Vec::with_capacity(parameters.len());
        let mut blockers = Vec::new();
        for parameter in parameters {
            let binding = &self.module.bindings()[parameter.binding];
            let Some(annotation) = parameter.annotation else {
                blockers.push(GeneralExprBlocker::MissingParameterType {
                    span: parameter.span,
                    name: binding.name.text().into(),
                });
                continue;
            };
            let Some(ty) = self.typing.lower_open_annotation(annotation) else {
                blockers.push(GeneralExprBlocker::UnknownExprType {
                    span: parameter.span,
                });
                continue;
            };
            env.locals.insert(parameter.binding, ty.clone());
            lowered.push(GeneralExprParameter {
                binding: parameter.binding,
                span: binding.span,
                name: binding.name.text().into(),
                ty,
            });
        }
        if blockers.is_empty() {
            Ok((lowered, env))
        } else {
            Err(blockers)
        }
    }

    fn lower_instance_member_parameters(
        &mut self,
        member: &InstanceMember,
        expected: &GateType,
    ) -> Result<(Vec<GeneralExprParameter>, GateExprEnv, GateType), Vec<GeneralExprBlocker>> {
        let mut env = GateExprEnv::default();
        let mut lowered = Vec::with_capacity(member.parameters.len());
        let mut current = expected.clone();
        for parameter in &member.parameters {
            let GateType::Arrow {
                parameter: parameter_ty,
                result,
            } = current
            else {
                return Err(vec![GeneralExprBlocker::UnknownExprType {
                    span: member.span,
                }]);
            };
            let binding = &self.module.bindings()[parameter.binding];
            let parameter_ty = parameter_ty.as_ref().clone();
            env.locals.insert(parameter.binding, parameter_ty.clone());
            lowered.push(GeneralExprParameter {
                binding: parameter.binding,
                span: binding.span,
                name: binding.name.text().into(),
                ty: parameter_ty,
            });
            current = *result;
        }
        Ok((lowered, env, current))
    }

    fn instance_class_item_id(&self, item: &InstanceItem) -> Option<ItemId> {
        let ResolutionState::Resolved(TypeResolution::Item(item_id)) =
            item.class.resolution.as_ref()
        else {
            return None;
        };
        matches!(self.module.items()[*item_id], Item::Class(_)).then_some(*item_id)
    }

    fn instance_argument_bindings(
        &mut self,
        class_item_id: ItemId,
        item: &InstanceItem,
    ) -> Option<PolyTypeBindings> {
        let Item::Class(class_item) = &self.module.items()[class_item_id] else {
            return None;
        };
        if class_item.parameters.len() != item.arguments.len() {
            return None;
        }
        let mut arguments = Vec::with_capacity(item.arguments.len());
        for argument in item.arguments.iter() {
            arguments.push(self.typing.poly_type_binding(*argument)?);
        }
        Some(
            class_item
                .parameters
                .iter()
                .copied()
                .zip(arguments)
                .collect(),
        )
    }

    fn lower_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimeExpr, Vec<GeneralExprBlocker>> {
        let expr = self.module.exprs()[expr_id].clone();
        if let ExprKind::Name(reference) = &expr.kind {
            if let Some(expected) = expected {
                if let Some(reference) =
                    self.constructor_reference_with_expected(&reference, expr.span)
                {
                    return Ok(GateRuntimeExpr {
                        span: expr.span,
                        ty: expected.clone(),
                        kind: GateRuntimeExprKind::Reference(reference),
                    });
                }
            }
        }
        match &expr.kind {
            ExprKind::Regex(_) => {
                return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                    span: expr.span,
                    kind: GateRuntimeUnsupportedKind::RegexLiteral,
                }]);
            }
            ExprKind::Cluster(_) => {
                return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                    span: expr.span,
                    kind: GateRuntimeUnsupportedKind::ApplicativeCluster,
                }]);
            }
            ExprKind::Markup(_) => {
                return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                    span: expr.span,
                    kind: GateRuntimeUnsupportedKind::Markup,
                }]);
            }
            _ => {}
        }
        let ty = self.expr_type(expr_id, env, ambient, expected)?;
        let kind = match expr.kind {
            ExprKind::Name(reference) => GateRuntimeExprKind::Reference(
                self.runtime_reference_for_name(expr.span, &reference, &ty)?,
            ),
            ExprKind::Integer(literal) => GateRuntimeExprKind::Integer(literal),
            ExprKind::Float(literal) => {
                GateRuntimeExprKind::Float(FloatLiteral { raw: literal.raw })
            }
            ExprKind::Decimal(literal) => {
                GateRuntimeExprKind::Decimal(DecimalLiteral { raw: literal.raw })
            }
            ExprKind::BigInt(literal) => {
                GateRuntimeExprKind::BigInt(BigIntLiteral { raw: literal.raw })
            }
            ExprKind::SuffixedInteger(literal) => GateRuntimeExprKind::SuffixedInteger(literal),
            ExprKind::Text(text) => {
                GateRuntimeExprKind::Text(self.lower_text_literal(&text, env, ambient)?)
            }
            ExprKind::Tuple(elements) => {
                let expected_elements = match expected {
                    Some(GateType::Tuple(expected_elements))
                        if expected_elements.len() == elements.len() =>
                    {
                        Some(expected_elements.clone())
                    }
                    _ => None,
                };
                GateRuntimeExprKind::Tuple(
                    elements
                        .iter()
                        .enumerate()
                        .map(|(index, element)| {
                            let expected = expected_elements
                                .as_ref()
                                .and_then(|items| items.get(index));
                            self.lower_expr(*element, env, ambient, expected)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            ExprKind::List(elements) => {
                let expected_element = match expected {
                    Some(GateType::List(element)) => Some(element.as_ref()),
                    _ => None,
                };
                GateRuntimeExprKind::List(
                    elements
                        .iter()
                        .map(|element| self.lower_expr(*element, env, ambient, expected_element))
                        .collect::<Result<_, _>>()?,
                )
            }
            ExprKind::Map(map) => {
                let (expected_key, expected_value) = match expected {
                    Some(GateType::Map { key, value }) => {
                        (Some(key.as_ref()), Some(value.as_ref()))
                    }
                    _ => (None, None),
                };
                GateRuntimeExprKind::Map(
                    map.entries
                        .iter()
                        .map(|entry| {
                            Ok(GateRuntimeMapEntry {
                                key: self.lower_expr(entry.key, env, ambient, expected_key)?,
                                value: self.lower_expr(
                                    entry.value,
                                    env,
                                    ambient,
                                    expected_value,
                                )?,
                            })
                        })
                        .collect::<Result<_, Vec<_>>>()?,
                )
            }
            ExprKind::Set(elements) => {
                let expected_element = match expected {
                    Some(GateType::Set(element)) => Some(element.as_ref()),
                    _ => None,
                };
                GateRuntimeExprKind::Set(
                    elements
                        .iter()
                        .map(|element| self.lower_expr(*element, env, ambient, expected_element))
                        .collect::<Result<_, _>>()?,
                )
            }
            ExprKind::Record(record) => {
                let expected_fields = match expected {
                    Some(GateType::Record(fields)) => Some(
                        fields
                            .iter()
                            .map(|field| (field.name.as_str(), field.ty.clone()))
                            .collect::<HashMap<_, _>>(),
                    ),
                    _ => None,
                };
                GateRuntimeExprKind::Record(
                    record
                        .fields
                        .into_iter()
                        .map(|field| {
                            let expected = expected_fields
                                .as_ref()
                                .and_then(|fields| fields.get(field.label.text()).cloned());
                            Ok(GateRuntimeRecordField {
                                label: field.label,
                                value: self.lower_expr(
                                    field.value,
                                    env,
                                    ambient,
                                    expected.as_ref(),
                                )?,
                            })
                        })
                        .collect::<Result<_, Vec<_>>>()?,
                )
            }
            ExprKind::AmbientSubject => GateRuntimeExprKind::AmbientSubject,
            ExprKind::Projection { base, path } => {
                let base = match base {
                    ProjectionBase::Ambient => GateRuntimeProjectionBase::AmbientSubject,
                    ProjectionBase::Expr(base) => GateRuntimeProjectionBase::Expr(Box::new(
                        self.lower_expr(base, env, ambient, None)?,
                    )),
                };
                GateRuntimeExprKind::Projection { base, path }
            }
            ExprKind::Apply { callee, arguments } => {
                self.lower_apply_expr(expr_id, callee, &arguments, env, ambient, &ty)?
            }
            ExprKind::Unary {
                operator,
                expr: inner,
            } => GateRuntimeExprKind::Unary {
                operator,
                expr: Box::new(self.lower_expr(inner, env, ambient, None)?),
            },
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let expected_operand = match operator {
                    crate::BinaryOperator::And | crate::BinaryOperator::Or => {
                        Some(GateType::Primitive(crate::BuiltinType::Bool))
                    }
                    crate::BinaryOperator::Add
                    | crate::BinaryOperator::Subtract
                    | crate::BinaryOperator::Multiply
                    | crate::BinaryOperator::Divide
                    | crate::BinaryOperator::Modulo => Some(ty.clone()),
                    crate::BinaryOperator::GreaterThan | crate::BinaryOperator::LessThan => None,
                    crate::BinaryOperator::Equals | crate::BinaryOperator::NotEquals => None,
                };
                GateRuntimeExprKind::Binary {
                    left: Box::new(self.lower_expr(
                        left,
                        env,
                        ambient,
                        expected_operand.as_ref(),
                    )?),
                    operator,
                    right: Box::new(self.lower_expr(
                        right,
                        env,
                        ambient,
                        expected_operand.as_ref(),
                    )?),
                }
            }
            ExprKind::Pipe(pipe) => GateRuntimeExprKind::Pipe(self.lower_pipe_expr(
                &pipe,
                env,
                ambient,
                Some(&ty),
                PipeLoweringMode::Full,
            )?),
            ExprKind::Regex(_) | ExprKind::Cluster(_) | ExprKind::Markup(_) => {
                unreachable!("unsupported runtime forms should be returned before type inference")
            }
        };
        Ok(GateRuntimeExpr {
            span: expr.span,
            ty,
            kind,
        })
    }

    fn lower_apply_expr(
        &mut self,
        _expr_id: ExprId,
        callee: ExprId,
        arguments: &crate::NonEmpty<ExprId>,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        result_ty: &GateType,
    ) -> Result<GateRuntimeExprKind, Vec<GeneralExprBlocker>> {
        let constructor_expectations = self.argument_expectations_from_result(callee, result_ty);
        let inferred_callee = self.typing.infer_expr(callee, env, ambient);
        let inferred_parameter_types = inferred_callee
            .actual_gate_type()
            .or_else(|| inferred_callee.ty.clone())
            .and_then(|ty| {
                self.function_signature(&ty, arguments.len())
                    .map(|(parameters, _)| parameters)
            });
        let argument_expectations = constructor_expectations.or(inferred_parameter_types.clone());

        let mut lowered_arguments = Vec::with_capacity(arguments.len());
        let mut argument_types = Vec::with_capacity(arguments.len());
        for (index, argument) in arguments.iter().enumerate() {
            let expected = argument_expectations
                .as_ref()
                .and_then(|types| types.get(index));
            let lowered = self.lower_expr(*argument, env, ambient, expected)?;
            argument_types.push(lowered.ty.clone());
            lowered_arguments.push(lowered);
        }

        let callee_expected = inferred_parameter_types
            .map(|parameters| self.arrow_type(parameters, result_ty.clone()))
            .unwrap_or_else(|| self.arrow_type(argument_types.clone(), result_ty.clone()));
        let lowered_callee = if let ExprKind::Name(reference) = &self.module.exprs()[callee].kind {
            if matches!(
                reference.resolution.as_ref(),
                ResolutionState::Resolved(TermResolution::ClassMember(_))
                    | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            ) {
                let Some(dispatch) = resolve_class_member_dispatch(
                    self.module,
                    reference,
                    &argument_types,
                    Some(result_ty),
                ) else {
                    return Err(vec![GeneralExprBlocker::UnknownExprType {
                        span: self.module.exprs()[callee].span,
                    }]);
                };
                GateRuntimeExpr {
                    span: self.module.exprs()[callee].span,
                    ty: callee_expected.clone(),
                    kind: GateRuntimeExprKind::Reference(GateRuntimeReference::ClassMember(
                        dispatch,
                    )),
                }
            } else {
                self.lower_expr(callee, env, ambient, Some(&callee_expected))?
            }
        } else {
            self.lower_expr(callee, env, ambient, Some(&callee_expected))?
        };
        Ok(GateRuntimeExprKind::Apply {
            callee: Box::new(lowered_callee),
            arguments: lowered_arguments,
        })
    }

    fn lower_pipe_expr(
        &mut self,
        pipe: &PipeExpr,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        final_expected: Option<&GateType>,
        mode: PipeLoweringMode,
    ) -> Result<GateRuntimePipeExpr, Vec<GeneralExprBlocker>> {
        let head = self.lower_expr(pipe.head, env, ambient, None)?;
        let mut current = head.ty.clone();
        let stages = pipe.stages.iter().collect::<Vec<_>>();
        let mut lowered = Vec::with_capacity(stages.len());
        let mut stage_index = 0usize;
        while stage_index < stages.len() {
            let stage = stages[stage_index];
            match &stage.kind {
                PipeStageKind::Transform { expr } => {
                    let mode = self.typing.infer_transform_stage_mode(*expr, env, &current);
                    let result_subject = self
                        .typing
                        .infer_transform_stage(*expr, env, &current)
                        .ok_or_else(|| {
                            vec![GeneralExprBlocker::UnknownExprType { span: stage.span }]
                        })?;
                    let body_expected = (stage_index + 1 == stages.len())
                        .then(|| self.inline_pipe_stage_result_body_type(&current, final_expected))
                        .flatten();
                    let body = self.lower_body_expr(
                        *expr,
                        env,
                        Some(current.gate_payload()),
                        body_expected.as_ref(),
                    )?;
                    lowered.push(GateRuntimePipeStage {
                        span: stage.span,
                        input_subject: current.gate_payload().clone(),
                        result_subject: result_subject.clone(),
                        kind: GateRuntimePipeStageKind::Transform { mode, expr: body },
                    });
                    current = result_subject;
                    stage_index += 1;
                }
                PipeStageKind::Tap { expr } => {
                    let body =
                        self.lower_body_expr(*expr, env, Some(current.gate_payload()), None)?;
                    lowered.push(GateRuntimePipeStage {
                        span: stage.span,
                        input_subject: current.gate_payload().clone(),
                        result_subject: current.clone(),
                        kind: GateRuntimePipeStageKind::Tap { expr: body },
                    });
                    stage_index += 1;
                }
                PipeStageKind::Gate { expr } => {
                    let predicate = self.lower_body_expr(
                        *expr,
                        env,
                        Some(current.gate_payload()),
                        Some(&GateType::Primitive(crate::BuiltinType::Bool)),
                    )?;
                    let result_subject = self
                        .typing
                        .infer_gate_stage(*expr, env, &current)
                        .ok_or_else(|| {
                            vec![GeneralExprBlocker::UnknownExprType { span: stage.span }]
                        })?;
                    let plan = GatePlanner::plan(self.typing.gate_carrier(&current));
                    if matches!(mode, PipeLoweringMode::PrefixBeforeSchedulerBoundary)
                        && current.is_signal()
                    {
                        break;
                    }
                    lowered.push(GateRuntimePipeStage {
                        span: stage.span,
                        input_subject: current.clone(),
                        result_subject: result_subject.clone(),
                        kind: GateRuntimePipeStageKind::Gate {
                            predicate,
                            emits_negative_update: plan.emits_negative_update(),
                        },
                    });
                    current = result_subject;
                    stage_index += 1;
                }
                PipeStageKind::Case { .. } => {
                    let case_start = stage_index;
                    while stage_index < stages.len()
                        && matches!(stages[stage_index].kind, PipeStageKind::Case { .. })
                    {
                        stage_index += 1;
                    }
                    let stage_expected = (stage_index == stages.len())
                        .then(|| final_expected.cloned())
                        .flatten();
                    let lowered_stage = self.lower_case_stage(
                        &stages[case_start..stage_index],
                        env,
                        &current,
                        stage_expected.as_ref(),
                    )?;
                    current = lowered_stage.result_subject.clone();
                    lowered.push(lowered_stage);
                }
                PipeStageKind::Truthy { .. } | PipeStageKind::Falsy { .. } => {
                    let Some(pair) = truthy_falsy_pair_stages(&stages, stage_index) else {
                        return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                            span: stage.span,
                            kind: GateRuntimeUnsupportedKind::PipeStage(
                                GateRuntimeUnsupportedPipeStageKind::Truthy,
                            ),
                        }]);
                    };
                    let stage_expected = (pair.next_index == stages.len())
                        .then(|| final_expected.cloned())
                        .flatten();
                    let lowered_stage = self.lower_truthy_falsy_stage(
                        &pair,
                        env,
                        &current,
                        stage_expected.as_ref(),
                    )?;
                    current = lowered_stage.result_subject.clone();
                    lowered.push(lowered_stage);
                    stage_index = pair.next_index;
                }
                PipeStageKind::Map { .. } => {
                    if matches!(mode, PipeLoweringMode::PrefixBeforeSchedulerBoundary) {
                        break;
                    }
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::Map,
                        ),
                    }]);
                }
                PipeStageKind::Apply { .. } => {
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::Apply,
                        ),
                    }]);
                }
                PipeStageKind::FanIn { .. } => {
                    if matches!(mode, PipeLoweringMode::PrefixBeforeSchedulerBoundary) {
                        break;
                    }
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::FanIn,
                        ),
                    }]);
                }
                PipeStageKind::RecurStart { .. } => {
                    if matches!(mode, PipeLoweringMode::PrefixBeforeSchedulerBoundary) {
                        break;
                    }
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::RecurStart,
                        ),
                    }]);
                }
                PipeStageKind::RecurStep { .. } => {
                    if matches!(mode, PipeLoweringMode::PrefixBeforeSchedulerBoundary) {
                        break;
                    }
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::RecurStep,
                        ),
                    }]);
                }
            }
        }
        Ok(GateRuntimePipeExpr {
            head: Box::new(head),
            stages: lowered,
        })
    }

    fn lower_case_stage(
        &mut self,
        stages: &[&crate::PipeStage],
        env: &GateExprEnv,
        subject: &GateType,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimePipeStage, Vec<GeneralExprBlocker>> {
        let mut arms = Vec::with_capacity(stages.len());
        let mut result_subject = None::<GateType>;
        let mut blockers = Vec::new();
        let branch_subject = subject.gate_payload().clone();
        let branch_expected = self.inline_pipe_stage_result_body_type(subject, expected);
        for stage in stages {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                continue;
            };
            let branch_env = self.case_branch_env(env, *pattern, &branch_subject);
            let lowered_body = match self.lower_body_expr(
                *body,
                &branch_env,
                Some(&branch_subject),
                branch_expected.as_ref(),
            ) {
                Ok(body) => body,
                Err(errors) => {
                    blockers.extend(errors);
                    continue;
                }
            };
            let branch_ty = lowered_body.ty.clone();
            match result_subject.as_ref() {
                Some(current) if !current.same_shape(&branch_ty) => {
                    blockers.push(GeneralExprBlocker::CaseBranchTypeMismatch {
                        span: stage.span,
                        expected: current.to_string(),
                        actual: branch_ty.to_string(),
                    });
                }
                None => result_subject = Some(branch_ty.clone()),
                Some(_) => {}
            }
            arms.push(GateRuntimeCaseArm {
                span: stage.span,
                pattern: *pattern,
                body: lowered_body,
            });
        }
        if !blockers.is_empty() {
            return Err(blockers);
        }
        let result_subject = result_subject.ok_or_else(|| {
            vec![GeneralExprBlocker::UnknownExprType {
                span: stages.first().map(|stage| stage.span).unwrap_or_default(),
            }]
        })?;
        let result_subject = if subject.is_signal() {
            GateType::Signal(Box::new(result_subject))
        } else {
            result_subject
        };
        Ok(GateRuntimePipeStage {
            span: join_stage_spans(stages),
            input_subject: branch_subject,
            result_subject: result_subject.clone(),
            kind: GateRuntimePipeStageKind::Case { arms },
        })
    }

    fn lower_truthy_falsy_stage(
        &mut self,
        pair: &crate::validate::TruthyFalsyPairStages<'_>,
        env: &GateExprEnv,
        subject: &GateType,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimePipeStage, Vec<GeneralExprBlocker>> {
        let branch_expected = self.inline_pipe_stage_result_body_type(subject, expected);
        let plan = self
            .typing
            .truthy_falsy_subject_plan(subject)
            .ok_or_else(|| {
                vec![GeneralExprBlocker::UnknownExprType {
                    span: join_spans(pair.truthy_stage.span, pair.falsy_stage.span),
                }]
            })?;
        let truthy_body = match plan.truthy_payload.as_ref() {
            Some(payload) => self.lower_body_expr(
                pair.truthy_expr,
                env,
                Some(payload),
                branch_expected.as_ref(),
            )?,
            None => self.lower_expr(pair.truthy_expr, env, None, branch_expected.as_ref())?,
        };
        let falsy_body = match plan.falsy_payload.as_ref() {
            Some(payload) => self.lower_body_expr(
                pair.falsy_expr,
                env,
                Some(payload),
                branch_expected.as_ref(),
            )?,
            None => self.lower_expr(pair.falsy_expr, env, None, branch_expected.as_ref())?,
        };
        if !truthy_body.ty.same_shape(&falsy_body.ty) {
            return Err(vec![GeneralExprBlocker::UnknownExprType {
                span: join_spans(pair.truthy_stage.span, pair.falsy_stage.span),
            }]);
        }
        let result_subject = self
            .typing
            .apply_truthy_falsy_result_type(subject, truthy_body.ty.clone());
        Ok(GateRuntimePipeStage {
            span: join_spans(pair.truthy_stage.span, pair.falsy_stage.span),
            input_subject: subject.gate_payload().clone(),
            result_subject: result_subject.clone(),
            kind: GateRuntimePipeStageKind::TruthyFalsy {
                truthy: GateRuntimeTruthyFalsyBranch {
                    span: pair.truthy_stage.span,
                    constructor: plan.truthy_constructor,
                    payload_subject: plan.truthy_payload,
                    result_type: truthy_body.ty.clone(),
                    body: truthy_body,
                },
                falsy: GateRuntimeTruthyFalsyBranch {
                    span: pair.falsy_stage.span,
                    constructor: plan.falsy_constructor,
                    payload_subject: plan.falsy_payload,
                    result_type: falsy_body.ty.clone(),
                    body: falsy_body,
                },
            },
        })
    }

    fn lower_body_expr(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimeExpr, Vec<GeneralExprBlocker>> {
        let mut lowered = match self.lower_expr(expr_id, env, ambient, expected) {
            Ok(lowered) => lowered,
            Err(blockers) if ambient.is_some() && blockers.iter().all(is_unknown_type_blocker) => {
                self.lower_function_pipe_body(
                    expr_id,
                    env,
                    ambient.expect("checked above"),
                    expected,
                )?
            }
            Err(blockers) => return Err(blockers),
        };
        let Some(ambient) = ambient else {
            return Ok(lowered);
        };
        if let Ok(function_body) = self.lower_function_pipe_body(expr_id, env, ambient, expected) {
            lowered = function_body;
        }
        let GateType::Arrow { parameter, result } = lowered.ty.clone() else {
            return Ok(lowered);
        };
        if !parameter.same_shape(ambient) {
            return Ok(lowered);
        }
        lowered = GateRuntimeExpr {
            span: self.module.exprs()[expr_id].span,
            ty: *result,
            kind: GateRuntimeExprKind::Apply {
                callee: Box::new(lowered),
                arguments: vec![GateRuntimeExpr {
                    span: self.module.exprs()[expr_id].span,
                    ty: ambient.clone(),
                    kind: GateRuntimeExprKind::AmbientSubject,
                }],
            },
        };
        Ok(lowered)
    }

    fn lower_function_pipe_body(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: &GateType,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimeExpr, Vec<GeneralExprBlocker>> {
        let expr = self.module.exprs()[expr_id].clone();
        let plan = self
            .typing
            .match_pipe_function_signature(expr_id, env, ambient, expected)
            .ok_or_else(|| vec![GeneralExprBlocker::UnknownExprType { span: expr.span }])?;
        let callee_ty = self.arrow_type(plan.parameter_types.clone(), plan.result_type.clone());
        let callee = self.lower_expr(plan.callee_expr, env, Some(ambient), Some(&callee_ty))?;
        let mut arguments = Vec::with_capacity(plan.explicit_arguments.len() + 1);
        for (argument, expected_parameter) in plan
            .explicit_arguments
            .iter()
            .zip(plan.parameter_types.iter())
        {
            arguments.push(self.lower_expr(
                *argument,
                env,
                Some(ambient),
                Some(expected_parameter),
            )?);
        }
        arguments.push(GateRuntimeExpr {
            span: expr.span,
            ty: ambient.clone(),
            kind: GateRuntimeExprKind::AmbientSubject,
        });
        Ok(GateRuntimeExpr {
            span: expr.span,
            ty: plan.result_type.clone(),
            kind: GateRuntimeExprKind::Apply {
                callee: Box::new(callee),
                arguments,
            },
        })
    }

    fn lower_text_literal(
        &mut self,
        text: &crate::TextLiteral,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
    ) -> Result<GateRuntimeTextLiteral, Vec<GeneralExprBlocker>> {
        let mut segments = Vec::with_capacity(text.segments.len());
        for segment in &text.segments {
            let lowered = match segment {
                crate::TextSegment::Text(fragment) => {
                    GateRuntimeTextSegment::Fragment(fragment.clone())
                }
                crate::TextSegment::Interpolation(interpolation) => {
                    GateRuntimeTextSegment::Interpolation(Box::new(self.lower_expr(
                        interpolation.expr,
                        env,
                        ambient,
                        None,
                    )?))
                }
            };
            segments.push(lowered);
        }
        Ok(GateRuntimeTextLiteral { segments })
    }

    fn expr_type(
        &mut self,
        expr_id: ExprId,
        env: &GateExprEnv,
        ambient: Option<&GateType>,
        expected: Option<&GateType>,
    ) -> Result<GateType, Vec<GeneralExprBlocker>> {
        if let Some(expected) = expected {
            if matches!(self.module.exprs()[expr_id].kind, ExprKind::Pipe(_))
                || expression_matches(self.module, expr_id, env, expected)
            {
                return Ok(expected.clone());
            }
        }
        let info = self.typing.infer_expr(expr_id, env, ambient);
        if !info.issues.is_empty() {
            return Err(self.blockers_from_issues(info.issues));
        }
        info.actual_gate_type().or(info.ty).ok_or_else(|| {
            vec![GeneralExprBlocker::UnknownExprType {
                span: self.module.exprs()[expr_id].span,
            }]
        })
    }

    fn runtime_reference_for_name(
        &self,
        span: SourceSpan,
        reference: &TermReference,
        expected: &GateType,
    ) -> Result<GateRuntimeReference, Vec<GeneralExprBlocker>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                Ok(GateRuntimeReference::Local(*binding))
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => Ok(self
                .module
                .sum_constructor_handle(*item_id, reference.path.segments().last().text())
                .map(GateRuntimeReference::SumConstructor)
                .unwrap_or(GateRuntimeReference::Item(*item_id))),
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => self
                .module
                .domain_member_handle(*resolution)
                .map(GateRuntimeReference::DomainMember)
                .ok_or_else(|| vec![GeneralExprBlocker::UnknownExprType { span }]),
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                Ok(GateRuntimeReference::Builtin(*builtin))
            }
            ResolutionState::Resolved(TermResolution::IntrinsicValue(value)) => {
                Ok(GateRuntimeReference::IntrinsicValue(*value))
            }
            ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_)) => {
                resolve_class_member_dispatch(self.module, reference, &[], Some(expected))
                    .map(GateRuntimeReference::ClassMember)
                    .ok_or_else(|| vec![GeneralExprBlocker::UnknownExprType { span }])
            }
            ResolutionState::Resolved(TermResolution::Import(import)) => {
                Ok(GateRuntimeReference::Import(*import))
            }
            ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates)) => {
                Err(vec![GeneralExprBlocker::AmbiguousDomainMember {
                    span,
                    name: reference.path.segments().last().text().to_owned(),
                    candidates: candidates
                        .iter()
                        .filter_map(|candidate| self.module.domain_member_handle(*candidate))
                        .map(|handle| format!("{}.{}", handle.domain_name, handle.member_name))
                        .collect(),
                }])
            }
            ResolutionState::Unresolved => Err(vec![GeneralExprBlocker::UnknownExprType { span }]),
        }
    }

    fn constructor_reference_with_expected(
        &self,
        reference: &TermReference,
        _span: SourceSpan,
    ) -> Option<GateRuntimeReference> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Builtin(
                BuiltinTerm::Some
                | BuiltinTerm::Ok
                | BuiltinTerm::Err
                | BuiltinTerm::Valid
                | BuiltinTerm::Invalid,
            )) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                    Some(GateRuntimeReference::Builtin(*builtin))
                }
                _ => None,
            },
            ResolutionState::Resolved(TermResolution::Item(item_id)) => self
                .module
                .sum_constructor_handle(*item_id, reference.path.segments().last().text())
                .map(GateRuntimeReference::SumConstructor),
            _ => None,
        }
    }

    fn blockers_from_issues(&self, issues: Vec<GateIssue>) -> Vec<GeneralExprBlocker> {
        issues
            .into_iter()
            .map(|issue| match issue {
                GateIssue::InvalidProjection {
                    span,
                    path,
                    subject,
                } => GeneralExprBlocker::InvalidProjection {
                    span,
                    path,
                    subject,
                },
                GateIssue::UnknownField {
                    span,
                    path,
                    subject,
                } => GeneralExprBlocker::UnknownField {
                    span,
                    path,
                    subject,
                },
                GateIssue::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                } => GeneralExprBlocker::AmbiguousDomainMember {
                    span,
                    name,
                    candidates,
                },
                GateIssue::CaseBranchTypeMismatch {
                    span,
                    expected,
                    actual,
                } => GeneralExprBlocker::CaseBranchTypeMismatch {
                    span,
                    expected,
                    actual,
                },
                GateIssue::AmbientSubjectOutsidePipe { span }
                | GateIssue::InvalidPipeStageInput { span, .. }
                | GateIssue::UnsupportedApplicativeClusterMember { span, .. }
                | GateIssue::ApplicativeClusterMismatch { span, .. }
                | GateIssue::InvalidClusterFinalizer { span, .. } => {
                    GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span,
                        kind: GateRuntimeUnsupportedKind::ApplicativeCluster,
                    }
                }
                GateIssue::AmbiguousDomainOperator { span, .. } => {
                    GeneralExprBlocker::UnknownExprType { span }
                }
            })
            .collect()
    }

    fn case_branch_env(
        &mut self,
        env: &GateExprEnv,
        pattern: crate::PatternId,
        subject: &GateType,
    ) -> GateExprEnv {
        let mut branch_env = env.clone();
        branch_env
            .locals
            .extend(self.case_pattern_bindings(pattern, subject).locals);
        branch_env
    }

    fn case_pattern_bindings(
        &mut self,
        pattern_id: crate::PatternId,
        subject: &GateType,
    ) -> GateExprEnv {
        let mut env = GateExprEnv::default();
        let mut work = vec![(pattern_id, subject.clone())];
        while let Some((pattern_id, subject_ty)) = work.pop() {
            let Some(pattern) = self.module.patterns().get(pattern_id).cloned() else {
                continue;
            };
            match pattern.kind {
                crate::PatternKind::Wildcard
                | crate::PatternKind::Integer(_)
                | crate::PatternKind::Text(_)
                | crate::PatternKind::UnresolvedName(_) => {}
                crate::PatternKind::Binding(binding) => {
                    env.locals.insert(binding.binding, subject_ty);
                }
                crate::PatternKind::Tuple(elements) => {
                    let GateType::Tuple(subject_elements) = &subject_ty else {
                        continue;
                    };
                    if elements.len() != subject_elements.len() {
                        continue;
                    }
                    let pairs = elements
                        .iter()
                        .zip(subject_elements.iter())
                        .collect::<Vec<_>>();
                    for (element, element_ty) in pairs.into_iter().rev() {
                        work.push((*element, element_ty.clone()));
                    }
                }
                crate::PatternKind::List { elements, rest } => {
                    let GateType::List(element_ty) = &subject_ty else {
                        continue;
                    };
                    for element in elements.into_iter().rev() {
                        work.push((element, element_ty.as_ref().clone()));
                    }
                    if let Some(rest) = rest {
                        work.push((rest, subject_ty));
                    }
                }
                crate::PatternKind::Record(fields) => {
                    let GateType::Record(subject_fields) = &subject_ty else {
                        continue;
                    };
                    for field in fields.into_iter().rev() {
                        let Some(field_ty) = subject_fields
                            .iter()
                            .find(|candidate| candidate.name == field.label.text())
                            .map(|field_ty| field_ty.ty.clone())
                        else {
                            continue;
                        };
                        work.push((field.pattern, field_ty));
                    }
                }
                crate::PatternKind::Constructor { callee, arguments } => {
                    let Some(field_types) = self.case_pattern_field_types(&callee, &subject_ty)
                    else {
                        continue;
                    };
                    if field_types.len() != arguments.len() {
                        continue;
                    }
                    for (argument, field_ty) in arguments.into_iter().zip(field_types).rev() {
                        work.push((argument, field_ty));
                    }
                }
            }
        }
        env
    }

    fn case_pattern_field_types(
        &mut self,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        match callee.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::True))
            | ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::False)) => {
                matches!(subject, GateType::Primitive(crate::BuiltinType::Bool)).then(Vec::new)
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Some)) => {
                match subject {
                    GateType::Option(payload) => Some(vec![payload.as_ref().clone()]),
                    _ => None,
                }
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::None)) => {
                matches!(subject, GateType::Option(_)).then(Vec::new)
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Ok)) => match subject {
                GateType::Result { value, .. } => Some(vec![value.as_ref().clone()]),
                _ => None,
            },
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Err)) => match subject {
                GateType::Result { error, .. } => Some(vec![error.as_ref().clone()]),
                _ => None,
            },
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Valid)) => match subject
            {
                GateType::Validation { value, .. } => Some(vec![value.as_ref().clone()]),
                _ => None,
            },
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Invalid)) => {
                match subject {
                    GateType::Validation { error, .. } => Some(vec![error.as_ref().clone()]),
                    _ => None,
                }
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                self.same_module_constructor_fields(*item_id, callee, subject)
            }
            ResolutionState::Resolved(TermResolution::Local(_))
            | ResolutionState::Resolved(TermResolution::Import(_))
            | ResolutionState::Resolved(TermResolution::IntrinsicValue(_))
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
            | ResolutionState::Resolved(TermResolution::ClassMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousClassMembers(_))
            | ResolutionState::Unresolved => None,
        }
    }

    fn same_module_constructor_fields(
        &mut self,
        item_id: ItemId,
        callee: &TermReference,
        subject: &GateType,
    ) -> Option<Vec<GateType>> {
        let Item::Type(item) = &self.module.items()[item_id] else {
            return None;
        };
        let TypeItemBody::Sum(variants) = &item.body else {
            return None;
        };
        let GateType::OpaqueItem {
            item: subject_item,
            arguments,
            ..
        } = subject
        else {
            return None;
        };
        if *subject_item != item_id || item.parameters.len() != arguments.len() {
            return None;
        }
        let variant_name = callee.path.segments().last().text();
        let variant = variants
            .iter()
            .find(|variant| variant.name.text() == variant_name)?;
        let substitutions = item
            .parameters
            .iter()
            .copied()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        variant
            .fields
            .iter()
            .map(|field| self.typing.lower_hir_type(*field, &substitutions))
            .collect()
    }

    fn argument_expectations_from_result(
        &mut self,
        callee: ExprId,
        result_ty: &GateType,
    ) -> Option<Vec<GateType>> {
        let ExprKind::Name(reference) = &self.module.exprs()[callee].kind else {
            return None;
        };
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Some)) => {
                if let GateType::Option(payload) = result_ty {
                    Some(vec![payload.as_ref().clone()])
                } else {
                    None
                }
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Ok)) => {
                if let GateType::Result { value, .. } = result_ty {
                    Some(vec![value.as_ref().clone()])
                } else {
                    None
                }
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Err)) => {
                if let GateType::Result { error, .. } = result_ty {
                    Some(vec![error.as_ref().clone()])
                } else {
                    None
                }
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Valid)) => {
                if let GateType::Validation { value, .. } = result_ty {
                    Some(vec![value.as_ref().clone()])
                } else {
                    None
                }
            }
            ResolutionState::Resolved(TermResolution::Builtin(BuiltinTerm::Invalid)) => {
                if let GateType::Validation { error, .. } = result_ty {
                    Some(vec![error.as_ref().clone()])
                } else {
                    None
                }
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                self.same_module_constructor_fields(*item_id, reference, result_ty)
            }
            _ => None,
        }
    }

    fn function_signature(&self, ty: &GateType, arity: usize) -> Option<(Vec<GateType>, GateType)> {
        let mut current = ty;
        let mut parameters = Vec::with_capacity(arity);
        for _ in 0..arity {
            let GateType::Arrow { parameter, result } = current else {
                return None;
            };
            parameters.push(parameter.as_ref().clone());
            current = result.as_ref();
        }
        Some((parameters, current.clone()))
    }

    fn arrow_type(&self, parameters: Vec<GateType>, result: GateType) -> GateType {
        parameters
            .into_iter()
            .rev()
            .fold(result, |result, parameter| GateType::Arrow {
                parameter: Box::new(parameter),
                result: Box::new(result),
            })
    }

    fn inline_pipe_stage_result_body_type(
        &self,
        input_subject: &GateType,
        expected: Option<&GateType>,
    ) -> Option<GateType> {
        let expected = expected?;
        match (input_subject, expected) {
            (GateType::Signal(_), GateType::Signal(payload)) => Some(payload.as_ref().clone()),
            _ => Some(expected.clone()),
        }
    }

    fn each_child_env(&mut self, env: &GateExprEnv, each: &crate::EachControl) -> GateExprEnv {
        let mut child_env = env.clone();
        if let Some(element_ty) = self
            .typing
            .infer_expr(each.collection, env, None)
            .ty
            .and_then(|collection| collection.fanout_element().cloned())
        {
            child_env.locals.insert(each.binding, element_ty);
        }
        child_env
    }

    fn with_child_env(&mut self, env: &GateExprEnv, with_node: &crate::WithControl) -> GateExprEnv {
        let mut child_env = env.clone();
        if let Some(value_ty) = self.typing.infer_expr(with_node.value, env, None).ty {
            child_env.locals.insert(with_node.binding, value_ty);
        }
        child_env
    }

    fn record_markup_runtime_expr_site(
        &mut self,
        expr: ExprId,
        env: &GateExprEnv,
        sites: &mut BTreeMap<ExprId, MarkupRuntimeExprSite>,
    ) -> Result<(), MarkupRuntimeExprSiteError> {
        let ty = self.typing.infer_expr(expr, env, None).ty.ok_or(
            MarkupRuntimeExprSiteError::UnknownExprType {
                expr,
                span: self.module.exprs()[expr].span,
            },
        )?;
        let parameters = env_parameters(self.module, env);
        sites.entry(expr).or_insert(MarkupRuntimeExprSite {
            expr,
            span: self.module.exprs()[expr].span,
            ty,
            parameters,
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipeLoweringMode {
    Full,
    PrefixBeforeSchedulerBoundary,
}

fn gate_env_from_parameters(parameters: &[GeneralExprParameter]) -> GateExprEnv {
    let mut env = GateExprEnv::default();
    for parameter in parameters {
        env.locals.insert(parameter.binding, parameter.ty.clone());
    }
    env
}

fn env_parameters(module: &Module, env: &GateExprEnv) -> Vec<GeneralExprParameter> {
    let mut parameters = env
        .locals
        .iter()
        .map(|(binding, ty)| {
            let binding_info = &module.bindings()[*binding];
            GeneralExprParameter {
                binding: *binding,
                span: binding_info.span,
                name: binding_info.name.text().into(),
                ty: ty.clone(),
            }
        })
        .collect::<Vec<_>>();
    parameters.sort_by_key(|parameter| parameter.binding.as_raw());
    parameters
}

fn join_stage_spans(stages: &[&crate::PipeStage]) -> SourceSpan {
    let mut span = stages
        .first()
        .map(|stage| stage.span)
        .unwrap_or_else(SourceSpan::default);
    for stage in stages.iter().skip(1) {
        span = join_spans(span, stage.span);
    }
    span
}

fn join_spans(left: SourceSpan, right: SourceSpan) -> SourceSpan {
    left.join(right)
        .expect("general-expression elaboration only joins spans from the same file")
}

fn is_unknown_type_blocker(blocker: &GeneralExprBlocker) -> bool {
    matches!(
        blocker,
        GeneralExprBlocker::UnknownExprType { .. }
            | GeneralExprBlocker::UnsupportedImportReference { .. }
    )
}

impl From<GateElaborationBlocker> for GeneralExprBlocker {
    fn from(blocker: GateElaborationBlocker) -> Self {
        match blocker {
            GateElaborationBlocker::UnknownSubjectType
            | GateElaborationBlocker::UnknownPredicateType
            | GateElaborationBlocker::UnknownRuntimeExprType { .. }
            | GateElaborationBlocker::ImpurePredicate
            | GateElaborationBlocker::PredicateNotBool { .. } => {
                GeneralExprBlocker::UnknownExprType {
                    span: SourceSpan::default(),
                }
            }
            GateElaborationBlocker::InvalidProjection { path, subject } => {
                GeneralExprBlocker::InvalidProjection {
                    span: SourceSpan::default(),
                    path,
                    subject,
                }
            }
            GateElaborationBlocker::UnknownField { path, subject } => {
                GeneralExprBlocker::UnknownField {
                    span: SourceSpan::default(),
                    path,
                    subject,
                }
            }
            GateElaborationBlocker::UnsupportedRuntimeExpr { span, kind } => {
                GeneralExprBlocker::UnsupportedRuntimeExpr { span, kind }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::{FileId, SourceDatabase, SourceSpan};
    use aivi_syntax::parse_module;

    use super::{
        ExprKind, GateRuntimeExprKind, GateRuntimePipeStageKind, GeneralExprBlocker,
        GeneralExprOutcome, Item, elaborate_general_expressions,
    };
    use crate::{
        BuiltinType, PipeTransformMode,
        typecheck::resolve_class_member_dispatch,
        validate::{GateExprEnv, GateTypeContext},
    };

    fn item_name(module: &crate::Module, item: crate::ItemId) -> Option<&str> {
        match &module.items()[item] {
            crate::Item::Value(item) => Some(item.name.text()),
            crate::Item::Function(item) => Some(item.name.text()),
            _ => None,
        }
    }

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
            "general-expression test input should parse: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        crate::lower_module(&parsed.module)
    }

    fn lower_fixture(path: &str) -> crate::LoweringResult {
        let text =
            fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
        lower_text(path, &text)
    }

    fn unit_span() -> SourceSpan {
        SourceSpan::default()
    }

    fn test_name(text: &str) -> crate::Name {
        crate::Name::new(text, unit_span()).expect("test name should stay valid")
    }

    fn test_path(text: &str) -> crate::NamePath {
        crate::NamePath::from_vec(vec![test_name(text)]).expect("single-segment path")
    }

    fn builtin_type(module: &mut crate::Module, builtin: BuiltinType) -> crate::TypeId {
        let builtin_name = match builtin {
            BuiltinType::Int => "Int",
            BuiltinType::Float => "Float",
            BuiltinType::Decimal => "Decimal",
            BuiltinType::BigInt => "BigInt",
            BuiltinType::Bool => "Bool",
            BuiltinType::Text => "Text",
            BuiltinType::Unit => "Unit",
            BuiltinType::Bytes => "Bytes",
            BuiltinType::List => "List",
            BuiltinType::Map => "Map",
            BuiltinType::Set => "Set",
            BuiltinType::Option => "Option",
            BuiltinType::Result => "Result",
            BuiltinType::Validation => "Validation",
            BuiltinType::Signal => "Signal",
            BuiltinType::Task => "Task",
        };
        module
            .alloc_type(crate::TypeNode {
                span: unit_span(),
                kind: crate::TypeKind::Name(crate::TypeReference::resolved(
                    test_path(builtin_name),
                    crate::TypeResolution::Builtin(builtin),
                )),
            })
            .expect("builtin type allocation should fit")
    }

    #[test]
    fn elaborates_function_case_bodies() {
        let lowered = lower_fixture("milestone-1/valid/patterns/pattern_matching.aivi");
        assert!(
            !lowered.has_errors(),
            "pattern fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_general_expressions(lowered.module());
        let loaded_name = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("loadedName"))
            .expect("expected loadedName elaboration");
        match &loaded_name.outcome {
            GeneralExprOutcome::Lowered(expr) => match &expr.kind {
                GateRuntimeExprKind::Pipe(pipe) => match &pipe.stages[0].kind {
                    GateRuntimePipeStageKind::Case { arms } => assert_eq!(arms.len(), 3),
                    other => panic!("expected case pipe stage, found {other:?}"),
                },
                other => panic!("expected pipe body, found {other:?}"),
            },
            other => panic!("expected lowered function body, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_pipe_transform_modes_for_callable_and_replacement_stages() {
        let mut module = crate::Module::new(FileId::new(0));
        let int_type = builtin_type(&mut module, BuiltinType::Int);
        let text_type = builtin_type(&mut module, BuiltinType::Text);
        let binding = module
            .alloc_binding(crate::Binding {
                span: unit_span(),
                name: test_name("value"),
                kind: crate::BindingKind::FunctionParameter,
            })
            .expect("binding allocation should fit");
        let local_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("value"),
                    crate::TermResolution::Local(binding),
                )),
            })
            .expect("local expression allocation should fit");
        let add_one = module
            .push_item(crate::Item::Function(crate::FunctionItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("addOne"),
                type_parameters: Vec::new(),
                context: Vec::new(),
                parameters: vec![crate::FunctionParameter {
                    span: unit_span(),
                    binding,
                    annotation: Some(int_type),
                }],
                annotation: Some(int_type),
                body: local_expr,
            }))
            .expect("function allocation should fit");
        let head = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Integer(crate::IntegerLiteral { raw: "1".into() }),
            })
            .expect("head allocation should fit");
        let callable_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Name(crate::TermReference::resolved(
                    test_path("addOne"),
                    crate::TermResolution::Item(add_one),
                )),
            })
            .expect("callable expression allocation should fit");
        let replacement_expr = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Text(crate::TextLiteral {
                    segments: vec![crate::TextSegment::Text(crate::TextFragment {
                        raw: "done".into(),
                        span: unit_span(),
                    })],
                }),
            })
            .expect("replacement expression allocation should fit");
        let pipe = module
            .alloc_expr(crate::Expr {
                span: unit_span(),
                kind: crate::ExprKind::Pipe(crate::PipeExpr {
                    head,
                    stages: crate::NonEmpty::new(
                        crate::PipeStage {
                            span: unit_span(),
                            kind: crate::PipeStageKind::Transform {
                                expr: callable_expr,
                            },
                        },
                        vec![crate::PipeStage {
                            span: unit_span(),
                            kind: crate::PipeStageKind::Transform {
                                expr: replacement_expr,
                            },
                        }],
                    ),
                }),
            })
            .expect("pipe allocation should fit");
        let final_label = module
            .push_item(crate::Item::Value(crate::ValueItem {
                header: crate::ItemHeader {
                    span: unit_span(),
                    decorators: Vec::new(),
                },
                name: test_name("finalLabel"),
                annotation: Some(text_type),
                body: pipe,
            }))
            .expect("value allocation should fit");

        let report = elaborate_general_expressions(&module);
        let final_label = report
            .items()
            .iter()
            .find(|item| item.owner == final_label)
            .expect("expected finalLabel elaboration");
        match &final_label.outcome {
            GeneralExprOutcome::Lowered(expr) => match &expr.kind {
                GateRuntimeExprKind::Pipe(pipe) => {
                    assert_eq!(pipe.stages.len(), 2);
                    let GateRuntimePipeStageKind::Transform {
                        mode: first_mode,
                        expr: first_expr,
                    } = &pipe.stages[0].kind
                    else {
                        panic!("expected callable transform stage first");
                    };
                    assert_eq!(*first_mode, PipeTransformMode::Apply);
                    assert!(matches!(first_expr.kind, GateRuntimeExprKind::Apply { .. }));

                    let GateRuntimePipeStageKind::Transform {
                        mode: second_mode,
                        expr: second_expr,
                    } = &pipe.stages[1].kind
                    else {
                        panic!("expected replacement transform stage second");
                    };
                    assert_eq!(*second_mode, PipeTransformMode::Replace);
                    assert!(matches!(second_expr.kind, GateRuntimeExprKind::Text(_)));
                }
                other => panic!("expected pipe body, found {other:?}"),
            },
            other => panic!("expected lowered value body, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_truthy_falsy_pairs_into_typed_branches() {
        let lowered = lower_fixture("milestone-1/valid/pipes/pipe_algebra.aivi");
        assert!(
            !lowered.has_errors(),
            "pipe fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_general_expressions(lowered.module());
        let start_or_wait = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("startOrWait"))
            .expect("expected startOrWait elaboration");
        match &start_or_wait.outcome {
            GeneralExprOutcome::Lowered(expr) => match &expr.kind {
                GateRuntimeExprKind::Pipe(pipe) => match &pipe.stages[0].kind {
                    GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                        assert_eq!(truthy.constructor, crate::BuiltinTerm::True);
                        assert_eq!(falsy.constructor, crate::BuiltinTerm::False);
                    }
                    other => panic!("expected truthy/falsy pipe stage, found {other:?}"),
                },
                other => panic!("expected pipe body, found {other:?}"),
            },
            other => panic!("expected lowered function body, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_truthy_falsy_branches_from_expected_result_types() {
        let lowered = lower_text(
            "expected-truthy-falsy-branches.aivi",
            "fun choose:(List Int) flag:Bool =>\n\
                flag\n\
                 T|> []\n\
                 F|> [1]\n",
        );
        assert!(
            !lowered.has_errors(),
            "expected truthy/falsy fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );
        let report = elaborate_general_expressions(lowered.module());
        let choose = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("choose"))
            .expect("expected choose elaboration");
        assert!(
            matches!(choose.outcome, GeneralExprOutcome::Lowered(_)),
            "expected choose body to lower using the annotated result type, got {:?}",
            choose.outcome
        );
    }

    #[test]
    fn elaborates_option_default_record_elision_into_runtime_record_fields() {
        let lowered = lower_text(
            "record-default-elision-general.aivi",
            "use aivi.defaults (Option)\n\
             type Profile = {\n\
                 name: Text,\n\
                 nickname: Option Text,\n\
                 bio: Option Text\n\
             }\n\
             val name = \"Ada\"\n\
             val nickname = Some \"Countess\"\n\
             val profile:Profile = { name, nickname }\n",
        );
        assert!(
            !lowered.has_errors(),
            "record-default-elision fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let profile = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("profile"))
            .expect("expected profile elaboration");
        match &profile.outcome {
            GeneralExprOutcome::Lowered(expr) => match &expr.kind {
                GateRuntimeExprKind::Record(fields) => {
                    assert_eq!(
                        fields.len(),
                        3,
                        "expected omitted record field to be lowered"
                    );
                    assert_eq!(
                        fields
                            .iter()
                            .map(|field| field.label.text())
                            .collect::<Vec<_>>(),
                        vec!["name", "nickname", "bio"]
                    );
                    match &fields[2].value.kind {
                        GateRuntimeExprKind::Reference(crate::GateRuntimeReference::Builtin(
                            crate::BuiltinTerm::None,
                        )) => {}
                        other => panic!(
                            "expected synthesized option default to lower as builtin None, found {other:?}"
                        ),
                    }
                }
                other => panic!("expected lowered runtime record, found {other:?}"),
            },
            other => panic!("expected lowered profile body, found {other:?}"),
        }
    }

    #[test]
    fn blocks_regex_literals_in_general_expr_bodies() {
        let lowered = lower_text("general-expr-blocked-regex.aivi", "val pattern = rx\"a+\"");
        assert!(
            !lowered.has_errors(),
            "regex general-expression fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let pattern = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("pattern"))
            .expect("expected pattern elaboration");
        match &pattern.outcome {
            GeneralExprOutcome::Blocked(blocked) => {
                assert!(matches!(
                    blocked.blockers.as_slice(),
                    [GeneralExprBlocker::UnsupportedRuntimeExpr {
                        kind: crate::GateRuntimeUnsupportedKind::RegexLiteral,
                        ..
                    }]
                ));
                assert_eq!(
                    blocked.to_string(),
                    "regex literal is not supported in typed-core general expressions"
                );
            }
            other => panic!("expected blocked pattern body, found {other:?}"),
        }
    }

    #[test]
    fn blocks_map_pipe_stages_in_general_expr_bodies() {
        let lowered = lower_text(
            "general-expr-blocked-map-stage.aivi",
            "fun identity:Int value:Int =>\n\
             value\n\
             \n\
             fun duplicate:List Int values:List Int =>\n\
             values\n\
              *|> identity\n",
        );
        assert!(
            !lowered.has_errors(),
            "map-stage general-expression fixture should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let duplicate = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("duplicate"))
            .expect("expected duplicate elaboration");
        match &duplicate.outcome {
            GeneralExprOutcome::Blocked(blocked) => {
                assert!(matches!(
                    blocked.blockers.as_slice(),
                    [GeneralExprBlocker::UnsupportedRuntimeExpr {
                        kind: crate::GateRuntimeUnsupportedKind::PipeStage(
                            crate::GateRuntimeUnsupportedPipeStageKind::Map
                        ),
                        ..
                    }]
                ));
                assert_eq!(
                    blocked.to_string(),
                    "map pipe stage is not supported in typed-core general expressions"
                );
            }
            other => panic!("expected blocked duplicate body, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_same_module_instance_member_bodies() {
        let lowered = lower_text(
            "general-expr-instance-member.aivi",
            r#"
class Semigroup A
    append : A -> A -> A

type Blob = Blob Int

instance Semigroup Blob
    append left right =
        left
"#,
        );
        assert!(
            !lowered.has_errors(),
            "instance-member example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        assert!(
            report.items().is_empty(),
            "instance-member-only module should not synthesize ordinary item elaborations"
        );
        let append = report
            .instance_members()
            .iter()
            .find(|member| member.member_index == 0)
            .expect("expected instance member elaboration");
        assert_eq!(
            append
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_ref())
                .collect::<Vec<_>>(),
            vec!["left", "right"]
        );
        match &append.outcome {
            GeneralExprOutcome::Lowered(expr) => {
                assert!(matches!(
                    expr.kind,
                    crate::GateRuntimeExprKind::Reference(crate::GateRuntimeReference::Local(_))
                ));
            }
            other => panic!("expected lowered instance member body, found {other:?}"),
        }
    }

    #[test]
    fn elaborates_generic_append_calls_in_function_bodies() {
        let lowered = lower_text(
            "general-expr-generic-append.aivi",
            r#"
fun appendOne:(List A) items:(List A) item:A =>
    append items [item]
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic append example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let append_one = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("appendOne"))
            .expect("expected appendOne elaboration");
        if !matches!(append_one.outcome, GeneralExprOutcome::Lowered(_)) {
            let module = lowered.module();
            let Item::Function(function) = &module.items()[append_one.owner] else {
                panic!("appendOne should be a function");
            };
            let mut typing = GateTypeContext::new(module);
            let mut env = GateExprEnv::default();
            for parameter in &function.parameters {
                let annotation = parameter.annotation.expect("test parameters are annotated");
                env.locals.insert(
                    parameter.binding,
                    typing
                        .lower_open_annotation(annotation)
                        .expect("test parameter types should lower"),
                );
            }
            let ExprKind::Apply { callee, arguments } = module.exprs()[function.body].kind.clone()
            else {
                panic!("appendOne body should be an apply expression");
            };
            let ExprKind::Name(reference) = &module.exprs()[callee].kind else {
                panic!("appendOne callee should be a name");
            };
            let argument_infos = arguments
                .iter()
                .map(|argument| (*argument, typing.infer_expr(*argument, &env, None)))
                .collect::<Vec<_>>();
            let argument_types = argument_infos
                .iter()
                .map(|(_, info)| info.actual_gate_type().or(info.ty.clone()))
                .collect::<Option<Vec<_>>>()
                .unwrap_or_else(|| {
                    panic!("appendOne arguments should infer: {argument_infos:?}");
                });
            let expected = typing
                .lower_open_annotation(function.annotation.expect("appendOne is annotated"))
                .expect("appendOne result type should lower");
            let dispatch =
                resolve_class_member_dispatch(module, reference, &argument_types, Some(&expected));
            panic!(
                "expected generic append body to lower, found {:?}; argument_types={argument_types:?}; expected={expected:?}; dispatch={dispatch:?}",
                append_one.outcome
            );
        }
    }

    #[test]
    fn elaborates_reduce_pipe_bodies_with_generic_step_functions() {
        let lowered = lower_text(
            "general-expr-generic-reduce.aivi",
            r#"
fun lengthStep:Int total:Int item:A =>
    total + 1

fun length:Int items:(List A) =>
    items
     |> reduce lengthStep 0
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic reduce example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let length = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("length"))
            .expect("expected length elaboration");
        if !matches!(length.outcome, GeneralExprOutcome::Lowered(_)) {
            let module = lowered.module();
            let Item::Function(function) = &module.items()[length.owner] else {
                panic!("length should be a function");
            };
            let mut typing = GateTypeContext::new(module);
            let mut env = GateExprEnv::default();
            for parameter in &function.parameters {
                let annotation = parameter.annotation.expect("test parameters are annotated");
                env.locals.insert(
                    parameter.binding,
                    typing
                        .lower_open_annotation(annotation)
                        .expect("test parameter types should lower"),
                );
            }
            let ExprKind::Pipe(pipe) = &module.exprs()[function.body].kind else {
                panic!("length body should be a pipe expression");
            };
            let stage_expr = match &pipe.stages.first().kind {
                crate::PipeStageKind::Transform { expr } => *expr,
                other => panic!("expected transform stage, found {other:?}"),
            };
            let ExprKind::Apply { callee, arguments } = module.exprs()[stage_expr].kind.clone()
            else {
                panic!("reduce stage should be an apply expression");
            };
            let ExprKind::Name(reference) = &module.exprs()[callee].kind else {
                panic!("reduce callee should be a name");
            };
            let argument_infos = arguments
                .iter()
                .map(|argument| (*argument, typing.infer_expr(*argument, &env, None)))
                .collect::<Vec<_>>();
            let argument_types = argument_infos
                .iter()
                .map(|(_, info)| info.actual_gate_type().or(info.ty.clone()))
                .collect::<Vec<_>>();
            let ambient = env
                .locals
                .values()
                .next()
                .expect("length should have one parameter")
                .clone();
            let plan = typing.match_pipe_function_signature(
                stage_expr,
                &env,
                ambient.gate_payload(),
                None,
            );
            let full_argument_types = argument_types
                .iter()
                .flatten()
                .cloned()
                .chain(std::iter::once(ambient.gate_payload().clone()))
                .collect::<Vec<_>>();
            let selection = typing.select_class_member_call(reference, &full_argument_types, None);
            panic!(
                "expected generic reduce body to lower, found {:?}; argument_infos={argument_infos:?}; argument_types={argument_types:?}; ambient={ambient:?}; full_argument_types={full_argument_types:?}; selection={selection:?}; plan={plan:?}; dispatch={:?}",
                length.outcome,
                resolve_class_member_dispatch(
                    module,
                    reference,
                    &argument_types.iter().flatten().cloned().collect::<Vec<_>>(),
                    None
                )
            );
        }
    }

    #[test]
    fn elaborates_reduce_pipe_bodies_with_generic_record_accumulators() {
        let lowered = lower_text(
            "general-expr-generic-reduce-record-acc.aivi",
            r#"
type TakeAcc A = {
    n: Int,
    items: List A
}

fun takeStep:(TakeAcc A) acc:(TakeAcc A) item:A =>
    acc.n > 0
     T|> { n: acc.n - 1, items: append acc.items [item] }
     F|> acc

fun take:(List A) n:Int xs:(List A) =>
    xs
     |> reduce takeStep { n, items: [] }
     |> .items
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic reduce record example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let take = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("take"))
            .expect("expected take elaboration");
        if !matches!(take.outcome, GeneralExprOutcome::Lowered(_)) {
            let module = lowered.module();
            let Item::Function(function) = &module.items()[take.owner] else {
                panic!("take should be a function");
            };
            let mut typing = GateTypeContext::new(module);
            let mut env = GateExprEnv::default();
            for parameter in &function.parameters {
                let annotation = parameter.annotation.expect("test parameters are annotated");
                env.locals.insert(
                    parameter.binding,
                    typing
                        .lower_open_annotation(annotation)
                        .expect("test parameter types should lower"),
                );
            }
            let ExprKind::Pipe(pipe) = &module.exprs()[function.body].kind else {
                panic!("take body should be a pipe expression");
            };
            let reduce_expr = match &pipe.stages.first().kind {
                crate::PipeStageKind::Transform { expr } => *expr,
                other => panic!("expected transform stage, found {other:?}"),
            };
            let ExprKind::Apply { callee, arguments } = module.exprs()[reduce_expr].kind.clone()
            else {
                panic!("take reduce stage should be an apply expression");
            };
            let ExprKind::Name(reference) = &module.exprs()[callee].kind else {
                panic!("take reduce callee should be a name");
            };
            let argument_infos = arguments
                .iter()
                .map(|argument| (*argument, typing.infer_expr(*argument, &env, None)))
                .collect::<Vec<_>>();
            let argument_types = argument_infos
                .iter()
                .map(|(_, info)| info.actual_gate_type().or(info.ty.clone()))
                .collect::<Vec<_>>();
            let ambient = env
                .locals
                .get(
                    &function
                        .parameters
                        .last()
                        .expect("take has xs parameter")
                        .binding,
                )
                .expect("take xs parameter should be in env")
                .clone();
            let plan = typing.match_pipe_function_signature(
                reduce_expr,
                &env,
                ambient.gate_payload(),
                None,
            );
            let full_argument_types = argument_types
                .iter()
                .flatten()
                .cloned()
                .chain(std::iter::once(ambient.gate_payload().clone()))
                .collect::<Vec<_>>();
            let selection = typing.select_class_member_call(reference, &full_argument_types, None);
            panic!(
                "expected generic reduce record body to lower, found {:?}; argument_infos={argument_infos:?}; argument_types={argument_types:?}; ambient={ambient:?}; full_argument_types={full_argument_types:?}; selection={selection:?}; plan={plan:?}",
                take.outcome
            );
        }
    }

    #[test]
    fn elaborates_reduce_pipe_bodies_with_generic_option_initializers() {
        let lowered = lower_text(
            "general-expr-generic-reduce-option-init.aivi",
            r#"
fun keepFirst:(Option A) found:(Option A) item:A =>
    found
     T|> found
     F|> Some item

fun head:(Option A) items:(List A) =>
    items
     |> reduce keepFirst None
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic reduce option-init example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let head = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("head"))
            .expect("expected head elaboration");
        if !matches!(head.outcome, GeneralExprOutcome::Lowered(_)) {
            let module = lowered.module();
            let Item::Function(function) = &module.items()[head.owner] else {
                panic!("head should be a function");
            };
            let mut typing = GateTypeContext::new(module);
            let mut env = GateExprEnv::default();
            for parameter in &function.parameters {
                let annotation = parameter.annotation.expect("test parameters are annotated");
                env.locals.insert(
                    parameter.binding,
                    typing
                        .lower_open_annotation(annotation)
                        .expect("test parameter types should lower"),
                );
            }
            let ExprKind::Pipe(pipe) = &module.exprs()[function.body].kind else {
                panic!("head body should be a pipe expression");
            };
            let reduce_expr = match &pipe.stages.first().kind {
                crate::PipeStageKind::Transform { expr } => *expr,
                other => panic!("expected transform stage, found {other:?}"),
            };
            let ExprKind::Apply { callee, arguments } = module.exprs()[reduce_expr].kind.clone()
            else {
                panic!("head reduce stage should be an apply expression");
            };
            let ExprKind::Name(reference) = &module.exprs()[callee].kind else {
                panic!("head reduce callee should be a name");
            };
            let argument_infos = arguments
                .iter()
                .map(|argument| (*argument, typing.infer_expr(*argument, &env, None)))
                .collect::<Vec<_>>();
            let argument_types = argument_infos
                .iter()
                .map(|(_, info)| info.actual_gate_type().or(info.ty.clone()))
                .collect::<Vec<_>>();
            let ambient = env
                .locals
                .values()
                .next()
                .expect("head should have one parameter")
                .clone();
            let expected = typing
                .lower_open_annotation(function.annotation.expect("head is annotated"))
                .expect("head result type should lower");
            let plan = typing.match_pipe_function_signature(
                reduce_expr,
                &env,
                ambient.gate_payload(),
                Some(&expected),
            );
            let full_argument_types = argument_types
                .iter()
                .flatten()
                .cloned()
                .chain(std::iter::once(ambient.gate_payload().clone()))
                .collect::<Vec<_>>();
            let selection =
                typing.select_class_member_call(reference, &full_argument_types, Some(&expected));
            panic!(
                "expected generic reduce option-init body to lower, found {:?}; argument_infos={argument_infos:?}; argument_types={argument_types:?}; ambient={ambient:?}; expected={expected:?}; full_argument_types={full_argument_types:?}; selection={selection:?}; plan={plan:?}",
                head.outcome
            );
        }
    }

    #[test]
    fn elaborates_reduce_pipe_bodies_with_partial_generic_step_functions() {
        let lowered = lower_text(
            "general-expr-generic-reduce-partial-step.aivi",
            r#"
fun anyStep:Bool predicate:(A -> Bool) found:Bool item:A =>
    found
     T|> True
     F|> predicate item

fun any:Bool predicate:(A -> Bool) items:(List A) =>
    items
     |> reduce (anyStep predicate) False
"#,
        );
        assert!(
            !lowered.has_errors(),
            "generic reduce partial-step example should lower to HIR: {:?}",
            lowered.diagnostics()
        );

        let report = elaborate_general_expressions(lowered.module());
        let any = report
            .items()
            .iter()
            .find(|item| item_name(lowered.module(), item.owner) == Some("any"))
            .expect("expected any elaboration");
        if !matches!(any.outcome, GeneralExprOutcome::Lowered(_)) {
            let module = lowered.module();
            let Item::Function(function) = &module.items()[any.owner] else {
                panic!("any should be a function");
            };
            let mut typing = GateTypeContext::new(module);
            let mut env = GateExprEnv::default();
            for parameter in &function.parameters {
                let annotation = parameter.annotation.expect("test parameters are annotated");
                env.locals.insert(
                    parameter.binding,
                    typing
                        .lower_open_annotation(annotation)
                        .expect("test parameter types should lower"),
                );
            }
            let ExprKind::Pipe(pipe) = &module.exprs()[function.body].kind else {
                panic!("any body should be a pipe expression");
            };
            let reduce_expr = match &pipe.stages.first().kind {
                crate::PipeStageKind::Transform { expr } => *expr,
                other => panic!("expected transform stage, found {other:?}"),
            };
            let ExprKind::Apply { callee, arguments } = module.exprs()[reduce_expr].kind.clone()
            else {
                panic!("any reduce stage should be an apply expression");
            };
            let ExprKind::Name(reference) = &module.exprs()[callee].kind else {
                panic!("any reduce callee should be a name");
            };
            let argument_infos = arguments
                .iter()
                .map(|argument| (*argument, typing.infer_expr(*argument, &env, None)))
                .collect::<Vec<_>>();
            let argument_types = argument_infos
                .iter()
                .map(|(_, info)| info.actual_gate_type().or(info.ty.clone()))
                .collect::<Vec<_>>();
            let expected = typing
                .lower_open_annotation(function.annotation.expect("any is annotated"))
                .expect("any result type should lower");
            let ambient = env
                .locals
                .get(
                    &function
                        .parameters
                        .last()
                        .expect("any has items parameter")
                        .binding,
                )
                .expect("any items parameter should be in env")
                .clone();
            let plan = typing.match_pipe_function_signature(
                reduce_expr,
                &env,
                ambient.gate_payload(),
                Some(&expected),
            );
            let full_argument_types = argument_types
                .iter()
                .flatten()
                .cloned()
                .chain(std::iter::once(ambient.gate_payload().clone()))
                .collect::<Vec<_>>();
            let selection =
                typing.select_class_member_call(reference, &full_argument_types, Some(&expected));
            panic!(
                "expected generic reduce partial-step body to lower, found {:?}; argument_infos={argument_infos:?}; argument_types={argument_types:?}; ambient={ambient:?}; expected={expected:?}; full_argument_types={full_argument_types:?}; selection={selection:?}; plan={plan:?}",
                any.outcome
            );
        }
    }
}
