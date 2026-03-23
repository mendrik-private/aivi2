use std::{collections::HashMap, fmt};

use aivi_base::SourceSpan;
use aivi_typing::GatePlanner;

use crate::{
    gate_elaboration::{GateElaborationBlocker, GateRuntimeMapEntry},
    typecheck::expression_matches,
    validate::{truthy_falsy_pair_stages, GateExprEnv, GateIssue, GateType, GateTypeContext},
    BindingId, BuiltinTerm, ExprId, ExprKind, FunctionItem, FunctionParameter, GateRuntimeCaseArm,
    GateRuntimeExpr, GateRuntimeExprKind, GateRuntimePipeExpr, GateRuntimePipeStage,
    GateRuntimePipeStageKind, GateRuntimeProjectionBase, GateRuntimeRecordField,
    GateRuntimeReference, GateRuntimeTextLiteral, GateRuntimeTextSegment,
    GateRuntimeTruthyFalsyBranch, GateRuntimeUnsupportedKind, GateRuntimeUnsupportedPipeStageKind,
    Item, ItemId, Module, PipeExpr, PipeStageKind, ProjectionBase, ResolutionState, TermReference,
    TermResolution, TypeItemBody, ValueItem,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeneralExprElaborationReport {
    items: Vec<GeneralExprItemElaboration>,
}

impl GeneralExprElaborationReport {
    pub fn new(items: Vec<GeneralExprItemElaboration>) -> Self {
        Self { items }
    }

    pub fn items(&self) -> &[GeneralExprItemElaboration] {
        &self.items
    }

    pub fn into_items(self) -> Vec<GeneralExprItemElaboration> {
        self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
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
pub struct GeneralExprParameter {
    pub binding: BindingId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub ty: GateType,
}

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
        for (item_id, item) in self.module.items().iter() {
            match item {
                Item::Value(value) => items.push(self.elaborate_value(item_id, value)),
                Item::Function(function) => items.push(self.elaborate_function(item_id, function)),
                Item::Type(_)
                | Item::Signal(_)
                | Item::Class(_)
                | Item::Domain(_)
                | Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => {}
            }
        }
        GeneralExprElaborationReport::new(items)
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
            .and_then(|annotation| self.typing.lower_annotation(annotation));
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
            let Some(ty) = self.typing.lower_annotation(annotation) else {
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
                self.runtime_reference_for_name(expr.span, &reference)?,
            ),
            ExprKind::Integer(literal) => GateRuntimeExprKind::Integer(literal),
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
                    crate::BinaryOperator::Add | crate::BinaryOperator::Subtract => {
                        Some(ty.clone())
                    }
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
            ExprKind::Pipe(pipe) => {
                GateRuntimeExprKind::Pipe(self.lower_pipe_expr(&pipe, env, ambient, Some(&ty))?)
            }
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
            .unwrap_or_else(|| self.arrow_type(argument_types, result_ty.clone()));
        let lowered_callee = self.lower_expr(callee, env, ambient, Some(&callee_expected))?;
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
                        input_subject: current.clone(),
                        result_subject: result_subject.clone(),
                        kind: GateRuntimePipeStageKind::Transform { expr: body },
                    });
                    current = result_subject;
                    stage_index += 1;
                }
                PipeStageKind::Tap { expr } => {
                    let body =
                        self.lower_body_expr(*expr, env, Some(current.gate_payload()), None)?;
                    lowered.push(GateRuntimePipeStage {
                        span: stage.span,
                        input_subject: current.clone(),
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
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::FanIn,
                        ),
                    }]);
                }
                PipeStageKind::RecurStart { .. } => {
                    return Err(vec![GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span: stage.span,
                        kind: GateRuntimeUnsupportedKind::PipeStage(
                            GateRuntimeUnsupportedPipeStageKind::RecurStart,
                        ),
                    }]);
                }
                PipeStageKind::RecurStep { .. } => {
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
        if subject.is_signal() {
            return Err(vec![GeneralExprBlocker::UnsupportedSignalCase {
                span: stages.first().map(|stage| stage.span).unwrap_or_default(),
                subject: subject.clone(),
            }]);
        }

        let mut arms = Vec::with_capacity(stages.len());
        let mut result_subject = None::<GateType>;
        let mut blockers = Vec::new();
        for stage in stages {
            let PipeStageKind::Case { pattern, body } = &stage.kind else {
                continue;
            };
            let branch_env = self.case_branch_env(env, *pattern, subject);
            let lowered_body =
                match self.lower_body_expr(*body, &branch_env, Some(subject), expected) {
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
        Ok(GateRuntimePipeStage {
            span: join_stage_spans(stages),
            input_subject: subject.clone(),
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
        let result_subject = self
            .typing
            .infer_truthy_falsy_pair(pair, env, subject)
            .ok_or_else(|| {
                vec![GeneralExprBlocker::UnknownExprType {
                    span: join_spans(pair.truthy_stage.span, pair.falsy_stage.span),
                }]
            })?;
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
        Ok(GateRuntimePipeStage {
            span: join_spans(pair.truthy_stage.span, pair.falsy_stage.span),
            input_subject: subject.clone(),
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
                self.lower_single_parameter_function_pipe_body(
                    expr_id,
                    ambient.expect("checked above"),
                    expected,
                )?
            }
            Err(blockers) => return Err(blockers),
        };
        let Some(ambient) = ambient else {
            return Ok(lowered);
        };
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

    fn lower_single_parameter_function_pipe_body(
        &mut self,
        expr_id: ExprId,
        ambient: &GateType,
        expected: Option<&GateType>,
    ) -> Result<GateRuntimeExpr, Vec<GeneralExprBlocker>> {
        let expr = self.module.exprs()[expr_id].clone();
        let ExprKind::Name(reference) = expr.kind else {
            return Err(vec![GeneralExprBlocker::UnknownExprType {
                span: expr.span,
            }]);
        };
        let ResolutionState::Resolved(TermResolution::Item(item_id)) =
            reference.resolution.as_ref()
        else {
            return Err(vec![GeneralExprBlocker::UnknownExprType {
                span: expr.span,
            }]);
        };
        let Item::Function(function) = &self.module.items()[*item_id] else {
            return Err(vec![GeneralExprBlocker::UnknownExprType {
                span: expr.span,
            }]);
        };
        if function.parameters.len() != 1 {
            return Err(vec![GeneralExprBlocker::UnknownExprType {
                span: expr.span,
            }]);
        }
        let parameter = function
            .parameters
            .first()
            .expect("checked single-parameter function above");
        if let Some(annotation) = parameter.annotation {
            let parameter_ty = self
                .typing
                .lower_annotation(annotation)
                .ok_or_else(|| vec![GeneralExprBlocker::UnknownExprType { span: expr.span }])?;
            if !parameter_ty.same_shape(ambient) {
                return Err(vec![GeneralExprBlocker::UnknownExprType {
                    span: expr.span,
                }]);
            }
        }

        let mut function_env = GateExprEnv::default();
        function_env
            .locals
            .insert(parameter.binding, ambient.clone());
        let body = self.lower_expr(function.body, &function_env, Some(ambient), expected)?;
        let callee = GateRuntimeExpr {
            span: expr.span,
            ty: GateType::Arrow {
                parameter: Box::new(ambient.clone()),
                result: Box::new(body.ty.clone()),
            },
            kind: GateRuntimeExprKind::Reference(GateRuntimeReference::Item(*item_id)),
        };
        Ok(GateRuntimeExpr {
            span: expr.span,
            ty: body.ty.clone(),
            kind: GateRuntimeExprKind::Apply {
                callee: Box::new(callee),
                arguments: vec![GateRuntimeExpr {
                    span: expr.span,
                    ty: ambient.clone(),
                    kind: GateRuntimeExprKind::AmbientSubject,
                }],
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
            if expression_matches(self.module, expr_id, env, expected) {
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
    ) -> Result<GateRuntimeReference, Vec<GeneralExprBlocker>> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Local(binding)) => {
                Ok(GateRuntimeReference::Local(*binding))
            }
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                Ok(GateRuntimeReference::Item(*item_id))
            }
            ResolutionState::Resolved(TermResolution::DomainMember(resolution)) => self
                .module
                .domain_member_handle(*resolution)
                .map(GateRuntimeReference::DomainMember)
                .ok_or_else(|| vec![GeneralExprBlocker::UnknownExprType { span }]),
            ResolutionState::Resolved(TermResolution::Builtin(builtin)) => {
                Ok(GateRuntimeReference::Builtin(*builtin))
            }
            ResolutionState::Resolved(TermResolution::Import(_)) => {
                Err(vec![GeneralExprBlocker::UnsupportedImportReference {
                    span,
                }])
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
        span: SourceSpan,
    ) -> Option<GateRuntimeReference> {
        match reference.resolution.as_ref() {
            ResolutionState::Resolved(TermResolution::Builtin(
                BuiltinTerm::Some
                | BuiltinTerm::Ok
                | BuiltinTerm::Err
                | BuiltinTerm::Valid
                | BuiltinTerm::Invalid,
            )) => self.runtime_reference_for_name(span, reference).ok(),
            ResolutionState::Resolved(TermResolution::Item(item_id)) => {
                let Item::Type(item) = &self.module.items()[*item_id] else {
                    return None;
                };
                let TypeItemBody::Sum(variants) = &item.body else {
                    return None;
                };
                let variant_name = reference.path.segments().last().text();
                variants
                    .iter()
                    .any(|variant| variant.name.text() == variant_name)
                    .then(|| GateRuntimeReference::Item(*item_id))
            }
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
                GateIssue::UnsupportedApplicativeClusterMember { span, .. }
                | GateIssue::ApplicativeClusterMismatch { span, .. }
                | GateIssue::InvalidClusterFinalizer { span, .. } => {
                    GeneralExprBlocker::UnsupportedRuntimeExpr {
                        span,
                        kind: GateRuntimeUnsupportedKind::ApplicativeCluster,
                    }
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
            | ResolutionState::Resolved(TermResolution::DomainMember(_))
            | ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(_))
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

    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{
        elaborate_general_expressions, GateRuntimeExprKind, GateRuntimePipeStageKind,
        GeneralExprBlocker, GeneralExprOutcome,
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
            "fun identity:Int #value:Int =>\n\
             value\n\
             \n\
             fun duplicate:List Int #values:List Int =>\n\
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
}
