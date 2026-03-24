use std::{
    collections::{BTreeSet, HashMap},
    fmt,
};

use aivi_base::SourceSpan;
use aivi_core::{self as core, Arena, ArenaOverflow};
use aivi_hir::{
    BinaryOperator as HirBinaryOperator, BuiltinTerm as HirBuiltinTerm,
    UnaryOperator as HirUnaryOperator,
};
use aivi_lambda::{self as lambda};
use aivi_typing::{
    DecodeExtraFieldPolicy as TypingDecodeExtraFieldPolicy,
    DecodeFieldRequirement as TypingDecodeFieldRequirement, DecodeMode as TypingDecodeMode,
    DecodeSumStrategy as TypingDecodeSumStrategy, FanoutCarrier as TypingFanoutCarrier,
    NonSourceWakeupCause as TypingNonSourceWakeupCause, PrimitiveType as TypingPrimitiveType,
    RecurrenceTarget as TypingRecurrenceTarget, RecurrenceWakeupKind as TypingRecurrenceWakeupKind,
    SourceCancellationPolicy as TypingSourceCancellationPolicy,
};

use crate::{
    AbiParameter, AbiResult, BigIntLiteral, BinaryOperator,
    BuiltinAppendCarrier as BackendBuiltinAppendCarrier,
    BuiltinApplicativeCarrier as BackendBuiltinApplicativeCarrier,
    BuiltinApplyCarrier as BackendBuiltinApplyCarrier,
    BuiltinClassMemberIntrinsic as BackendBuiltinClassMemberIntrinsic,
    BuiltinFoldableCarrier as BackendBuiltinFoldableCarrier,
    BuiltinFunctorCarrier as BackendBuiltinFunctorCarrier,
    BuiltinOrdSubject as BackendBuiltinOrdSubject, BuiltinTerm, CallingConvention,
    CallingConventionKind, DecimalLiteral, DecodeExtraFieldPolicy, DecodeField,
    DecodeFieldRequirement, DecodeMode, DecodePlan, DecodePlanId, DecodeStep, DecodeStepId,
    DecodeStepKind, DecodeSumStrategy, DecodeVariant, DomainDecodeSurface, DomainDecodeSurfaceKind,
    EnvSlotId, FanoutCarrier, FanoutJoin, FanoutStage, FloatLiteral, GateStage, InlinePipeCaseArm,
    InlinePipeConstructor, InlinePipeExpr, InlinePipePattern, InlinePipePatternKind,
    InlinePipeRecordPatternField, InlinePipeStage, InlinePipeStageKind,
    InlinePipeTruthyFalsyBranch, InlineSubjectId, IntegerLiteral, Item, ItemId, ItemKind, Kernel,
    KernelExpr, KernelExprId, KernelExprKind, KernelId, KernelOrigin, KernelOriginKind, Layout,
    LayoutId, LayoutKind, LoweringError::*, MapEntry, NonSourceWakeup, NonSourceWakeupCause,
    ParameterRole, Pipeline, PipelineId, PipelineOrigin, PrimitiveType, Program, ProjectionBase,
    RecordExprField, RecordFieldLayout, Recurrence, RecurrenceStage, RecurrenceTarget,
    RecurrenceWakeupKind, SignalInfo, SourceArgumentKernel, SourceCancellationPolicy,
    SourceInstanceId, SourceOptionBinding, SourceOptionKernel, SourcePlan, SourceProvider,
    SourceReplacementPolicy, SourceStaleWorkPolicy, SourceTeardownPolicy, Stage, StageKind,
    SubjectRef, SuffixedIntegerLiteral, TextLiteral, TextSegment, TruthyFalsyBranch,
    TruthyFalsyStage, UnaryOperator, ValidationError, VariantLayout, validate_program,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoweringErrors {
    errors: Vec<LoweringError>,
}

impl LoweringErrors {
    pub fn new(errors: Vec<LoweringError>) -> Self {
        Self { errors }
    }

    pub fn errors(&self) -> &[LoweringError] {
        &self.errors
    }

    pub fn into_errors(self) -> Vec<LoweringError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for LoweringErrors {
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

impl std::error::Error for LoweringErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoweringError {
    InvalidLambdaModule(lambda::ValidationError),
    InvalidBackendProgram(ValidationError),
    UnknownLambdaItem {
        item: core::ItemId,
        span: SourceSpan,
    },
    UnresolvedItemReference {
        span: SourceSpan,
    },
    UnsupportedInlinePipeStage {
        span: SourceSpan,
    },
    UnsupportedInlinePipePattern {
        span: SourceSpan,
    },
    MissingInputSubjectContract {
        span: SourceSpan,
    },
    SubjectLayoutMismatch {
        span: SourceSpan,
        expected: LayoutId,
        found: LayoutId,
    },
    UnsupportedLocalReference {
        span: SourceSpan,
        binding: u32,
    },
    OpenTypeParameter {
        name: Box<str>,
    },
    ArenaOverflow {
        family: &'static str,
        attempted_len: usize,
    },
}

impl fmt::Display for LoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLambdaModule(error) => {
                write!(
                    f,
                    "backend lowering requires valid typed-lambda input: {error}"
                )
            }
            Self::InvalidBackendProgram(error) => {
                write!(f, "backend lowering produced invalid backend IR: {error}")
            }
            Self::UnknownLambdaItem { item, .. } => {
                write!(f, "backend lowering cannot map typed-lambda item {item}")
            }
            Self::UnresolvedItemReference { .. } => {
                f.write_str("backend lowering rejects unresolved HIR item references")
            }
            Self::UnsupportedInlinePipeStage { .. } => f.write_str(
                "backend lowering does not yet encode inline case/truthy-falsy pipe stages",
            ),
            Self::UnsupportedInlinePipePattern { .. } => f.write_str(
                "backend lowering cannot encode this inline pipe pattern/runtime carrier yet",
            ),
            Self::MissingInputSubjectContract { .. } => f.write_str(
                "backend kernel uses an input subject without an explicit backend contract",
            ),
            Self::SubjectLayoutMismatch {
                expected, found, ..
            } => write!(
                f,
                "backend subject reference changed layout unexpectedly: expected layout{expected}, found layout{found}"
            ),
            Self::UnsupportedLocalReference { binding, .. } => write!(
                f,
                "backend lowering does not yet encode closure-local binding {binding} inside runtime kernels"
            ),
            Self::OpenTypeParameter { name } => write!(
                f,
                "backend lowering requires closed specialized types, but encountered open type parameter `{name}`"
            ),
            Self::ArenaOverflow {
                family,
                attempted_len,
            } => write!(
                f,
                "backend {family} arena overflow after {attempted_len} entries; ids are limited to u32::MAX"
            ),
        }
    }
}

impl std::error::Error for LoweringError {}

pub fn lower_module(lambda_module: &lambda::Module) -> Result<Program, LoweringErrors> {
    if let Err(errors) = lambda::validate_module(lambda_module) {
        return Err(LoweringErrors::new(
            errors
                .into_errors()
                .into_iter()
                .map(LoweringError::InvalidLambdaModule)
                .collect(),
        ));
    }

    ProgramLowerer::new(lambda_module).build()
}

struct ProgramLowerer<'a> {
    lambda: &'a lambda::Module,
    program: Program,
    item_map: HashMap<core::ItemId, ItemId>,
    layout_interner: HashMap<Layout, LayoutId>,
    core_layouts: HashMap<core::Type, LayoutId>,
}

impl<'a> ProgramLowerer<'a> {
    fn new(lambda: &'a lambda::Module) -> Self {
        Self {
            lambda,
            program: Program::new(),
            item_map: HashMap::new(),
            layout_interner: HashMap::new(),
            core_layouts: HashMap::new(),
        }
    }

    fn build(mut self) -> Result<Program, LoweringErrors> {
        self.seed_items().map_err(wrap_one)?;
        self.seed_signal_dependencies().map_err(wrap_one)?;
        self.lower_item_bodies().map_err(wrap_one)?;
        self.lower_pipelines().map_err(wrap_one)?;
        self.lower_sources().map_err(wrap_one)?;
        if let Err(errors) = validate_program(&self.program) {
            return Err(LoweringErrors::new(
                errors
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::InvalidBackendProgram)
                    .collect::<Vec<_>>(),
            ));
        }
        Ok(self.program)
    }

    fn seed_items(&mut self) -> Result<(), LoweringError> {
        for (core_id, item) in self.lambda.items().iter() {
            let kind = match &item.kind {
                core::ItemKind::Value => ItemKind::Value,
                core::ItemKind::Function => ItemKind::Function,
                core::ItemKind::Signal(_) => ItemKind::Signal(SignalInfo::default()),
                core::ItemKind::Instance => ItemKind::Instance,
            };
            let parameters = item
                .parameters
                .iter()
                .map(|parameter| self.intern_core_type(&parameter.ty))
                .collect::<Result<Vec<_>, _>>()?;
            let item_id = self
                .program
                .items_mut()
                .alloc(Item {
                    origin: core_id,
                    span: item.span,
                    name: item.name.clone(),
                    kind,
                    parameters,
                    body: None,
                    pipelines: Vec::new(),
                })
                .map_err(|overflow| arena_overflow("items", overflow))?;
            self.item_map.insert(core_id, item_id);
        }
        Ok(())
    }

    fn lower_item_bodies(&mut self) -> Result<(), LoweringError> {
        for (core_id, item) in self.lambda.items().iter() {
            let Some(body) = item.body else {
                continue;
            };
            let item_id = self.require_item(core_id, item.span)?;
            let kernel = self.lower_item_body_kernel(item, item_id, body)?;
            self.program
                .items_mut()
                .get_mut(item_id)
                .expect("seeded backend item should exist")
                .body = Some(kernel);
        }
        Ok(())
    }

    fn seed_signal_dependencies(&mut self) -> Result<(), LoweringError> {
        for (core_id, item) in self.lambda.items().iter() {
            let core::ItemKind::Signal(signal) = &item.kind else {
                continue;
            };
            let item_id = self.require_item(core_id, item.span)?;
            let dependencies = signal
                .dependencies
                .iter()
                .map(|dependency| self.require_item(*dependency, item.span))
                .collect::<Result<Vec<_>, _>>()?;
            let backend_item = self
                .program
                .items_mut()
                .get_mut(item_id)
                .expect("seeded signal item should exist");
            let ItemKind::Signal(info) = &mut backend_item.kind else {
                unreachable!("signal dependency seeding only touches signal items");
            };
            info.dependencies = dependencies;
        }
        Ok(())
    }

    fn lower_pipelines(&mut self) -> Result<(), LoweringError> {
        for (core_pipe_id, pipe) in self.lambda.pipes().iter() {
            let owner = self.require_item(pipe.owner, pipe.origin.span)?;
            let pipeline_id = self
                .program
                .pipelines_mut()
                .alloc(Pipeline {
                    owner,
                    origin: PipelineOrigin {
                        span: pipe.origin.span,
                        core_pipe: core_pipe_id,
                    },
                    stages: Vec::new(),
                    recurrence: None,
                })
                .map_err(|overflow| arena_overflow("pipelines", overflow))?;

            let mut stages = Vec::with_capacity(pipe.stages.len());
            for stage_id in &pipe.stages {
                let stage = &self.lambda.stages()[*stage_id];
                stages.push(self.lower_stage(pipeline_id, stage)?);
            }
            let recurrence = pipe
                .recurrence
                .as_ref()
                .map(|recurrence| self.lower_recurrence(pipeline_id, recurrence))
                .transpose()?;

            let backend_pipeline = self
                .program
                .pipelines_mut()
                .get_mut(pipeline_id)
                .expect("allocated pipeline should exist");
            backend_pipeline.stages = stages;
            backend_pipeline.recurrence = recurrence;
            self.program
                .items_mut()
                .get_mut(owner)
                .expect("pipeline owner should exist")
                .pipelines
                .push(pipeline_id);
        }
        Ok(())
    }

    fn lower_stage(
        &mut self,
        pipeline_id: PipelineId,
        stage: &lambda::Stage,
    ) -> Result<Stage, LoweringError> {
        let input_layout = self.intern_core_type(&stage.input_subject)?;
        let result_layout = self.intern_core_type(&stage.result_subject)?;
        let kind = match &stage.kind {
            lambda::StageKind::Gate(lambda::GateStage::Ordinary {
                when_true,
                when_false,
            }) => StageKind::Gate(GateStage::Ordinary {
                when_true: self.lower_kernel(
                    KernelOriginKind::GateTrue {
                        pipeline: pipeline_id,
                        stage_index: stage.index,
                    },
                    *when_true,
                )?,
                when_false: self.lower_kernel(
                    KernelOriginKind::GateFalse {
                        pipeline: pipeline_id,
                        stage_index: stage.index,
                    },
                    *when_false,
                )?,
            }),
            lambda::StageKind::Gate(lambda::GateStage::SignalFilter {
                payload_type,
                predicate,
                emits_negative_update,
            }) => {
                let payload_layout = self.intern_core_type(payload_type)?;
                StageKind::Gate(GateStage::SignalFilter {
                    payload_layout,
                    predicate: self.lower_kernel(
                        KernelOriginKind::SignalFilterPredicate {
                            pipeline: pipeline_id,
                            stage_index: stage.index,
                        },
                        *predicate,
                    )?,
                    emits_negative_update: *emits_negative_update,
                })
            }
            lambda::StageKind::TruthyFalsy(pair) => StageKind::TruthyFalsy(TruthyFalsyStage {
                truthy_stage_index: pair.truthy_stage_index,
                truthy_stage_span: pair.truthy_stage_span,
                falsy_stage_index: pair.falsy_stage_index,
                falsy_stage_span: pair.falsy_stage_span,
                truthy: TruthyFalsyBranch {
                    constructor: map_builtin_term(pair.truthy.constructor),
                    payload_layout: pair
                        .truthy
                        .payload_subject
                        .as_ref()
                        .map(|payload| self.intern_core_type(payload))
                        .transpose()?,
                    result_layout: self.intern_core_type(&pair.truthy.result_type)?,
                },
                falsy: TruthyFalsyBranch {
                    constructor: map_builtin_term(pair.falsy.constructor),
                    payload_layout: pair
                        .falsy
                        .payload_subject
                        .as_ref()
                        .map(|payload| self.intern_core_type(payload))
                        .transpose()?,
                    result_layout: self.intern_core_type(&pair.falsy.result_type)?,
                },
            }),
            lambda::StageKind::Fanout(fanout) => StageKind::Fanout(FanoutStage {
                carrier: map_fanout_carrier(fanout.carrier),
                element_layout: self.intern_core_type(&fanout.element_subject)?,
                mapped_element_layout: self.intern_core_type(&fanout.mapped_element_type)?,
                mapped_collection_layout: self.intern_core_type(&fanout.mapped_collection_type)?,
                join: fanout
                    .join
                    .as_ref()
                    .map(|join| {
                        Ok(FanoutJoin {
                            stage_index: join.stage_index,
                            stage_span: join.stage_span,
                            input_layout: self.intern_core_type(&join.input_subject)?,
                            collection_layout: self.intern_core_type(&join.collection_subject)?,
                            result_layout: self.intern_core_type(&join.result_type)?,
                        })
                    })
                    .transpose()?,
            }),
        };

        Ok(Stage {
            index: stage.index,
            span: stage.span,
            input_layout,
            result_layout,
            kind,
        })
    }

    fn lower_recurrence(
        &mut self,
        pipeline_id: PipelineId,
        recurrence: &lambda::PipeRecurrence,
    ) -> Result<Recurrence, LoweringError> {
        let start = self.lower_recurrence_stage(
            KernelOriginKind::RecurrenceStart {
                pipeline: pipeline_id,
                stage_index: recurrence.start.stage_index,
            },
            &recurrence.start,
        )?;
        let mut steps = Vec::with_capacity(recurrence.steps.len());
        for step in &recurrence.steps {
            steps.push(self.lower_recurrence_stage(
                KernelOriginKind::RecurrenceStep {
                    pipeline: pipeline_id,
                    stage_index: step.stage_index,
                },
                step,
            )?);
        }
        let non_source_wakeup = recurrence
            .non_source_wakeup
            .as_ref()
            .map(|wakeup| {
                Ok(NonSourceWakeup {
                    cause: map_non_source_wakeup_cause(wakeup.cause),
                    kernel: self.lower_kernel(
                        KernelOriginKind::RecurrenceWakeupWitness {
                            pipeline: pipeline_id,
                        },
                        wakeup.runtime,
                    )?,
                })
            })
            .transpose()?;

        Ok(Recurrence {
            target: map_recurrence_target(recurrence.target.target()),
            wakeup_kind: map_recurrence_wakeup_kind(recurrence.wakeup.kind()),
            start,
            steps,
            non_source_wakeup,
        })
    }

    fn lower_recurrence_stage(
        &mut self,
        kind: KernelOriginKind,
        stage: &lambda::RecurrenceStage,
    ) -> Result<RecurrenceStage, LoweringError> {
        let input_layout = self.intern_core_type(&stage.input_subject)?;
        let result_layout = self.intern_core_type(&stage.result_subject)?;
        Ok(RecurrenceStage {
            stage_index: stage.stage_index,
            stage_span: stage.stage_span,
            input_layout,
            result_layout,
            kernel: self.lower_kernel(kind, stage.runtime)?,
        })
    }
    fn lower_sources(&mut self) -> Result<(), LoweringError> {
        for (_, source) in self.lambda.sources().iter() {
            let owner = self.require_item(source.owner, source.span)?;
            let reconfiguration_dependencies = source
                .reconfiguration_dependencies
                .iter()
                .map(|dependency| self.require_item(*dependency, source.span))
                .collect::<Result<Vec<_>, _>>()?;
            let explicit_triggers = source
                .explicit_triggers
                .iter()
                .map(|binding| SourceOptionBinding {
                    option_span: binding.option_span,
                    option_name: binding.option_name.clone(),
                })
                .collect();
            let active_when = source
                .active_when
                .as_ref()
                .map(|binding| SourceOptionBinding {
                    option_span: binding.option_span,
                    option_name: binding.option_name.clone(),
                });
            let source_id = self
                .program
                .sources_mut()
                .alloc(SourcePlan {
                    owner,
                    span: source.span,
                    instance: SourceInstanceId::from_raw(source_instance_raw(source)),
                    provider: map_source_provider(&source.provider),
                    teardown: map_teardown_policy(source.teardown),
                    replacement: map_replacement_policy(source.replacement),
                    arguments: Vec::new(),
                    options: Vec::new(),
                    reconfiguration_dependencies,
                    explicit_triggers,
                    active_when,
                    cancellation: map_cancellation_policy(source.cancellation),
                    stale_work: map_stale_work_policy(source.stale_work),
                    decode: None,
                })
                .map_err(|overflow| arena_overflow("sources", overflow))?;
            let backend_item = self
                .program
                .items_mut()
                .get_mut(owner)
                .expect("source owner should exist");
            let ItemKind::Signal(info) = &mut backend_item.kind else {
                unreachable!("typed-lambda validation should prevent non-signal sources");
            };
            info.source = Some(source_id);
            let mut arguments = Vec::with_capacity(source.arguments.len());
            for (index, argument) in source.arguments.iter().enumerate() {
                arguments.push(SourceArgumentKernel {
                    kernel: self.lower_root_expr_kernel(
                        owner,
                        source.span,
                        KernelOriginKind::SourceArgument {
                            source: source_id,
                            index,
                        },
                        argument.runtime_expr,
                    )?,
                });
            }
            let mut options = Vec::with_capacity(source.options.len());
            for (index, option) in source.options.iter().enumerate() {
                options.push(SourceOptionKernel {
                    option_name: option.option_name.clone(),
                    kernel: self.lower_root_expr_kernel(
                        owner,
                        option.option_span,
                        KernelOriginKind::SourceOption {
                            source: source_id,
                            index,
                        },
                        option.runtime_expr,
                    )?,
                });
            }
            let lowered_source = self
                .program
                .sources_mut()
                .get_mut(source_id)
                .expect("allocated source should exist");
            lowered_source.arguments = arguments;
            lowered_source.options = options;
            if let Some(decode) = source.decode {
                let decode_id =
                    self.lower_decode_plan(owner, &self.lambda.decode_programs()[decode])?;
                self.program
                    .sources_mut()
                    .get_mut(source_id)
                    .expect("allocated source should exist")
                    .decode = Some(decode_id);
            }
        }
        Ok(())
    }

    fn lower_decode_plan(
        &mut self,
        owner: ItemId,
        decode: &core::DecodeProgram,
    ) -> Result<DecodePlanId, LoweringError> {
        enum Task {
            Visit(core::DecodeStepId),
            Build(DecodeBuildTask),
        }

        enum DecodeBuildTask {
            Scalar {
                scalar: PrimitiveType,
            },
            Tuple {
                len: usize,
            },
            Record {
                fields: Vec<(Box<str>, DecodeFieldRequirement)>,
                extra_fields: DecodeExtraFieldPolicy,
            },
            Sum {
                variants: Vec<(Box<str>, bool)>,
                strategy: DecodeSumStrategy,
            },
            Domain {
                surface: DomainDecodeSurface,
            },
            List,
            Option,
            Result,
            Validation,
        }

        let mut steps = Arena::new();
        let mut tasks = vec![Task::Visit(decode.root)];
        let mut values: Vec<(DecodeStepId, LayoutId)> = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(step_id) => {
                    let step = &decode.steps()[step_id];
                    match step {
                        core::DecodeStep::Scalar { scalar } => {
                            tasks.push(Task::Build(DecodeBuildTask::Scalar {
                                scalar: map_decode_primitive(*scalar),
                            }));
                        }
                        core::DecodeStep::Tuple { elements } => {
                            tasks.push(Task::Build(DecodeBuildTask::Tuple {
                                len: elements.len(),
                            }));
                            for child in elements.iter().rev() {
                                tasks.push(Task::Visit(*child));
                            }
                        }
                        core::DecodeStep::Record {
                            fields,
                            extra_fields,
                        } => {
                            tasks.push(Task::Build(DecodeBuildTask::Record {
                                fields: fields
                                    .iter()
                                    .map(|field| {
                                        (
                                            field.name.clone(),
                                            map_decode_requirement(field.requirement),
                                        )
                                    })
                                    .collect(),
                                extra_fields: map_extra_fields(*extra_fields),
                            }));
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(field.step));
                            }
                        }
                        core::DecodeStep::Sum { variants, strategy } => {
                            tasks.push(Task::Build(DecodeBuildTask::Sum {
                                variants: variants
                                    .iter()
                                    .map(|variant| {
                                        (variant.name.clone(), variant.payload.is_some())
                                    })
                                    .collect(),
                                strategy: map_decode_strategy(*strategy),
                            }));
                            for variant in variants.iter().rev() {
                                if let Some(payload) = variant.payload {
                                    tasks.push(Task::Visit(payload));
                                }
                            }
                        }
                        core::DecodeStep::Domain { carrier, surface } => {
                            tasks.push(Task::Build(DecodeBuildTask::Domain {
                                surface: DomainDecodeSurface {
                                    member_index: surface.member_index,
                                    member_name: surface.member_name.clone(),
                                    kind: map_domain_surface_kind(surface.kind),
                                    span: surface.span,
                                },
                            }));
                            tasks.push(Task::Visit(*carrier));
                        }
                        core::DecodeStep::List { element } => {
                            tasks.push(Task::Build(DecodeBuildTask::List));
                            tasks.push(Task::Visit(*element));
                        }
                        core::DecodeStep::Option { element } => {
                            tasks.push(Task::Build(DecodeBuildTask::Option));
                            tasks.push(Task::Visit(*element));
                        }
                        core::DecodeStep::Result { error, value } => {
                            tasks.push(Task::Build(DecodeBuildTask::Result));
                            tasks.push(Task::Visit(*value));
                            tasks.push(Task::Visit(*error));
                        }
                        core::DecodeStep::Validation { error, value } => {
                            tasks.push(Task::Build(DecodeBuildTask::Validation));
                            tasks.push(Task::Visit(*value));
                            tasks.push(Task::Visit(*error));
                        }
                    }
                }
                Task::Build(build) => {
                    let (step, layout) = match build {
                        DecodeBuildTask::Scalar { scalar } => {
                            let layout =
                                self.intern_layout(Layout::new(LayoutKind::Primitive(scalar)))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Scalar { scalar },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Tuple { len } => {
                            let lowered = drain_tail(&mut values, len);
                            let layout = self.intern_layout(Layout::new(LayoutKind::Tuple(
                                lowered.iter().map(|(_, layout)| *layout).collect(),
                            )))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Tuple {
                                        elements: lowered
                                            .into_iter()
                                            .map(|(step, _)| step)
                                            .collect(),
                                    },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Record {
                            fields,
                            extra_fields,
                        } => {
                            let lowered = drain_tail(&mut values, fields.len());
                            let layout = self.intern_layout(Layout::new(LayoutKind::Record(
                                fields
                                    .iter()
                                    .zip(lowered.iter())
                                    .map(|((name, _), (_, layout))| RecordFieldLayout {
                                        name: name.clone(),
                                        layout: *layout,
                                    })
                                    .collect(),
                            )))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Record {
                                        fields: fields
                                            .into_iter()
                                            .zip(lowered.into_iter())
                                            .map(|((name, requirement), (step, _))| DecodeField {
                                                name,
                                                requirement,
                                                step,
                                            })
                                            .collect(),
                                        extra_fields,
                                    },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Sum { variants, strategy } => {
                            let payload_count = variants
                                .iter()
                                .filter(|(_, has_payload)| *has_payload)
                                .count();
                            let payloads = drain_tail(&mut values, payload_count);
                            let mut payload_iter = payloads.into_iter();
                            let mut layout_variants = Vec::with_capacity(variants.len());
                            let mut decode_variants = Vec::with_capacity(variants.len());
                            for (name, has_payload) in variants {
                                let payload = if has_payload {
                                    payload_iter.next()
                                } else {
                                    None
                                };
                                layout_variants.push(VariantLayout {
                                    name: name.clone(),
                                    payload: payload.map(|(_, layout)| layout),
                                });
                                decode_variants.push(DecodeVariant {
                                    name,
                                    payload: payload.map(|(step, _)| step),
                                });
                            }
                            let layout =
                                self.intern_layout(Layout::new(LayoutKind::Sum(layout_variants)))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Sum {
                                        variants: decode_variants,
                                        strategy,
                                    },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Domain { surface } => {
                            let (carrier, carrier_layout) =
                                values.pop().expect("domain carrier should exist");
                            let layout =
                                self.intern_layout(Layout::new(LayoutKind::AnonymousDomain {
                                    carrier: carrier_layout,
                                    surface_member: surface.member_name.clone(),
                                }))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Domain { carrier, surface },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::List => {
                            let (element, element_layout) =
                                values.pop().expect("list element should exist");
                            let layout = self.intern_layout(Layout::new(LayoutKind::List {
                                element: element_layout,
                            }))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::List { element },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Option => {
                            let (element, element_layout) =
                                values.pop().expect("option element should exist");
                            let layout = self.intern_layout(Layout::new(LayoutKind::Option {
                                element: element_layout,
                            }))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Option { element },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Result => {
                            let lowered = drain_tail(&mut values, 2);
                            let layout = self.intern_layout(Layout::new(LayoutKind::Result {
                                error: lowered[0].1,
                                value: lowered[1].1,
                            }))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Result {
                                        error: lowered[0].0,
                                        value: lowered[1].0,
                                    },
                                },
                                layout,
                            )
                        }
                        DecodeBuildTask::Validation => {
                            let lowered = drain_tail(&mut values, 2);
                            let layout =
                                self.intern_layout(Layout::new(LayoutKind::Validation {
                                    error: lowered[0].1,
                                    value: lowered[1].1,
                                }))?;
                            (
                                DecodeStep {
                                    layout,
                                    kind: DecodeStepKind::Validation {
                                        error: lowered[0].0,
                                        value: lowered[1].0,
                                    },
                                },
                                layout,
                            )
                        }
                    };
                    let step_id = steps
                        .alloc(step)
                        .map_err(|overflow| arena_overflow("decode steps", overflow))?;
                    values.push((step_id, layout));
                }
            }
        }

        let (root, _) = values
            .pop()
            .expect("decode lowering should produce one root step");
        self.program
            .decode_plans_mut()
            .alloc(DecodePlan::new(
                owner,
                map_decode_mode(decode.mode),
                root,
                steps,
            ))
            .map_err(|overflow| arena_overflow("decode plans", overflow))
    }

    fn lower_kernel(
        &mut self,
        kind: KernelOriginKind,
        closure_id: lambda::ClosureId,
    ) -> Result<KernelId, LoweringError> {
        let closure = self.lambda.closures()[closure_id].clone();
        let owner = self.require_item(closure.owner, closure.span)?;
        let input_hint = closure
            .ambient_subject
            .as_ref()
            .map(|subject| self.intern_core_type(subject))
            .transpose()?;
        let contract = self.collect_kernel_contract(&closure, input_hint)?;
        let lowered = self.lower_kernel_exprs(closure.root, input_hint, &contract.env_map)?;
        let result_layout = self.runtime_expr_layout(closure.root)?;
        let input_subject = input_hint.filter(|_| contract.uses_input_subject);
        self.alloc_kernel(
            owner,
            closure.span,
            kind,
            input_subject,
            contract,
            lowered,
            result_layout,
        )
    }

    fn lower_item_body_kernel(
        &mut self,
        item: &lambda::Item,
        owner: ItemId,
        closure_id: lambda::ClosureId,
    ) -> Result<KernelId, LoweringError> {
        let closure = self.lambda.closures()[closure_id].clone();
        debug_assert_eq!(closure.parameters.len(), item.parameters.len());

        let mut environment =
            Vec::with_capacity(item.parameters.len().saturating_add(closure.captures.len()));
        let mut env_map =
            HashMap::with_capacity(item.parameters.len().saturating_add(closure.captures.len()));

        for parameter in &item.parameters {
            let slot = EnvSlotId::from_raw(environment.len() as u32);
            environment.push(self.intern_core_type(&parameter.ty)?);
            env_map.insert(parameter.binding.as_raw(), slot);
        }
        for capture_id in &closure.captures {
            let capture = &self.lambda.captures()[*capture_id];
            let slot = EnvSlotId::from_raw(environment.len() as u32);
            environment.push(self.intern_core_type(&capture.ty)?);
            env_map.insert(capture.binding.as_raw(), slot);
        }

        let contract =
            self.collect_root_kernel_contract(closure.root, None, environment, env_map)?;
        let lowered = self.lower_kernel_exprs(closure.root, None, &contract.env_map)?;
        let result_layout = self.runtime_item_body_layout(item, closure.root)?;
        self.alloc_kernel(
            owner,
            closure.span,
            KernelOriginKind::ItemBody { item: owner },
            None,
            contract,
            lowered,
            result_layout,
        )
    }

    fn lower_root_expr_kernel(
        &mut self,
        owner: ItemId,
        span: SourceSpan,
        kind: KernelOriginKind,
        root: core::ExprId,
    ) -> Result<KernelId, LoweringError> {
        let contract = self.collect_root_kernel_contract(root, None, Vec::new(), HashMap::new())?;
        let lowered = self.lower_kernel_exprs(root, None, &contract.env_map)?;
        let result_layout = self.runtime_expr_layout(root)?;
        self.alloc_kernel(owner, span, kind, None, contract, lowered, result_layout)
    }

    fn alloc_kernel(
        &mut self,
        owner: ItemId,
        span: SourceSpan,
        kind: KernelOriginKind,
        input_subject: Option<LayoutId>,
        contract: KernelContract,
        lowered: LoweredKernelExprs,
        result_layout: LayoutId,
    ) -> Result<KernelId, LoweringError> {
        let convention =
            self.build_calling_convention(input_subject, &contract.environment, result_layout);
        self.program
            .kernels_mut()
            .alloc(Kernel::new(
                KernelOrigin {
                    item: owner,
                    span,
                    kind,
                },
                input_subject,
                lowered.inline_subjects,
                contract.environment,
                result_layout,
                convention,
                contract.global_items,
                lowered.root,
                lowered.exprs,
            ))
            .map_err(|overflow| arena_overflow("kernels", overflow))
    }

    fn collect_kernel_contract(
        &mut self,
        closure: &lambda::Closure,
        input_hint: Option<LayoutId>,
    ) -> Result<KernelContract, LoweringError> {
        let mut environment = Vec::with_capacity(closure.captures.len());
        let mut env_map = HashMap::with_capacity(closure.captures.len());
        for (index, capture_id) in closure.captures.iter().enumerate() {
            let capture = &self.lambda.captures()[*capture_id];
            environment.push(self.intern_core_type(&capture.ty)?);
            env_map.insert(capture.binding.as_raw(), EnvSlotId::from_raw(index as u32));
        }
        self.collect_root_kernel_contract(closure.root, input_hint, environment, env_map)
    }

    fn collect_root_kernel_contract(
        &mut self,
        root: core::ExprId,
        input_hint: Option<LayoutId>,
        environment: Vec<LayoutId>,
        env_map: HashMap<u32, EnvSlotId>,
    ) -> Result<KernelContract, LoweringError> {
        let mut globals = BTreeSet::new();
        let mut uses_input_subject = false;
        let mut work = vec![(root, SubjectKind::Input)];

        while let Some((expr_id, subject)) = work.pop() {
            let expr = &self.lambda.exprs()[expr_id];
            match &expr.kind {
                core::ExprKind::AmbientSubject => {
                    if matches!(subject, SubjectKind::Input) {
                        if input_hint.is_none() {
                            return Err(MissingInputSubjectContract { span: expr.span });
                        }
                        uses_input_subject = true;
                    }
                }
                core::ExprKind::OptionSome { payload } => work.push((*payload, subject)),
                core::ExprKind::OptionNone
                | core::ExprKind::Integer(_)
                | core::ExprKind::Float(_)
                | core::ExprKind::Decimal(_)
                | core::ExprKind::BigInt(_)
                | core::ExprKind::SuffixedInteger(_) => {}
                core::ExprKind::Reference(reference) => match reference {
                    core::Reference::Local(_) => {}
                    core::Reference::Item(item) => {
                        globals.insert(self.require_item(*item, expr.span)?);
                    }
                    core::Reference::SumConstructor(_) => {}
                    core::Reference::DomainMember(_) => {}
                    core::Reference::BuiltinClassMember(_) => {}
                    core::Reference::IntrinsicValue(_) => {}
                    core::Reference::HirItem(_) => {
                        return Err(UnresolvedItemReference { span: expr.span });
                    }
                    core::Reference::Builtin(_) => {}
                },
                core::ExprKind::Text(text) => {
                    for segment in text.segments.iter().rev() {
                        if let core::TextSegment::Interpolation { expr, .. } = segment {
                            work.push((*expr, subject));
                        }
                    }
                }
                core::ExprKind::Tuple(elements)
                | core::ExprKind::List(elements)
                | core::ExprKind::Set(elements) => {
                    for child in elements.iter().rev() {
                        work.push((*child, subject));
                    }
                }
                core::ExprKind::Map(entries) => {
                    for entry in entries.iter().rev() {
                        work.push((entry.value, subject));
                        work.push((entry.key, subject));
                    }
                }
                core::ExprKind::Record(fields) => {
                    for field in fields.iter().rev() {
                        work.push((field.value, subject));
                    }
                }
                core::ExprKind::Projection { base, .. } => match base {
                    core::ProjectionBase::AmbientSubject => {
                        if matches!(subject, SubjectKind::Input) {
                            if input_hint.is_none() {
                                return Err(MissingInputSubjectContract { span: expr.span });
                            }
                            uses_input_subject = true;
                        }
                    }
                    core::ProjectionBase::Expr(base) => work.push((*base, subject)),
                },
                core::ExprKind::Apply { callee, arguments } => {
                    for argument in arguments.iter().rev() {
                        work.push((*argument, subject));
                    }
                    work.push((*callee, subject));
                }
                core::ExprKind::Unary { expr, .. } => work.push((*expr, subject)),
                core::ExprKind::Binary { left, right, .. } => {
                    work.push((*right, subject));
                    work.push((*left, subject));
                }
                core::ExprKind::Pipe(pipe) => {
                    for stage in pipe.stages.iter().rev() {
                        match &stage.kind {
                            core::PipeStageKind::Transform { expr }
                            | core::PipeStageKind::Tap { expr } => {
                                work.push((*expr, SubjectKind::Inline));
                            }
                            core::PipeStageKind::Gate { predicate, .. } => {
                                work.push((*predicate, SubjectKind::Inline));
                            }
                            core::PipeStageKind::Case { arms } => {
                                for arm in arms.iter().rev() {
                                    work.push((arm.body, SubjectKind::Inline));
                                }
                            }
                            core::PipeStageKind::TruthyFalsy(pair) => {
                                work.push((pair.falsy.body, SubjectKind::Inline));
                                work.push((pair.truthy.body, SubjectKind::Inline));
                            }
                        }
                    }
                    work.push((pipe.head, subject));
                }
            }
        }

        Ok(KernelContract {
            uses_input_subject,
            environment,
            env_map,
            global_items: globals.into_iter().collect(),
        })
    }
    fn lower_kernel_exprs(
        &mut self,
        root: core::ExprId,
        input_hint: Option<LayoutId>,
        env_map: &HashMap<u32, EnvSlotId>,
    ) -> Result<LoweredKernelExprs, LoweringError> {
        enum Task {
            Visit(core::ExprId, Option<SubjectContext>, LocalBindings),
            BuildOptionSome {
                span: SourceSpan,
                layout: LayoutId,
            },
            BuildText {
                span: SourceSpan,
                layout: LayoutId,
                segments: Vec<SegmentSpec>,
            },
            BuildTuple {
                span: SourceSpan,
                layout: LayoutId,
                len: usize,
            },
            BuildList {
                span: SourceSpan,
                layout: LayoutId,
                len: usize,
            },
            BuildMap {
                span: SourceSpan,
                layout: LayoutId,
                entries: usize,
            },
            BuildSet {
                span: SourceSpan,
                layout: LayoutId,
                len: usize,
            },
            BuildRecord {
                span: SourceSpan,
                layout: LayoutId,
                labels: Vec<Box<str>>,
            },
            BuildProjection {
                span: SourceSpan,
                layout: LayoutId,
                base: ProjectionBaseBuild,
                path: Vec<Box<str>>,
            },
            BuildApply {
                span: SourceSpan,
                layout: LayoutId,
                arguments: usize,
            },
            BuildUnary {
                span: SourceSpan,
                layout: LayoutId,
                operator: UnaryOperator,
            },
            BuildBinary {
                span: SourceSpan,
                layout: LayoutId,
                operator: BinaryOperator,
            },
            BuildPipe {
                span: SourceSpan,
                layout: LayoutId,
                stages: Vec<InlinePipeStageSpec>,
            },
        }

        enum SegmentSpec {
            Fragment { raw: Box<str>, span: SourceSpan },
            Interpolation { span: SourceSpan },
        }

        enum ProjectionBaseBuild {
            Subject(SubjectRef),
            Expr,
        }

        #[derive(Clone)]
        struct InlinePipeStageSpec {
            subject: InlineSubjectId,
            span: SourceSpan,
            input_layout: LayoutId,
            result_layout: LayoutId,
            kind: InlinePipeStageBuild,
        }

        #[derive(Clone)]
        enum InlinePipeStageBuild {
            Transform,
            Tap,
            Gate {
                emits_negative_update: bool,
            },
            Case {
                arms: Vec<InlinePipeCaseArmSpec>,
            },
            TruthyFalsy {
                truthy: InlinePipeTruthyFalsyBranchSpec,
                falsy: InlinePipeTruthyFalsyBranchSpec,
            },
        }

        impl InlinePipeStageSpec {
            fn child_count(&self) -> usize {
                match &self.kind {
                    InlinePipeStageBuild::Transform
                    | InlinePipeStageBuild::Tap
                    | InlinePipeStageBuild::Gate { .. } => 1,
                    InlinePipeStageBuild::Case { arms } => arms.len(),
                    InlinePipeStageBuild::TruthyFalsy { .. } => 2,
                }
            }
        }

        struct PipeChildSpec {
            expr: core::ExprId,
            subject: Option<SubjectContext>,
            locals: LocalBindings,
        }

        let mut tasks = vec![Task::Visit(
            root,
            input_hint.map(|layout| SubjectContext {
                reference: SubjectRef::Input,
                layout,
            }),
            LocalBindings::default(),
        )];
        let mut values = Vec::new();
        let mut exprs = Arena::new();
        let mut inline_subjects = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr_id, subject, locals) => {
                    let expr = &self.lambda.exprs()[expr_id];
                    let layout = self.runtime_expr_layout(expr_id)?;
                    match &expr.kind {
                        core::ExprKind::AmbientSubject => {
                            let subject =
                                subject.ok_or(MissingInputSubjectContract { span: expr.span })?;
                            if subject.layout != layout {
                                return Err(SubjectLayoutMismatch {
                                    span: expr.span,
                                    expected: subject.layout,
                                    found: layout,
                                });
                            }
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::Subject(subject.reference),
                                },
                            )?);
                        }
                        core::ExprKind::OptionSome { payload } => {
                            tasks.push(Task::BuildOptionSome {
                                span: expr.span,
                                layout,
                            });
                            tasks.push(Task::Visit(*payload, subject, locals));
                        }
                        core::ExprKind::OptionNone => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::OptionNone,
                                },
                            )?);
                        }
                        core::ExprKind::Reference(reference) => {
                            let kind = match reference {
                                core::Reference::Local(binding) => {
                                    if let Some(local) = locals.get(binding.as_raw()) {
                                        if local.layout != layout {
                                            return Err(SubjectLayoutMismatch {
                                                span: expr.span,
                                                expected: local.layout,
                                                found: layout,
                                            });
                                        }
                                        KernelExprKind::Subject(local.reference)
                                    } else {
                                        let Some(slot) = env_map.get(&binding.as_raw()).copied()
                                        else {
                                            return Err(UnsupportedLocalReference {
                                                span: expr.span,
                                                binding: binding.as_raw(),
                                            });
                                        };
                                        KernelExprKind::Environment(slot)
                                    }
                                }
                                core::Reference::Item(item) => {
                                    KernelExprKind::Item(self.require_item(*item, expr.span)?)
                                }
                                core::Reference::SumConstructor(handle) => {
                                    KernelExprKind::SumConstructor(handle.clone())
                                }
                                core::Reference::DomainMember(handle) => {
                                    KernelExprKind::DomainMember(handle.clone())
                                }
                                core::Reference::BuiltinClassMember(intrinsic) => {
                                    KernelExprKind::BuiltinClassMember(
                                        map_builtin_class_member_intrinsic(*intrinsic),
                                    )
                                }
                                core::Reference::HirItem(_) => {
                                    return Err(UnresolvedItemReference { span: expr.span });
                                }
                                core::Reference::Builtin(term) => {
                                    KernelExprKind::Builtin(map_builtin_term(*term))
                                }
                                core::Reference::IntrinsicValue(value) => {
                                    KernelExprKind::IntrinsicValue(*value)
                                }
                            };
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind,
                                },
                            )?);
                        }
                        core::ExprKind::Integer(integer) => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::Integer(IntegerLiteral {
                                        raw: integer.raw.clone(),
                                    }),
                                },
                            )?);
                        }
                        core::ExprKind::Float(float) => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::Float(FloatLiteral {
                                        raw: float.raw.clone(),
                                    }),
                                },
                            )?);
                        }
                        core::ExprKind::Decimal(decimal) => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::Decimal(DecimalLiteral {
                                        raw: decimal.raw.clone(),
                                    }),
                                },
                            )?);
                        }
                        core::ExprKind::BigInt(bigint) => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::BigInt(BigIntLiteral {
                                        raw: bigint.raw.clone(),
                                    }),
                                },
                            )?);
                        }
                        core::ExprKind::SuffixedInteger(integer) => {
                            values.push(alloc_kernel_expr(
                                &mut exprs,
                                KernelExpr {
                                    span: expr.span,
                                    layout,
                                    kind: KernelExprKind::SuffixedInteger(SuffixedIntegerLiteral {
                                        raw: integer.raw.clone(),
                                        suffix: integer.suffix.text().into(),
                                    }),
                                },
                            )?);
                        }
                        core::ExprKind::Text(text) => {
                            tasks.push(Task::BuildText {
                                span: expr.span,
                                layout,
                                segments: text
                                    .segments
                                    .iter()
                                    .map(|segment| match segment {
                                        core::TextSegment::Fragment { raw, span } => {
                                            SegmentSpec::Fragment {
                                                raw: raw.clone(),
                                                span: *span,
                                            }
                                        }
                                        core::TextSegment::Interpolation { span, .. } => {
                                            SegmentSpec::Interpolation { span: *span }
                                        }
                                    })
                                    .collect(),
                            });
                            for segment in text.segments.iter().rev() {
                                if let core::TextSegment::Interpolation { expr, .. } = segment {
                                    tasks.push(Task::Visit(*expr, subject, locals.clone()));
                                }
                            }
                        }
                        core::ExprKind::Tuple(elements) => {
                            tasks.push(Task::BuildTuple {
                                span: expr.span,
                                layout,
                                len: elements.len(),
                            });
                            for child in elements.iter().rev() {
                                tasks.push(Task::Visit(*child, subject, locals.clone()));
                            }
                        }
                        core::ExprKind::List(elements) => {
                            tasks.push(Task::BuildList {
                                span: expr.span,
                                layout,
                                len: elements.len(),
                            });
                            for child in elements.iter().rev() {
                                tasks.push(Task::Visit(*child, subject, locals.clone()));
                            }
                        }
                        core::ExprKind::Map(entries) => {
                            tasks.push(Task::BuildMap {
                                span: expr.span,
                                layout,
                                entries: entries.len(),
                            });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(entry.value, subject, locals.clone()));
                                tasks.push(Task::Visit(entry.key, subject, locals.clone()));
                            }
                        }
                        core::ExprKind::Set(elements) => {
                            tasks.push(Task::BuildSet {
                                span: expr.span,
                                layout,
                                len: elements.len(),
                            });
                            for child in elements.iter().rev() {
                                tasks.push(Task::Visit(*child, subject, locals.clone()));
                            }
                        }
                        core::ExprKind::Record(fields) => {
                            tasks.push(Task::BuildRecord {
                                span: expr.span,
                                layout,
                                labels: fields.iter().map(|field| field.label.clone()).collect(),
                            });
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(field.value, subject, locals.clone()));
                            }
                        }
                        core::ExprKind::Projection { base, path } => {
                            let build_base = match base {
                                core::ProjectionBase::AmbientSubject => {
                                    ProjectionBaseBuild::Subject(
                                        subject
                                            .ok_or(MissingInputSubjectContract { span: expr.span })?
                                            .reference,
                                    )
                                }
                                core::ProjectionBase::Expr(_) => ProjectionBaseBuild::Expr,
                            };
                            tasks.push(Task::BuildProjection {
                                span: expr.span,
                                layout,
                                base: build_base,
                                path: path.clone(),
                            });
                            if let core::ProjectionBase::Expr(base_expr) = base {
                                tasks.push(Task::Visit(*base_expr, subject, locals));
                            }
                        }
                        core::ExprKind::Apply { callee, arguments } => {
                            tasks.push(Task::BuildApply {
                                span: expr.span,
                                layout,
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(*argument, subject, locals.clone()));
                            }
                            tasks.push(Task::Visit(*callee, subject, locals));
                        }
                        core::ExprKind::Unary {
                            operator,
                            expr: inner,
                        } => {
                            tasks.push(Task::BuildUnary {
                                span: expr.span,
                                layout,
                                operator: map_unary_operator(*operator),
                            });
                            tasks.push(Task::Visit(*inner, subject, locals));
                        }
                        core::ExprKind::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            tasks.push(Task::BuildBinary {
                                span: expr.span,
                                layout,
                                operator: map_binary_operator(*operator),
                            });
                            tasks.push(Task::Visit(*right, subject, locals.clone()));
                            tasks.push(Task::Visit(*left, subject, locals));
                        }
                        core::ExprKind::Pipe(pipe) => {
                            let mut stage_specs = Vec::with_capacity(pipe.stages.len());
                            let mut children = Vec::new();
                            for stage in &pipe.stages {
                                let input_layout = self.intern_core_type(&stage.input_subject)?;
                                let result_layout = self.intern_core_type(&stage.result_subject)?;
                                let subject_slot =
                                    alloc_inline_subject(&mut inline_subjects, input_layout)?;
                                let child_subject = Some(SubjectContext {
                                    reference: SubjectRef::Inline(subject_slot),
                                    layout: input_layout,
                                });
                                let kind = match &stage.kind {
                                    core::PipeStageKind::Transform { expr } => {
                                        children.push(PipeChildSpec {
                                            expr: *expr,
                                            subject: child_subject,
                                            locals: locals.clone(),
                                        });
                                        InlinePipeStageBuild::Transform
                                    }
                                    core::PipeStageKind::Tap { expr } => {
                                        children.push(PipeChildSpec {
                                            expr: *expr,
                                            subject: child_subject,
                                            locals: locals.clone(),
                                        });
                                        InlinePipeStageBuild::Tap
                                    }
                                    core::PipeStageKind::Gate {
                                        predicate,
                                        emits_negative_update,
                                    } => {
                                        children.push(PipeChildSpec {
                                            expr: *predicate,
                                            subject: child_subject,
                                            locals: locals.clone(),
                                        });
                                        InlinePipeStageBuild::Gate {
                                            emits_negative_update: *emits_negative_update,
                                        }
                                    }
                                    core::PipeStageKind::Case { arms } => {
                                        let mut lowered_arms = Vec::with_capacity(arms.len());
                                        for arm in arms {
                                            let mut arm_locals = locals.clone();
                                            let pattern = self.lower_inline_pipe_pattern(
                                                &arm.pattern,
                                                input_layout,
                                                &mut inline_subjects,
                                                &mut arm_locals,
                                            )?;
                                            children.push(PipeChildSpec {
                                                expr: arm.body,
                                                subject: child_subject,
                                                locals: arm_locals,
                                            });
                                            lowered_arms.push(InlinePipeCaseArmSpec {
                                                span: arm.span,
                                                pattern,
                                            });
                                        }
                                        InlinePipeStageBuild::Case { arms: lowered_arms }
                                    }
                                    core::PipeStageKind::TruthyFalsy(stage_pair) => {
                                        let truthy = self.lower_inline_truthy_falsy_branch_spec(
                                            &stage_pair.truthy,
                                            &mut inline_subjects,
                                        )?;
                                        let falsy = self.lower_inline_truthy_falsy_branch_spec(
                                            &stage_pair.falsy,
                                            &mut inline_subjects,
                                        )?;
                                        children.push(PipeChildSpec {
                                            expr: stage_pair.truthy.body,
                                            subject: truthy.payload_subject.map(|slot| {
                                                SubjectContext {
                                                    reference: SubjectRef::Inline(slot),
                                                    layout: inline_subjects[slot.index()],
                                                }
                                            }),
                                            locals: locals.clone(),
                                        });
                                        children.push(PipeChildSpec {
                                            expr: stage_pair.falsy.body,
                                            subject: falsy.payload_subject.map(|slot| {
                                                SubjectContext {
                                                    reference: SubjectRef::Inline(slot),
                                                    layout: inline_subjects[slot.index()],
                                                }
                                            }),
                                            locals: locals.clone(),
                                        });
                                        InlinePipeStageBuild::TruthyFalsy { truthy, falsy }
                                    }
                                };
                                stage_specs.push(InlinePipeStageSpec {
                                    subject: subject_slot,
                                    span: stage.span,
                                    input_layout,
                                    result_layout,
                                    kind,
                                });
                            }
                            tasks.push(Task::BuildPipe {
                                span: expr.span,
                                layout,
                                stages: stage_specs,
                            });
                            for child in children.into_iter().rev() {
                                tasks.push(Task::Visit(child.expr, child.subject, child.locals));
                            }
                            tasks.push(Task::Visit(pipe.head, subject, locals));
                        }
                    }
                }
                Task::BuildOptionSome { span, layout } => {
                    let payload = values.pop().expect("option payload should exist");
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::OptionSome { payload },
                        },
                    )?);
                }
                Task::BuildText {
                    span,
                    layout,
                    segments,
                } => {
                    let interpolation_count = segments
                        .iter()
                        .filter(|segment| matches!(segment, SegmentSpec::Interpolation { .. }))
                        .count();
                    let mut lowered = drain_tail(&mut values, interpolation_count).into_iter();
                    let segments = segments
                        .into_iter()
                        .map(|segment| match segment {
                            SegmentSpec::Fragment { raw, span } => {
                                TextSegment::Fragment { raw, span }
                            }
                            SegmentSpec::Interpolation { span } => TextSegment::Interpolation {
                                expr: lowered
                                    .next()
                                    .expect("text interpolation count should match"),
                                span,
                            },
                        })
                        .collect();
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Text(TextLiteral { segments }),
                        },
                    )?);
                }
                Task::BuildTuple { span, layout, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Tuple(elements),
                        },
                    )?);
                }
                Task::BuildList { span, layout, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::List(elements),
                        },
                    )?);
                }
                Task::BuildMap {
                    span,
                    layout,
                    entries,
                } => {
                    let lowered = drain_tail(&mut values, entries * 2);
                    let mut iter = lowered.into_iter();
                    let entries = (0..entries)
                        .map(|_| MapEntry {
                            key: iter.next().expect("map key should exist"),
                            value: iter.next().expect("map value should exist"),
                        })
                        .collect();
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Map(entries),
                        },
                    )?);
                }
                Task::BuildSet { span, layout, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Set(elements),
                        },
                    )?);
                }
                Task::BuildRecord {
                    span,
                    layout,
                    labels,
                } => {
                    let len = labels.len();
                    let fields = labels
                        .into_iter()
                        .zip(drain_tail(&mut values, len))
                        .map(|(label, value)| RecordExprField { label, value })
                        .collect();
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Record(fields),
                        },
                    )?);
                }
                Task::BuildProjection {
                    span,
                    layout,
                    base,
                    path,
                } => {
                    let base = match base {
                        ProjectionBaseBuild::Subject(subject) => ProjectionBase::Subject(subject),
                        ProjectionBaseBuild::Expr => ProjectionBase::Expr(
                            values.pop().expect("projection base should exist"),
                        ),
                    };
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Projection { base, path },
                        },
                    )?);
                }
                Task::BuildApply {
                    span,
                    layout,
                    arguments,
                } => {
                    let lowered = drain_tail(&mut values, arguments + 1);
                    let mut iter = lowered.into_iter();
                    let callee = iter.next().expect("apply callee should exist");
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Apply {
                                callee,
                                arguments: iter.collect(),
                            },
                        },
                    )?);
                }
                Task::BuildUnary {
                    span,
                    layout,
                    operator,
                } => {
                    let inner = values.pop().expect("unary child should exist");
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Unary {
                                operator,
                                expr: inner,
                            },
                        },
                    )?);
                }
                Task::BuildBinary {
                    span,
                    layout,
                    operator,
                } => {
                    let lowered = drain_tail(&mut values, 2);
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Binary {
                                left: lowered[0],
                                operator,
                                right: lowered[1],
                            },
                        },
                    )?);
                }
                Task::BuildPipe {
                    span,
                    layout,
                    stages,
                } => {
                    let lowered = drain_tail(
                        &mut values,
                        1 + stages
                            .iter()
                            .map(InlinePipeStageSpec::child_count)
                            .sum::<usize>(),
                    );
                    let mut iter = lowered.into_iter();
                    let head = iter.next().expect("pipe head should exist");
                    let stages = stages
                        .into_iter()
                        .map(|stage| InlinePipeStage {
                            subject: stage.subject,
                            span: stage.span,
                            input_layout: stage.input_layout,
                            result_layout: stage.result_layout,
                            kind: match stage.kind {
                                InlinePipeStageBuild::Transform => {
                                    let expr = iter.next().expect("pipe stage child should exist");
                                    InlinePipeStageKind::Transform { expr }
                                }
                                InlinePipeStageBuild::Tap => {
                                    let expr = iter.next().expect("pipe stage child should exist");
                                    InlinePipeStageKind::Tap { expr }
                                }
                                InlinePipeStageBuild::Gate {
                                    emits_negative_update,
                                } => {
                                    let expr = iter.next().expect("pipe stage child should exist");
                                    InlinePipeStageKind::Gate {
                                        predicate: expr,
                                        emits_negative_update,
                                    }
                                }
                                InlinePipeStageBuild::Case { arms } => InlinePipeStageKind::Case {
                                    arms: arms
                                        .into_iter()
                                        .map(|arm| InlinePipeCaseArm {
                                            span: arm.span,
                                            pattern: arm.pattern,
                                            body: iter.next().expect("case arm child should exist"),
                                        })
                                        .collect(),
                                },
                                InlinePipeStageBuild::TruthyFalsy { truthy, falsy } => {
                                    InlinePipeStageKind::TruthyFalsy {
                                        truthy: InlinePipeTruthyFalsyBranch {
                                            span: truthy.span,
                                            constructor: truthy.constructor,
                                            payload_subject: truthy.payload_subject,
                                            body: iter
                                                .next()
                                                .expect("truthy branch child should exist"),
                                        },
                                        falsy: InlinePipeTruthyFalsyBranch {
                                            span: falsy.span,
                                            constructor: falsy.constructor,
                                            payload_subject: falsy.payload_subject,
                                            body: iter
                                                .next()
                                                .expect("falsy branch child should exist"),
                                        },
                                    }
                                }
                            },
                        })
                        .collect();
                    values.push(alloc_kernel_expr(
                        &mut exprs,
                        KernelExpr {
                            span,
                            layout,
                            kind: KernelExprKind::Pipe(InlinePipeExpr { head, stages }),
                        },
                    )?);
                }
            }
        }

        Ok(LoweredKernelExprs {
            root: values
                .pop()
                .expect("kernel lowering should yield one root expression"),
            inline_subjects,
            exprs,
        })
    }

    fn lower_inline_truthy_falsy_branch_spec(
        &mut self,
        branch: &core::PipeTruthyFalsyBranch,
        inline_subjects: &mut Vec<LayoutId>,
    ) -> Result<InlinePipeTruthyFalsyBranchSpec, LoweringError> {
        let payload_subject = branch
            .payload_subject
            .as_ref()
            .map(|payload| {
                let layout = self.intern_core_type(payload)?;
                alloc_inline_subject(inline_subjects, layout)
            })
            .transpose()?;
        Ok(InlinePipeTruthyFalsyBranchSpec {
            span: branch.span,
            constructor: map_builtin_term(branch.constructor),
            payload_subject,
        })
    }

    fn lower_inline_pipe_pattern(
        &mut self,
        pattern: &core::Pattern,
        layout: LayoutId,
        inline_subjects: &mut Vec<LayoutId>,
        locals: &mut LocalBindings,
    ) -> Result<InlinePipePattern, LoweringError> {
        let kind = match &pattern.kind {
            core::PatternKind::Wildcard => InlinePipePatternKind::Wildcard,
            core::PatternKind::Binding(binding) => {
                let subject = alloc_inline_subject(inline_subjects, layout)?;
                locals.insert(
                    binding.binding.as_raw(),
                    SubjectContext {
                        reference: SubjectRef::Inline(subject),
                        layout,
                    },
                );
                InlinePipePatternKind::Binding { subject }
            }
            core::PatternKind::Integer(integer) => {
                let LayoutKind::Primitive(PrimitiveType::Int) =
                    &self.program.layouts()[layout].kind
                else {
                    return Err(UnsupportedInlinePipePattern { span: pattern.span });
                };
                InlinePipePatternKind::Integer(IntegerLiteral {
                    raw: integer.raw.clone(),
                })
            }
            core::PatternKind::Text(raw) => {
                let LayoutKind::Primitive(PrimitiveType::Text) =
                    &self.program.layouts()[layout].kind
                else {
                    return Err(UnsupportedInlinePipePattern { span: pattern.span });
                };
                InlinePipePatternKind::Text(raw.clone())
            }
            core::PatternKind::Tuple(elements) => {
                let layouts = match &self.program.layouts()[layout].kind {
                    LayoutKind::Tuple(layouts) => layouts.clone(),
                    _ => return Err(UnsupportedInlinePipePattern { span: pattern.span }),
                };
                if layouts.len() != elements.len() {
                    return Err(UnsupportedInlinePipePattern { span: pattern.span });
                }
                InlinePipePatternKind::Tuple(
                    elements
                        .iter()
                        .zip(layouts.into_iter())
                        .map(|(element, layout)| {
                            self.lower_inline_pipe_pattern(element, layout, inline_subjects, locals)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            core::PatternKind::Record(fields) => {
                let layout_fields = match &self.program.layouts()[layout].kind {
                    LayoutKind::Record(layout_fields) => layout_fields.clone(),
                    _ => return Err(UnsupportedInlinePipePattern { span: pattern.span }),
                };
                InlinePipePatternKind::Record(
                    fields
                        .iter()
                        .map(|field| {
                            let Some(layout_field) = layout_fields
                                .iter()
                                .find(|candidate| candidate.name.as_ref() == field.label.as_ref())
                            else {
                                return Err(UnsupportedInlinePipePattern { span: pattern.span });
                            };
                            Ok(InlinePipeRecordPatternField {
                                label: field.label.clone(),
                                pattern: self.lower_inline_pipe_pattern(
                                    &field.pattern,
                                    layout_field.layout,
                                    inline_subjects,
                                    locals,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            core::PatternKind::Constructor { callee, arguments } => {
                let (constructor, argument_layouts) = match &callee.reference {
                    core::Reference::Builtin(term) => {
                        let constructor = map_builtin_term(*term);
                        let argument_layouts =
                            match (constructor, &self.program.layouts()[layout].kind) {
                                (
                                    BuiltinTerm::True | BuiltinTerm::False,
                                    LayoutKind::Primitive(PrimitiveType::Bool),
                                ) => Vec::new(),
                                (BuiltinTerm::None, LayoutKind::Option { .. }) => Vec::new(),
                                (BuiltinTerm::Some, LayoutKind::Option { element }) => {
                                    vec![*element]
                                }
                                (BuiltinTerm::Ok, LayoutKind::Result { value, .. }) => vec![*value],
                                (BuiltinTerm::Err, LayoutKind::Result { error, .. }) => {
                                    vec![*error]
                                }
                                (BuiltinTerm::Valid, LayoutKind::Validation { value, .. }) => {
                                    vec![*value]
                                }
                                (BuiltinTerm::Invalid, LayoutKind::Validation { error, .. }) => {
                                    vec![*error]
                                }
                                _ => {
                                    return Err(UnsupportedInlinePipePattern {
                                        span: pattern.span,
                                    });
                                }
                            };
                        (
                            InlinePipeConstructor::Builtin(constructor),
                            argument_layouts,
                        )
                    }
                    core::Reference::SumConstructor(handle) => {
                        let matches_layout = match &self.program.layouts()[layout].kind {
                            LayoutKind::Opaque { name, .. } => {
                                name.as_ref() == handle.type_name.as_ref()
                            }
                            LayoutKind::Sum(variants) => variants.iter().any(|variant| {
                                variant.name.as_ref() == handle.variant_name.as_ref()
                            }),
                            _ => false,
                        };
                        if !matches_layout {
                            return Err(UnsupportedInlinePipePattern { span: pattern.span });
                        }
                        let field_types = callee
                            .field_types
                            .as_ref()
                            .ok_or(UnsupportedInlinePipePattern { span: pattern.span })?;
                        let argument_layouts = field_types
                            .iter()
                            .map(|field| self.intern_core_type(field))
                            .collect::<Result<Vec<_>, _>>()?;
                        (InlinePipeConstructor::Sum(handle.clone()), argument_layouts)
                    }
                    _ => return Err(UnsupportedInlinePipePattern { span: pattern.span }),
                };
                if arguments.len() != argument_layouts.len() {
                    return Err(UnsupportedInlinePipePattern { span: pattern.span });
                }
                InlinePipePatternKind::Constructor {
                    constructor,
                    arguments: arguments
                        .iter()
                        .zip(argument_layouts.into_iter())
                        .map(|(argument, layout)| {
                            self.lower_inline_pipe_pattern(
                                argument,
                                layout,
                                inline_subjects,
                                locals,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
        };
        Ok(InlinePipePattern {
            span: pattern.span,
            kind,
        })
    }

    fn build_calling_convention(
        &self,
        input_subject: Option<LayoutId>,
        environment: &[LayoutId],
        result_layout: LayoutId,
    ) -> CallingConvention {
        let mut parameters =
            Vec::with_capacity(environment.len() + usize::from(input_subject.is_some()));
        if let Some(layout) = input_subject {
            parameters.push(AbiParameter {
                role: ParameterRole::InputSubject,
                layout,
                pass_mode: self.program.layouts()[layout].abi,
            });
        }
        for (index, layout) in environment.iter().enumerate() {
            parameters.push(AbiParameter {
                role: ParameterRole::Environment(EnvSlotId::from_raw(index as u32)),
                layout: *layout,
                pass_mode: self.program.layouts()[*layout].abi,
            });
        }
        CallingConvention {
            kind: CallingConventionKind::RuntimeKernelV1,
            parameters,
            result: AbiResult {
                layout: result_layout,
                pass_mode: self.program.layouts()[result_layout].abi,
            },
        }
    }

    fn require_item(&self, item: core::ItemId, span: SourceSpan) -> Result<ItemId, LoweringError> {
        self.item_map
            .get(&item)
            .copied()
            .ok_or(UnknownLambdaItem { item, span })
    }

    fn runtime_expr_layout(&mut self, expr_id: core::ExprId) -> Result<LayoutId, LoweringError> {
        let ty = self.runtime_expr_type(expr_id);
        self.intern_core_type(&ty)
    }

    fn runtime_item_body_layout(
        &mut self,
        item: &lambda::Item,
        expr_id: core::ExprId,
    ) -> Result<LayoutId, LoweringError> {
        let ty = match (&item.kind, self.runtime_expr_type(expr_id)) {
            (core::ItemKind::Signal(_), core::Type::Signal(payload)) => *payload,
            (_, ty) => ty,
        };
        self.intern_core_type(&ty)
    }

    fn runtime_expr_type(&self, expr_id: core::ExprId) -> core::Type {
        let expr = &self.lambda.exprs()[expr_id];
        match &expr.kind {
            core::ExprKind::Apply { callee, arguments } => {
                applied_result_type(self.lambda.exprs()[*callee].ty.clone(), arguments.len())
                    .unwrap_or_else(|| expr.ty.clone())
            }
            core::ExprKind::Pipe(pipe) => pipe
                .stages
                .last()
                .map(|stage| stage.result_subject.clone())
                .unwrap_or_else(|| expr.ty.clone()),
            _ => expr.ty.clone(),
        }
    }

    fn intern_layout(&mut self, layout: Layout) -> Result<LayoutId, LoweringError> {
        if let Some(id) = self.layout_interner.get(&layout).copied() {
            return Ok(id);
        }
        let id = self
            .program
            .layouts_mut()
            .alloc(layout.clone())
            .map_err(|overflow| arena_overflow("layouts", overflow))?;
        self.layout_interner.insert(layout, id);
        Ok(id)
    }

    fn intern_core_type(&mut self, root: &core::Type) -> Result<LayoutId, LoweringError> {
        if let Some(id) = self.core_layouts.get(root).copied() {
            return Ok(id);
        }

        enum Task<'a> {
            Visit(&'a core::Type),
            Build(core::Type, TypeBuildTask),
        }

        enum TypeBuildTask {
            Primitive(PrimitiveType),
            Tuple(usize),
            Record(Vec<Box<str>>),
            Arrow,
            List,
            Map,
            Set,
            Option,
            Result,
            Validation,
            Signal,
            Task,
            Domain { name: Box<str>, arguments: usize },
            Opaque { name: Box<str>, arguments: usize },
        }

        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(ty) => {
                    if let Some(id) = self.core_layouts.get(ty).copied() {
                        values.push(id);
                        continue;
                    }
                    match ty {
                        core::Type::Primitive(builtin) => tasks.push(Task::Build(
                            ty.clone(),
                            TypeBuildTask::Primitive(PrimitiveType::from_builtin(*builtin)),
                        )),
                        core::Type::TypeParameter { name, .. } => {
                            return Err(OpenTypeParameter { name: name.clone() });
                        }
                        core::Type::Tuple(elements) => {
                            tasks.push(Task::Build(
                                ty.clone(),
                                TypeBuildTask::Tuple(elements.len()),
                            ));
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        core::Type::Record(fields) => {
                            tasks.push(Task::Build(
                                ty.clone(),
                                TypeBuildTask::Record(
                                    fields.iter().map(|field| field.name.clone()).collect(),
                                ),
                            ));
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(&field.ty));
                            }
                        }
                        core::Type::Arrow { parameter, result } => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Arrow));
                            tasks.push(Task::Visit(result));
                            tasks.push(Task::Visit(parameter));
                        }
                        core::Type::List(element) => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::List));
                            tasks.push(Task::Visit(element));
                        }
                        core::Type::Map { key, value } => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Map));
                            tasks.push(Task::Visit(value));
                            tasks.push(Task::Visit(key));
                        }
                        core::Type::Set(element) => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Set));
                            tasks.push(Task::Visit(element));
                        }
                        core::Type::Option(element) => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Option));
                            tasks.push(Task::Visit(element));
                        }
                        core::Type::Result { error, value } => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Result));
                            tasks.push(Task::Visit(value));
                            tasks.push(Task::Visit(error));
                        }
                        core::Type::Validation { error, value } => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Validation));
                            tasks.push(Task::Visit(value));
                            tasks.push(Task::Visit(error));
                        }
                        core::Type::Signal(element) => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Signal));
                            tasks.push(Task::Visit(element));
                        }
                        core::Type::Task { error, value } => {
                            tasks.push(Task::Build(ty.clone(), TypeBuildTask::Task));
                            tasks.push(Task::Visit(value));
                            tasks.push(Task::Visit(error));
                        }
                        core::Type::Domain {
                            name, arguments, ..
                        } => {
                            tasks.push(Task::Build(
                                ty.clone(),
                                TypeBuildTask::Domain {
                                    name: name.clone(),
                                    arguments: arguments.len(),
                                },
                            ));
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument));
                            }
                        }
                        core::Type::OpaqueItem {
                            name, arguments, ..
                        } => {
                            tasks.push(Task::Build(
                                ty.clone(),
                                TypeBuildTask::Opaque {
                                    name: name.clone(),
                                    arguments: arguments.len(),
                                },
                            ));
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument));
                            }
                        }
                    }
                }
                Task::Build(cache_key, build) => {
                    let layout = match build {
                        TypeBuildTask::Primitive(primitive) => {
                            Layout::new(LayoutKind::Primitive(primitive))
                        }
                        TypeBuildTask::Tuple(len) => {
                            Layout::new(LayoutKind::Tuple(drain_tail(&mut values, len)))
                        }
                        TypeBuildTask::Record(names) => {
                            let len = names.len();
                            Layout::new(LayoutKind::Record(
                                names
                                    .into_iter()
                                    .zip(drain_tail(&mut values, len))
                                    .map(|(name, layout)| RecordFieldLayout { name, layout })
                                    .collect(),
                            ))
                        }
                        TypeBuildTask::Arrow => {
                            let lowered = drain_tail(&mut values, 2);
                            Layout::new(LayoutKind::Arrow {
                                parameter: lowered[0],
                                result: lowered[1],
                            })
                        }
                        TypeBuildTask::List => Layout::new(LayoutKind::List {
                            element: values.pop().expect("list child should exist"),
                        }),
                        TypeBuildTask::Map => {
                            let lowered = drain_tail(&mut values, 2);
                            Layout::new(LayoutKind::Map {
                                key: lowered[0],
                                value: lowered[1],
                            })
                        }
                        TypeBuildTask::Set => Layout::new(LayoutKind::Set {
                            element: values.pop().expect("set child should exist"),
                        }),
                        TypeBuildTask::Option => Layout::new(LayoutKind::Option {
                            element: values.pop().expect("option child should exist"),
                        }),
                        TypeBuildTask::Result => {
                            let lowered = drain_tail(&mut values, 2);
                            Layout::new(LayoutKind::Result {
                                error: lowered[0],
                                value: lowered[1],
                            })
                        }
                        TypeBuildTask::Validation => {
                            let lowered = drain_tail(&mut values, 2);
                            Layout::new(LayoutKind::Validation {
                                error: lowered[0],
                                value: lowered[1],
                            })
                        }
                        TypeBuildTask::Signal => Layout::new(LayoutKind::Signal {
                            element: values.pop().expect("signal child should exist"),
                        }),
                        TypeBuildTask::Task => {
                            let lowered = drain_tail(&mut values, 2);
                            Layout::new(LayoutKind::Task {
                                error: lowered[0],
                                value: lowered[1],
                            })
                        }
                        TypeBuildTask::Domain { name, arguments } => {
                            Layout::new(LayoutKind::Domain {
                                name,
                                arguments: drain_tail(&mut values, arguments),
                            })
                        }
                        TypeBuildTask::Opaque { name, arguments } => {
                            Layout::new(LayoutKind::Opaque {
                                name,
                                arguments: drain_tail(&mut values, arguments),
                            })
                        }
                    };
                    let id = self.intern_layout(layout)?;
                    self.core_layouts.insert(cache_key, id);
                    values.push(id);
                }
            }
        }

        Ok(values
            .pop()
            .expect("core type lowering should produce one backend layout"))
    }
}

struct KernelContract {
    uses_input_subject: bool,
    environment: Vec<LayoutId>,
    env_map: HashMap<u32, EnvSlotId>,
    global_items: Vec<ItemId>,
}

#[derive(Clone, Copy)]
struct SubjectContext {
    reference: SubjectRef,
    layout: LayoutId,
}

#[derive(Clone, Default)]
struct LocalBindings(Vec<(u32, SubjectContext)>);

impl LocalBindings {
    fn get(&self, binding: u32) -> Option<SubjectContext> {
        self.0
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == binding)
            .map(|(_, subject)| *subject)
    }

    fn insert(&mut self, binding: u32, subject: SubjectContext) {
        self.0.push((binding, subject));
    }
}

#[derive(Clone)]
struct InlinePipeCaseArmSpec {
    span: SourceSpan,
    pattern: InlinePipePattern,
}

#[derive(Clone)]
struct InlinePipeTruthyFalsyBranchSpec {
    span: SourceSpan,
    constructor: BuiltinTerm,
    payload_subject: Option<InlineSubjectId>,
}

#[derive(Clone, Copy)]
enum SubjectKind {
    Input,
    Inline,
}

struct LoweredKernelExprs {
    root: KernelExprId,
    inline_subjects: Vec<LayoutId>,
    exprs: Arena<KernelExprId, KernelExpr>,
}

fn applied_result_type(mut ty: core::Type, arguments: usize) -> Option<core::Type> {
    for _ in 0..arguments {
        match ty {
            core::Type::Arrow { result, .. } => ty = *result,
            _ => return None,
        }
    }
    Some(ty)
}

fn alloc_kernel_expr(
    exprs: &mut Arena<KernelExprId, KernelExpr>,
    expr: KernelExpr,
) -> Result<KernelExprId, LoweringError> {
    exprs
        .alloc(expr)
        .map_err(|overflow| arena_overflow("kernel expressions", overflow))
}

fn alloc_inline_subject(
    inline_subjects: &mut Vec<LayoutId>,
    layout: LayoutId,
) -> Result<InlineSubjectId, LoweringError> {
    let attempted_len = inline_subjects.len();
    let raw = u32::try_from(attempted_len).map_err(|_| LoweringError::ArenaOverflow {
        family: "inline subjects",
        attempted_len,
    })?;
    inline_subjects.push(layout);
    Ok(InlineSubjectId::from_raw(raw))
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("requested more lowered values than available");
    values.drain(split..).collect()
}

fn wrap_one(error: LoweringError) -> LoweringErrors {
    LoweringErrors::new(vec![error])
}

fn arena_overflow(family: &'static str, overflow: ArenaOverflow) -> LoweringError {
    LoweringError::ArenaOverflow {
        family,
        attempted_len: overflow.attempted_len(),
    }
}

fn map_builtin_term(term: HirBuiltinTerm) -> BuiltinTerm {
    match term {
        HirBuiltinTerm::True => BuiltinTerm::True,
        HirBuiltinTerm::False => BuiltinTerm::False,
        HirBuiltinTerm::None => BuiltinTerm::None,
        HirBuiltinTerm::Some => BuiltinTerm::Some,
        HirBuiltinTerm::Ok => BuiltinTerm::Ok,
        HirBuiltinTerm::Err => BuiltinTerm::Err,
        HirBuiltinTerm::Valid => BuiltinTerm::Valid,
        HirBuiltinTerm::Invalid => BuiltinTerm::Invalid,
    }
}

fn map_builtin_class_member_intrinsic(
    intrinsic: core::BuiltinClassMemberIntrinsic,
) -> BackendBuiltinClassMemberIntrinsic {
    match intrinsic {
        core::BuiltinClassMemberIntrinsic::StructuralEq => {
            BackendBuiltinClassMemberIntrinsic::StructuralEq
        }
        core::BuiltinClassMemberIntrinsic::Compare {
            subject,
            ordering_item,
        } => BackendBuiltinClassMemberIntrinsic::Compare {
            subject: map_builtin_ord_subject(subject),
            ordering_item,
        },
        core::BuiltinClassMemberIntrinsic::Append(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Append(map_builtin_append_carrier(carrier))
        }
        core::BuiltinClassMemberIntrinsic::Empty(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Empty(map_builtin_append_carrier(carrier))
        }
        core::BuiltinClassMemberIntrinsic::Map(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Map(map_builtin_functor_carrier(carrier))
        }
        core::BuiltinClassMemberIntrinsic::Pure(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Pure(map_builtin_applicative_carrier(carrier))
        }
        core::BuiltinClassMemberIntrinsic::Apply(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Apply(map_builtin_apply_carrier(carrier))
        }
        core::BuiltinClassMemberIntrinsic::Reduce(carrier) => {
            BackendBuiltinClassMemberIntrinsic::Reduce(map_builtin_foldable_carrier(carrier))
        }
    }
}

fn map_builtin_functor_carrier(
    carrier: core::BuiltinFunctorCarrier,
) -> BackendBuiltinFunctorCarrier {
    match carrier {
        core::BuiltinFunctorCarrier::List => BackendBuiltinFunctorCarrier::List,
        core::BuiltinFunctorCarrier::Option => BackendBuiltinFunctorCarrier::Option,
        core::BuiltinFunctorCarrier::Result => BackendBuiltinFunctorCarrier::Result,
        core::BuiltinFunctorCarrier::Validation => BackendBuiltinFunctorCarrier::Validation,
        core::BuiltinFunctorCarrier::Signal => BackendBuiltinFunctorCarrier::Signal,
    }
}

fn map_builtin_applicative_carrier(
    carrier: core::BuiltinApplicativeCarrier,
) -> BackendBuiltinApplicativeCarrier {
    match carrier {
        core::BuiltinApplicativeCarrier::List => BackendBuiltinApplicativeCarrier::List,
        core::BuiltinApplicativeCarrier::Option => BackendBuiltinApplicativeCarrier::Option,
        core::BuiltinApplicativeCarrier::Result => BackendBuiltinApplicativeCarrier::Result,
        core::BuiltinApplicativeCarrier::Validation => BackendBuiltinApplicativeCarrier::Validation,
        core::BuiltinApplicativeCarrier::Signal => BackendBuiltinApplicativeCarrier::Signal,
    }
}

fn map_builtin_apply_carrier(carrier: core::BuiltinApplyCarrier) -> BackendBuiltinApplyCarrier {
    match carrier {
        core::BuiltinApplyCarrier::List => BackendBuiltinApplyCarrier::List,
        core::BuiltinApplyCarrier::Option => BackendBuiltinApplyCarrier::Option,
        core::BuiltinApplyCarrier::Result => BackendBuiltinApplyCarrier::Result,
        core::BuiltinApplyCarrier::Validation => BackendBuiltinApplyCarrier::Validation,
        core::BuiltinApplyCarrier::Signal => BackendBuiltinApplyCarrier::Signal,
    }
}

fn map_builtin_foldable_carrier(
    carrier: core::BuiltinFoldableCarrier,
) -> BackendBuiltinFoldableCarrier {
    match carrier {
        core::BuiltinFoldableCarrier::List => BackendBuiltinFoldableCarrier::List,
        core::BuiltinFoldableCarrier::Option => BackendBuiltinFoldableCarrier::Option,
        core::BuiltinFoldableCarrier::Result => BackendBuiltinFoldableCarrier::Result,
        core::BuiltinFoldableCarrier::Validation => BackendBuiltinFoldableCarrier::Validation,
    }
}

fn map_builtin_append_carrier(carrier: core::BuiltinAppendCarrier) -> BackendBuiltinAppendCarrier {
    match carrier {
        core::BuiltinAppendCarrier::Text => BackendBuiltinAppendCarrier::Text,
        core::BuiltinAppendCarrier::List => BackendBuiltinAppendCarrier::List,
    }
}

fn map_builtin_ord_subject(subject: core::BuiltinOrdSubject) -> BackendBuiltinOrdSubject {
    match subject {
        core::BuiltinOrdSubject::Int => BackendBuiltinOrdSubject::Int,
        core::BuiltinOrdSubject::Bool => BackendBuiltinOrdSubject::Bool,
        core::BuiltinOrdSubject::Text => BackendBuiltinOrdSubject::Text,
        core::BuiltinOrdSubject::Ordering => BackendBuiltinOrdSubject::Ordering,
    }
}

fn map_unary_operator(operator: HirUnaryOperator) -> UnaryOperator {
    match operator {
        HirUnaryOperator::Not => UnaryOperator::Not,
    }
}

fn map_binary_operator(operator: HirBinaryOperator) -> BinaryOperator {
    match operator {
        HirBinaryOperator::Add => BinaryOperator::Add,
        HirBinaryOperator::Subtract => BinaryOperator::Subtract,
        HirBinaryOperator::Multiply => BinaryOperator::Multiply,
        HirBinaryOperator::Divide => BinaryOperator::Divide,
        HirBinaryOperator::Modulo => BinaryOperator::Modulo,
        HirBinaryOperator::GreaterThan => BinaryOperator::GreaterThan,
        HirBinaryOperator::LessThan => BinaryOperator::LessThan,
        HirBinaryOperator::Equals => BinaryOperator::Equals,
        HirBinaryOperator::NotEquals => BinaryOperator::NotEquals,
        HirBinaryOperator::And => BinaryOperator::And,
        HirBinaryOperator::Or => BinaryOperator::Or,
    }
}

fn map_fanout_carrier(carrier: TypingFanoutCarrier) -> FanoutCarrier {
    match carrier {
        TypingFanoutCarrier::Ordinary => FanoutCarrier::Ordinary,
        TypingFanoutCarrier::Signal => FanoutCarrier::Signal,
    }
}

fn map_recurrence_target(target: TypingRecurrenceTarget) -> RecurrenceTarget {
    match target {
        TypingRecurrenceTarget::Signal => RecurrenceTarget::Signal,
        TypingRecurrenceTarget::Task => RecurrenceTarget::Task,
        TypingRecurrenceTarget::SourceHelper => RecurrenceTarget::SourceHelper,
    }
}

fn map_recurrence_wakeup_kind(kind: TypingRecurrenceWakeupKind) -> RecurrenceWakeupKind {
    match kind {
        TypingRecurrenceWakeupKind::Timer => RecurrenceWakeupKind::Timer,
        TypingRecurrenceWakeupKind::Backoff => RecurrenceWakeupKind::Backoff,
        TypingRecurrenceWakeupKind::SourceEvent => RecurrenceWakeupKind::SourceEvent,
        TypingRecurrenceWakeupKind::ProviderDefinedTrigger => {
            RecurrenceWakeupKind::ProviderDefinedTrigger
        }
    }
}

fn map_non_source_wakeup_cause(cause: TypingNonSourceWakeupCause) -> NonSourceWakeupCause {
    match cause {
        TypingNonSourceWakeupCause::ExplicitTimer => NonSourceWakeupCause::ExplicitTimer,
        TypingNonSourceWakeupCause::ExplicitBackoff => NonSourceWakeupCause::ExplicitBackoff,
    }
}

fn map_decode_mode(mode: TypingDecodeMode) -> DecodeMode {
    match mode {
        TypingDecodeMode::Strict => DecodeMode::Strict,
        TypingDecodeMode::Permissive => DecodeMode::Permissive,
    }
}

fn map_extra_fields(policy: TypingDecodeExtraFieldPolicy) -> DecodeExtraFieldPolicy {
    match policy {
        TypingDecodeExtraFieldPolicy::Reject => DecodeExtraFieldPolicy::Reject,
        TypingDecodeExtraFieldPolicy::Ignore => DecodeExtraFieldPolicy::Ignore,
    }
}

fn map_decode_requirement(requirement: TypingDecodeFieldRequirement) -> DecodeFieldRequirement {
    match requirement {
        TypingDecodeFieldRequirement::Required => DecodeFieldRequirement::Required,
    }
}

fn map_decode_strategy(strategy: TypingDecodeSumStrategy) -> DecodeSumStrategy {
    match strategy {
        TypingDecodeSumStrategy::Explicit => DecodeSumStrategy::Explicit,
    }
}

fn map_domain_surface_kind(kind: core::DomainDecodeSurfaceKind) -> DomainDecodeSurfaceKind {
    match kind {
        core::DomainDecodeSurfaceKind::Direct => DomainDecodeSurfaceKind::Direct,
        core::DomainDecodeSurfaceKind::FallibleResult => DomainDecodeSurfaceKind::FallibleResult,
    }
}

fn map_decode_primitive(primitive: TypingPrimitiveType) -> PrimitiveType {
    match primitive {
        TypingPrimitiveType::Int => PrimitiveType::Int,
        TypingPrimitiveType::Float => PrimitiveType::Float,
        TypingPrimitiveType::Decimal => PrimitiveType::Decimal,
        TypingPrimitiveType::BigInt => PrimitiveType::BigInt,
        TypingPrimitiveType::Bool => PrimitiveType::Bool,
        TypingPrimitiveType::Text => PrimitiveType::Text,
        TypingPrimitiveType::Unit => PrimitiveType::Unit,
        TypingPrimitiveType::Bytes => PrimitiveType::Bytes,
    }
}

fn map_source_provider(provider: &aivi_hir::SourceProviderRef) -> SourceProvider {
    if let Some(builtin) = provider.builtin() {
        return SourceProvider::Builtin(builtin.key().into());
    }
    if let Some(custom) = provider.custom_key() {
        return SourceProvider::Custom(custom.into());
    }
    match provider.key() {
        Some(key) => SourceProvider::InvalidShape(key.into()),
        None => SourceProvider::Missing,
    }
}

fn map_teardown_policy(_: aivi_hir::SourceTeardownPolicy) -> SourceTeardownPolicy {
    SourceTeardownPolicy::DisposeOnOwnerTeardown
}

fn map_replacement_policy(_: aivi_hir::SourceReplacementPolicy) -> SourceReplacementPolicy {
    SourceReplacementPolicy::DisposeSupersededBeforePublish
}

fn map_stale_work_policy(_: aivi_hir::SourceStaleWorkPolicy) -> SourceStaleWorkPolicy {
    SourceStaleWorkPolicy::DropStalePublications
}

fn map_cancellation_policy(policy: TypingSourceCancellationPolicy) -> SourceCancellationPolicy {
    match policy {
        TypingSourceCancellationPolicy::ProviderManaged => {
            SourceCancellationPolicy::ProviderManaged
        }
        TypingSourceCancellationPolicy::CancelInFlight => SourceCancellationPolicy::CancelInFlight,
    }
}

fn source_instance_raw(source: &core::SourceNode) -> u32 {
    source
        .instance
        .to_string()
        .parse::<u32>()
        .expect("typed-core source instances should print raw u32 values")
}
