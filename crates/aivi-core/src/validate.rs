use std::{collections::BTreeSet, fmt};

use crate::{
    DecodeProgram, DecodeProgramId, DecodeStep, DecodeStepId, ExprId, Module, PipeId, SourceId,
    StageId, StageKind,
    expr::{ExprKind, Pattern, PatternKind, PipeStageKind, ProjectionBase, Reference, TextSegment},
    module::{GateStage, ItemKind},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    pub fn new(errors: Vec<ValidationError>) -> Self {
        Self { errors }
    }

    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    pub fn into_errors(self) -> Vec<ValidationError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    MissingItemBody {
        item: crate::ItemId,
    },
    UnexpectedItemParameters {
        item: crate::ItemId,
    },
    UnknownItemBody {
        item: crate::ItemId,
        expr: ExprId,
    },
    ItemPipeBackrefMissing {
        item: crate::ItemId,
        pipe: PipeId,
    },
    UnknownPipeOwner {
        pipe: PipeId,
        owner: crate::ItemId,
    },
    UnknownStagePipe {
        stage: StageId,
        pipe: PipeId,
    },
    PipeStageBackrefMissing {
        pipe: PipeId,
        stage: StageId,
    },
    PipeStageOrder {
        pipe: PipeId,
        previous: usize,
        current: usize,
    },
    PipeStageTypeDiscontinuity {
        pipe: PipeId,
        previous: crate::ty::Type,
        current: crate::ty::Type,
    },
    GatePredicateNotBool {
        stage: StageId,
    },
    TruthyFalsyResultMismatch {
        stage: StageId,
    },
    FanoutResultMismatch {
        stage: StageId,
    },
    RecurrenceMissingSteps {
        pipe: PipeId,
    },
    RecurrenceStepInputMismatch {
        pipe: PipeId,
    },
    RecurrenceDoesNotClose {
        pipe: PipeId,
        expected: crate::ty::Type,
        found: crate::ty::Type,
    },
    UnknownSourceOwner {
        source: SourceId,
        owner: crate::ItemId,
    },
    SourceOwnerNotSignal {
        source: SourceId,
        owner: crate::ItemId,
    },
    UnknownSourceDependency {
        source: SourceId,
        dependency: crate::ItemId,
    },
    SourceDependencyNotSignal {
        source: SourceId,
        dependency: crate::ItemId,
    },
    UnknownSourceArgumentExpr {
        source: SourceId,
        expr: ExprId,
    },
    UnknownSourceOptionExpr {
        source: SourceId,
        option_name: Box<str>,
        expr: ExprId,
    },
    UnknownDecodeOwner {
        decode: DecodeProgramId,
        owner: crate::ItemId,
    },
    DecodeOwnerNotSignal {
        decode: DecodeProgramId,
        owner: crate::ItemId,
    },
    UnknownDecodeRoot {
        decode: DecodeProgramId,
        root: DecodeStepId,
    },
    UnknownDecodeStep {
        decode: DecodeProgramId,
        step: DecodeStepId,
    },
    SignalDependencyNotSignal {
        item: crate::ItemId,
        dependency: crate::ItemId,
    },
    UnknownExpr {
        expr: ExprId,
    },
    UnknownItemReference {
        expr: ExprId,
        item: crate::ItemId,
    },
    OptionSomePayloadMismatch {
        expr: ExprId,
    },
    OptionNoneTypeMismatch {
        expr: ExprId,
    },
    InlinePipeStageTypeDiscontinuity {
        expr: ExprId,
        stage_index: usize,
        previous: crate::ty::Type,
        current: crate::ty::Type,
    },
    InlinePipeTransformResultMismatch {
        expr: ExprId,
        stage_index: usize,
        expected: crate::ty::Type,
        found: crate::ty::Type,
    },
    InlinePipeGatePredicateNotBool {
        expr: ExprId,
        stage_index: usize,
    },
    InlinePipeGateResultMismatch {
        expr: ExprId,
        stage_index: usize,
        expected: crate::ty::Type,
        found: crate::ty::Type,
    },
    InlinePipeCaseEmpty {
        expr: ExprId,
        stage_index: usize,
    },
    InlinePipeCaseArmResultMismatch {
        expr: ExprId,
        stage_index: usize,
        arm_index: usize,
        expected: crate::ty::Type,
        found: crate::ty::Type,
    },
    InlinePipeTruthyFalsyResultMismatch {
        expr: ExprId,
        stage_index: usize,
        expected: crate::ty::Type,
        truthy: crate::ty::Type,
        falsy: crate::ty::Type,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingItemBody { item } => {
                write!(f, "item {item} is missing its typed-core body expression")
            }
            Self::UnexpectedItemParameters { item } => {
                write!(
                    f,
                    "item {item} carries parameters in a non-function core item"
                )
            }
            Self::UnknownItemBody { item, expr } => {
                write!(f, "item {item} references unknown body expression {expr}")
            }
            Self::ItemPipeBackrefMissing { item, pipe } => {
                write!(f, "item {item} does not list pipe {pipe} in its backrefs")
            }
            Self::UnknownPipeOwner { pipe, owner } => {
                write!(f, "pipe {pipe} references unknown owner item {owner}")
            }
            Self::UnknownStagePipe { stage, pipe } => {
                write!(f, "stage {stage} references unknown pipe {pipe}")
            }
            Self::PipeStageBackrefMissing { pipe, stage } => {
                write!(
                    f,
                    "pipe {pipe} does not list stage {stage} in its stage order"
                )
            }
            Self::PipeStageOrder {
                pipe,
                previous,
                current,
            } => write!(
                f,
                "pipe {pipe} stage order regressed or duplicated ({previous} then {current})"
            ),
            Self::PipeStageTypeDiscontinuity {
                pipe,
                previous,
                current,
            } => write!(
                f,
                "pipe {pipe} changes subject shape between stages: {previous} then {current}"
            ),
            Self::GatePredicateNotBool { stage } => {
                write!(f, "gate stage {stage} does not carry a Bool predicate")
            }
            Self::TruthyFalsyResultMismatch { stage } => {
                write!(
                    f,
                    "truthy/falsy stage {stage} does not preserve one unified result type"
                )
            }
            Self::FanoutResultMismatch { stage } => {
                write!(
                    f,
                    "fanout stage {stage} does not agree with its join/map result type"
                )
            }
            Self::RecurrenceMissingSteps { pipe } => {
                write!(
                    f,
                    "recurrence on pipe {pipe} must contain at least one step"
                )
            }
            Self::RecurrenceStepInputMismatch { pipe } => {
                write!(
                    f,
                    "recurrence step chain on pipe {pipe} is not type-continuous"
                )
            }
            Self::RecurrenceDoesNotClose {
                pipe,
                expected,
                found,
            } => write!(
                f,
                "recurrence on pipe {pipe} does not close: expected {expected}, found {found}"
            ),
            Self::UnknownSourceOwner { source, owner } => {
                write!(f, "source {source} references unknown owner item {owner}")
            }
            Self::SourceOwnerNotSignal { source, owner } => {
                write!(f, "source {source} is attached to non-signal item {owner}")
            }
            Self::UnknownSourceDependency { source, dependency } => {
                write!(
                    f,
                    "source {source} references unknown dependency item {dependency}"
                )
            }
            Self::SourceDependencyNotSignal { source, dependency } => {
                write!(f, "source {source} depends on non-signal item {dependency}")
            }
            Self::UnknownSourceArgumentExpr { source, expr } => {
                write!(
                    f,
                    "source {source} references unknown argument expression {expr}"
                )
            }
            Self::UnknownSourceOptionExpr {
                source,
                option_name,
                expr,
            } => write!(
                f,
                "source {source} option `{option_name}` references unknown expression {expr}"
            ),
            Self::UnknownDecodeOwner { decode, owner } => {
                write!(
                    f,
                    "decode program {decode} references unknown owner item {owner}"
                )
            }
            Self::DecodeOwnerNotSignal { decode, owner } => {
                write!(
                    f,
                    "decode program {decode} is attached to non-signal item {owner}"
                )
            }
            Self::UnknownDecodeRoot { decode, root } => {
                write!(
                    f,
                    "decode program {decode} references unknown root step {root}"
                )
            }
            Self::UnknownDecodeStep { decode, step } => {
                write!(f, "decode program {decode} references unknown step {step}")
            }
            Self::SignalDependencyNotSignal { item, dependency } => {
                write!(
                    f,
                    "signal item {item} depends on non-signal item {dependency}"
                )
            }
            Self::UnknownExpr { expr } => write!(f, "expression {expr} does not exist"),
            Self::UnknownItemReference { expr, item } => {
                write!(f, "expression {expr} references unknown item {item}")
            }
            Self::OptionSomePayloadMismatch { expr } => {
                write!(
                    f,
                    "option-some expression {expr} does not wrap a matching payload type"
                )
            }
            Self::OptionNoneTypeMismatch { expr } => {
                write!(
                    f,
                    "option-none expression {expr} does not have an Option result type"
                )
            }
            Self::InlinePipeStageTypeDiscontinuity {
                expr,
                stage_index,
                previous,
                current,
            } => write!(
                f,
                "inline pipe expression {expr} changes subject shape before stage {stage_index}: {previous} then {current}"
            ),
            Self::InlinePipeTransformResultMismatch {
                expr,
                stage_index,
                expected,
                found,
            } => write!(
                f,
                "inline pipe expression {expr} stage {stage_index} lowers to `{found}` but should produce `{expected}`"
            ),
            Self::InlinePipeGatePredicateNotBool { expr, stage_index } => write!(
                f,
                "inline pipe expression {expr} stage {stage_index} does not carry a Bool predicate"
            ),
            Self::InlinePipeGateResultMismatch {
                expr,
                stage_index,
                expected,
                found,
            } => write!(
                f,
                "inline pipe expression {expr} stage {stage_index} produces `{found}` but gate semantics require `{expected}`"
            ),
            Self::InlinePipeCaseEmpty { expr, stage_index } => write!(
                f,
                "inline pipe expression {expr} stage {stage_index} has no case arms"
            ),
            Self::InlinePipeCaseArmResultMismatch {
                expr,
                stage_index,
                arm_index,
                expected,
                found,
            } => write!(
                f,
                "inline pipe expression {expr} case stage {stage_index} arm {arm_index} produces `{found}` but expected `{expected}`"
            ),
            Self::InlinePipeTruthyFalsyResultMismatch {
                expr,
                stage_index,
                expected,
                truthy,
                falsy,
            } => write!(
                f,
                "inline pipe expression {expr} truthy/falsy stage {stage_index} should produce `{expected}` but branches yield `{truthy}` and `{falsy}`"
            ),
        }
    }
}

pub fn validate_module(module: &Module) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();

    for (item_id, item) in module.items().iter() {
        match item.kind {
            ItemKind::Value => {
                if !item.parameters.is_empty() {
                    errors.push(ValidationError::UnexpectedItemParameters { item: item_id });
                }
                if let Some(body) = item.body {
                    if !module.exprs().contains(body) {
                        errors.push(ValidationError::UnknownItemBody {
                            item: item_id,
                            expr: body,
                        });
                    }
                }
            }
            ItemKind::Function => {
                if let Some(body) = item.body {
                    if !module.exprs().contains(body) {
                        errors.push(ValidationError::UnknownItemBody {
                            item: item_id,
                            expr: body,
                        });
                    }
                }
            }
            ItemKind::Signal(_) | ItemKind::Instance => {
                if !item.parameters.is_empty() {
                    errors.push(ValidationError::UnexpectedItemParameters { item: item_id });
                }
            }
        }
        for pipe in &item.pipes {
            if !module.pipes().contains(*pipe) {
                errors.push(ValidationError::ItemPipeBackrefMissing {
                    item: item_id,
                    pipe: *pipe,
                });
            }
        }
        if let ItemKind::Signal(info) = &item.kind {
            for dependency in &info.dependencies {
                match module.items().get(*dependency) {
                    Some(dependency_item)
                        if matches!(dependency_item.kind, ItemKind::Signal(_)) => {}
                    Some(_) | None => errors.push(ValidationError::SignalDependencyNotSignal {
                        item: item_id,
                        dependency: *dependency,
                    }),
                }
            }
        }
    }

    for (pipe_id, pipe) in module.pipes().iter() {
        if !module.items().contains(pipe.owner) {
            errors.push(ValidationError::UnknownPipeOwner {
                pipe: pipe_id,
                owner: pipe.owner,
            });
            continue;
        }
        if !module.items()[pipe.owner].pipes.contains(&pipe_id) {
            errors.push(ValidationError::ItemPipeBackrefMissing {
                item: pipe.owner,
                pipe: pipe_id,
            });
        }
        let mut previous_index = None;
        let mut previous_result: Option<crate::ty::Type> = None;
        for stage_id in &pipe.stages {
            let Some(stage) = module.stages().get(*stage_id) else {
                errors.push(ValidationError::PipeStageBackrefMissing {
                    pipe: pipe_id,
                    stage: *stage_id,
                });
                continue;
            };
            if stage.pipe != pipe_id {
                errors.push(ValidationError::UnknownStagePipe {
                    stage: *stage_id,
                    pipe: stage.pipe,
                });
            }
            if let Some(previous) = previous_index {
                if stage.index <= previous {
                    errors.push(ValidationError::PipeStageOrder {
                        pipe: pipe_id,
                        previous,
                        current: stage.index,
                    });
                }
            }
            if let Some(previous) = previous_result.as_ref() {
                if previous != &stage.input_subject {
                    errors.push(ValidationError::PipeStageTypeDiscontinuity {
                        pipe: pipe_id,
                        previous: previous.clone(),
                        current: stage.input_subject.clone(),
                    });
                }
            }
            previous_index = Some(stage.index);
            previous_result = Some(stage.result_subject.clone());
            validate_stage(module, *stage_id, stage, &mut errors);
        }
        validate_recurrence(module, pipe_id, pipe, &mut errors);
    }

    for (source_id, source) in module.sources().iter() {
        let Some(owner) = module.items().get(source.owner) else {
            errors.push(ValidationError::UnknownSourceOwner {
                source: source_id,
                owner: source.owner,
            });
            continue;
        };
        if !matches!(owner.kind, ItemKind::Signal(_)) {
            errors.push(ValidationError::SourceOwnerNotSignal {
                source: source_id,
                owner: source.owner,
            });
        }
        for dependency in &source.reconfiguration_dependencies {
            match module.items().get(*dependency) {
                Some(item) if matches!(item.kind, ItemKind::Signal(_)) => {}
                Some(_) => errors.push(ValidationError::SourceDependencyNotSignal {
                    source: source_id,
                    dependency: *dependency,
                }),
                None => errors.push(ValidationError::UnknownSourceDependency {
                    source: source_id,
                    dependency: *dependency,
                }),
            }
        }
        for argument in &source.arguments {
            if !module.exprs().contains(argument.runtime_expr) {
                errors.push(ValidationError::UnknownSourceArgumentExpr {
                    source: source_id,
                    expr: argument.runtime_expr,
                });
            }
        }
        for option in &source.options {
            if !module.exprs().contains(option.runtime_expr) {
                errors.push(ValidationError::UnknownSourceOptionExpr {
                    source: source_id,
                    option_name: option.option_name.clone(),
                    expr: option.runtime_expr,
                });
            }
        }
    }

    for (decode_id, decode) in module.decode_programs().iter() {
        let Some(owner) = module.items().get(decode.owner) else {
            errors.push(ValidationError::UnknownDecodeOwner {
                decode: decode_id,
                owner: decode.owner,
            });
            continue;
        };
        if !matches!(owner.kind, ItemKind::Signal(_)) {
            errors.push(ValidationError::DecodeOwnerNotSignal {
                decode: decode_id,
                owner: decode.owner,
            });
        }
        validate_decode_program(decode_id, decode, &mut errors);
    }

    let mut visited = BTreeSet::new();
    let mut work = module.exprs().iter().map(|(id, _)| id).collect::<Vec<_>>();
    while let Some(expr_id) = work.pop() {
        if !visited.insert(expr_id) {
            continue;
        }
        let Some(expr) = module.exprs().get(expr_id) else {
            errors.push(ValidationError::UnknownExpr { expr: expr_id });
            continue;
        };
        match &expr.kind {
            ExprKind::AmbientSubject
            | ExprKind::OptionNone
            | ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::SuffixedInteger(_)
            | ExprKind::Reference(Reference::Local(_))
            | ExprKind::Reference(Reference::BuiltinClassMember(_))
            | ExprKind::Reference(Reference::Builtin(_))
            | ExprKind::Reference(Reference::IntrinsicValue(_))
            | ExprKind::Reference(Reference::DomainMember(_))
            | ExprKind::Reference(Reference::SumConstructor(_))
            | ExprKind::Reference(Reference::HirItem(_)) => {}
            ExprKind::Reference(Reference::Item(item)) => {
                if !module.items().contains(*item) {
                    errors.push(ValidationError::UnknownItemReference {
                        expr: expr_id,
                        item: *item,
                    });
                }
            }
            ExprKind::OptionSome { payload } => {
                if !module.exprs().contains(*payload) {
                    errors.push(ValidationError::UnknownExpr { expr: *payload });
                } else {
                    work.push(*payload);
                    match &expr.ty {
                        crate::ty::Type::Option(inner)
                            if inner.as_ref() == &module.exprs()[*payload].ty => {}
                        _ => errors
                            .push(ValidationError::OptionSomePayloadMismatch { expr: expr_id }),
                    }
                }
            }
            ExprKind::Text(text) => {
                for segment in &text.segments {
                    if let TextSegment::Interpolation { expr, .. } = segment {
                        if module.exprs().contains(*expr) {
                            work.push(*expr);
                        } else {
                            errors.push(ValidationError::UnknownExpr { expr: *expr });
                        }
                    }
                }
            }
            ExprKind::Tuple(elements) | ExprKind::List(elements) | ExprKind::Set(elements) => {
                push_exprs(module, elements.as_slice(), &mut work, &mut errors)
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    push_expr(module, entry.key, &mut work, &mut errors);
                    push_expr(module, entry.value, &mut work, &mut errors);
                }
            }
            ExprKind::Record(fields) => {
                for field in fields {
                    push_expr(module, field.value, &mut work, &mut errors);
                }
            }
            ExprKind::Projection { base, .. } => {
                if let ProjectionBase::Expr(base) = base {
                    push_expr(module, *base, &mut work, &mut errors);
                }
            }
            ExprKind::Apply { callee, arguments } => {
                push_expr(module, *callee, &mut work, &mut errors);
                push_exprs(module, arguments.as_slice(), &mut work, &mut errors);
            }
            ExprKind::Unary { expr, .. } => push_expr(module, *expr, &mut work, &mut errors),
            ExprKind::Binary { left, right, .. } => {
                push_expr(module, *left, &mut work, &mut errors);
                push_expr(module, *right, &mut work, &mut errors);
            }
            ExprKind::Pipe(pipe) => {
                push_expr(module, pipe.head, &mut work, &mut errors);
                let mut previous = module.exprs()[pipe.head].ty.clone();
                for (stage_index, stage) in pipe.stages.iter().enumerate() {
                    if !inline_pipe_stage_input_matches(&previous, &stage.input_subject) {
                        errors.push(ValidationError::InlinePipeStageTypeDiscontinuity {
                            expr: expr_id,
                            stage_index,
                            previous: previous.clone(),
                            current: stage.input_subject.clone(),
                        });
                    }
                    match &stage.kind {
                        PipeStageKind::Transform { expr, .. } | PipeStageKind::Tap { expr } => {
                            push_expr(module, *expr, &mut work, &mut errors);
                            if let PipeStageKind::Transform { .. } = &stage.kind {
                                let expected = inline_pipe_body_result_type(
                                    &stage.input_subject,
                                    &stage.result_subject,
                                );
                                if module.exprs()[*expr].ty != expected {
                                    errors.push(
                                        ValidationError::InlinePipeTransformResultMismatch {
                                            expr: expr_id,
                                            stage_index,
                                            expected,
                                            found: module.exprs()[*expr].ty.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        PipeStageKind::Debug { .. } => {}
                        PipeStageKind::Gate { predicate, .. } => {
                            push_expr(module, *predicate, &mut work, &mut errors);
                            if !module.exprs()[*predicate].ty.is_bool() {
                                errors.push(ValidationError::InlinePipeGatePredicateNotBool {
                                    expr: expr_id,
                                    stage_index,
                                });
                            }
                            let expected = gate_result_type(&stage.input_subject);
                            if stage.result_subject != expected {
                                errors.push(ValidationError::InlinePipeGateResultMismatch {
                                    expr: expr_id,
                                    stage_index,
                                    expected,
                                    found: stage.result_subject.clone(),
                                });
                            }
                        }
                        PipeStageKind::Case { arms } => {
                            if arms.is_empty() {
                                errors.push(ValidationError::InlinePipeCaseEmpty {
                                    expr: expr_id,
                                    stage_index,
                                });
                            }
                            let expected = case_arm_result_type(&stage.result_subject);
                            for (arm_index, arm) in arms.iter().enumerate() {
                                push_expr(module, arm.body, &mut work, &mut errors);
                                validate_pattern(&arm.pattern, module, &mut work, &mut errors);
                                if module.exprs()[arm.body].ty != expected {
                                    errors.push(ValidationError::InlinePipeCaseArmResultMismatch {
                                        expr: expr_id,
                                        stage_index,
                                        arm_index,
                                        expected: expected.clone(),
                                        found: module.exprs()[arm.body].ty.clone(),
                                    });
                                }
                            }
                        }
                        PipeStageKind::TruthyFalsy(stage_pair) => {
                            push_expr(module, stage_pair.truthy.body, &mut work, &mut errors);
                            push_expr(module, stage_pair.falsy.body, &mut work, &mut errors);
                            let expected = truthy_falsy_result_type(
                                &stage.input_subject,
                                &stage.result_subject,
                            );
                            if stage_pair.truthy.result_type != expected
                                || stage_pair.falsy.result_type != expected
                                || module.exprs()[stage_pair.truthy.body].ty
                                    != stage_pair.truthy.result_type
                                || module.exprs()[stage_pair.falsy.body].ty
                                    != stage_pair.falsy.result_type
                            {
                                errors.push(ValidationError::InlinePipeTruthyFalsyResultMismatch {
                                    expr: expr_id,
                                    stage_index,
                                    expected,
                                    truthy: stage_pair.truthy.result_type.clone(),
                                    falsy: stage_pair.falsy.result_type.clone(),
                                });
                            }
                        }
                    }
                    previous = stage.result_subject.clone();
                }
            }
        }
        if matches!(expr.kind, ExprKind::OptionNone)
            && !matches!(expr.ty, crate::ty::Type::Option(_))
        {
            errors.push(ValidationError::OptionNoneTypeMismatch { expr: expr_id });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors::new(errors))
    }
}

fn validate_stage(
    module: &Module,
    stage_id: StageId,
    stage: &crate::Stage,
    errors: &mut Vec<ValidationError>,
) {
    match &stage.kind {
        StageKind::Gate(GateStage::Ordinary {
            when_true,
            when_false,
        }) => {
            if module.exprs()[*when_true].ty != stage.result_subject
                || module.exprs()[*when_false].ty != stage.result_subject
            {
                errors.push(ValidationError::FanoutResultMismatch { stage: stage_id });
            }
        }
        StageKind::Gate(GateStage::SignalFilter { predicate, .. }) => {
            if !module.exprs()[*predicate].ty.is_bool() {
                errors.push(ValidationError::GatePredicateNotBool { stage: stage_id });
            }
        }
        StageKind::TruthyFalsy(pair) => {
            let expected = truthy_falsy_result_type(&stage.input_subject, &stage.result_subject);
            if pair.truthy.result_type != pair.falsy.result_type
                || pair.truthy.result_type != expected
            {
                errors.push(ValidationError::TruthyFalsyResultMismatch { stage: stage_id });
            }
        }
        StageKind::Fanout(fanout) => {
            if !module.exprs().contains(fanout.runtime_map) {
                errors.push(ValidationError::UnknownExpr {
                    expr: fanout.runtime_map,
                });
            }
            for filter in &fanout.filters {
                if !module.exprs().contains(filter.runtime_predicate) {
                    errors.push(ValidationError::UnknownExpr {
                        expr: filter.runtime_predicate,
                    });
                } else if !module.exprs()[filter.runtime_predicate].ty.is_bool() {
                    errors.push(ValidationError::GatePredicateNotBool { stage: stage_id });
                }
            }
            if let Some(join) = &fanout.join {
                if !module.exprs().contains(join.runtime_expr) {
                    errors.push(ValidationError::UnknownExpr {
                        expr: join.runtime_expr,
                    });
                }
            }
            let expected = fanout
                .join
                .as_ref()
                .map(|join| &join.result_type)
                .unwrap_or(&fanout.mapped_collection_type);
            if expected != &stage.result_subject {
                errors.push(ValidationError::FanoutResultMismatch { stage: stage_id });
            }
        }
    }
}

fn validate_recurrence(
    module: &Module,
    pipe_id: PipeId,
    pipe: &crate::Pipe,
    errors: &mut Vec<ValidationError>,
) {
    let Some(recurrence) = &pipe.recurrence else {
        return;
    };
    let mut current = recurrence.start.result_subject.clone();
    for step in &recurrence.steps {
        if step.input_subject != current {
            errors.push(ValidationError::RecurrenceStepInputMismatch { pipe: pipe_id });
            return;
        }
        if !module.exprs().contains(step.runtime_expr) {
            errors.push(ValidationError::UnknownExpr {
                expr: step.runtime_expr,
            });
        }
        current = step.result_subject.clone();
    }
    if current != recurrence.start.result_subject {
        errors.push(ValidationError::RecurrenceDoesNotClose {
            pipe: pipe_id,
            expected: recurrence.start.result_subject.clone(),
            found: current,
        });
    }
    if !module.exprs().contains(recurrence.start.runtime_expr) {
        errors.push(ValidationError::UnknownExpr {
            expr: recurrence.start.runtime_expr,
        });
    }
    for guard in &recurrence.guards {
        if !module.exprs().contains(guard.runtime_predicate) {
            errors.push(ValidationError::UnknownExpr {
                expr: guard.runtime_predicate,
            });
        }
    }
    if let Some(witness) = &recurrence.non_source_wakeup {
        if !module.exprs().contains(witness.runtime_witness) {
            errors.push(ValidationError::UnknownExpr {
                expr: witness.runtime_witness,
            });
        }
    }
}

fn validate_decode_program(
    decode_id: DecodeProgramId,
    decode: &DecodeProgram,
    errors: &mut Vec<ValidationError>,
) {
    if !decode.steps().contains(decode.root) {
        errors.push(ValidationError::UnknownDecodeRoot {
            decode: decode_id,
            root: decode.root,
        });
    }
    for (_, step) in decode.steps().iter() {
        let referenced = match step {
            DecodeStep::Scalar { .. } => Vec::new(),
            DecodeStep::Tuple { elements } => elements.clone(),
            DecodeStep::Record { fields, .. } => fields.iter().map(|field| field.step).collect(),
            DecodeStep::Sum { variants, .. } => variants
                .iter()
                .filter_map(|variant| variant.payload)
                .collect(),
            DecodeStep::Domain { carrier, .. } => vec![*carrier],
            DecodeStep::List { element } | DecodeStep::Option { element } => vec![*element],
            DecodeStep::Result { error, value } | DecodeStep::Validation { error, value } => {
                vec![*error, *value]
            }
        };
        for referenced in referenced {
            if !decode.steps().contains(referenced) {
                errors.push(ValidationError::UnknownDecodeStep {
                    decode: decode_id,
                    step: referenced,
                });
            }
        }
    }
}

fn push_expr(
    module: &Module,
    expr: ExprId,
    work: &mut Vec<ExprId>,
    errors: &mut Vec<ValidationError>,
) {
    if module.exprs().contains(expr) {
        work.push(expr);
    } else {
        errors.push(ValidationError::UnknownExpr { expr });
    }
}

fn push_exprs(
    module: &Module,
    exprs: &[ExprId],
    work: &mut Vec<ExprId>,
    errors: &mut Vec<ValidationError>,
) {
    for expr in exprs {
        push_expr(module, *expr, work, errors);
    }
}

fn validate_pattern(
    pattern: &Pattern,
    _module: &Module,
    _work: &mut Vec<ExprId>,
    _errors: &mut Vec<ValidationError>,
) {
    match &pattern.kind {
        PatternKind::Wildcard
        | PatternKind::Binding(_)
        | PatternKind::Integer(_)
        | PatternKind::Text(_) => {}
        PatternKind::Tuple(elements) => {
            for element in elements {
                validate_pattern(element, _module, _work, _errors);
            }
        }
        PatternKind::List { elements, rest } => {
            for element in elements {
                validate_pattern(element, _module, _work, _errors);
            }
            if let Some(rest) = rest {
                validate_pattern(rest, _module, _work, _errors);
            }
        }
        PatternKind::Record(fields) => {
            for field in fields {
                validate_pattern(&field.pattern, _module, _work, _errors);
            }
        }
        PatternKind::Constructor { arguments, .. } => {
            for argument in arguments {
                validate_pattern(argument, _module, _work, _errors);
            }
        }
    }
}

fn inline_pipe_body_result_type(
    input: &crate::ty::Type,
    result: &crate::ty::Type,
) -> crate::ty::Type {
    match (input, result) {
        (_, crate::ty::Type::Signal(payload)) => payload.as_ref().clone(),
        _ => result.clone(),
    }
}

fn inline_pipe_stage_input_matches(previous: &crate::ty::Type, current: &crate::ty::Type) -> bool {
    previous == current
        || matches!(
            previous,
            crate::ty::Type::Signal(payload) if payload.as_ref() == current
        )
}

fn gate_result_type(subject: &crate::ty::Type) -> crate::ty::Type {
    match subject {
        crate::ty::Type::Signal(payload) => crate::ty::Type::Signal(payload.clone()),
        other => crate::ty::Type::Option(Box::new(other.clone())),
    }
}

fn truthy_falsy_result_type(input: &crate::ty::Type, result: &crate::ty::Type) -> crate::ty::Type {
    match (input, result) {
        (crate::ty::Type::Signal(_), crate::ty::Type::Signal(payload)) => payload.as_ref().clone(),
        (_, crate::ty::Type::Signal(payload)) => payload.as_ref().clone(),
        _ => result.clone(),
    }
}

fn case_arm_result_type(result: &crate::ty::Type) -> crate::ty::Type {
    match result {
        crate::ty::Type::Signal(payload) => payload.as_ref().clone(),
        _ => result.clone(),
    }
}
