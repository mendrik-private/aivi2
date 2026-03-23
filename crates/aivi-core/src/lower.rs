use std::collections::{BTreeMap, HashMap, HashSet};

use aivi_base::SourceSpan;
use aivi_hir::{
    BlockedFanoutSegment, BlockedGateStage, BlockedGeneralExpr as BlockedGeneralExprBody,
    BlockedRecurrenceNode, BlockedSourceDecodeProgram, BlockedSourceLifecycleNode,
    BlockedTruthyFalsyStage, ExprId as HirExprId, GateRuntimeExpr, GateRuntimeExprKind,
    GateRuntimePipeExpr, GateRuntimePipeStageKind, GateRuntimeProjectionBase, GateRuntimeReference,
    GateRuntimeTextLiteral, GateRuntimeTextSegment, GateRuntimeTruthyFalsyBranch, GateStageOutcome,
    GeneralExprOutcome, GeneralExprParameter, Item as HirItem, ItemId as HirItemId,
    PatternId as HirPatternId, RecurrenceNodeOutcome, SourceDecodeProgram,
    SourceDecodeProgramOutcome, SourceLifecycleNodeOutcome, TruthyFalsyStageOutcome,
    elaborate_fanouts, elaborate_gates, elaborate_general_expressions, elaborate_recurrences,
    elaborate_source_lifecycles, elaborate_truthy_falsy, generate_source_decode_programs,
};

use crate::{
    Arena, ArenaOverflow, DecodeField, DecodeProgram, DecodeProgramId, DecodeStep, DecodeStepId,
    DomainDecodeSurface, DomainDecodeSurfaceKind, Expr, ExprId, FanoutFilter, FanoutJoin,
    FanoutStage, GateStage, Item, ItemId, ItemKind, ItemParameter, MapEntry, Module,
    NonSourceWakeup, Pattern, PatternBinding, PatternConstructor, PatternKind, Pipe, PipeCaseArm,
    PipeExpr, PipeOrigin, PipeRecurrence, PipeStage, PipeTruthyFalsyBranch, PipeTruthyFalsyStage,
    ProjectionBase, RecordExprField, RecordPatternField, RecurrenceGuard, RecurrenceStage,
    Reference, SignalInfo, SourceArgumentValue, SourceId, SourceInstanceId, SourceNode,
    SourceOptionBinding, SourceOptionValue, Stage, StageKind, TextLiteral, TextSegment,
    TruthyFalsyBranch, TruthyFalsyStage, Type,
    expr::ExprKind,
    validate::{ValidationError, validate_module},
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

impl std::fmt::Display for LoweringErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    UnknownOwner {
        owner: HirItemId,
    },
    BlockedGateStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        stage_index: usize,
        span: SourceSpan,
        blocked: BlockedGateStage,
    },
    BlockedTruthyFalsyStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        truthy_stage_index: usize,
        falsy_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedTruthyFalsyStage,
    },
    BlockedFanoutStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        map_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedFanoutSegment,
    },
    BlockedRecurrence {
        owner: HirItemId,
        pipe_expr: HirExprId,
        start_stage_index: usize,
        span: SourceSpan,
        blocked: BlockedRecurrenceNode,
    },
    BlockedSourceLifecycle {
        owner: HirItemId,
        span: SourceSpan,
        blocked: BlockedSourceLifecycleNode,
    },
    BlockedDecodeProgram {
        owner: HirItemId,
        span: SourceSpan,
        blocked: BlockedSourceDecodeProgram,
    },
    BlockedGeneralExpr {
        owner: HirItemId,
        body_expr: HirExprId,
        span: SourceSpan,
        blocked: BlockedGeneralExprBody,
    },
    DuplicatePipeStage {
        owner: HirItemId,
        pipe_expr: HirExprId,
        stage_index: usize,
    },
    DuplicatePipeRecurrence {
        owner: HirItemId,
        pipe_expr: HirExprId,
    },
    DuplicateSourceOwner {
        owner: HirItemId,
    },
    DuplicateDecodeOwner {
        owner: HirItemId,
    },
    MissingSourceForDecode {
        owner: HirItemId,
    },
    DependencyOutsideCore {
        owner: HirItemId,
        dependency: HirItemId,
    },
    ArenaOverflow {
        arena: &'static str,
        attempted_len: usize,
    },
    Validation(ValidationError),
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOwner { owner } => {
                write!(f, "typed-core lowering cannot find owner item {owner}")
            }
            Self::BlockedGateStage {
                owner,
                stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on gate stage {stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedTruthyFalsyStage {
                owner,
                truthy_stage_index,
                falsy_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on truthy/falsy pair {truthy_stage_index}/{falsy_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedFanoutStage {
                owner,
                map_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on fanout stage {map_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedRecurrence {
                owner,
                start_stage_index,
                blocked,
                ..
            } => write!(
                f,
                "typed-core lowering blocked on recurrence stage {start_stage_index} for item {owner}: {blocked:?}"
            ),
            Self::BlockedSourceLifecycle { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on source lifecycle for item {owner}: {blocked:?}"
            ),
            Self::BlockedDecodeProgram { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on decode program for item {owner}: {blocked:?}"
            ),
            Self::BlockedGeneralExpr { owner, blocked, .. } => write!(
                f,
                "typed-core lowering blocked on general expression body for item {owner}: {blocked}"
            ),
            Self::DuplicatePipeStage {
                owner,
                pipe_expr,
                stage_index,
            } => write!(
                f,
                "typed-core lowering saw duplicate stage {stage_index} for pipe {pipe_expr} owned by item {owner}"
            ),
            Self::DuplicatePipeRecurrence { owner, pipe_expr } => write!(
                f,
                "typed-core lowering saw duplicate recurrence attachment for pipe {pipe_expr} owned by item {owner}"
            ),
            Self::DuplicateSourceOwner { owner } => {
                write!(
                    f,
                    "typed-core lowering saw more than one source for item {owner}"
                )
            }
            Self::DuplicateDecodeOwner { owner } => {
                write!(
                    f,
                    "typed-core lowering saw more than one decode program for item {owner}"
                )
            }
            Self::MissingSourceForDecode { owner } => write!(
                f,
                "typed-core lowering cannot attach decode program because item {owner} has no lowered source node"
            ),
            Self::DependencyOutsideCore { owner, dependency } => write!(
                f,
                "typed-core lowering cannot map dependency {dependency} owned by item {owner} into the current core slice"
            ),
            Self::ArenaOverflow {
                arena,
                attempted_len,
            } => write!(
                f,
                "typed-core {arena} arena overflowed after {attempted_len} entries"
            ),
            Self::Validation(error) => write!(f, "typed-core validation failed: {error}"),
        }
    }
}

pub fn lower_module(hir: &aivi_hir::Module) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new(hir).build()
}

pub fn lower_runtime_module(hir: &aivi_hir::Module) -> Result<Module, LoweringErrors> {
    ModuleLowerer::new_runtime(hir).build()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFragmentSpec {
    pub name: Box<str>,
    pub owner: HirItemId,
    pub body_expr: HirExprId,
    pub parameters: Vec<GeneralExprParameter>,
    pub body: GateRuntimeExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoweredRuntimeFragment {
    pub entry_name: Box<str>,
    pub module: Module,
}

pub fn lower_runtime_fragment(
    hir: &aivi_hir::Module,
    fragment: &RuntimeFragmentSpec,
) -> Result<LoweredRuntimeFragment, LoweringErrors> {
    RuntimeFragmentLowerer::new(hir, fragment).build()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PipeKey {
    owner: HirItemId,
    pipe_expr: HirExprId,
}

struct PipeBuilder {
    owner: ItemId,
    origin: PipeOrigin,
    stages: BTreeMap<usize, PendingStage>,
    recurrence: Option<PipeRecurrence>,
}

enum PendingStage {
    Lowered {
        span: SourceSpan,
        input_subject: Type,
        result_subject: Type,
        kind: StageKind,
    },
}

struct ModuleLowerer<'a> {
    hir: &'a aivi_hir::Module,
    included_items: Option<HashSet<HirItemId>>,
    module: Module,
    item_map: HashMap<HirItemId, ItemId>,
    pipe_builders: BTreeMap<PipeKey, PipeBuilder>,
    source_by_owner: HashMap<ItemId, SourceId>,
    decode_by_owner: HashMap<ItemId, DecodeProgramId>,
    errors: Vec<LoweringError>,
}

struct RuntimeFragmentLowerer<'a> {
    lowerer: ModuleLowerer<'a>,
    fragment: &'a RuntimeFragmentSpec,
    report_by_owner: HashMap<HirItemId, aivi_hir::GeneralExprItemElaboration>,
    lowering: HashSet<HirItemId>,
    lowered: HashSet<HirItemId>,
}

impl<'a> ModuleLowerer<'a> {
    fn new(hir: &'a aivi_hir::Module) -> Self {
        Self {
            hir,
            included_items: None,
            module: Module::new(),
            item_map: HashMap::new(),
            pipe_builders: BTreeMap::new(),
            source_by_owner: HashMap::new(),
            decode_by_owner: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn new_runtime(hir: &'a aivi_hir::Module) -> Self {
        let included_items = hir
            .items()
            .iter()
            .filter_map(|(item_id, item)| match item {
                HirItem::Value(value)
                    if matches!(hir.exprs()[value.body].kind, aivi_hir::ExprKind::Markup(_)) =>
                {
                    None
                }
                _ => Some(item_id),
            })
            .collect::<HashSet<_>>();
        Self {
            hir,
            included_items: Some(included_items),
            module: Module::new(),
            item_map: HashMap::new(),
            pipe_builders: BTreeMap::new(),
            source_by_owner: HashMap::new(),
            decode_by_owner: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn includes_item(&self, item: HirItemId) -> bool {
        self.included_items
            .as_ref()
            .is_none_or(|included| included.contains(&item))
    }

    fn build(mut self) -> Result<Module, LoweringErrors> {
        self.seed_items()?;
        self.lower_general_exprs();
        self.seed_signal_dependencies();
        self.lower_gate_stages();
        self.lower_truthy_falsy_stages();
        self.lower_fanout_stages();
        self.lower_recurrences();
        self.finalize_pipes()?;
        self.lower_sources()?;
        self.lower_decode_programs()?;

        if !self.errors.is_empty() {
            return Err(LoweringErrors::new(self.errors));
        }

        if let Err(validation) = validate_module(&self.module) {
            self.errors.extend(
                validation
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::Validation),
            );
            return Err(LoweringErrors::new(self.errors));
        }

        Ok(self.module)
    }

    fn seed_items(&mut self) -> Result<(), LoweringErrors> {
        for (hir_id, item) in self.hir.items().iter() {
            if !self.includes_item(hir_id) {
                continue;
            }
            let (span, name, kind) = match item {
                HirItem::Value(item) => {
                    (item.header.span, item.name.text().into(), ItemKind::Value)
                }
                HirItem::Function(item) => (
                    item.header.span,
                    item.name.text().into(),
                    ItemKind::Function,
                ),
                HirItem::Signal(item) => (
                    item.header.span,
                    item.name.text().into(),
                    ItemKind::Signal(SignalInfo::default()),
                ),
                HirItem::Instance(item) => (
                    item.header.span,
                    format!("instance#{}", hir_id.as_raw()).into_boxed_str(),
                    ItemKind::Instance,
                ),
                HirItem::Type(_)
                | HirItem::Class(_)
                | HirItem::Domain(_)
                | HirItem::SourceProviderContract(_)
                | HirItem::Use(_)
                | HirItem::Export(_) => continue,
            };
            let item_id = self
                .module
                .items_mut()
                .alloc(Item {
                    origin: hir_id,
                    span,
                    name,
                    kind,
                    parameters: Vec::new(),
                    body: None,
                    pipes: Vec::new(),
                })
                .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("items", overflow)]))?;
            self.item_map.insert(hir_id, item_id);
        }
        Ok(())
    }

    fn lower_general_exprs(&mut self) {
        for item in elaborate_general_expressions(self.hir).into_items() {
            if !self.includes_item(item.owner) {
                continue;
            }
            let Some(owner) = self.item_map.get(&item.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: item.owner });
                continue;
            };
            match item.outcome {
                GeneralExprOutcome::Lowered(body) => {
                    let body = match self.lower_runtime_expr(item.owner, &body) {
                        Ok(body) => body,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let parameters = item
                        .parameters
                        .into_iter()
                        .map(|parameter| ItemParameter {
                            binding: parameter.binding,
                            span: parameter.span,
                            name: parameter.name,
                            ty: Type::lower(&parameter.ty),
                        })
                        .collect::<Vec<_>>();
                    let Some(core_item) = self.module.items_mut().get_mut(owner) else {
                        self.errors
                            .push(LoweringError::UnknownOwner { owner: item.owner });
                        continue;
                    };
                    core_item.parameters = parameters;
                    core_item.body = Some(body);
                }
                GeneralExprOutcome::Blocked(blocked) => {
                    if !blocked.requires_typed_core_error() {
                        continue;
                    }
                    let span = blocked
                        .primary_span()
                        .unwrap_or(self.hir.exprs()[item.body_expr].span);
                    self.errors.push(LoweringError::BlockedGeneralExpr {
                        owner: item.owner,
                        body_expr: item.body_expr,
                        span,
                        blocked,
                    });
                }
            }
        }
    }

    fn seed_signal_dependencies(&mut self) {
        for (hir_id, item) in self.hir.items().iter() {
            if !self.includes_item(hir_id) {
                continue;
            }
            let HirItem::Signal(signal) = item else {
                continue;
            };
            let Some(item_id) = self.item_map.get(&hir_id).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: hir_id });
                continue;
            };
            let dependencies = signal
                .signal_dependencies
                .iter()
                .filter_map(|dependency| self.map_dependency(hir_id, *dependency))
                .collect::<Vec<_>>();
            let Some(item) = self.module.items_mut().get_mut(item_id) else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: hir_id });
                continue;
            };
            let ItemKind::Signal(info) = &mut item.kind else {
                continue;
            };
            let mut dependencies = dependencies;
            dependencies.sort();
            dependencies.dedup();
            info.dependencies = dependencies;
        }
    }

    fn lower_gate_stages(&mut self) {
        for stage in elaborate_gates(self.hir).into_stages() {
            if !self.item_map.contains_key(&stage.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: stage.owner });
                continue;
            }
            let key = PipeKey {
                owner: stage.owner,
                pipe_expr: stage.pipe_expr,
            };
            let lowered = match stage.outcome {
                GateStageOutcome::Ordinary(plan) => {
                    let input_subject = Type::lower(&plan.input_subject);
                    let result_subject = Type::lower(&plan.result_type);
                    let ambient = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: input_subject.clone(),
                            kind: ExprKind::AmbientSubject,
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let when_true = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: result_subject.clone(),
                            kind: ExprKind::OptionSome { payload: ambient },
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let when_false = match self.alloc_expr(
                        stage.owner,
                        stage.stage_span,
                        Expr {
                            span: stage.stage_span,
                            ty: result_subject.clone(),
                            kind: ExprKind::OptionNone,
                        },
                    ) {
                        Ok(id) => id,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject,
                        result_subject,
                        kind: StageKind::Gate(GateStage::Ordinary {
                            when_true,
                            when_false,
                        }),
                    }
                }
                GateStageOutcome::SignalFilter(plan) => {
                    let predicate =
                        match self.lower_runtime_expr(stage.owner, &plan.runtime_predicate) {
                            Ok(expr) => expr,
                            Err(error) => {
                                self.errors.push(error);
                                continue;
                            }
                        };
                    PendingStage::Lowered {
                        span: stage.stage_span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::Gate(GateStage::SignalFilter {
                            payload_type: Type::lower(&plan.payload_type),
                            predicate,
                            emits_negative_update: plan.emits_negative_update,
                        }),
                    }
                }
                GateStageOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedGateStage {
                        owner: stage.owner,
                        pipe_expr: stage.pipe_expr,
                        stage_index: stage.stage_index,
                        span: stage.stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder.stages.insert(stage.stage_index, lowered).is_some() {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: stage.owner,
                    pipe_expr: stage.pipe_expr,
                    stage_index: stage.stage_index,
                });
            }
        }
    }

    fn lower_truthy_falsy_stages(&mut self) {
        for stage in elaborate_truthy_falsy(self.hir).into_stages() {
            if !self.item_map.contains_key(&stage.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: stage.owner });
                continue;
            }
            let key = PipeKey {
                owner: stage.owner,
                pipe_expr: stage.pipe_expr,
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            let outcome = match stage.outcome {
                TruthyFalsyStageOutcome::Planned(plan) => {
                    let span = join_spans(stage.truthy_stage_span, stage.falsy_stage_span);
                    PendingStage::Lowered {
                        span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::TruthyFalsy(TruthyFalsyStage {
                            truthy_stage_index: stage.truthy_stage_index,
                            truthy_stage_span: stage.truthy_stage_span,
                            falsy_stage_index: stage.falsy_stage_index,
                            falsy_stage_span: stage.falsy_stage_span,
                            truthy: TruthyFalsyBranch {
                                constructor: plan.truthy.constructor,
                                payload_subject: plan
                                    .truthy
                                    .payload_subject
                                    .as_ref()
                                    .map(Type::lower),
                                result_type: Type::lower(&plan.truthy.result_type),
                                origin_expr: plan.truthy.expr,
                            },
                            falsy: TruthyFalsyBranch {
                                constructor: plan.falsy.constructor,
                                payload_subject: plan
                                    .falsy
                                    .payload_subject
                                    .as_ref()
                                    .map(Type::lower),
                                result_type: Type::lower(&plan.falsy.result_type),
                                origin_expr: plan.falsy.expr,
                            },
                        }),
                    }
                }
                TruthyFalsyStageOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedTruthyFalsyStage {
                        owner: stage.owner,
                        pipe_expr: stage.pipe_expr,
                        truthy_stage_index: stage.truthy_stage_index,
                        falsy_stage_index: stage.falsy_stage_index,
                        span: join_spans(stage.truthy_stage_span, stage.falsy_stage_span),
                        blocked,
                    });
                    continue;
                }
            };
            if builder
                .stages
                .insert(stage.truthy_stage_index, outcome)
                .is_some()
            {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: stage.owner,
                    pipe_expr: stage.pipe_expr,
                    stage_index: stage.truthy_stage_index,
                });
            }
        }
    }

    fn lower_fanout_stages(&mut self) {
        for segment in elaborate_fanouts(self.hir).into_segments() {
            if !self.item_map.contains_key(&segment.owner) {
                self.errors.push(LoweringError::UnknownOwner {
                    owner: segment.owner,
                });
                continue;
            }
            let key = PipeKey {
                owner: segment.owner,
                pipe_expr: segment.pipe_expr,
            };
            let outcome = match segment.outcome {
                aivi_hir::FanoutSegmentOutcome::Planned(plan) => {
                    let span = plan
                        .join
                        .as_ref()
                        .map(|join| join_spans(segment.map_stage_span, join.stage_span))
                        .unwrap_or(segment.map_stage_span);
                    let mut filters = Vec::with_capacity(plan.filters.len());
                    let mut failed = false;
                    for filter in &plan.filters {
                        match self.lower_fanout_filter(segment.owner, filter) {
                            Ok(filter) => filters.push(filter),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    PendingStage::Lowered {
                        span,
                        input_subject: Type::lower(&plan.input_subject),
                        result_subject: Type::lower(&plan.result_type),
                        kind: StageKind::Fanout(FanoutStage {
                            carrier: plan.carrier,
                            element_subject: Type::lower(&plan.element_subject),
                            mapped_element_type: Type::lower(&plan.mapped_element_type),
                            mapped_collection_type: Type::lower(&plan.mapped_collection_type),
                            filters,
                            join: plan.join.map(|join| FanoutJoin {
                                stage_index: join.stage_index,
                                stage_span: join.stage_span,
                                origin_expr: join.expr,
                                input_subject: Type::lower(&join.input_subject),
                                collection_subject: Type::lower(&join.collection_subject),
                                result_type: Type::lower(&join.result_type),
                            }),
                        }),
                    }
                }
                aivi_hir::FanoutSegmentOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedFanoutStage {
                        owner: segment.owner,
                        pipe_expr: segment.pipe_expr,
                        map_stage_index: segment.map_stage_index,
                        span: segment.map_stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder
                .stages
                .insert(segment.map_stage_index, outcome)
                .is_some()
            {
                self.errors.push(LoweringError::DuplicatePipeStage {
                    owner: segment.owner,
                    pipe_expr: segment.pipe_expr,
                    stage_index: segment.map_stage_index,
                });
            }
        }
    }

    fn lower_recurrences(&mut self) {
        for node in elaborate_recurrences(self.hir).into_nodes() {
            if !self.item_map.contains_key(&node.owner) {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            }
            let key = PipeKey {
                owner: node.owner,
                pipe_expr: node.pipe_expr,
            };
            let recurrence = match node.outcome {
                RecurrenceNodeOutcome::Planned(plan) => {
                    let start = match self.lower_recurrence_stage(node.owner, &plan.start) {
                        Ok(stage) => stage,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    };
                    let mut guards = Vec::with_capacity(plan.guards.len());
                    let mut failed = false;
                    for guard in &plan.guards {
                        match self.lower_recurrence_guard(node.owner, guard) {
                            Ok(guard) => guards.push(guard),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    let mut steps = Vec::with_capacity(plan.steps.len());
                    failed = false;
                    for step in &plan.steps {
                        match self.lower_recurrence_stage(node.owner, step) {
                            Ok(stage) => steps.push(stage),
                            Err(error) => {
                                self.errors.push(error);
                                failed = true;
                                break;
                            }
                        }
                    }
                    if failed {
                        continue;
                    }
                    let non_source_wakeup = match plan.non_source_wakeup {
                        Some(binding) => {
                            match self.lower_runtime_expr(node.owner, &binding.runtime_witness) {
                                Ok(runtime_witness) => Some(NonSourceWakeup {
                                    cause: binding.cause,
                                    witness_expr: binding.witness,
                                    runtime_witness,
                                }),
                                Err(error) => {
                                    self.errors.push(error);
                                    continue;
                                }
                            }
                        }
                        None => None,
                    };
                    PipeRecurrence {
                        target: plan.target,
                        wakeup: plan.wakeup,
                        start,
                        guards,
                        steps,
                        non_source_wakeup,
                    }
                }
                RecurrenceNodeOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedRecurrence {
                        owner: node.owner,
                        pipe_expr: node.pipe_expr,
                        start_stage_index: node.start_stage_index,
                        span: node.start_stage_span,
                        blocked,
                    });
                    continue;
                }
            };
            let builder = match self.pipe_builder(key) {
                Some(builder) => builder,
                None => continue,
            };
            if builder.recurrence.replace(recurrence).is_some() {
                self.errors.push(LoweringError::DuplicatePipeRecurrence {
                    owner: node.owner,
                    pipe_expr: node.pipe_expr,
                });
            }
        }
    }

    fn finalize_pipes(&mut self) -> Result<(), LoweringErrors> {
        let builders = std::mem::take(&mut self.pipe_builders);
        for (_, builder) in builders {
            let pipe_id = self
                .module
                .pipes_mut()
                .alloc(Pipe {
                    owner: builder.owner,
                    origin: builder.origin,
                    stages: Vec::new(),
                    recurrence: builder.recurrence,
                })
                .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("pipes", overflow)]))?;
            let mut stage_ids = Vec::with_capacity(builder.stages.len());
            for (index, pending) in builder.stages {
                let PendingStage::Lowered {
                    span,
                    input_subject,
                    result_subject,
                    kind,
                } = pending;
                let stage_id = self
                    .module
                    .stages_mut()
                    .alloc(Stage {
                        pipe: pipe_id,
                        index,
                        span,
                        input_subject,
                        result_subject,
                        kind,
                    })
                    .map_err(|overflow| {
                        LoweringErrors::new(vec![arena_overflow("stages", overflow)])
                    })?;
                stage_ids.push(stage_id);
            }
            self.module
                .pipes_mut()
                .get_mut(pipe_id)
                .expect("pipe just allocated")
                .stages = stage_ids;
            self.module
                .items_mut()
                .get_mut(builder.owner)
                .expect("pipe owner should exist")
                .pipes
                .push(pipe_id);
        }
        Ok(())
    }

    fn lower_sources(&mut self) -> Result<(), LoweringErrors> {
        for node in elaborate_source_lifecycles(self.hir).into_nodes() {
            let Some(owner) = self.item_map.get(&node.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let plan = match node.outcome {
                SourceLifecycleNodeOutcome::Planned(plan) => plan,
                SourceLifecycleNodeOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedSourceLifecycle {
                        owner: node.owner,
                        span: node.source_span,
                        blocked,
                    });
                    continue;
                }
            };
            if self.source_by_owner.contains_key(&owner) {
                self.errors
                    .push(LoweringError::DuplicateSourceOwner { owner: node.owner });
                continue;
            }
            let reconfiguration_dependencies = plan
                .reconfiguration_dependencies
                .iter()
                .filter_map(|dependency| self.map_dependency(node.owner, *dependency))
                .collect::<Vec<_>>();
            let mut arguments = Vec::with_capacity(plan.arguments.len());
            let mut failed = false;
            for argument in plan.arguments {
                match self.lower_runtime_expr(node.owner, &argument.runtime_expr) {
                    Ok(runtime_expr) => arguments.push(SourceArgumentValue {
                        origin_expr: argument.expr,
                        runtime_expr,
                    }),
                    Err(error) => {
                        self.errors.push(error);
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            let mut options = Vec::with_capacity(plan.options.len());
            for option in plan.options {
                match self.lower_runtime_expr(node.owner, &option.runtime_expr) {
                    Ok(runtime_expr) => options.push(SourceOptionValue {
                        option_span: option.option_span,
                        option_name: option.option_name.text().into(),
                        origin_expr: option.expr,
                        runtime_expr,
                    }),
                    Err(error) => {
                        self.errors.push(error);
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            let source_id = self
                .module
                .sources_mut()
                .alloc(SourceNode {
                    owner,
                    span: node.source_span,
                    instance: SourceInstanceId::from_raw(plan.instance.decorator().as_raw()),
                    provider: plan.provider,
                    teardown: plan.teardown,
                    replacement: plan.replacement,
                    arguments,
                    options,
                    reconfiguration_dependencies,
                    explicit_triggers: plan
                        .explicit_triggers
                        .into_iter()
                        .map(|binding| SourceOptionBinding {
                            option_span: binding.option_span,
                            option_name: binding.option_name.text().into(),
                            origin_expr: binding.expr,
                        })
                        .collect(),
                    active_when: plan.active_when.map(|binding| SourceOptionBinding {
                        option_span: binding.option_span,
                        option_name: binding.option_name.text().into(),
                        origin_expr: binding.expr,
                    }),
                    cancellation: plan.cancellation,
                    stale_work: plan.stale_work,
                    decode: None,
                })
                .map_err(|overflow| {
                    LoweringErrors::new(vec![arena_overflow("sources", overflow)])
                })?;
            self.source_by_owner.insert(owner, source_id);
            let Some(item) = self.module.items_mut().get_mut(owner) else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let ItemKind::Signal(info) = &mut item.kind else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            info.source = Some(source_id);
        }
        Ok(())
    }

    fn lower_decode_programs(&mut self) -> Result<(), LoweringErrors> {
        for node in generate_source_decode_programs(self.hir).into_nodes() {
            let Some(owner) = self.item_map.get(&node.owner).copied() else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: node.owner });
                continue;
            };
            let Some(source_id) = self.source_by_owner.get(&owner).copied() else {
                match node.outcome {
                    SourceDecodeProgramOutcome::Planned(_) => {
                        self.errors
                            .push(LoweringError::MissingSourceForDecode { owner: node.owner });
                    }
                    SourceDecodeProgramOutcome::Blocked(blocked) => {
                        self.errors.push(LoweringError::BlockedDecodeProgram {
                            owner: node.owner,
                            span: node.source_span,
                            blocked,
                        });
                    }
                }
                continue;
            };
            if self.decode_by_owner.contains_key(&owner) {
                self.errors
                    .push(LoweringError::DuplicateDecodeOwner { owner: node.owner });
                continue;
            }
            let program = match node.outcome {
                SourceDecodeProgramOutcome::Planned(program) => {
                    match self.lower_decode_program(owner, &program) {
                        Ok(program) => program,
                        Err(error) => {
                            self.errors.push(error);
                            continue;
                        }
                    }
                }
                SourceDecodeProgramOutcome::Blocked(blocked) => {
                    self.errors.push(LoweringError::BlockedDecodeProgram {
                        owner: node.owner,
                        span: node.source_span,
                        blocked,
                    });
                    continue;
                }
            };
            let decode_id =
                self.module
                    .decode_programs_mut()
                    .alloc(program)
                    .map_err(|overflow| {
                        LoweringErrors::new(vec![arena_overflow("decode-programs", overflow)])
                    })?;
            self.decode_by_owner.insert(owner, decode_id);
            self.module
                .sources_mut()
                .get_mut(source_id)
                .expect("source should exist when attaching decode")
                .decode = Some(decode_id);
        }
        Ok(())
    }

    fn pipe_builder(&mut self, key: PipeKey) -> Option<&mut PipeBuilder> {
        if !self.pipe_builders.contains_key(&key) {
            let owner = self.item_map.get(&key.owner).copied();
            let Some(owner) = owner else {
                self.errors
                    .push(LoweringError::UnknownOwner { owner: key.owner });
                return None;
            };
            let span = self.hir.exprs()[key.pipe_expr].span;
            self.pipe_builders.insert(
                key,
                PipeBuilder {
                    owner,
                    origin: PipeOrigin {
                        owner: key.owner,
                        pipe_expr: key.pipe_expr,
                        span,
                    },
                    stages: BTreeMap::new(),
                    recurrence: None,
                },
            );
        }
        self.pipe_builders.get_mut(&key)
    }

    fn lower_recurrence_stage(
        &mut self,
        owner: HirItemId,
        stage: &aivi_hir::RecurrenceStagePlan,
    ) -> Result<RecurrenceStage, LoweringError> {
        Ok(RecurrenceStage {
            stage_index: stage.stage_index,
            stage_span: stage.stage_span,
            origin_expr: stage.expr,
            input_subject: Type::lower(&stage.input_subject),
            result_subject: Type::lower(&stage.result_subject),
            runtime_expr: self.lower_runtime_expr(owner, &stage.runtime_expr)?,
        })
    }

    fn lower_recurrence_guard(
        &mut self,
        owner: HirItemId,
        guard: &aivi_hir::RecurrenceGuardPlan,
    ) -> Result<RecurrenceGuard, LoweringError> {
        Ok(RecurrenceGuard {
            stage_index: guard.stage_index,
            stage_span: guard.stage_span,
            predicate_expr: guard.predicate,
            input_subject: Type::lower(&guard.input_subject),
            runtime_predicate: self.lower_runtime_expr(owner, &guard.runtime_predicate)?,
        })
    }

    fn lower_fanout_filter(
        &mut self,
        owner: HirItemId,
        filter: &aivi_hir::FanoutFilterPlan,
    ) -> Result<FanoutFilter, LoweringError> {
        Ok(FanoutFilter {
            stage_index: filter.stage_index,
            stage_span: filter.stage_span,
            predicate_expr: filter.predicate,
            input_subject: Type::lower(&filter.input_subject),
            runtime_predicate: self.lower_runtime_expr(owner, &filter.runtime_predicate)?,
        })
    }

    fn map_dependency(&mut self, owner: HirItemId, dependency: HirItemId) -> Option<ItemId> {
        match self.item_map.get(&dependency).copied() {
            Some(item) => Some(item),
            None => {
                self.errors
                    .push(LoweringError::DependencyOutsideCore { owner, dependency });
                None
            }
        }
    }

    fn lower_pattern(&self, pattern_id: HirPatternId) -> Pattern {
        let pattern = self.hir.patterns()[pattern_id].clone();
        let kind = match pattern.kind {
            aivi_hir::PatternKind::Wildcard => PatternKind::Wildcard,
            aivi_hir::PatternKind::Binding(binding) => PatternKind::Binding(PatternBinding {
                binding: binding.binding,
                name: binding.name.text().into(),
            }),
            aivi_hir::PatternKind::Integer(literal) => PatternKind::Integer(literal),
            aivi_hir::PatternKind::Text(text) => PatternKind::Text(lower_text_pattern(&text)),
            aivi_hir::PatternKind::Tuple(elements) => PatternKind::Tuple(
                elements
                    .iter()
                    .map(|element| self.lower_pattern(*element))
                    .collect(),
            ),
            aivi_hir::PatternKind::Record(fields) => PatternKind::Record(
                fields
                    .into_iter()
                    .map(|field| RecordPatternField {
                        label: field.label.text().into(),
                        pattern: self.lower_pattern(field.pattern),
                    })
                    .collect(),
            ),
            aivi_hir::PatternKind::Constructor { callee, arguments } => PatternKind::Constructor {
                callee: PatternConstructor {
                    display: callee.path.to_string().into_boxed_str(),
                    reference: self.lower_term_reference(&callee),
                },
                arguments: arguments
                    .into_iter()
                    .map(|argument| self.lower_pattern(argument))
                    .collect(),
            },
            aivi_hir::PatternKind::UnresolvedName(callee) => PatternKind::Constructor {
                callee: PatternConstructor {
                    display: callee.path.to_string().into_boxed_str(),
                    reference: self.lower_term_reference(&callee),
                },
                arguments: Vec::new(),
            },
        };
        Pattern {
            span: pattern.span,
            kind,
        }
    }

    fn lower_term_reference(&self, reference: &aivi_hir::TermReference) -> Reference {
        match reference.resolution.as_ref() {
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Local(binding)) => {
                Reference::Local(*binding)
            }
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Item(item)) => self
                .hir
                .sum_constructor_handle(*item, reference.path.segments().last().text())
                .map(Reference::SumConstructor)
                .or_else(|| self.item_map.get(item).copied().map(Reference::Item))
                .unwrap_or(Reference::HirItem(*item)),
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::DomainMember(
                resolution,
            )) => self
                .hir
                .domain_member_handle(*resolution)
                .map(Reference::DomainMember)
                .unwrap_or(Reference::HirItem(resolution.domain)),
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Builtin(term)) => {
                Reference::Builtin(*term)
            }
            aivi_hir::ResolutionState::Resolved(aivi_hir::TermResolution::Import(_))
            | aivi_hir::ResolutionState::Resolved(
                aivi_hir::TermResolution::AmbiguousDomainMembers(_),
            )
            | aivi_hir::ResolutionState::Unresolved => unreachable!(
                "typed-core general-expression lowering should only see resolved constructor references"
            ),
        }
    }

    fn alloc_expr(
        &mut self,
        _owner: HirItemId,
        _span: SourceSpan,
        expr: Expr,
    ) -> Result<ExprId, LoweringError> {
        self.module
            .exprs_mut()
            .alloc(expr)
            .map_err(|overflow: ArenaOverflow| LoweringError::ArenaOverflow {
                arena: "exprs",
                attempted_len: overflow.attempted_len(),
            })
    }

    fn lower_runtime_expr(
        &mut self,
        owner: HirItemId,
        root: &GateRuntimeExpr,
    ) -> Result<ExprId, LoweringError> {
        enum Task<'a> {
            Visit(&'a GateRuntimeExpr),
            BuildText {
                span: SourceSpan,
                ty: Type,
                segments: Vec<SegmentSpec>,
            },
            BuildTuple {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildList {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildMap {
                span: SourceSpan,
                ty: Type,
                entries: usize,
            },
            BuildSet {
                span: SourceSpan,
                ty: Type,
                len: usize,
            },
            BuildRecord {
                span: SourceSpan,
                ty: Type,
                labels: Vec<Box<str>>,
            },
            BuildProjection {
                span: SourceSpan,
                ty: Type,
                base_is_expr: bool,
                path: Vec<Box<str>>,
            },
            BuildApply {
                span: SourceSpan,
                ty: Type,
                arguments: usize,
            },
            BuildUnary {
                span: SourceSpan,
                ty: Type,
                operator: aivi_hir::UnaryOperator,
            },
            BuildBinary {
                span: SourceSpan,
                ty: Type,
                operator: aivi_hir::BinaryOperator,
            },
            BuildPipe {
                span: SourceSpan,
                ty: Type,
                stages: Vec<PipeStageSpec>,
            },
        }

        let mut tasks = vec![Task::Visit(root)];
        let mut values = Vec::new();

        while let Some(task) = tasks.pop() {
            match task {
                Task::Visit(expr) => {
                    let ty = Type::lower(&expr.ty);
                    match &expr.kind {
                        GateRuntimeExprKind::AmbientSubject => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::AmbientSubject,
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Reference(reference) => {
                            values.push(
                                self.alloc_expr(
                                    owner,
                                    expr.span,
                                    Expr {
                                        span: expr.span,
                                        ty,
                                        kind: ExprKind::Reference(match reference {
                                            GateRuntimeReference::Local(binding) => {
                                                Reference::Local(*binding)
                                            }
                                            GateRuntimeReference::Item(item) => self
                                                .item_map
                                                .get(item)
                                                .copied()
                                                .map(Reference::Item)
                                                .unwrap_or(Reference::HirItem(*item)),
                                            GateRuntimeReference::SumConstructor(handle) => {
                                                Reference::SumConstructor(handle.clone())
                                            }
                                            GateRuntimeReference::DomainMember(handle) => {
                                                Reference::DomainMember(handle.clone())
                                            }
                                            GateRuntimeReference::Builtin(term) => {
                                                Reference::Builtin(*term)
                                            }
                                        }),
                                    },
                                )?,
                            );
                        }
                        GateRuntimeExprKind::Integer(integer) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::Integer(integer.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::SuffixedInteger(integer) => {
                            values.push(self.alloc_expr(
                                owner,
                                expr.span,
                                Expr {
                                    span: expr.span,
                                    ty,
                                    kind: ExprKind::SuffixedInteger(integer.clone()),
                                },
                            )?);
                        }
                        GateRuntimeExprKind::Text(text) => {
                            tasks.push(Task::BuildText {
                                span: expr.span,
                                ty,
                                segments: text_segment_specs(text),
                            });
                            for segment in text.segments.iter().rev() {
                                if let GateRuntimeTextSegment::Interpolation(interpolation) =
                                    segment
                                {
                                    tasks.push(Task::Visit(interpolation));
                                }
                            }
                        }
                        GateRuntimeExprKind::Tuple(elements) => {
                            tasks.push(Task::BuildTuple {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::List(elements) => {
                            tasks.push(Task::BuildList {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::Map(entries) => {
                            tasks.push(Task::BuildMap {
                                span: expr.span,
                                ty,
                                entries: entries.len(),
                            });
                            for entry in entries.iter().rev() {
                                tasks.push(Task::Visit(&entry.value));
                                tasks.push(Task::Visit(&entry.key));
                            }
                        }
                        GateRuntimeExprKind::Set(elements) => {
                            tasks.push(Task::BuildSet {
                                span: expr.span,
                                ty,
                                len: elements.len(),
                            });
                            for element in elements.iter().rev() {
                                tasks.push(Task::Visit(element));
                            }
                        }
                        GateRuntimeExprKind::Record(fields) => {
                            tasks.push(Task::BuildRecord {
                                span: expr.span,
                                ty,
                                labels: fields
                                    .iter()
                                    .map(|field| field.label.text().into())
                                    .collect(),
                            });
                            for field in fields.iter().rev() {
                                tasks.push(Task::Visit(&field.value));
                            }
                        }
                        GateRuntimeExprKind::Projection { base, path } => {
                            tasks.push(Task::BuildProjection {
                                span: expr.span,
                                ty,
                                base_is_expr: matches!(base, GateRuntimeProjectionBase::Expr(_)),
                                path: path
                                    .segments()
                                    .iter()
                                    .map(|segment| segment.text().into())
                                    .collect(),
                            });
                            if let GateRuntimeProjectionBase::Expr(base) = base {
                                tasks.push(Task::Visit(base));
                            }
                        }
                        GateRuntimeExprKind::Apply { callee, arguments } => {
                            tasks.push(Task::BuildApply {
                                span: expr.span,
                                ty,
                                arguments: arguments.len(),
                            });
                            for argument in arguments.iter().rev() {
                                tasks.push(Task::Visit(argument));
                            }
                            tasks.push(Task::Visit(callee));
                        }
                        GateRuntimeExprKind::Unary {
                            operator,
                            expr: inner,
                        } => {
                            tasks.push(Task::BuildUnary {
                                span: expr.span,
                                ty,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(inner));
                        }
                        GateRuntimeExprKind::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            tasks.push(Task::BuildBinary {
                                span: expr.span,
                                ty,
                                operator: *operator,
                            });
                            tasks.push(Task::Visit(right));
                            tasks.push(Task::Visit(left));
                        }
                        GateRuntimeExprKind::Pipe(pipe) => {
                            tasks.push(Task::BuildPipe {
                                span: expr.span,
                                ty,
                                stages: pipe_stage_specs(pipe),
                            });
                            for stage in pipe.stages.iter().rev() {
                                match &stage.kind {
                                    GateRuntimePipeStageKind::Transform { expr }
                                    | GateRuntimePipeStageKind::Tap { expr } => {
                                        tasks.push(Task::Visit(expr));
                                    }
                                    GateRuntimePipeStageKind::Gate { predicate, .. } => {
                                        tasks.push(Task::Visit(predicate));
                                    }
                                    GateRuntimePipeStageKind::Case { arms } => {
                                        for arm in arms.iter().rev() {
                                            tasks.push(Task::Visit(&arm.body));
                                        }
                                    }
                                    GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                                        tasks.push(Task::Visit(&falsy.body));
                                        tasks.push(Task::Visit(&truthy.body));
                                    }
                                }
                            }
                            tasks.push(Task::Visit(&pipe.head));
                        }
                    }
                }
                Task::BuildText { span, ty, segments } => {
                    let interpolation_count = segments
                        .iter()
                        .filter(|segment| matches!(segment, SegmentSpec::Interpolation { .. }))
                        .count();
                    let mut exprs = drain_tail(&mut values, interpolation_count).into_iter();
                    let segments = segments
                        .into_iter()
                        .map(|segment| match segment {
                            SegmentSpec::Fragment { raw, span } => {
                                TextSegment::Fragment { raw, span }
                            }
                            SegmentSpec::Interpolation { span } => TextSegment::Interpolation {
                                expr: exprs.next().expect("text interpolation count should match"),
                                span,
                            },
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Text(TextLiteral { segments }),
                        },
                    )?);
                }
                Task::BuildTuple { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Tuple(elements),
                        },
                    )?);
                }
                Task::BuildList { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::List(elements),
                        },
                    )?);
                }
                Task::BuildMap { span, ty, entries } => {
                    let lowered = drain_tail(&mut values, entries * 2);
                    let mut iter = lowered.into_iter();
                    let entries = (0..entries)
                        .map(|_| MapEntry {
                            key: iter.next().expect("map key should exist"),
                            value: iter.next().expect("map value should exist"),
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Map(entries),
                        },
                    )?);
                }
                Task::BuildSet { span, ty, len } => {
                    let elements = drain_tail(&mut values, len);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Set(elements),
                        },
                    )?);
                }
                Task::BuildRecord { span, ty, labels } => {
                    let len = labels.len();
                    let fields = labels
                        .into_iter()
                        .zip(drain_tail(&mut values, len))
                        .map(|(label, value)| RecordExprField { label, value })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Record(fields),
                        },
                    )?);
                }
                Task::BuildProjection {
                    span,
                    ty,
                    base_is_expr,
                    path,
                } => {
                    let base = if base_is_expr {
                        ProjectionBase::Expr(values.pop().expect("projection base should exist"))
                    } else {
                        ProjectionBase::AmbientSubject
                    };
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Projection { base, path },
                        },
                    )?);
                }
                Task::BuildApply {
                    span,
                    ty,
                    arguments,
                } => {
                    let lowered = drain_tail(&mut values, arguments + 1);
                    let mut iter = lowered.into_iter();
                    let callee = iter.next().expect("apply callee should exist");
                    let arguments = iter.collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Apply { callee, arguments },
                        },
                    )?);
                }
                Task::BuildUnary { span, ty, operator } => {
                    let inner = values.pop().expect("unary child should exist");
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Unary {
                                operator,
                                expr: inner,
                            },
                        },
                    )?);
                }
                Task::BuildBinary { span, ty, operator } => {
                    let lowered = drain_tail(&mut values, 2);
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Binary {
                                left: lowered[0],
                                operator,
                                right: lowered[1],
                            },
                        },
                    )?);
                }
                Task::BuildPipe { span, ty, stages } => {
                    let lowered = drain_tail(
                        &mut values,
                        1 + stages
                            .iter()
                            .map(PipeStageSpec::child_expr_count)
                            .sum::<usize>(),
                    );
                    let mut iter = lowered.into_iter();
                    let head = iter.next().expect("pipe head should exist");
                    let stages = stages
                        .into_iter()
                        .map(|stage| {
                            let children = (0..stage.child_expr_count())
                                .map(|_| iter.next().expect("pipe stage child should exist"))
                                .collect::<Vec<_>>();
                            PipeStage {
                                span: stage.span,
                                input_subject: stage.input_subject,
                                result_subject: stage.result_subject,
                                kind: match stage.kind {
                                    PipeStageKindSpec::Transform => {
                                        let expr = children[0];
                                        crate::expr::PipeStageKind::Transform { expr }
                                    }
                                    PipeStageKindSpec::Tap => {
                                        let expr = children[0];
                                        crate::expr::PipeStageKind::Tap { expr }
                                    }
                                    PipeStageKindSpec::Gate {
                                        emits_negative_update,
                                    } => {
                                        let predicate = children[0];
                                        crate::expr::PipeStageKind::Gate {
                                            predicate,
                                            emits_negative_update,
                                        }
                                    }
                                    PipeStageKindSpec::Case { arms } => {
                                        let mut bodies = children.into_iter();
                                        crate::expr::PipeStageKind::Case {
                                            arms: arms
                                                .into_iter()
                                                .map(|arm| PipeCaseArm {
                                                    span: arm.span,
                                                    pattern: self.lower_pattern(arm.pattern),
                                                    body: bodies
                                                        .next()
                                                        .expect("case arm body should exist"),
                                                })
                                                .collect(),
                                        }
                                    }
                                    PipeStageKindSpec::TruthyFalsy { truthy, falsy } => {
                                        let mut bodies = children.into_iter();
                                        crate::expr::PipeStageKind::TruthyFalsy(
                                            PipeTruthyFalsyStage {
                                                truthy: PipeTruthyFalsyBranch {
                                                    span: truthy.span,
                                                    constructor: truthy.constructor,
                                                    payload_subject: truthy
                                                        .payload_subject
                                                        .map(|payload| Type::lower(&payload)),
                                                    result_type: Type::lower(&truthy.result_type),
                                                    body: bodies
                                                        .next()
                                                        .expect("truthy body should exist"),
                                                },
                                                falsy: PipeTruthyFalsyBranch {
                                                    span: falsy.span,
                                                    constructor: falsy.constructor,
                                                    payload_subject: falsy
                                                        .payload_subject
                                                        .map(|payload| Type::lower(&payload)),
                                                    result_type: Type::lower(&falsy.result_type),
                                                    body: bodies
                                                        .next()
                                                        .expect("falsy body should exist"),
                                                },
                                            },
                                        )
                                    }
                                },
                            }
                        })
                        .collect();
                    values.push(self.alloc_expr(
                        owner,
                        span,
                        Expr {
                            span,
                            ty,
                            kind: ExprKind::Pipe(PipeExpr { head, stages }),
                        },
                    )?);
                }
            }
        }

        Ok(values
            .pop()
            .expect("runtime expression lowering should produce one expression"))
    }

    fn lower_decode_program(
        &mut self,
        owner: ItemId,
        program: &SourceDecodeProgram,
    ) -> Result<DecodeProgram, LoweringError> {
        let mut steps = Arena::new();
        let step_positions = program
            .steps()
            .iter()
            .enumerate()
            .map(|(index, step)| (step as *const _, index))
            .collect::<HashMap<_, _>>();

        let step_id_for = |program: &SourceDecodeProgram,
                           step_positions: &HashMap<*const aivi_hir::DecodeProgramStep, usize>,
                           step_id: aivi_hir::DecodeProgramStepId|
         -> DecodeStepId {
            let ptr = program.step(step_id) as *const _;
            let index = step_positions[&ptr];
            DecodeStepId::from_raw(index as u32)
        };

        for step in program.steps() {
            let lowered = match step {
                aivi_hir::DecodeProgramStep::Scalar { scalar } => {
                    DecodeStep::Scalar { scalar: *scalar }
                }
                aivi_hir::DecodeProgramStep::Tuple { elements } => DecodeStep::Tuple {
                    elements: elements
                        .iter()
                        .map(|step| step_id_for(program, &step_positions, *step))
                        .collect(),
                },
                aivi_hir::DecodeProgramStep::Record {
                    fields,
                    extra_fields,
                } => DecodeStep::Record {
                    fields: fields
                        .iter()
                        .map(|field| DecodeField {
                            name: field.name.as_str().into(),
                            requirement: field.requirement,
                            step: step_id_for(program, &step_positions, field.step),
                        })
                        .collect(),
                    extra_fields: *extra_fields,
                },
                aivi_hir::DecodeProgramStep::Sum { variants, strategy } => DecodeStep::Sum {
                    variants: variants
                        .iter()
                        .map(|variant| crate::DecodeVariant {
                            name: variant.name.as_str().into(),
                            payload: variant
                                .payload
                                .map(|payload| step_id_for(program, &step_positions, payload)),
                        })
                        .collect(),
                    strategy: *strategy,
                },
                aivi_hir::DecodeProgramStep::Domain { carrier, surface } => DecodeStep::Domain {
                    carrier: step_id_for(program, &step_positions, *carrier),
                    surface: DomainDecodeSurface {
                        domain_item: surface.domain_item,
                        member_index: surface.member_index,
                        member_name: surface.member_name.clone(),
                        kind: match surface.kind {
                            aivi_hir::DomainDecodeSurfaceKind::Direct => {
                                DomainDecodeSurfaceKind::Direct
                            }
                            aivi_hir::DomainDecodeSurfaceKind::FallibleResult => {
                                DomainDecodeSurfaceKind::FallibleResult
                            }
                        },
                        span: surface.span,
                    },
                },
                aivi_hir::DecodeProgramStep::List { element } => DecodeStep::List {
                    element: step_id_for(program, &step_positions, *element),
                },
                aivi_hir::DecodeProgramStep::Option { element } => DecodeStep::Option {
                    element: step_id_for(program, &step_positions, *element),
                },
                aivi_hir::DecodeProgramStep::Result { error, value } => DecodeStep::Result {
                    error: step_id_for(program, &step_positions, *error),
                    value: step_id_for(program, &step_positions, *value),
                },
                aivi_hir::DecodeProgramStep::Validation { error, value } => {
                    DecodeStep::Validation {
                        error: step_id_for(program, &step_positions, *error),
                        value: step_id_for(program, &step_positions, *value),
                    }
                }
            };
            let _ = steps
                .alloc(lowered)
                .map_err(|overflow| LoweringError::ArenaOverflow {
                    arena: "decode-steps",
                    attempted_len: overflow.attempted_len(),
                })?;
        }

        let root_index = step_positions[&(program.root_step() as *const _)] as u32;
        Ok(DecodeProgram::new(
            owner,
            program.mode,
            program.payload_annotation,
            DecodeStepId::from_raw(root_index),
            steps,
        ))
    }
}

fn arena_overflow(arena: &'static str, overflow: ArenaOverflow) -> LoweringError {
    LoweringError::ArenaOverflow {
        arena,
        attempted_len: overflow.attempted_len(),
    }
}

fn join_spans(left: SourceSpan, right: SourceSpan) -> SourceSpan {
    left.join(right)
        .expect("typed-core lowering only joins spans from the same source file")
}

fn drain_tail<T>(values: &mut Vec<T>, len: usize) -> Vec<T> {
    let split = values
        .len()
        .checked_sub(len)
        .expect("requested more lowered values than available");
    values.drain(split..).collect()
}

fn text_segment_specs(text: &GateRuntimeTextLiteral) -> Vec<SegmentSpec> {
    text.segments
        .iter()
        .map(|segment| match segment {
            GateRuntimeTextSegment::Fragment(fragment) => SegmentSpec::Fragment {
                raw: fragment.raw.clone(),
                span: fragment.span,
            },
            GateRuntimeTextSegment::Interpolation(interpolation) => SegmentSpec::Interpolation {
                span: interpolation.span,
            },
        })
        .collect()
}

fn lower_text_pattern(text: &aivi_hir::TextLiteral) -> Box<str> {
    let mut raw = String::new();
    for segment in &text.segments {
        match segment {
            aivi_hir::TextSegment::Text(fragment) => raw.push_str(&fragment.raw),
            aivi_hir::TextSegment::Interpolation(_) => raw.push_str("{...}"),
        }
    }
    raw.into_boxed_str()
}

fn pipe_stage_specs(pipe: &GateRuntimePipeExpr) -> Vec<PipeStageSpec> {
    pipe.stages
        .iter()
        .map(|stage| PipeStageSpec {
            span: stage.span,
            input_subject: Type::lower(&stage.input_subject),
            result_subject: Type::lower(&stage.result_subject),
            kind: match &stage.kind {
                GateRuntimePipeStageKind::Transform { .. } => PipeStageKindSpec::Transform,
                GateRuntimePipeStageKind::Tap { .. } => PipeStageKindSpec::Tap,
                GateRuntimePipeStageKind::Gate {
                    emits_negative_update,
                    ..
                } => PipeStageKindSpec::Gate {
                    emits_negative_update: *emits_negative_update,
                },
                GateRuntimePipeStageKind::Case { arms } => PipeStageKindSpec::Case {
                    arms: arms
                        .iter()
                        .map(|arm| CaseArmSpec {
                            span: arm.span,
                            pattern: arm.pattern,
                        })
                        .collect(),
                },
                GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                    PipeStageKindSpec::TruthyFalsy {
                        truthy: TruthyFalsyArmSpec::from_hir(truthy),
                        falsy: TruthyFalsyArmSpec::from_hir(falsy),
                    }
                }
            },
        })
        .collect()
}

#[derive(Clone)]
enum SegmentSpec {
    Fragment { raw: Box<str>, span: SourceSpan },
    Interpolation { span: SourceSpan },
}

#[derive(Clone)]
struct PipeStageSpec {
    span: SourceSpan,
    input_subject: Type,
    result_subject: Type,
    kind: PipeStageKindSpec,
}

impl PipeStageSpec {
    fn child_expr_count(&self) -> usize {
        self.kind.child_expr_count()
    }
}

#[derive(Clone)]
enum PipeStageKindSpec {
    Transform,
    Tap,
    Gate {
        emits_negative_update: bool,
    },
    Case {
        arms: Vec<CaseArmSpec>,
    },
    TruthyFalsy {
        truthy: TruthyFalsyArmSpec,
        falsy: TruthyFalsyArmSpec,
    },
}

impl PipeStageKindSpec {
    fn child_expr_count(&self) -> usize {
        match self {
            Self::Transform | Self::Tap | Self::Gate { .. } => 1,
            Self::Case { arms } => arms.len(),
            Self::TruthyFalsy { .. } => 2,
        }
    }
}

#[derive(Clone)]
struct CaseArmSpec {
    span: SourceSpan,
    pattern: HirPatternId,
}

#[derive(Clone)]
struct TruthyFalsyArmSpec {
    span: SourceSpan,
    constructor: aivi_hir::BuiltinTerm,
    payload_subject: Option<aivi_hir::GateType>,
    result_type: aivi_hir::GateType,
}

impl TruthyFalsyArmSpec {
    fn from_hir(branch: &GateRuntimeTruthyFalsyBranch) -> Self {
        Self {
            span: branch.span,
            constructor: branch.constructor,
            payload_subject: branch.payload_subject.clone(),
            result_type: branch.result_type.clone(),
        }
    }
}

impl<'a> RuntimeFragmentLowerer<'a> {
    fn new(hir: &'a aivi_hir::Module, fragment: &'a RuntimeFragmentSpec) -> Self {
        let report_by_owner = elaborate_general_expressions(hir)
            .into_items()
            .into_iter()
            .map(|item| (item.owner, item))
            .collect();
        Self {
            lowerer: ModuleLowerer::new(hir),
            fragment,
            report_by_owner,
            lowering: HashSet::new(),
            lowered: HashSet::new(),
        }
    }

    fn build(mut self) -> Result<LoweredRuntimeFragment, LoweringErrors> {
        for dependency in referenced_hir_items(&self.fragment.body) {
            self.ensure_hir_item_lowered(dependency);
        }

        let fragment_item = self
            .lowerer
            .module
            .items_mut()
            .alloc(Item {
                origin: self.fragment.owner,
                span: self.lowerer.hir.exprs()[self.fragment.body_expr].span,
                name: self.fragment.name.clone(),
                kind: if self.fragment.parameters.is_empty() {
                    ItemKind::Value
                } else {
                    ItemKind::Function
                },
                parameters: self
                    .fragment
                    .parameters
                    .iter()
                    .map(|parameter| ItemParameter {
                        binding: parameter.binding,
                        span: parameter.span,
                        name: parameter.name.clone(),
                        ty: Type::lower(&parameter.ty),
                    })
                    .collect(),
                body: None,
                pipes: Vec::new(),
            })
            .map_err(|overflow| LoweringErrors::new(vec![arena_overflow("items", overflow)]))?;

        match self
            .lowerer
            .lower_runtime_expr(self.fragment.owner, &self.fragment.body)
        {
            Ok(body) => {
                let item = self
                    .lowerer
                    .module
                    .items_mut()
                    .get_mut(fragment_item)
                    .expect("freshly allocated runtime fragment item should exist");
                item.body = Some(body);
            }
            Err(error) => self.lowerer.errors.push(error),
        }

        if !self.lowerer.errors.is_empty() {
            return Err(LoweringErrors::new(self.lowerer.errors));
        }
        if let Err(validation) = validate_module(&self.lowerer.module) {
            self.lowerer.errors.extend(
                validation
                    .into_errors()
                    .into_iter()
                    .map(LoweringError::Validation),
            );
            return Err(LoweringErrors::new(self.lowerer.errors));
        }

        Ok(LoweredRuntimeFragment {
            entry_name: self.fragment.name.clone(),
            module: self.lowerer.module,
        })
    }

    fn ensure_hir_item_lowered(&mut self, owner: HirItemId) {
        if self.lowered.contains(&owner) || self.lowering.contains(&owner) {
            return;
        }
        let Some(report) = self.report_by_owner.get(&owner).cloned() else {
            self.lowerer
                .errors
                .push(LoweringError::UnknownOwner { owner });
            return;
        };
        let Some(core_item) = self.seed_hir_item(owner) else {
            return;
        };
        let body = match report.outcome {
            GeneralExprOutcome::Lowered(body) => body,
            GeneralExprOutcome::Blocked(blocked) => {
                self.lowerer.errors.push(LoweringError::BlockedGeneralExpr {
                    owner,
                    body_expr: report.body_expr,
                    span: blocked.primary_span().unwrap_or_default(),
                    blocked,
                });
                return;
            }
        };

        self.lowering.insert(owner);
        for dependency in referenced_hir_items(&body) {
            self.ensure_hir_item_lowered(dependency);
        }
        if self.lowerer.errors.is_empty() {
            match self.lowerer.lower_runtime_expr(owner, &body) {
                Ok(lowered_body) => {
                    let item = self
                        .lowerer
                        .module
                        .items_mut()
                        .get_mut(core_item)
                        .expect("seeded runtime dependency item should exist");
                    item.parameters = report
                        .parameters
                        .iter()
                        .map(|parameter| ItemParameter {
                            binding: parameter.binding,
                            span: parameter.span,
                            name: parameter.name.clone(),
                            ty: Type::lower(&parameter.ty),
                        })
                        .collect();
                    item.body = Some(lowered_body);
                }
                Err(error) => self.lowerer.errors.push(error),
            }
        }
        self.lowering.remove(&owner);
        self.lowered.insert(owner);
    }

    fn seed_hir_item(&mut self, owner: HirItemId) -> Option<ItemId> {
        if let Some(item) = self.lowerer.item_map.get(&owner).copied() {
            return Some(item);
        }
        let item = self.lowerer.hir.items().get(owner)?;
        let (span, name, kind) = match item {
            HirItem::Value(item) => (item.header.span, item.name.text().into(), ItemKind::Value),
            HirItem::Function(item) => (
                item.header.span,
                item.name.text().into(),
                ItemKind::Function,
            ),
            HirItem::Signal(item) => (
                item.header.span,
                item.name.text().into(),
                ItemKind::Signal(SignalInfo::default()),
            ),
            HirItem::Instance(item) => (
                item.header.span,
                format!("instance#{}", owner.as_raw()).into_boxed_str(),
                ItemKind::Instance,
            ),
            HirItem::Type(_)
            | HirItem::Class(_)
            | HirItem::Domain(_)
            | HirItem::SourceProviderContract(_)
            | HirItem::Use(_)
            | HirItem::Export(_) => {
                self.lowerer
                    .errors
                    .push(LoweringError::UnknownOwner { owner });
                return None;
            }
        };
        let item_id = match self.lowerer.module.items_mut().alloc(Item {
            origin: owner,
            span,
            name,
            kind,
            parameters: Vec::new(),
            body: None,
            pipes: Vec::new(),
        }) {
            Ok(item_id) => item_id,
            Err(overflow) => {
                self.lowerer.errors.push(arena_overflow("items", overflow));
                return None;
            }
        };
        self.lowerer.item_map.insert(owner, item_id);
        Some(item_id)
    }
}

fn referenced_hir_items(root: &GateRuntimeExpr) -> Vec<HirItemId> {
    let mut seen = HashSet::new();
    let mut work = vec![root];
    while let Some(expr) = work.pop() {
        match &expr.kind {
            GateRuntimeExprKind::AmbientSubject
            | GateRuntimeExprKind::Integer(_)
            | GateRuntimeExprKind::SuffixedInteger(_)
            | GateRuntimeExprKind::Reference(GateRuntimeReference::Local(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::Builtin(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::DomainMember(_))
            | GateRuntimeExprKind::Reference(GateRuntimeReference::SumConstructor(_)) => {}
            GateRuntimeExprKind::Reference(GateRuntimeReference::Item(item)) => {
                seen.insert(*item);
            }
            GateRuntimeExprKind::Text(text) => {
                for segment in text.segments.iter().rev() {
                    if let GateRuntimeTextSegment::Interpolation(interpolation) = segment {
                        work.push(interpolation);
                    }
                }
            }
            GateRuntimeExprKind::Tuple(elements)
            | GateRuntimeExprKind::List(elements)
            | GateRuntimeExprKind::Set(elements) => {
                for element in elements.iter().rev() {
                    work.push(element);
                }
            }
            GateRuntimeExprKind::Map(entries) => {
                for entry in entries.iter().rev() {
                    work.push(&entry.value);
                    work.push(&entry.key);
                }
            }
            GateRuntimeExprKind::Record(fields) => {
                for field in fields.iter().rev() {
                    work.push(&field.value);
                }
            }
            GateRuntimeExprKind::Projection { base, .. } => {
                if let GateRuntimeProjectionBase::Expr(base) = base {
                    work.push(base);
                }
            }
            GateRuntimeExprKind::Apply { callee, arguments } => {
                for argument in arguments.iter().rev() {
                    work.push(argument);
                }
                work.push(callee);
            }
            GateRuntimeExprKind::Unary { expr, .. } => work.push(expr),
            GateRuntimeExprKind::Binary { left, right, .. } => {
                work.push(right);
                work.push(left);
            }
            GateRuntimeExprKind::Pipe(pipe) => {
                work.push(&pipe.head);
                for stage in pipe.stages.iter().rev() {
                    match &stage.kind {
                        GateRuntimePipeStageKind::Transform { expr }
                        | GateRuntimePipeStageKind::Tap { expr }
                        | GateRuntimePipeStageKind::Gate {
                            predicate: expr, ..
                        } => work.push(expr),
                        GateRuntimePipeStageKind::Case { arms } => {
                            for arm in arms.iter().rev() {
                                work.push(&arm.body);
                            }
                        }
                        GateRuntimePipeStageKind::TruthyFalsy { truthy, falsy } => {
                            work.push(&falsy.body);
                            work.push(&truthy.body);
                        }
                    }
                }
            }
        }
    }
    let mut items = seen.into_iter().collect::<Vec<_>>();
    items.sort_by_key(|item| item.as_raw());
    items
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::SourceDatabase;
    use aivi_syntax::parse_module;

    use super::{LoweringError, lower_module};
    use crate::{
        DecodeStep, GateStage, ItemKind, StageKind, Type,
        validate::{ValidationError, validate_module},
    };

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
    }

    fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        aivi_hir::lower_module(&parsed.module)
    }

    fn lower_fixture(path: &str) -> aivi_hir::LoweringResult {
        let text =
            fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
        lower_text(path, &text)
    }

    #[test]
    fn lowers_pipe_and_source_fixtures_into_core_ir() {
        let lowered = lower_fixture("milestone-2/valid/pipe-gate-carriers/main.aivi");
        assert!(
            !lowered.has_errors(),
            "gate fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        validate_module(&core).expect("lowered core module should validate");

        let maybe_active = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "maybeActive")
            .map(|(id, _)| id)
            .expect("expected maybeActive item");
        let pipes = &core.items()[maybe_active].pipes;
        assert_eq!(pipes.len(), 1);
        let pipe = &core.pipes()[pipes[0]];
        let first_stage = &core.stages()[pipe.stages[0]];
        assert!(matches!(
            &first_stage.kind,
            StageKind::Gate(GateStage::Ordinary { .. })
        ));
        let pretty = core.pretty();
        assert!(
            pretty.contains("gate"),
            "pretty dump should mention gate stages: {pretty}"
        );
    }

    #[test]
    fn lowers_source_and_decode_programs_into_core_ir() {
        let lowered = lower_text(
            "typed-core-source-decode.aivi",
            r#"
domain Duration over Int
    parse : Int -> Result Text Duration
    value : Duration -> Int

@source custom.feed
sig timeout : Signal Duration
"#,
        );
        assert!(
            !lowered.has_errors(),
            "source/decode example should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let timeout = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "timeout")
            .map(|(id, _)| id)
            .expect("expected timeout signal item");
        let ItemKind::Signal(info) = &core.items()[timeout].kind else {
            panic!("timeout should remain a signal item");
        };
        let source = info
            .source
            .expect("timeout should carry a lowered source node");
        let decode = core.sources()[source]
            .decode
            .expect("source should carry a decode program");
        match &core.decode_programs()[decode].steps()[core.decode_programs()[decode].root] {
            DecodeStep::Domain { surface, .. } => {
                assert_eq!(surface.member_name.as_ref(), "parse");
                assert_eq!(surface.kind, crate::DomainDecodeSurfaceKind::FallibleResult);
            }
            other => panic!("expected domain decode root, found {other:?}"),
        }
    }

    #[test]
    fn lowers_source_payload_values_into_typed_core_ir() {
        let lowered = lower_text(
            "typed-core-source-config.aivi",
            r#"
sig apiHost = "https://api.example.com"
sig refresh = 0
sig enabled = True
sig pollInterval = 5

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
sig users : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "source config fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let users = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "users")
            .map(|(id, _)| id)
            .expect("expected users signal item");
        let ItemKind::Signal(info) = &core.items()[users].kind else {
            panic!("users should remain a signal item");
        };
        let source = &core.sources()[info
            .source
            .expect("users should carry a lowered source node")];
        assert_eq!(source.arguments.len(), 1);
        assert_eq!(source.options.len(), 3);
        assert!(matches!(
            core.exprs()[source.arguments[0].runtime_expr].kind,
            crate::ExprKind::Text(_)
        ));
        assert_eq!(source.options[0].option_name.as_ref(), "refreshOn");
        assert_eq!(source.options[1].option_name.as_ref(), "activeWhen");
        assert_eq!(source.options[2].option_name.as_ref(), "refreshEvery");
    }

    #[test]
    fn lowers_value_and_function_bodies_into_typed_core_exprs() {
        let lowered = lower_text(
            "typed-core-general-exprs.aivi",
            r#"
val answer = 42

fun add:Int #x:Int #y:Int =>
    x + y
"#,
        );
        assert!(
            !lowered.has_errors(),
            "general-expression fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let answer = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "answer")
            .map(|(id, _)| id)
            .expect("expected answer value item");
        let answer_body = core.items()[answer]
            .body
            .expect("answer should carry a lowered value body");
        assert!(matches!(
            core.exprs()[answer_body].kind,
            crate::ExprKind::Integer(_)
        ));

        let add = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "add")
            .map(|(id, _)| id)
            .expect("expected add function item");
        assert_eq!(core.items()[add].parameters.len(), 2);
        let add_body = core.items()[add]
            .body
            .expect("add should carry a lowered function body");
        assert!(matches!(
            core.exprs()[add_body].kind,
            crate::ExprKind::Binary {
                operator: aivi_hir::BinaryOperator::Add,
                ..
            }
        ));
    }

    #[test]
    fn lowers_case_and_truthy_falsy_pipe_bodies() {
        let lowered = lower_fixture("milestone-1/valid/pipes/pipe_algebra.aivi");
        assert!(
            !lowered.has_errors(),
            "pipe algebra fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let status_label = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "statusLabel")
            .map(|(id, _)| id)
            .expect("expected statusLabel function item");
        let status_body = core.items()[status_label]
            .body
            .expect("statusLabel should carry a lowered body");
        let crate::ExprKind::Pipe(status_pipe) = &core.exprs()[status_body].kind else {
            panic!("statusLabel should lower to a pipe expression");
        };
        assert!(matches!(
            status_pipe.stages[0].kind,
            crate::PipeStageKind::Case { .. }
        ));

        let start_or_wait = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "startOrWait")
            .map(|(id, _)| id)
            .expect("expected startOrWait function item");
        let start_or_wait_body = core.items()[start_or_wait]
            .body
            .expect("startOrWait should carry a lowered body");
        let crate::ExprKind::Pipe(branch_pipe) = &core.exprs()[start_or_wait_body].kind else {
            panic!("startOrWait should lower to a pipe expression");
        };
        assert!(matches!(
            branch_pipe.stages[0].kind,
            crate::PipeStageKind::TruthyFalsy(_)
        ));
    }

    #[test]
    fn lowers_recurrence_reports_into_pipe_nodes() {
        let lowered = lower_text(
            "typed-core-recurrence.aivi",
            r#"
domain Duration over Int
    literal s : Int -> Duration

domain Retry over Int
    literal x : Int -> Retry

fun step:Int #value:Int =>
    value

@recur.timer 5s
sig polled : Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3x
val retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
        );
        assert!(
            !lowered.has_errors(),
            "recurrence fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let polled = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "polled")
            .map(|(id, _)| id)
            .expect("expected polled signal item");
        let pipe = &core.pipes()[core.items()[polled].pipes[0]];
        let recurrence = pipe
            .recurrence
            .as_ref()
            .expect("expected recurrence attachment");
        assert!(recurrence.guards.is_empty());
        assert_eq!(recurrence.steps.len(), 1);
        assert!(recurrence.non_source_wakeup.is_some());
    }

    #[test]
    fn lowers_recurrence_guards_into_pipe_nodes() {
        let lowered = lower_text(
            "typed-core-recurrence-guard.aivi",
            r#"
domain Duration over Int
    literal s : Int -> Duration

type Cursor = {
    hasNext: Bool
}

fun keep:Cursor #cursor:Cursor =>
    cursor

val seed:Cursor = { hasNext: True }

@recur.timer 1s
sig cursor : Signal Cursor =
    seed
     @|> keep
     ?|> .hasNext
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "guarded recurrence should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let cursor = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "cursor")
            .map(|(id, _)| id)
            .expect("expected cursor signal item");
        let pipe = &core.pipes()[core.items()[cursor].pipes[0]];
        let recurrence = pipe
            .recurrence
            .as_ref()
            .expect("expected guarded recurrence attachment");
        assert_eq!(recurrence.guards.len(), 1);
        assert_eq!(recurrence.steps.len(), 1);
    }

    #[test]
    fn rejects_blocked_hir_handoffs_instead_of_guessing() {
        let lowered = lower_fixture("milestone-2/invalid/gate-predicate-not-bool/main.aivi");
        let errors = lower_module(lowered.module()).expect_err("blocked gate should stop lowering");
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| matches!(error, LoweringError::BlockedGateStage { .. }))
        );
    }

    #[test]
    fn rejects_blocked_general_expr_handoffs_instead_of_guessing() {
        let lowered = lower_fixture("milestone-2/valid/use-member-imports/main.aivi");
        assert!(
            !lowered.has_errors(),
            "blocked general-expression fixture should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let errors = lower_module(lowered.module())
            .expect_err("blocked general expression should stop lowering");
        let blocked = errors
            .errors()
            .iter()
            .find_map(|error| match error {
                LoweringError::BlockedGeneralExpr { blocked, .. } => Some(blocked),
                _ => None,
            })
            .expect("expected blocked general-expression error");
        assert!(matches!(
            blocked.blockers.as_slice(),
            [aivi_hir::GeneralExprBlocker::UnsupportedImportReference { .. }]
        ));
        let rendered = errors.to_string();
        assert!(
            rendered.contains("imported names are not supported in typed-core general expressions"),
            "blocked general-expression error should explain the unsupported import handoff: {rendered}"
        );
    }

    #[test]
    fn rejects_blocked_decode_programs() {
        let lowered = lower_text(
            "typed-core-blocked-decode.aivi",
            r#"
domain Duration over Int
    millis : Int -> Duration
    tryMillis : Int -> Result Text Duration
    value : Duration -> Int

@source custom.feed
sig timeout : Signal Duration
"#,
        );
        assert!(
            !lowered.has_errors(),
            "ambiguous decode example should lower cleanly before typed-core lowering: {:?}",
            lowered.diagnostics()
        );

        let errors =
            lower_module(lowered.module()).expect_err("ambiguous decode should block lowering");
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| matches!(error, LoweringError::BlockedDecodeProgram { .. }))
        );
    }

    #[test]
    fn validator_catches_broken_recurrence_closure() {
        let lowered = lower_text(
            "typed-core-recurrence.aivi",
            r#"
domain Duration over Int
    literal s : Int -> Duration

domain Retry over Int
    literal x : Int -> Retry

fun step:Int #value:Int =>
    value

@recur.timer 5s
sig polled : Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3x
val retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
        );
        let mut core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let pipe_id = core
            .pipes()
            .iter()
            .find(|(_, pipe)| pipe.recurrence.is_some())
            .map(|(id, _)| id)
            .expect("expected recurrence pipe");
        let pipe = core
            .pipes_mut()
            .get_mut(pipe_id)
            .expect("pipe should exist");
        let recurrence = pipe.recurrence.as_mut().expect("recurrence should exist");
        recurrence.steps[0].result_subject = Type::Primitive(aivi_hir::BuiltinType::Text);
        let errors =
            validate_module(&core).expect_err("manually broken recurrence should fail validation");
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| matches!(error, ValidationError::RecurrenceDoesNotClose { .. }))
        );
    }

    #[test]
    fn validator_catches_broken_inline_case_stage_result_types() {
        let lowered = lower_fixture("milestone-1/valid/patterns/pattern_matching.aivi");
        let mut core = lower_module(lowered.module()).expect("typed-core lowering should succeed");
        let loaded_name = core
            .items()
            .iter()
            .find(|(_, item)| item.name.as_ref() == "loadedName")
            .map(|(id, _)| id)
            .expect("expected loadedName function item");
        let body = core.items()[loaded_name]
            .body
            .expect("loadedName should carry a lowered body");
        let crate::ExprKind::Pipe(pipe) = &core.exprs()[body].kind else {
            panic!("loadedName should lower to a pipe expression");
        };
        let crate::PipeStageKind::Case { arms } = &pipe.stages[0].kind else {
            panic!("loadedName should start with a case stage");
        };
        let bad_arm = arms[0].body;
        core.exprs_mut()
            .get_mut(bad_arm)
            .expect("case arm body should exist")
            .ty = Type::Primitive(aivi_hir::BuiltinType::Int);
        let errors = validate_module(&core).expect_err("broken inline case stage should fail");
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            ValidationError::InlinePipeCaseArmResultMismatch { .. }
        )));
    }
}
