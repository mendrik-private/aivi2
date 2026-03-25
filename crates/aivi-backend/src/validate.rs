use std::{collections::HashMap, fmt};

use crate::{
    CallingConvention, DecodePlanId, DecodeStepId, EnvSlotId, InlineSubjectId, ItemId,
    KernelExprId, KernelId, LayoutId, PipelineId, Program, SourceId,
    kernel::{
        InlinePipePattern, InlinePipePatternKind, InlinePipeRecordPatternField,
        InlinePipeStageKind, KernelExprKind, ParameterRole, ProjectionBase, SubjectRef,
    },
    layout::{LayoutKind, PrimitiveType},
    program::{DecodeStepKind, GateStage, ItemKind, StageKind},
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
    ItemUnknownParameterLayout {
        item: ItemId,
        parameter_index: usize,
        layout: LayoutId,
    },
    ItemUnknownBodyKernel {
        item: ItemId,
        kernel: KernelId,
    },
    ItemBodyOwnerMismatch {
        item: ItemId,
        kernel: KernelId,
        expected_owner: ItemId,
        found_owner: ItemId,
    },
    ItemBodyHasInput {
        item: ItemId,
        kernel: KernelId,
        layout: LayoutId,
    },
    ItemBodyParameterCountMismatch {
        item: ItemId,
        kernel: KernelId,
        expected: usize,
        found: usize,
    },
    ItemBodyParameterLayoutMismatch {
        item: ItemId,
        kernel: KernelId,
        parameter_index: usize,
        expected: LayoutId,
        found: LayoutId,
    },
    ItemPipelineBackrefMissing {
        item: ItemId,
        pipeline: PipelineId,
    },
    UnknownPipelineOwner {
        pipeline: PipelineId,
        owner: ItemId,
    },
    SignalDependencyNotSignal {
        item: ItemId,
        dependency: ItemId,
    },
    LayoutChildMissing {
        layout: LayoutId,
        child: LayoutId,
    },
    UnknownStageLayout {
        pipeline: PipelineId,
        stage_index: usize,
        layout: LayoutId,
    },
    UnknownKernel {
        kernel: KernelId,
    },
    KernelInputMismatch {
        kernel: KernelId,
        expected: Option<LayoutId>,
        found: Option<LayoutId>,
    },
    KernelResultMismatch {
        kernel: KernelId,
        expected: LayoutId,
        found: LayoutId,
    },
    SignalFilterPredicateNotBool {
        kernel: KernelId,
    },
    InlinePipeGatePredicateNotBool {
        kernel: KernelId,
        expr: KernelExprId,
    },
    InlinePipeCaseGuardNotBool {
        kernel: KernelId,
        expr: KernelExprId,
    },
    RecurrenceMissingSteps {
        pipeline: PipelineId,
    },
    RecurrenceStepInputMismatch {
        pipeline: PipelineId,
    },
    RecurrenceDoesNotClose {
        pipeline: PipelineId,
        expected: LayoutId,
        found: LayoutId,
    },
    TruthyFalsyResultMismatch {
        pipeline: PipelineId,
        stage_index: usize,
    },
    FanoutResultMismatch {
        pipeline: PipelineId,
        stage_index: usize,
    },
    UnknownSourceOwner {
        source: SourceId,
        owner: ItemId,
    },
    SourceOwnerNotSignal {
        source: SourceId,
        owner: ItemId,
    },
    SourceDependencyNotSignal {
        source: SourceId,
        dependency: ItemId,
    },
    SourceUnknownArgumentKernel {
        source: SourceId,
        index: usize,
        kernel: KernelId,
    },
    SourceUnknownOptionKernel {
        source: SourceId,
        option_name: Box<str>,
        kernel: KernelId,
    },
    SourceKernelHasInput {
        source: SourceId,
        kernel: KernelId,
        layout: LayoutId,
    },
    SourceKernelOwnerMismatch {
        source: SourceId,
        kernel: KernelId,
        expected_owner: ItemId,
        found_owner: ItemId,
    },
    SourceUnknownDecode {
        source: SourceId,
        decode: DecodePlanId,
    },
    UnknownDecodeOwner {
        decode: DecodePlanId,
        owner: ItemId,
    },
    DecodeOwnerNotSignal {
        decode: DecodePlanId,
        owner: ItemId,
    },
    UnknownDecodeRoot {
        decode: DecodePlanId,
        root: DecodeStepId,
    },
    UnknownDecodeStep {
        decode: DecodePlanId,
        step: DecodeStepId,
    },
    UnknownDecodeLayout {
        decode: DecodePlanId,
        step: DecodeStepId,
        layout: LayoutId,
    },
    KernelConventionMismatch {
        kernel: KernelId,
    },
    KernelUnknownExpr {
        kernel: KernelId,
        expr: KernelExprId,
    },
    KernelUnknownLayout {
        kernel: KernelId,
        layout: LayoutId,
    },
    KernelMissingInputSubject {
        kernel: KernelId,
        expr: KernelExprId,
    },
    KernelUnknownEnvironmentSlot {
        kernel: KernelId,
        expr: KernelExprId,
        slot: EnvSlotId,
    },
    KernelUnknownInlineSubject {
        kernel: KernelId,
        expr: KernelExprId,
        subject: InlineSubjectId,
    },
    KernelUnknownItemRef {
        kernel: KernelId,
        expr: KernelExprId,
        item: ItemId,
    },
    KernelGlobalDependencyMissing {
        kernel: KernelId,
        item: ItemId,
    },
    KernelSubjectLayoutMismatch {
        kernel: KernelId,
        expr: KernelExprId,
        expected: LayoutId,
        found: LayoutId,
    },
    ItemCyclicDependency {
        cycle: Vec<ItemId>,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemUnknownParameterLayout {
                item,
                parameter_index,
                layout,
            } => write!(
                f,
                "item {item} parameter {parameter_index} references unknown layout {layout}"
            ),
            Self::ItemUnknownBodyKernel { item, kernel } => {
                write!(f, "item {item} references unknown body kernel {kernel}")
            }
            Self::ItemBodyOwnerMismatch {
                item,
                kernel,
                expected_owner,
                found_owner,
            } => write!(
                f,
                "item {item} body kernel {kernel} belongs to item {found_owner}, expected {expected_owner}"
            ),
            Self::ItemBodyHasInput {
                item,
                kernel,
                layout,
            } => write!(
                f,
                "item {item} body kernel {kernel} unexpectedly requires input layout {layout}"
            ),
            Self::ItemBodyParameterCountMismatch {
                item,
                kernel,
                expected,
                found,
            } => write!(
                f,
                "item {item} body kernel {kernel} expects {found} environment slot(s), but the item declares {expected} parameter(s)"
            ),
            Self::ItemBodyParameterLayoutMismatch {
                item,
                kernel,
                parameter_index,
                expected,
                found,
            } => write!(
                f,
                "item {item} body kernel {kernel} parameter {parameter_index} changed layout unexpectedly: expected {expected}, found {found}"
            ),
            Self::ItemPipelineBackrefMissing { item, pipeline } => {
                write!(
                    f,
                    "item {item} does not list pipeline {pipeline} in its backrefs"
                )
            }
            Self::UnknownPipelineOwner { pipeline, owner } => {
                write!(
                    f,
                    "pipeline {pipeline} references unknown owner item {owner}"
                )
            }
            Self::SignalDependencyNotSignal { item, dependency } => {
                write!(
                    f,
                    "signal item {item} depends on non-signal item {dependency}"
                )
            }
            Self::LayoutChildMissing { layout, child } => {
                write!(f, "layout {layout} references unknown child layout {child}")
            }
            Self::UnknownStageLayout {
                pipeline,
                stage_index,
                layout,
            } => write!(
                f,
                "pipeline {pipeline} stage {stage_index} references unknown layout {layout}"
            ),
            Self::UnknownKernel { kernel } => write!(f, "kernel {kernel} does not exist"),
            Self::KernelInputMismatch {
                kernel,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} input contract mismatch: expected {:?}, found {:?}",
                expected, found
            ),
            Self::KernelResultMismatch {
                kernel,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} result contract mismatch: expected layout{expected}, found layout{found}"
            ),
            Self::SignalFilterPredicateNotBool { kernel } => {
                write!(
                    f,
                    "signal-filter predicate kernel {kernel} does not return Bool"
                )
            }
            Self::InlinePipeGatePredicateNotBool { kernel, expr } => {
                write!(
                    f,
                    "inline-pipe gate predicate expression {expr} in kernel {kernel} does not return Bool"
                )
            }
            Self::InlinePipeCaseGuardNotBool { kernel, expr } => {
                write!(
                    f,
                    "inline-pipe case guard expression {expr} in kernel {kernel} does not return Bool"
                )
            }
            Self::RecurrenceMissingSteps { pipeline } => {
                write!(
                    f,
                    "recurrence on pipeline {pipeline} must contain at least one step"
                )
            }
            Self::RecurrenceStepInputMismatch { pipeline } => {
                write!(
                    f,
                    "recurrence step chain on pipeline {pipeline} is not layout-continuous"
                )
            }
            Self::RecurrenceDoesNotClose {
                pipeline,
                expected,
                found,
            } => write!(
                f,
                "recurrence on pipeline {pipeline} does not close: expected layout{expected}, found layout{found}"
            ),
            Self::TruthyFalsyResultMismatch {
                pipeline,
                stage_index,
            } => write!(
                f,
                "truthy/falsy stage {stage_index} on pipeline {pipeline} does not preserve one unified result layout"
            ),
            Self::FanoutResultMismatch {
                pipeline,
                stage_index,
            } => write!(
                f,
                "fanout stage {stage_index} on pipeline {pipeline} disagrees with its join/map result layout"
            ),
            Self::UnknownSourceOwner { source, owner } => {
                write!(f, "source {source} references unknown owner item {owner}")
            }
            Self::SourceOwnerNotSignal { source, owner } => {
                write!(f, "source {source} is attached to non-signal item {owner}")
            }
            Self::SourceDependencyNotSignal { source, dependency } => {
                write!(f, "source {source} depends on non-signal item {dependency}")
            }
            Self::SourceUnknownArgumentKernel {
                source,
                index,
                kernel,
            } => write!(
                f,
                "source {source} argument {index} references unknown kernel {kernel}"
            ),
            Self::SourceUnknownOptionKernel {
                source,
                option_name,
                kernel,
            } => write!(
                f,
                "source {source} option `{option_name}` references unknown kernel {kernel}"
            ),
            Self::SourceKernelHasInput {
                source,
                kernel,
                layout,
            } => write!(
                f,
                "source {source} kernel {kernel} unexpectedly requires input layout{layout}"
            ),
            Self::SourceKernelOwnerMismatch {
                source,
                kernel,
                expected_owner,
                found_owner,
            } => write!(
                f,
                "source {source} kernel {kernel} belongs to item{found_owner}, expected item{expected_owner}"
            ),
            Self::SourceUnknownDecode { source, decode } => {
                write!(f, "source {source} references unknown decode plan {decode}")
            }
            Self::UnknownDecodeOwner { decode, owner } => {
                write!(
                    f,
                    "decode plan {decode} references unknown owner item {owner}"
                )
            }
            Self::DecodeOwnerNotSignal { decode, owner } => {
                write!(
                    f,
                    "decode plan {decode} is attached to non-signal item {owner}"
                )
            }
            Self::UnknownDecodeRoot { decode, root } => {
                write!(
                    f,
                    "decode plan {decode} references unknown root step {root}"
                )
            }
            Self::UnknownDecodeStep { decode, step } => {
                write!(f, "decode plan {decode} references unknown step {step}")
            }
            Self::UnknownDecodeLayout {
                decode,
                step,
                layout,
            } => write!(
                f,
                "decode plan {decode} step {step} references unknown layout {layout}"
            ),
            Self::KernelConventionMismatch { kernel } => {
                write!(
                    f,
                    "kernel {kernel} calling convention no longer matches its signature"
                )
            }
            Self::KernelUnknownExpr { kernel, expr } => {
                write!(f, "kernel {kernel} references unknown expression {expr}")
            }
            Self::KernelUnknownLayout { kernel, layout } => {
                write!(f, "kernel {kernel} references unknown layout {layout}")
            }
            Self::KernelMissingInputSubject { kernel, expr } => write!(
                f,
                "kernel {kernel} expression {expr} references a missing input subject"
            ),
            Self::KernelUnknownEnvironmentSlot { kernel, expr, slot } => write!(
                f,
                "kernel {kernel} expression {expr} references unknown environment slot {slot}"
            ),
            Self::KernelUnknownInlineSubject {
                kernel,
                expr,
                subject,
            } => write!(
                f,
                "kernel {kernel} expression {expr} references unknown inline subject {subject}"
            ),
            Self::KernelUnknownItemRef { kernel, expr, item } => write!(
                f,
                "kernel {kernel} expression {expr} references unknown item {item}"
            ),
            Self::KernelGlobalDependencyMissing { kernel, item } => write!(
                f,
                "kernel {kernel} references item {item} without listing it in global dependencies"
            ),
            Self::KernelSubjectLayoutMismatch {
                kernel,
                expr,
                expected,
                found,
            } => write!(
                f,
                "kernel {kernel} expression {expr} expected layout{expected} for its subject, found layout{found}"
            ),
            Self::ItemCyclicDependency { cycle } => {
                let ids: Vec<String> = cycle.iter().map(|id| format!("{id}")).collect();
                write!(f, "circular dependency between items: {}", ids.join(" -> "))
            }
        }
    }
}

pub fn validate_program(program: &Program) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();

    validate_layouts(program, &mut errors);

    for (item_id, item) in program.items().iter() {
        for (parameter_index, layout) in item.parameters.iter().enumerate() {
            if !program.layouts().contains(*layout) {
                errors.push(ValidationError::ItemUnknownParameterLayout {
                    item: item_id,
                    parameter_index,
                    layout: *layout,
                });
            }
        }
        if let Some(kernel_id) = item.body {
            match program.kernels().get(kernel_id) {
                Some(kernel) => {
                    if kernel.origin.item != item_id {
                        errors.push(ValidationError::ItemBodyOwnerMismatch {
                            item: item_id,
                            kernel: kernel_id,
                            expected_owner: item_id,
                            found_owner: kernel.origin.item,
                        });
                    }
                    if let Some(layout) = kernel.input_subject {
                        errors.push(ValidationError::ItemBodyHasInput {
                            item: item_id,
                            kernel: kernel_id,
                            layout,
                        });
                    }
                    if kernel.environment.len() != item.parameters.len() {
                        errors.push(ValidationError::ItemBodyParameterCountMismatch {
                            item: item_id,
                            kernel: kernel_id,
                            expected: item.parameters.len(),
                            found: kernel.environment.len(),
                        });
                    } else {
                        for (parameter_index, (expected, found)) in item
                            .parameters
                            .iter()
                            .zip(kernel.environment.iter())
                            .enumerate()
                        {
                            if expected != found {
                                errors.push(ValidationError::ItemBodyParameterLayoutMismatch {
                                    item: item_id,
                                    kernel: kernel_id,
                                    parameter_index,
                                    expected: *expected,
                                    found: *found,
                                });
                            }
                        }
                    }
                }
                None => errors.push(ValidationError::ItemUnknownBodyKernel {
                    item: item_id,
                    kernel: kernel_id,
                }),
            }
        }
        for pipeline in &item.pipelines {
            if !program.pipelines().contains(*pipeline) {
                errors.push(ValidationError::ItemPipelineBackrefMissing {
                    item: item_id,
                    pipeline: *pipeline,
                });
            }
        }
        if let ItemKind::Signal(signal) = &item.kind {
            for dependency in &signal.dependencies {
                match program.items().get(*dependency) {
                    Some(item) if matches!(item.kind, ItemKind::Signal(_)) => {}
                    Some(_) | None => errors.push(ValidationError::SignalDependencyNotSignal {
                        item: item_id,
                        dependency: *dependency,
                    }),
                }
            }
        }
    }

    for (pipeline_id, pipeline) in program.pipelines().iter() {
        let Some(owner) = program.items().get(pipeline.owner) else {
            errors.push(ValidationError::UnknownPipelineOwner {
                pipeline: pipeline_id,
                owner: pipeline.owner,
            });
            continue;
        };
        if !owner.pipelines.contains(&pipeline_id) {
            errors.push(ValidationError::ItemPipelineBackrefMissing {
                item: pipeline.owner,
                pipeline: pipeline_id,
            });
        }
        validate_pipeline(program, pipeline_id, pipeline, &mut errors);
    }

    for (source_id, source) in program.sources().iter() {
        let Some(owner) = program.items().get(source.owner) else {
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
            match program.items().get(*dependency) {
                Some(item) if matches!(item.kind, ItemKind::Signal(_)) => {}
                Some(_) | None => errors.push(ValidationError::SourceDependencyNotSignal {
                    source: source_id,
                    dependency: *dependency,
                }),
            }
        }
        for (index, argument) in source.arguments.iter().enumerate() {
            validate_source_kernel(
                program,
                source_id,
                source.owner,
                argument.kernel,
                &mut errors,
            )
            .unwrap_or_else(|kernel| {
                errors.push(ValidationError::SourceUnknownArgumentKernel {
                    source: source_id,
                    index,
                    kernel,
                });
            });
        }
        for option in &source.options {
            validate_source_kernel(program, source_id, source.owner, option.kernel, &mut errors)
                .unwrap_or_else(|kernel| {
                    errors.push(ValidationError::SourceUnknownOptionKernel {
                        source: source_id,
                        option_name: option.option_name.clone(),
                        kernel,
                    });
                });
        }
        if let Some(decode) = source.decode {
            if !program.decode_plans().contains(decode) {
                errors.push(ValidationError::SourceUnknownDecode {
                    source: source_id,
                    decode,
                });
            }
        }
    }

    for (decode_id, decode) in program.decode_plans().iter() {
        let Some(owner) = program.items().get(decode.owner) else {
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
        if !decode.steps().contains(decode.root) {
            errors.push(ValidationError::UnknownDecodeRoot {
                decode: decode_id,
                root: decode.root,
            });
        }
        for (step_id, step) in decode.steps().iter() {
            if !program.layouts().contains(step.layout) {
                errors.push(ValidationError::UnknownDecodeLayout {
                    decode: decode_id,
                    step: step_id,
                    layout: step.layout,
                });
            }
            match &step.kind {
                DecodeStepKind::Scalar { .. } => {}
                DecodeStepKind::Tuple { elements } => {
                    push_decode_steps(decode_id, elements, decode, &mut errors);
                }
                DecodeStepKind::Record { fields, .. } => {
                    for field in fields {
                        push_decode_step(decode_id, field.step, decode, &mut errors);
                    }
                }
                DecodeStepKind::Sum { variants, .. } => {
                    for variant in variants {
                        if let Some(payload) = variant.payload {
                            push_decode_step(decode_id, payload, decode, &mut errors);
                        }
                    }
                }
                DecodeStepKind::Domain { carrier, .. } => {
                    push_decode_step(decode_id, *carrier, decode, &mut errors);
                }
                DecodeStepKind::List { element } | DecodeStepKind::Option { element } => {
                    push_decode_step(decode_id, *element, decode, &mut errors);
                }
                DecodeStepKind::Result { error, value }
                | DecodeStepKind::Validation { error, value } => {
                    push_decode_step(decode_id, *error, decode, &mut errors);
                    push_decode_step(decode_id, *value, decode, &mut errors);
                }
            }
        }
    }

    for (kernel_id, kernel) in program.kernels().iter() {
        validate_kernel(program, kernel_id, kernel, &mut errors);
    }

    validate_no_item_dep_cycles(program, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors::new(errors))
    }
}

fn validate_layouts(program: &Program, errors: &mut Vec<ValidationError>) {
    for (layout_id, layout) in program.layouts().iter() {
        let children: Vec<LayoutId> = match &layout.kind {
            LayoutKind::Primitive(_) => Vec::new(),
            LayoutKind::Tuple(elements) => elements.clone(),
            LayoutKind::Record(fields) => fields.iter().map(|field| field.layout).collect(),
            LayoutKind::Sum(variants) => variants
                .iter()
                .filter_map(|variant| variant.payload)
                .collect(),
            LayoutKind::Arrow { parameter, result } => vec![*parameter, *result],
            LayoutKind::List { element }
            | LayoutKind::Set { element }
            | LayoutKind::Option { element }
            | LayoutKind::Signal { element } => vec![*element],
            LayoutKind::Map { key, value }
            | LayoutKind::Result { error: key, value }
            | LayoutKind::Validation { error: key, value }
            | LayoutKind::Task { error: key, value } => vec![*key, *value],
            LayoutKind::AnonymousDomain { carrier, .. } => vec![*carrier],
            LayoutKind::Domain { arguments, .. } | LayoutKind::Opaque { arguments, .. } => {
                arguments.clone()
            }
        };
        for child in children {
            if !program.layouts().contains(child) {
                errors.push(ValidationError::LayoutChildMissing {
                    layout: layout_id,
                    child,
                });
            }
        }
    }
}

fn validate_pipeline(
    program: &Program,
    pipeline_id: PipelineId,
    pipeline: &crate::Pipeline,
    errors: &mut Vec<ValidationError>,
) {
    for stage in &pipeline.stages {
        if !program.layouts().contains(stage.input_layout) {
            errors.push(ValidationError::UnknownStageLayout {
                pipeline: pipeline_id,
                stage_index: stage.index,
                layout: stage.input_layout,
            });
        }
        if !program.layouts().contains(stage.result_layout) {
            errors.push(ValidationError::UnknownStageLayout {
                pipeline: pipeline_id,
                stage_index: stage.index,
                layout: stage.result_layout,
            });
        }
        match &stage.kind {
            StageKind::Gate(GateStage::Ordinary {
                when_true,
                when_false,
            }) => {
                validate_kernel_contract(
                    program,
                    *when_true,
                    Some(stage.input_layout),
                    true,
                    stage.result_layout,
                    errors,
                );
                validate_kernel_contract(
                    program,
                    *when_false,
                    Some(stage.input_layout),
                    true,
                    stage.result_layout,
                    errors,
                );
                // TODO: validate gate predicate layout is Bool — needs layout lookup from kernel context.
                // GateStage::Ordinary does not carry a separate predicate kernel; the Bool
                // constraint is enforced by the type system upstream. A dedicated predicate
                // kernel ID would be required here to check `is_bool_layout` at the backend level.
            }
            StageKind::Gate(GateStage::SignalFilter {
                payload_layout,
                predicate,
                ..
            }) => {
                if let Some(bool_layout) = lookup_bool_layout(program) {
                    validate_kernel_contract(
                        program,
                        *predicate,
                        Some(*payload_layout),
                        true,
                        bool_layout,
                        errors,
                    );
                } else if let Some(kernel) = program.kernels().get(*predicate) {
                    validate_kernel_contract(
                        program,
                        *predicate,
                        Some(*payload_layout),
                        true,
                        kernel.result_layout,
                        errors,
                    );
                } else {
                    errors.push(ValidationError::UnknownKernel { kernel: *predicate });
                }
                if let Some(kernel) = program.kernels().get(*predicate) {
                    if !is_bool_layout(program, kernel.result_layout) {
                        errors.push(ValidationError::SignalFilterPredicateNotBool {
                            kernel: *predicate,
                        });
                    }
                }
            }
            StageKind::TruthyFalsy(pair) => {
                let expected = truthy_falsy_result_layout(program, stage.result_layout);
                if pair.truthy.result_layout != pair.falsy.result_layout
                    || pair.truthy.result_layout != expected
                {
                    errors.push(ValidationError::TruthyFalsyResultMismatch {
                        pipeline: pipeline_id,
                        stage_index: stage.index,
                    });
                }
                for payload in [pair.truthy.payload_layout, pair.falsy.payload_layout]
                    .into_iter()
                    .flatten()
                {
                    if !program.layouts().contains(payload) {
                        errors.push(ValidationError::LayoutChildMissing {
                            layout: stage.result_layout,
                            child: payload,
                        });
                    }
                }
            }
            StageKind::Fanout(fanout) => {
                let expected = fanout
                    .join
                    .as_ref()
                    .map(|join| join.result_layout)
                    .unwrap_or(fanout.mapped_collection_layout);
                if expected != stage.result_layout {
                    errors.push(ValidationError::FanoutResultMismatch {
                        pipeline: pipeline_id,
                        stage_index: stage.index,
                    });
                }
            }
        }
    }

    if let Some(recurrence) = &pipeline.recurrence {
        validate_kernel_contract(
            program,
            recurrence.start.kernel,
            Some(recurrence.start.input_layout),
            true,
            recurrence.start.result_layout,
            errors,
        );
        if recurrence.steps.is_empty() {
            errors.push(ValidationError::RecurrenceMissingSteps {
                pipeline: pipeline_id,
            });
        }
        let mut current = recurrence.start.result_layout;
        for step in &recurrence.steps {
            validate_kernel_contract(
                program,
                step.kernel,
                Some(step.input_layout),
                true,
                step.result_layout,
                errors,
            );
            if step.input_layout != current {
                errors.push(ValidationError::RecurrenceStepInputMismatch {
                    pipeline: pipeline_id,
                });
                break;
            }
            current = step.result_layout;
        }
        if current != recurrence.start.result_layout {
            errors.push(ValidationError::RecurrenceDoesNotClose {
                pipeline: pipeline_id,
                expected: recurrence.start.result_layout,
                found: current,
            });
        }
        if let Some(witness) = &recurrence.non_source_wakeup {
            validate_kernel_contract(
                program,
                witness.kernel,
                None,
                false,
                program.kernels()[witness.kernel].result_layout,
                errors,
            );
        }
    }
}

fn validate_kernel_contract(
    program: &Program,
    kernel_id: KernelId,
    expected_input: Option<LayoutId>,
    allow_missing_input: bool,
    expected_result: LayoutId,
    errors: &mut Vec<ValidationError>,
) {
    let Some(kernel) = program.kernels().get(kernel_id) else {
        errors.push(ValidationError::UnknownKernel { kernel: kernel_id });
        return;
    };
    match (expected_input, kernel.input_subject) {
        (Some(expected), Some(found)) if expected != found => {
            errors.push(ValidationError::KernelInputMismatch {
                kernel: kernel_id,
                expected: Some(expected),
                found: Some(found),
            })
        }
        (Some(expected), None) if !allow_missing_input => {
            errors.push(ValidationError::KernelInputMismatch {
                kernel: kernel_id,
                expected: Some(expected),
                found: None,
            })
        }
        (None, Some(found)) => errors.push(ValidationError::KernelInputMismatch {
            kernel: kernel_id,
            expected: None,
            found: Some(found),
        }),
        _ => {}
    }
    if kernel.result_layout != expected_result {
        errors.push(ValidationError::KernelResultMismatch {
            kernel: kernel_id,
            expected: expected_result,
            found: kernel.result_layout,
        });
    }
}

fn validate_source_kernel(
    program: &Program,
    source: SourceId,
    expected_owner: ItemId,
    kernel_id: KernelId,
    errors: &mut Vec<ValidationError>,
) -> Result<(), KernelId> {
    let Some(kernel) = program.kernels().get(kernel_id) else {
        return Err(kernel_id);
    };
    if let Some(layout) = kernel.input_subject {
        errors.push(ValidationError::SourceKernelHasInput {
            source,
            kernel: kernel_id,
            layout,
        });
    }
    if kernel.origin.item != expected_owner {
        errors.push(ValidationError::SourceKernelOwnerMismatch {
            source,
            kernel: kernel_id,
            expected_owner,
            found_owner: kernel.origin.item,
        });
    }
    Ok(())
}

fn validate_kernel(
    program: &Program,
    kernel_id: KernelId,
    kernel: &crate::Kernel,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(layout) = kernel.input_subject {
        if !program.layouts().contains(layout) {
            errors.push(ValidationError::KernelUnknownLayout {
                kernel: kernel_id,
                layout,
            });
        }
    }
    for layout in &kernel.inline_subjects {
        if !program.layouts().contains(*layout) {
            errors.push(ValidationError::KernelUnknownLayout {
                kernel: kernel_id,
                layout: *layout,
            });
        }
    }
    for layout in &kernel.environment {
        if !program.layouts().contains(*layout) {
            errors.push(ValidationError::KernelUnknownLayout {
                kernel: kernel_id,
                layout: *layout,
            });
        }
    }
    if !program.layouts().contains(kernel.result_layout) {
        errors.push(ValidationError::KernelUnknownLayout {
            kernel: kernel_id,
            layout: kernel.result_layout,
        });
    }
    for item in &kernel.global_items {
        if !program.items().contains(*item) {
            errors.push(ValidationError::KernelUnknownItemRef {
                kernel: kernel_id,
                expr: kernel.root,
                item: *item,
            });
        }
    }
    if kernel.convention != expected_calling_convention(program, kernel) {
        errors.push(ValidationError::KernelConventionMismatch { kernel: kernel_id });
    }
    if !kernel.exprs().contains(kernel.root) {
        errors.push(ValidationError::KernelUnknownExpr {
            kernel: kernel_id,
            expr: kernel.root,
        });
        return;
    }

    let mut work = vec![kernel.root];
    while let Some(expr_id) = work.pop() {
        let Some(expr) = kernel.exprs().get(expr_id) else {
            errors.push(ValidationError::KernelUnknownExpr {
                kernel: kernel_id,
                expr: expr_id,
            });
            continue;
        };
        if !program.layouts().contains(expr.layout) {
            errors.push(ValidationError::KernelUnknownLayout {
                kernel: kernel_id,
                layout: expr.layout,
            });
        }
        match &expr.kind {
            KernelExprKind::Subject(SubjectRef::Input) => match kernel.input_subject {
                Some(layout) if layout == expr.layout => {}
                Some(layout) => errors.push(ValidationError::KernelSubjectLayoutMismatch {
                    kernel: kernel_id,
                    expr: expr_id,
                    expected: layout,
                    found: expr.layout,
                }),
                None => errors.push(ValidationError::KernelMissingInputSubject {
                    kernel: kernel_id,
                    expr: expr_id,
                }),
            },
            KernelExprKind::Subject(SubjectRef::Inline(subject)) => {
                match kernel.inline_subjects.get(subject.index()) {
                    Some(layout) if *layout == expr.layout => {}
                    Some(layout) => errors.push(ValidationError::KernelSubjectLayoutMismatch {
                        kernel: kernel_id,
                        expr: expr_id,
                        expected: *layout,
                        found: expr.layout,
                    }),
                    None => errors.push(ValidationError::KernelUnknownInlineSubject {
                        kernel: kernel_id,
                        expr: expr_id,
                        subject: *subject,
                    }),
                }
            }
            KernelExprKind::OptionSome { payload } => {
                push_expr(kernel_id, *payload, kernel, &mut work, errors)
            }
            KernelExprKind::OptionNone => {}
            KernelExprKind::Environment(slot) => match kernel.environment.get(slot.index()) {
                Some(layout) if *layout == expr.layout => {}
                Some(layout) => errors.push(ValidationError::KernelSubjectLayoutMismatch {
                    kernel: kernel_id,
                    expr: expr_id,
                    expected: *layout,
                    found: expr.layout,
                }),
                None => errors.push(ValidationError::KernelUnknownEnvironmentSlot {
                    kernel: kernel_id,
                    expr: expr_id,
                    slot: *slot,
                }),
            },
            KernelExprKind::Item(item) => {
                if !program.items().contains(*item) {
                    errors.push(ValidationError::KernelUnknownItemRef {
                        kernel: kernel_id,
                        expr: expr_id,
                        item: *item,
                    });
                }
                if !kernel.global_items.contains(item) {
                    errors.push(ValidationError::KernelGlobalDependencyMissing {
                        kernel: kernel_id,
                        item: *item,
                    });
                }
            }
            KernelExprKind::SumConstructor(_)
            | KernelExprKind::DomainMember(_)
            | KernelExprKind::BuiltinClassMember(_)
            | KernelExprKind::Builtin(_)
            | KernelExprKind::IntrinsicValue(_)
            | KernelExprKind::Integer(_)
            | KernelExprKind::Float(_)
            | KernelExprKind::Decimal(_)
            | KernelExprKind::BigInt(_)
            | KernelExprKind::SuffixedInteger(_) => {}
            KernelExprKind::Text(text) => {
                for segment in &text.segments {
                    if let crate::TextSegment::Interpolation { expr, .. } = segment {
                        push_expr(kernel_id, *expr, kernel, &mut work, errors);
                    }
                }
            }
            KernelExprKind::Tuple(elements)
            | KernelExprKind::List(elements)
            | KernelExprKind::Set(elements) => {
                push_exprs(kernel_id, elements, kernel, &mut work, errors)
            }
            KernelExprKind::Map(entries) => {
                for entry in entries {
                    push_expr(kernel_id, entry.key, kernel, &mut work, errors);
                    push_expr(kernel_id, entry.value, kernel, &mut work, errors);
                }
            }
            KernelExprKind::Record(fields) => {
                for field in fields {
                    push_expr(kernel_id, field.value, kernel, &mut work, errors);
                }
            }
            KernelExprKind::Projection { base, .. } => match base {
                ProjectionBase::Subject(SubjectRef::Input) => {
                    if kernel.input_subject.is_none() {
                        errors.push(ValidationError::KernelMissingInputSubject {
                            kernel: kernel_id,
                            expr: expr_id,
                        });
                    }
                }
                ProjectionBase::Subject(SubjectRef::Inline(subject)) => {
                    if kernel.inline_subjects.get(subject.index()).is_none() {
                        errors.push(ValidationError::KernelUnknownInlineSubject {
                            kernel: kernel_id,
                            expr: expr_id,
                            subject: *subject,
                        });
                    }
                }
                ProjectionBase::Expr(base) => {
                    push_expr(kernel_id, *base, kernel, &mut work, errors)
                }
            },
            KernelExprKind::Apply { callee, arguments } => {
                push_expr(kernel_id, *callee, kernel, &mut work, errors);
                push_exprs(kernel_id, arguments, kernel, &mut work, errors);
            }
            KernelExprKind::Unary { expr, .. } => {
                push_expr(kernel_id, *expr, kernel, &mut work, errors)
            }
            KernelExprKind::Binary { left, right, .. } => {
                push_expr(kernel_id, *left, kernel, &mut work, errors);
                push_expr(kernel_id, *right, kernel, &mut work, errors);
            }
            KernelExprKind::Pipe(pipe) => {
                push_expr(kernel_id, pipe.head, kernel, &mut work, errors);
                for stage in &pipe.stages {
                    if !program.layouts().contains(stage.input_layout) {
                        errors.push(ValidationError::KernelUnknownLayout {
                            kernel: kernel_id,
                            layout: stage.input_layout,
                        });
                    }
                    if !program.layouts().contains(stage.result_layout) {
                        errors.push(ValidationError::KernelUnknownLayout {
                            kernel: kernel_id,
                            layout: stage.result_layout,
                        });
                    }
                    match kernel.inline_subjects.get(stage.subject.index()) {
                        Some(layout) if *layout == stage.input_layout => {}
                        Some(layout) => errors.push(ValidationError::KernelSubjectLayoutMismatch {
                            kernel: kernel_id,
                            expr: expr_id,
                            expected: *layout,
                            found: stage.input_layout,
                        }),
                        None => errors.push(ValidationError::KernelUnknownInlineSubject {
                            kernel: kernel_id,
                            expr: expr_id,
                            subject: stage.subject,
                        }),
                    }
                    match &stage.kind {
                        InlinePipeStageKind::Transform { expr }
                        | InlinePipeStageKind::Tap { expr } => {
                            push_expr(kernel_id, *expr, kernel, &mut work, errors)
                        }
                        InlinePipeStageKind::Gate { predicate, .. } => {
                            push_expr(kernel_id, *predicate, kernel, &mut work, errors);
                            if let Some(pred_expr) = kernel.exprs().get(*predicate) {
                                if !is_bool_layout(program, pred_expr.layout) {
                                    errors.push(ValidationError::InlinePipeGatePredicateNotBool {
                                        kernel: kernel_id,
                                        expr: *predicate,
                                    });
                                }
                            }
                        }
                        InlinePipeStageKind::Case { arms } => {
                            for arm in arms {
                                if let Some(guard) = arm.guard {
                                    push_expr(kernel_id, guard, kernel, &mut work, errors);
                                    let guard_expr = &kernel.exprs()[guard];
                                    if !is_bool_layout(program, guard_expr.layout) {
                                        errors.push(ValidationError::InlinePipeCaseGuardNotBool {
                                            kernel: kernel_id,
                                            expr: guard,
                                        });
                                    }
                                }
                                push_expr(kernel_id, arm.body, kernel, &mut work, errors);
                                validate_inline_pipe_pattern(
                                    kernel_id,
                                    expr_id,
                                    kernel,
                                    &arm.pattern,
                                    errors,
                                );
                            }
                        }
                        InlinePipeStageKind::TruthyFalsy { truthy, falsy } => {
                            push_expr(kernel_id, truthy.body, kernel, &mut work, errors);
                            push_expr(kernel_id, falsy.body, kernel, &mut work, errors);
                            for branch in [truthy, falsy] {
                                if let Some(subject) = branch.payload_subject {
                                    match kernel.inline_subjects.get(subject.index()) {
                                        Some(_) => {}
                                        None => errors.push(
                                            ValidationError::KernelUnknownInlineSubject {
                                                kernel: kernel_id,
                                                expr: expr_id,
                                                subject,
                                            },
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn expected_calling_convention(program: &Program, kernel: &crate::Kernel) -> CallingConvention {
    let mut parameters =
        Vec::with_capacity(kernel.environment.len() + usize::from(kernel.input_subject.is_some()));
    if let Some(layout) = kernel.input_subject {
        parameters.push(crate::AbiParameter {
            role: ParameterRole::InputSubject,
            layout,
            pass_mode: program.layouts()[layout].abi,
        });
    }
    for (index, layout) in kernel.environment.iter().enumerate() {
        parameters.push(crate::AbiParameter {
            role: ParameterRole::Environment(EnvSlotId::from_raw(index as u32)),
            layout: *layout,
            pass_mode: program.layouts()[*layout].abi,
        });
    }
    CallingConvention {
        kind: crate::CallingConventionKind::RuntimeKernelV1,
        parameters,
        result: crate::AbiResult {
            layout: kernel.result_layout,
            pass_mode: program.layouts()[kernel.result_layout].abi,
        },
    }
}

fn lookup_bool_layout(program: &Program) -> Option<LayoutId> {
    program
        .layouts()
        .iter()
        .find(|(_, layout)| matches!(layout.kind, LayoutKind::Primitive(PrimitiveType::Bool)))
        .map(|(id, _)| id)
}

fn is_bool_layout(program: &Program, layout_id: LayoutId) -> bool {
    program
        .layouts()
        .get(layout_id)
        .is_some_and(|layout| matches!(layout.kind, LayoutKind::Primitive(PrimitiveType::Bool)))
}

fn truthy_falsy_result_layout(program: &Program, layout_id: LayoutId) -> LayoutId {
    match program.layouts().get(layout_id).map(|layout| &layout.kind) {
        Some(LayoutKind::Signal { element }) => *element,
        _ => layout_id,
    }
}

fn push_expr(
    kernel_id: KernelId,
    expr: KernelExprId,
    kernel: &crate::Kernel,
    work: &mut Vec<KernelExprId>,
    errors: &mut Vec<ValidationError>,
) {
    if kernel.exprs().contains(expr) {
        work.push(expr);
    } else {
        errors.push(ValidationError::KernelUnknownExpr {
            kernel: kernel_id,
            expr,
        });
    }
}

fn push_exprs(
    kernel_id: KernelId,
    exprs: &[KernelExprId],
    kernel: &crate::Kernel,
    work: &mut Vec<KernelExprId>,
    errors: &mut Vec<ValidationError>,
) {
    for expr in exprs {
        push_expr(kernel_id, *expr, kernel, work, errors);
    }
}

fn validate_inline_pipe_pattern(
    kernel_id: KernelId,
    expr_id: KernelExprId,
    kernel: &crate::Kernel,
    pattern: &InlinePipePattern,
    errors: &mut Vec<ValidationError>,
) {
    let mut work = vec![pattern];
    while let Some(pattern) = work.pop() {
        match &pattern.kind {
            InlinePipePatternKind::Wildcard
            | InlinePipePatternKind::Integer(_)
            | InlinePipePatternKind::Text(_) => {}
            InlinePipePatternKind::Binding { subject } => {
                if kernel.inline_subjects.get(subject.index()).is_none() {
                    errors.push(ValidationError::KernelUnknownInlineSubject {
                        kernel: kernel_id,
                        expr: expr_id,
                        subject: *subject,
                    });
                }
            }
            InlinePipePatternKind::Tuple(elements) => {
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            InlinePipePatternKind::List { elements, rest } => {
                if let Some(rest) = rest {
                    work.push(rest);
                }
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            InlinePipePatternKind::Record(fields) => {
                for InlinePipeRecordPatternField { pattern, .. } in fields.iter().rev() {
                    work.push(pattern);
                }
            }
            InlinePipePatternKind::Constructor { arguments, .. } => {
                for argument in arguments.iter().rev() {
                    work.push(argument);
                }
            }
        }
    }
}

fn push_decode_step(
    decode_id: DecodePlanId,
    step: DecodeStepId,
    decode: &crate::DecodePlan,
    errors: &mut Vec<ValidationError>,
) {
    if !decode.steps().contains(step) {
        errors.push(ValidationError::UnknownDecodeStep {
            decode: decode_id,
            step,
        });
    }
}

fn push_decode_steps(
    decode_id: DecodePlanId,
    steps: &[DecodeStepId],
    decode: &crate::DecodePlan,
    errors: &mut Vec<ValidationError>,
) {
    for step in steps {
        push_decode_step(decode_id, *step, decode, errors);
    }
}

/// Validate that there are no circular dependencies between global items.
///
/// A cycle in the item dependency graph (item A transitively depends on itself) means that
/// runtime evaluation would loop forever. This function builds a dependency map from the
/// `global_items` lists of all kernels owned by each item, then performs a DFS with
/// white/gray/black coloring to detect back-edges.
fn validate_no_item_dep_cycles(program: &Program, errors: &mut Vec<ValidationError>) {
    // Build item -> deps map: for each item, collect all items referenced in any kernel it owns.
    let mut deps: HashMap<ItemId, Vec<ItemId>> = HashMap::new();
    for (item_id, _item) in program.items().iter() {
        deps.entry(item_id).or_default();
    }
    for (_kernel_id, kernel) in program.kernels().iter() {
        let owner = kernel.origin.item;
        let entry = deps.entry(owner).or_default();
        for &dep in &kernel.global_items {
            if dep != owner && !entry.contains(&dep) {
                entry.push(dep);
            }
        }
    }

    // DFS with white(0)/gray(1)/black(2) coloring to find cycles.
    // color: 0 = unvisited, 1 = in stack (gray), 2 = done (black)
    let mut color: HashMap<ItemId, u8> = HashMap::with_capacity(deps.len());
    let mut path: Vec<ItemId> = Vec::new();

    let all_items: Vec<ItemId> = deps.keys().copied().collect();
    'outer: for start in all_items {
        if *color.get(&start).unwrap_or(&0) == 0 {
            // Iterative DFS using an explicit stack of (item, dep_index).
            // We mark a node gray (1) when we first visit it, and black (2) when we finish it.
            color.insert(start, 1);
            path.push(start);
            let mut stack: Vec<(ItemId, usize)> = vec![(start, 0)];
            loop {
                let (node, idx) = match stack.last().copied() {
                    Some(top) => top,
                    None => break,
                };
                let neighbors: Vec<ItemId> = deps.get(&node).cloned().unwrap_or_default();
                if idx < neighbors.len() {
                    // Advance the index on the stack top before touching stack structure.
                    stack.last_mut().unwrap().1 += 1;
                    let neighbor = neighbors[idx];
                    let neighbor_color = *color.get(&neighbor).unwrap_or(&0);
                    if neighbor_color == 1 {
                        // Back-edge found: extract cycle from path.
                        let cycle_start = path.iter().position(|&n| n == neighbor).unwrap_or(0);
                        let mut cycle = path[cycle_start..].to_vec();
                        cycle.push(neighbor);
                        errors.push(ValidationError::ItemCyclicDependency { cycle });
                        // Mark all gray nodes on path as black to avoid duplicate reports.
                        for &n in &path {
                            color.insert(n, 2);
                        }
                        stack.clear();
                        path.clear();
                        continue 'outer;
                    } else if neighbor_color == 0 {
                        color.insert(neighbor, 1);
                        path.push(neighbor);
                        stack.push((neighbor, 0));
                    }
                } else {
                    color.insert(node, 2);
                    path.pop();
                    stack.pop();
                }
            }
        }
    }
}
