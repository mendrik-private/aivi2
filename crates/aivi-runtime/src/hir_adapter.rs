use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
};

use aivi_backend::{CommittedValueStore, InlineCommittedValueStore};
use aivi_base::SourceSpan;
use aivi_hir as hir;
use aivi_typing::RecurrenceTarget;

use crate::{
    effects::{
        RuntimeSourceProvider, SourceInstanceId, SourceReplacementPolicy, SourceRuntimeSpec,
        SourceStaleWorkPolicy, TaskInstanceId, TaskRuntimeSpec, TaskSourceRuntime,
        TaskSourceRuntimeError,
    },
    graph::{
        DerivedHandle, GraphBuildError, InputHandle, OwnerHandle, SignalGraph, SignalGraphBuilder,
        SignalHandle,
    },
};

/// Build a concrete runtime assembly directly from the current HIR handoffs.
///
/// The current compiler wave already exposes source lifecycle plans, decode programs, gate plans,
/// and recurrence plans, but it does not yet define full evaluation/codegen semantics for every
/// signal body. This adapter therefore chooses the narrowest coherent runtime boundary:
///
/// - every `sig` with a body becomes one public runtime signal node,
/// - every `@source` contributes one concrete source runtime spec and publication input,
/// - every directly annotated top-level `val ... : Task E A` contributes one concrete task
///   runtime spec plus a dedicated scheduler-owned sink input,
/// - source-decorated signals with bodies keep those identities separate so the unresolved
///   publication-to-body lowering gap stays explicit,
/// - and gate/recurrence plans are preserved as typed runtime-facing attachments rather than being
///   re-derived from raw HIR later.
///
/// This keeps the compiler/runtime seam honest, explicit, and testable without inventing execution
/// semantics the current frontend has not justified yet.
pub fn assemble_hir_runtime(
    module: &hir::Module,
) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
    let source_lifecycles = hir::elaborate_source_lifecycles(module);
    let source_decode_programs = hir::generate_source_decode_programs(module);
    let recurrences = hir::elaborate_recurrences(module);
    let gates = hir::elaborate_gates(module);
    HirRuntimeAssemblyBuilder::new(
        module,
        &source_lifecycles,
        &source_decode_programs,
        &recurrences,
        &gates,
    )
    .build()
}

pub fn assemble_hir_runtime_with_items(
    module: &hir::Module,
    included_items: &HashSet<hir::ItemId>,
) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
    let source_lifecycles = hir::elaborate_source_lifecycles(module);
    let source_decode_programs = hir::generate_source_decode_programs(module);
    let recurrences = hir::elaborate_recurrences(module);
    let gates = hir::elaborate_gates(module);
    HirRuntimeAssemblyBuilder::new_with_items(
        module,
        &source_lifecycles,
        &source_decode_programs,
        &recurrences,
        &gates,
        included_items,
    )
    .build()
}

#[derive(Clone, Debug)]
pub struct HirRuntimeAssemblyBuilder<'a> {
    module: &'a hir::Module,
    source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
    source_decode_programs: &'a hir::SourceDecodeProgramReport,
    recurrences: &'a hir::RecurrenceElaborationReport,
    gates: &'a hir::GateElaborationReport,
    included_items: Option<&'a HashSet<hir::ItemId>>,
}

impl<'a> HirRuntimeAssemblyBuilder<'a> {
    pub const fn new(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
    ) -> Self {
        Self::new_internal(
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            None,
        )
    }

    pub const fn new_with_items(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
        included_items: &'a HashSet<hir::ItemId>,
    ) -> Self {
        Self::new_internal(
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            Some(included_items),
        )
    }

    const fn new_internal(
        module: &'a hir::Module,
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        recurrences: &'a hir::RecurrenceElaborationReport,
        gates: &'a hir::GateElaborationReport,
        included_items: Option<&'a HashSet<hir::ItemId>>,
    ) -> Self {
        Self {
            module,
            source_lifecycles,
            source_decode_programs,
            recurrences,
            gates,
            included_items,
        }
    }

    fn includes_item(&self, item: hir::ItemId) -> bool {
        self.included_items
            .as_ref()
            .is_none_or(|included| included.contains(&item))
    }

    pub fn build(self) -> Result<HirRuntimeAssembly, HirRuntimeAdapterErrors> {
        let mut errors = Vec::new();
        let report_index = ReportIndex::new(
            self.source_lifecycles,
            self.source_decode_programs,
            &mut errors,
        );
        let mut graph_builder = SignalGraphBuilder::new();
        let mut owners = Vec::new();
        let mut signals = Vec::new();
        let mut public_signals = BTreeMap::<hir::ItemId, SignalHandle>::new();
        let mut source_inputs = BTreeMap::<hir::ItemId, InputHandle>::new();

        for (item_id, item) in self.module.items().iter() {
            if !self.includes_item(item_id) {
                continue;
            }
            let hir::Item::Signal(signal) = item else {
                continue;
            };
            let has_source = signal.source_metadata.is_some();
            let has_body = signal.body.is_some();

            let owner = match graph_builder.add_owner(signal.name.text(), None) {
                Ok(owner) => owner,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            owners.push(HirOwnerBinding {
                item: item_id,
                span: signal.header.span,
                name: signal.name.text().into(),
                handle: owner,
            });

            let (kind, source_input) = if has_body {
                let derived = match graph_builder.add_derived(signal.name.text(), Some(owner)) {
                    Ok(derived) => derived,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                let source_input = if has_source {
                    match graph_builder
                        .add_input(format!("{}#source", signal.name.text()), Some(owner))
                    {
                        Ok(input) => Some(input),
                        Err(err) => {
                            errors.push(HirRuntimeAdapterError::GraphBuild(err));
                            continue;
                        }
                    }
                } else {
                    None
                };
                public_signals.insert(item_id, derived.as_signal());
                (
                    HirSignalBindingKind::Derived {
                        signal: derived,
                        dependencies: Vec::new().into_boxed_slice(),
                    },
                    source_input,
                )
            } else {
                let input = match graph_builder.add_input(signal.name.text(), Some(owner)) {
                    Ok(input) => input,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
                public_signals.insert(item_id, input.as_signal());
                (HirSignalBindingKind::Input { signal: input }, Some(input))
            };

            if let Some(input) = source_input {
                source_inputs.insert(item_id, input);
            }
            signals.push(HirSignalBinding {
                item: item_id,
                span: signal.header.span,
                name: signal.name.text().into(),
                owner,
                kind,
                source_input,
            });
        }

        for binding in &mut signals {
            let Some(hir::Item::Signal(signal)) = self.module.items().get(binding.item) else {
                continue;
            };
            let HirSignalBindingKind::Derived {
                signal: derived,
                dependencies,
            } = &mut binding.kind
            else {
                continue;
            };

            let mut resolved = Vec::with_capacity(signal.signal_dependencies.len());
            let mut blocked = false;
            for dependency in &signal.signal_dependencies {
                match self.module.items().get(*dependency) {
                    Some(hir::Item::Signal(_)) => match public_signals.get(dependency).copied() {
                        Some(handle) => resolved.push(handle),
                        None => {
                            errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                                owner: binding.item,
                                dependency: *dependency,
                            });
                            blocked = true;
                        }
                    },
                    Some(item) => {
                        errors.push(HirRuntimeAdapterError::DependencyIsNotSignal {
                            owner: binding.item,
                            dependency: *dependency,
                            kind: item.kind(),
                        });
                        blocked = true;
                    }
                    None => {
                        errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                            owner: binding.item,
                            dependency: *dependency,
                        });
                        blocked = true;
                    }
                }
            }
            if blocked {
                continue;
            }

            // For source-backed derived signals, also depend on the source input signal.
            if let Some(source_input) = binding.source_input {
                resolved.push(source_input.as_signal());
            }

            if let Err(err) = graph_builder.define_derived(*derived, resolved.iter().copied()) {
                errors.push(HirRuntimeAdapterError::GraphBuild(err));
                continue;
            }
            *dependencies = resolved.into_boxed_slice();
        }

        let mut task_wakeups = BTreeMap::new();
        for node in self.recurrences.nodes() {
            if !self.includes_item(node.owner) {
                continue;
            }
            let hir::RecurrenceNodeOutcome::Planned(plan) = &node.outcome else {
                continue;
            };
            if plan.target.target() != RecurrenceTarget::Task {
                continue;
            }
            if task_wakeups.insert(node.owner, plan.wakeup).is_some() {
                errors.push(HirRuntimeAdapterError::DuplicateTaskOwner { owner: node.owner });
            }
        }

        let mut sources = Vec::new();
        let mut seen_source_owners = BTreeSet::new();
        for binding in &signals {
            let Some(hir::Item::Signal(signal)) = self.module.items().get(binding.item) else {
                continue;
            };
            if signal.source_metadata.is_none() {
                continue;
            }
            seen_source_owners.insert(binding.item);

            let Some(lifecycle_node) = report_index.source_lifecycles.get(&binding.item).copied()
            else {
                errors.push(HirRuntimeAdapterError::MissingSourceLifecycle {
                    owner: binding.item,
                });
                continue;
            };
            let lifecycle = match &lifecycle_node.outcome {
                hir::SourceLifecycleNodeOutcome::Planned(plan) => plan.clone(),
                hir::SourceLifecycleNodeOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedSourceLifecycle {
                        owner: binding.item,
                        source_span: lifecycle_node.source_span,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                    continue;
                }
            };

            let Some(decode_node) = report_index
                .source_decode_programs
                .get(&binding.item)
                .copied()
            else {
                errors.push(HirRuntimeAdapterError::MissingSourceDecodeProgram {
                    owner: binding.item,
                });
                continue;
            };
            let decode = match &decode_node.outcome {
                hir::SourceDecodeProgramOutcome::Planned(program) => program.clone(),
                hir::SourceDecodeProgramOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedSourceDecodeProgram {
                        owner: binding.item,
                        source_span: decode_node.source_span,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                    continue;
                }
            };

            let Some(input) = source_inputs.get(&binding.item).copied() else {
                errors.push(HirRuntimeAdapterError::MissingSourceInput {
                    owner: binding.item,
                });
                continue;
            };
            let spec = match self.build_source_spec(
                binding.item,
                input,
                &lifecycle,
                &decode,
                &public_signals,
            ) {
                Ok(spec) => spec,
                Err(mut source_errors) => {
                    errors.append(&mut source_errors);
                    continue;
                }
            };

            sources.push(HirSourceBinding {
                owner: binding.item,
                source_instance: lifecycle.instance,
                source_span: lifecycle_node.source_span,
                teardown: lifecycle.teardown,
                signal: binding.signal(),
                input,
                spec,
            });
        }

        for owner in report_index.source_lifecycles.keys() {
            if !self.includes_item(*owner) {
                continue;
            }
            if !seen_source_owners.contains(owner) {
                errors
                    .push(HirRuntimeAdapterError::UnexpectedSourceLifecycleOwner { owner: *owner });
            }
        }
        for owner in report_index.source_decode_programs.keys() {
            if !self.includes_item(*owner) {
                continue;
            }
            if !seen_source_owners.contains(owner) {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceDecodeOwner { owner: *owner });
            }
        }

        let mut tasks = Vec::new();
        for (item_id, item) in self.module.items().iter() {
            if !self.includes_item(item_id) {
                continue;
            }
            let hir::Item::Value(value) = item else {
                continue;
            };
            let Some(annotation) = value.annotation else {
                continue;
            };
            if !annotation_is_task(self.module, annotation) {
                continue;
            }

            let owner = match graph_builder.add_owner(value.name.text(), None) {
                Ok(owner) => owner,
                Err(err) => {
                    errors.push(HirRuntimeAdapterError::GraphBuild(err));
                    continue;
                }
            };
            owners.push(HirOwnerBinding {
                item: item_id,
                span: value.header.span,
                name: value.name.text().into(),
                handle: owner,
            });

            let input =
                match graph_builder.add_input(format!("{}#task", value.name.text()), Some(owner)) {
                    Ok(input) => input,
                    Err(err) => {
                        errors.push(HirRuntimeAdapterError::GraphBuild(err));
                        continue;
                    }
                };
            let mut spec = TaskRuntimeSpec::new(runtime_task_instance(item_id), input);
            spec.wakeup = task_wakeups.get(&item_id).copied();
            tasks.push(HirTaskBinding {
                owner: item_id,
                owner_handle: owner,
                task_span: value.header.span,
                input,
                spec,
            });
        }

        let mut gates = Vec::new();
        let mut gate_sites = BTreeSet::new();
        for stage in self.gates.stages() {
            if !self.includes_item(stage.owner) {
                continue;
            }
            let site = HirGateStageId::new(stage.owner, stage.pipe_expr, stage.stage_index);
            if !gate_sites.insert(site) {
                errors.push(HirRuntimeAdapterError::DuplicateGateStage { site });
                continue;
            }
            let owner_signal = signal_derived_handle(&signals, stage.owner);
            match &stage.outcome {
                hir::GateStageOutcome::Ordinary(plan) => gates.push(HirGateStageBinding {
                    site,
                    stage_span: stage.stage_span,
                    predicate: stage.predicate,
                    owner_signal,
                    plan: HirRuntimeGatePlan::Ordinary(plan.clone()),
                }),
                hir::GateStageOutcome::SignalFilter(plan) => gates.push(HirGateStageBinding {
                    site,
                    stage_span: stage.stage_span,
                    predicate: stage.predicate,
                    owner_signal,
                    plan: HirRuntimeGatePlan::SignalFilter(plan.clone()),
                }),
                hir::GateStageOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedGateStage {
                        site,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                }
            }
        }

        let mut recurrences = Vec::new();
        let mut recurrence_sites = BTreeSet::new();
        for node in self.recurrences.nodes() {
            if !self.includes_item(node.owner) {
                continue;
            }
            let site = HirRecurrenceNodeId::new(node.owner, node.pipe_expr, node.start_stage_index);
            if !recurrence_sites.insert(site) {
                errors.push(HirRuntimeAdapterError::DuplicateRecurrenceNode { site });
                continue;
            }
            let owner_signal = signal_derived_handle(&signals, node.owner);
            match &node.outcome {
                hir::RecurrenceNodeOutcome::Planned(plan) => {
                    recurrences.push(HirRecurrenceBinding {
                        site,
                        start_stage_span: node.start_stage_span,
                        owner_signal,
                        plan: plan.clone(),
                    });
                }
                hir::RecurrenceNodeOutcome::Blocked(blocked) => {
                    errors.push(HirRuntimeAdapterError::BlockedRecurrenceNode {
                        site,
                        blockers: blocked.blockers.clone().into_boxed_slice(),
                    });
                }
            }
        }

        if !errors.is_empty() {
            return Err(HirRuntimeAdapterErrors::new(errors));
        }

        let graph = graph_builder.build().map_err(|err| {
            HirRuntimeAdapterErrors::new(vec![HirRuntimeAdapterError::GraphBuild(err)])
        })?;

        Ok(HirRuntimeAssembly {
            graph,
            owners: owners.into_boxed_slice(),
            signals: signals.into_boxed_slice(),
            sources: sources.into_boxed_slice(),
            tasks: tasks.into_boxed_slice(),
            gates: gates.into_boxed_slice(),
            recurrences: recurrences.into_boxed_slice(),
        })
    }

    fn build_source_spec(
        &self,
        owner: hir::ItemId,
        input: InputHandle,
        lifecycle: &hir::SourceLifecyclePlan,
        decode: &hir::SourceDecodeProgram,
        public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    ) -> Result<SourceRuntimeSpec<hir::SourceDecodeProgram>, Vec<HirRuntimeAdapterError>> {
        let mut errors = Vec::new();
        let provider = match adapt_source_provider(owner, &lifecycle.provider) {
            Ok(provider) => provider,
            Err(error) => {
                errors.push(error);
                RuntimeSourceProvider::custom("<invalid-source-provider>")
            }
        };
        let reconfiguration_dependencies = resolve_signal_dependencies(
            self.module,
            owner,
            &lifecycle.reconfiguration_dependencies,
            public_signals,
            &mut errors,
        );
        let explicit_triggers = lifecycle
            .explicit_triggers
            .iter()
            .filter_map(|binding| {
                resolve_source_option_binding(
                    self.module,
                    owner,
                    binding,
                    public_signals,
                    &mut errors,
                )
            })
            .collect::<Vec<_>>();
        let active_when = lifecycle.active_when.as_ref().and_then(|binding| {
            resolve_source_option_binding(self.module, owner, binding, public_signals, &mut errors)
        });

        if !errors.is_empty() {
            return Err(errors);
        }

        let mut spec =
            SourceRuntimeSpec::new(runtime_source_instance(lifecycle.instance), input, provider);
        spec.reconfiguration_dependencies = reconfiguration_dependencies.into_boxed_slice();
        spec.explicit_triggers = explicit_triggers.into_boxed_slice();
        spec.active_when = active_when;
        spec.cancellation = lifecycle.cancellation;
        spec.replacement = match lifecycle.replacement {
            hir::SourceReplacementPolicy::DisposeSupersededBeforePublish => {
                SourceReplacementPolicy::DisposeSupersededBeforePublish
            }
        };
        spec.stale_work = match lifecycle.stale_work {
            hir::SourceStaleWorkPolicy::DropStalePublications => {
                SourceStaleWorkPolicy::DropStalePublications
            }
        };
        spec.decode = Some(decode.clone());
        Ok(spec)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRuntimeAssembly {
    graph: SignalGraph,
    owners: Box<[HirOwnerBinding]>,
    signals: Box<[HirSignalBinding]>,
    sources: Box<[HirSourceBinding]>,
    tasks: Box<[HirTaskBinding]>,
    gates: Box<[HirGateStageBinding]>,
    recurrences: Box<[HirRecurrenceBinding]>,
}

impl HirRuntimeAssembly {
    pub fn graph(&self) -> &SignalGraph {
        &self.graph
    }

    pub fn owners(&self) -> &[HirOwnerBinding] {
        &self.owners
    }

    pub fn signals(&self) -> &[HirSignalBinding] {
        &self.signals
    }

    pub fn sources(&self) -> &[HirSourceBinding] {
        &self.sources
    }

    pub fn tasks(&self) -> &[HirTaskBinding] {
        &self.tasks
    }

    pub fn gates(&self) -> &[HirGateStageBinding] {
        &self.gates
    }

    pub fn recurrences(&self) -> &[HirRecurrenceBinding] {
        &self.recurrences
    }

    pub fn owner(&self, item: hir::ItemId) -> Option<&HirOwnerBinding> {
        self.owners.iter().find(|binding| binding.item == item)
    }

    pub fn signal(&self, item: hir::ItemId) -> Option<&HirSignalBinding> {
        self.signals.iter().find(|binding| binding.item == item)
    }

    pub fn source_by_owner(&self, item: hir::ItemId) -> Option<&HirSourceBinding> {
        self.sources.iter().find(|binding| binding.owner == item)
    }

    pub fn task_by_owner(&self, item: hir::ItemId) -> Option<&HirTaskBinding> {
        self.tasks.iter().find(|binding| binding.owner == item)
    }

    pub fn instantiate_runtime<V>(
        &self,
    ) -> Result<TaskSourceRuntime<V, hir::SourceDecodeProgram>, HirRuntimeInstantiationError> {
        self.instantiate_runtime_with_value_store(InlineCommittedValueStore::default())
    }

    pub fn instantiate_runtime_with_value_store<V, S>(
        &self,
        storage: S,
    ) -> Result<TaskSourceRuntime<V, hir::SourceDecodeProgram, S>, HirRuntimeInstantiationError>
    where
        S: CommittedValueStore<V>,
    {
        let mut runtime = TaskSourceRuntime::with_value_store(self.graph.clone(), storage);
        for source in &self.sources {
            runtime
                .register_source(source.spec.clone())
                .map_err(|error| HirRuntimeInstantiationError::RegisterSource {
                    owner: source.owner,
                    instance: source.spec.instance,
                    error,
                })?;
        }
        for task in &self.tasks {
            runtime.register_task(task.spec.clone()).map_err(|error| {
                HirRuntimeInstantiationError::RegisterTask {
                    owner: task.owner,
                    instance: task.spec.instance,
                    error,
                }
            })?;
        }
        Ok(runtime)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirOwnerBinding {
    pub item: hir::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub handle: OwnerHandle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirSignalBinding {
    pub item: hir::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub owner: OwnerHandle,
    pub kind: HirSignalBindingKind,
    pub source_input: Option<InputHandle>,
}

impl HirSignalBinding {
    pub fn signal(&self) -> SignalHandle {
        match self.kind {
            HirSignalBindingKind::Input { signal } => signal.as_signal(),
            HirSignalBindingKind::Derived { signal, .. } => signal.as_signal(),
        }
    }

    pub fn input(&self) -> Option<InputHandle> {
        match self.kind {
            HirSignalBindingKind::Input { signal } => Some(signal),
            HirSignalBindingKind::Derived { .. } => None,
        }
    }

    pub fn derived(&self) -> Option<DerivedHandle> {
        match self.kind {
            HirSignalBindingKind::Input { .. } => None,
            HirSignalBindingKind::Derived { signal, .. } => Some(signal),
        }
    }

    pub fn dependencies(&self) -> &[SignalHandle] {
        match &self.kind {
            HirSignalBindingKind::Input { .. } => &[],
            HirSignalBindingKind::Derived { dependencies, .. } => dependencies,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirSignalBindingKind {
    Input {
        signal: InputHandle,
    },
    Derived {
        signal: DerivedHandle,
        dependencies: Box<[SignalHandle]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirSourceBinding {
    pub owner: hir::ItemId,
    pub source_instance: hir::SourceInstanceId,
    pub source_span: SourceSpan,
    pub teardown: hir::SourceTeardownPolicy,
    pub signal: SignalHandle,
    pub input: InputHandle,
    pub spec: SourceRuntimeSpec<hir::SourceDecodeProgram>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirTaskBinding {
    pub owner: hir::ItemId,
    pub owner_handle: OwnerHandle,
    pub task_span: SourceSpan,
    pub input: InputHandle,
    pub spec: TaskRuntimeSpec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HirGateStageId {
    pub owner: hir::ItemId,
    pub pipe_expr: hir::ExprId,
    pub stage_index: usize,
}

impl HirGateStageId {
    pub const fn new(owner: hir::ItemId, pipe_expr: hir::ExprId, stage_index: usize) -> Self {
        Self {
            owner,
            pipe_expr,
            stage_index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirRuntimeGatePlan {
    Ordinary(hir::OrdinaryGateStage),
    SignalFilter(hir::SignalGateFilter),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirGateStageBinding {
    pub site: HirGateStageId,
    pub stage_span: SourceSpan,
    pub predicate: hir::ExprId,
    pub owner_signal: Option<DerivedHandle>,
    pub plan: HirRuntimeGatePlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HirRecurrenceNodeId {
    pub owner: hir::ItemId,
    pub pipe_expr: hir::ExprId,
    pub start_stage_index: usize,
}

impl HirRecurrenceNodeId {
    pub const fn new(owner: hir::ItemId, pipe_expr: hir::ExprId, start_stage_index: usize) -> Self {
        Self {
            owner,
            pipe_expr,
            start_stage_index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRecurrenceBinding {
    pub site: HirRecurrenceNodeId,
    pub start_stage_span: SourceSpan,
    pub owner_signal: Option<DerivedHandle>,
    pub plan: hir::RecurrenceNodePlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirRuntimeInstantiationError {
    RegisterSource {
        owner: hir::ItemId,
        instance: SourceInstanceId,
        error: TaskSourceRuntimeError,
    },
    RegisterTask {
        owner: hir::ItemId,
        instance: TaskInstanceId,
        error: TaskSourceRuntimeError,
    },
}

impl fmt::Display for HirRuntimeInstantiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegisterSource {
                owner,
                instance,
                error,
            } => write!(
                f,
                "failed to register adapted source {instance:?} for owner {owner}: {error:?}"
            ),
            Self::RegisterTask {
                owner,
                instance,
                error,
            } => write!(
                f,
                "failed to register adapted task {instance:?} for owner {owner}: {error:?}"
            ),
        }
    }
}

impl std::error::Error for HirRuntimeInstantiationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HirRuntimeAdapterErrors {
    errors: Box<[HirRuntimeAdapterError]>,
}

impl HirRuntimeAdapterErrors {
    pub fn new(errors: Vec<HirRuntimeAdapterError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[HirRuntimeAdapterError] {
        &self.errors
    }
}

impl fmt::Display for HirRuntimeAdapterErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "failed to adapt HIR into a runtime assembly:")?;
        for error in &self.errors {
            writeln!(f, "- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for HirRuntimeAdapterErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirRuntimeAdapterError {
    SignalHasNoRuntimeNode {
        item: hir::ItemId,
        span: SourceSpan,
    },
    MissingSourceInput {
        owner: hir::ItemId,
    },
    MissingSourceLifecycle {
        owner: hir::ItemId,
    },
    MissingSourceDecodeProgram {
        owner: hir::ItemId,
    },
    UnexpectedSourceLifecycleOwner {
        owner: hir::ItemId,
    },
    UnexpectedSourceDecodeOwner {
        owner: hir::ItemId,
    },
    DuplicateTaskOwner {
        owner: hir::ItemId,
    },
    BlockedSourceLifecycle {
        owner: hir::ItemId,
        source_span: SourceSpan,
        blockers: Box<[hir::SourceLifecycleElaborationBlocker]>,
    },
    BlockedSourceDecodeProgram {
        owner: hir::ItemId,
        source_span: SourceSpan,
        blockers: Box<[hir::SourceDecodeProgramBlocker]>,
    },
    UnsupportedSourceProvider {
        owner: hir::ItemId,
        provider: hir::SourceProviderRef,
    },
    UnknownSignalDependency {
        owner: hir::ItemId,
        dependency: hir::ItemId,
    },
    DependencyIsNotSignal {
        owner: hir::ItemId,
        dependency: hir::ItemId,
        kind: hir::ItemKind,
    },
    UnsupportedSourceOptionSignalExpr {
        owner: hir::ItemId,
        option_name: Box<str>,
        expr: hir::ExprId,
    },
    SourceOptionBindingNotSignal {
        owner: hir::ItemId,
        option_name: Box<str>,
        item: hir::ItemId,
        kind: hir::ItemKind,
    },
    DuplicateGateStage {
        site: HirGateStageId,
    },
    BlockedGateStage {
        site: HirGateStageId,
        blockers: Box<[hir::GateElaborationBlocker]>,
    },
    DuplicateRecurrenceNode {
        site: HirRecurrenceNodeId,
    },
    BlockedRecurrenceNode {
        site: HirRecurrenceNodeId,
        blockers: Box<[hir::RecurrenceElaborationBlocker]>,
    },
    GraphBuild(GraphBuildError),
}

impl fmt::Display for HirRuntimeAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignalHasNoRuntimeNode { item, .. } => write!(
                f,
                "signal item {item} has neither a body nor an @source input to adapt"
            ),
            Self::MissingSourceInput { owner } => {
                write!(
                    f,
                    "source-backed signal {owner} is missing its source input handle"
                )
            }
            Self::MissingSourceLifecycle { owner } => write!(
                f,
                "source-backed signal {owner} has no source lifecycle handoff"
            ),
            Self::MissingSourceDecodeProgram { owner } => write!(
                f,
                "source-backed signal {owner} has no source decode program handoff"
            ),
            Self::UnexpectedSourceLifecycleOwner { owner } => write!(
                f,
                "source lifecycle report references non-source owner {owner}"
            ),
            Self::UnexpectedSourceDecodeOwner { owner } => write!(
                f,
                "source decode report references non-source owner {owner}"
            ),
            Self::DuplicateTaskOwner { owner } => {
                write!(
                    f,
                    "task owner {owner} exposes more than one recurrence handoff"
                )
            }
            Self::BlockedSourceLifecycle {
                owner, blockers, ..
            } => write!(
                f,
                "source lifecycle for owner {owner} is blocked: {blockers:?}"
            ),
            Self::BlockedSourceDecodeProgram {
                owner, blockers, ..
            } => write!(
                f,
                "source decode program for owner {owner} is blocked: {blockers:?}"
            ),
            Self::UnsupportedSourceProvider { owner, provider } => write!(
                f,
                "source owner {owner} uses a provider the runtime adapter cannot lower yet: {provider:?}"
            ),
            Self::UnknownSignalDependency { owner, dependency } => write!(
                f,
                "signal owner {owner} depends on unknown or unbound signal item {dependency}"
            ),
            Self::DependencyIsNotSignal {
                owner,
                dependency,
                kind,
            } => write!(
                f,
                "signal owner {owner} depends on item {dependency}, but it is a {kind:?}"
            ),
            Self::UnsupportedSourceOptionSignalExpr {
                owner,
                option_name,
                expr,
            } => write!(
                f,
                "source owner {owner} uses `{option_name}` expression {expr} that is not a direct signal reference"
            ),
            Self::SourceOptionBindingNotSignal {
                owner,
                option_name,
                item,
                kind,
            } => write!(
                f,
                "source owner {owner} binds `{option_name}` to item {item}, but it is a {kind:?}, not a signal"
            ),
            Self::DuplicateGateStage { site } => {
                write!(f, "duplicate gate handoff encountered at {site:?}")
            }
            Self::BlockedGateStage { site, blockers } => {
                write!(f, "gate handoff {site:?} is blocked: {blockers:?}")
            }
            Self::DuplicateRecurrenceNode { site } => {
                write!(f, "duplicate recurrence handoff encountered at {site:?}")
            }
            Self::BlockedRecurrenceNode { site, blockers } => {
                write!(f, "recurrence handoff {site:?} is blocked: {blockers:?}")
            }
            Self::GraphBuild(error) => write!(f, "signal graph build failed: {error:?}"),
        }
    }
}

impl std::error::Error for HirRuntimeAdapterError {}

#[derive(Clone, Debug)]
struct ReportIndex<'a> {
    source_lifecycles: BTreeMap<hir::ItemId, &'a hir::SourceLifecycleNodeElaboration>,
    source_decode_programs: BTreeMap<hir::ItemId, &'a hir::SourceDecodeProgramNode>,
}

impl<'a> ReportIndex<'a> {
    fn new(
        source_lifecycles: &'a hir::SourceLifecycleElaborationReport,
        source_decode_programs: &'a hir::SourceDecodeProgramReport,
        errors: &mut Vec<HirRuntimeAdapterError>,
    ) -> Self {
        let mut lifecycle_map = BTreeMap::new();
        for node in source_lifecycles.nodes() {
            if lifecycle_map.insert(node.owner, node).is_some() {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceLifecycleOwner {
                    owner: node.owner,
                });
            }
        }

        let mut decode_map = BTreeMap::new();
        for node in source_decode_programs.nodes() {
            if decode_map.insert(node.owner, node).is_some() {
                errors.push(HirRuntimeAdapterError::UnexpectedSourceDecodeOwner {
                    owner: node.owner,
                });
            }
        }

        Self {
            source_lifecycles: lifecycle_map,
            source_decode_programs: decode_map,
        }
    }
}

fn signal_derived_handle(
    signals: &[HirSignalBinding],
    owner: hir::ItemId,
) -> Option<DerivedHandle> {
    signals
        .iter()
        .find(|binding| binding.item == owner)
        .and_then(HirSignalBinding::derived)
}

fn runtime_source_instance(instance: hir::SourceInstanceId) -> SourceInstanceId {
    SourceInstanceId::from_raw(instance.decorator().as_raw())
}

fn runtime_task_instance(item: hir::ItemId) -> TaskInstanceId {
    TaskInstanceId::from_raw(item.as_raw())
}

fn annotation_is_task(module: &hir::Module, ty: hir::TypeId) -> bool {
    type_head_with_arity(module, ty).is_some_and(|(head, arity)| {
        head == hir::TypeResolution::Builtin(hir::BuiltinType::Task) && arity == 2
    })
}

fn type_head_with_arity(
    module: &hir::Module,
    mut ty: hir::TypeId,
) -> Option<(hir::TypeResolution, usize)> {
    let mut arity = 0usize;
    loop {
        match &module.types()[ty].kind {
            hir::TypeKind::Apply { callee, arguments } => {
                arity += arguments.len();
                ty = *callee;
            }
            hir::TypeKind::Name(reference) => {
                return match reference.resolution {
                    hir::ResolutionState::Resolved(head) => Some((head, arity)),
                    hir::ResolutionState::Unresolved => None,
                };
            }
            hir::TypeKind::Tuple(_) | hir::TypeKind::Record(_) | hir::TypeKind::Arrow { .. } => {
                return None;
            }
        }
    }
}

fn adapt_source_provider(
    owner: hir::ItemId,
    provider: &hir::SourceProviderRef,
) -> Result<RuntimeSourceProvider, HirRuntimeAdapterError> {
    match provider {
        hir::SourceProviderRef::Builtin(provider) => Ok(RuntimeSourceProvider::builtin(*provider)),
        hir::SourceProviderRef::Custom(key) => Ok(RuntimeSourceProvider::custom(key.clone())),
        other => Err(HirRuntimeAdapterError::UnsupportedSourceProvider {
            owner,
            provider: other.clone(),
        }),
    }
}

fn resolve_signal_dependencies(
    module: &hir::Module,
    owner: hir::ItemId,
    dependencies: &[hir::ItemId],
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    errors: &mut Vec<HirRuntimeAdapterError>,
) -> Vec<SignalHandle> {
    let mut resolved = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        match module.items().get(*dependency) {
            Some(hir::Item::Signal(_)) => match public_signals.get(dependency).copied() {
                Some(handle) => resolved.push(handle),
                None => errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                    owner,
                    dependency: *dependency,
                }),
            },
            Some(item) => errors.push(HirRuntimeAdapterError::DependencyIsNotSignal {
                owner,
                dependency: *dependency,
                kind: item.kind(),
            }),
            None => errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                owner,
                dependency: *dependency,
            }),
        }
    }
    resolved
}

fn resolve_source_option_binding(
    module: &hir::Module,
    owner: hir::ItemId,
    binding: &hir::SourceOptionSignalBinding,
    public_signals: &BTreeMap<hir::ItemId, SignalHandle>,
    errors: &mut Vec<HirRuntimeAdapterError>,
) -> Option<SignalHandle> {
    let target = match resolve_direct_signal_expr(module, binding.expr) {
        Ok(item) => item,
        Err(DirectSignalExprError::UnsupportedExpr) => {
            errors.push(HirRuntimeAdapterError::UnsupportedSourceOptionSignalExpr {
                owner,
                option_name: binding.option_name.text().into(),
                expr: binding.expr,
            });
            return None;
        }
        Err(DirectSignalExprError::ResolvedNonSignal { item, kind }) => {
            errors.push(HirRuntimeAdapterError::SourceOptionBindingNotSignal {
                owner,
                option_name: binding.option_name.text().into(),
                item,
                kind,
            });
            return None;
        }
    };

    match public_signals.get(&target).copied() {
        Some(handle) => Some(handle),
        None => {
            errors.push(HirRuntimeAdapterError::UnknownSignalDependency {
                owner,
                dependency: target,
            });
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectSignalExprError {
    UnsupportedExpr,
    ResolvedNonSignal {
        item: hir::ItemId,
        kind: hir::ItemKind,
    },
}

fn resolve_direct_signal_expr(
    module: &hir::Module,
    expr: hir::ExprId,
) -> Result<hir::ItemId, DirectSignalExprError> {
    let Some(expr) = module.exprs().get(expr) else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    let hir::ExprKind::Name(reference) = &expr.kind else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    let hir::ResolutionState::Resolved(hir::TermResolution::Item(item)) = reference.resolution
    else {
        return Err(DirectSignalExprError::UnsupportedExpr);
    };
    match module.items().get(item) {
        Some(hir::Item::Signal(_)) => Ok(item),
        Some(item_ref) => Err(DirectSignalExprError::ResolvedNonSignal {
            item,
            kind: item_ref.kind(),
        }),
        None => Err(DirectSignalExprError::UnsupportedExpr),
    }
}

#[cfg(test)]
mod tests {
    use aivi_base::SourceDatabase;
    use aivi_hir::{DecodeProgramStep, Item, lower_module};
    use aivi_syntax::parse_module;
    use aivi_typing::{BuiltinSourceProvider, RecurrenceWakeupKind};

    use super::*;

    fn lower_text(path: &str, text: &str) -> hir::LoweringResult {
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

    fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Type(item) if item.name.text() == name => Some(item_id),
                Item::Value(item) if item.name.text() == name => Some(item_id),
                Item::Function(item) if item.name.text() == name => Some(item_id),
                Item::Signal(item) if item.name.text() == name => Some(item_id),
                Item::Class(item) if item.name.text() == name => Some(item_id),
                Item::Domain(item) if item.name.text() == name => Some(item_id),
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    #[test]
    fn assembles_signal_graph_and_source_specs_from_hir_reports() {
        let lowered = lower_text(
            "runtime-hir-adapter-positive.aivi",
            r#"
domain Duration over Int
    literal sec : Int -> Duration

fun keep value =>
    value

signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval : Signal Duration = 5sec

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int

signal gatedUsers : Signal Int =
    users
     ?|> True

@recur.timer 5sec
signal retried : Signal Int =
    0
     @|> keep
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "adapter-positive fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("positive runtime adapter fixture should assemble");

        let api_host = assembly
            .signal(item_id(lowered.module(), "apiHost"))
            .expect("apiHost signal binding should exist");
        let refresh = assembly
            .signal(item_id(lowered.module(), "refresh"))
            .expect("refresh signal binding should exist");
        let enabled = assembly
            .signal(item_id(lowered.module(), "enabled"))
            .expect("enabled signal binding should exist");
        let poll_interval = assembly
            .signal(item_id(lowered.module(), "pollInterval"))
            .expect("pollInterval signal binding should exist");
        let users_id = item_id(lowered.module(), "users");
        let users = assembly
            .signal(users_id)
            .expect("users signal binding should exist");
        let gated_users_id = item_id(lowered.module(), "gatedUsers");
        let gated_users = assembly
            .signal(gated_users_id)
            .expect("gatedUsers signal binding should exist");
        let retried_id = item_id(lowered.module(), "retried");
        let retried = assembly
            .signal(retried_id)
            .expect("retried signal binding should exist");

        assert!(matches!(users.kind, HirSignalBindingKind::Input { .. }));
        assert!(matches!(
            gated_users.kind,
            HirSignalBindingKind::Derived { .. }
        ));
        assert!(matches!(retried.kind, HirSignalBindingKind::Derived { .. }));
        assert_eq!(gated_users.dependencies(), &[users.signal()]);

        let source = assembly
            .source_by_owner(users_id)
            .expect("users source binding should exist");
        assert_eq!(source.signal, users.signal());
        assert_eq!(
            source.input,
            users.input().expect("users should be input-backed")
        );
        assert_eq!(
            source.spec.provider,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::HttpGet)
        );
        assert_eq!(
            source.spec.reconfiguration_dependencies.as_ref(),
            &[api_host.signal(), poll_interval.signal()]
        );
        assert_eq!(source.spec.explicit_triggers.as_ref(), &[refresh.signal()]);
        assert_eq!(source.spec.active_when, Some(enabled.signal()));
        assert!(matches!(
            source
                .spec
                .decode
                .as_ref()
                .expect("source decode program should be attached")
                .root_step(),
            DecodeProgramStep::Scalar { .. }
        ));

        let gate = assembly
            .gates()
            .iter()
            .find(|gate| gate.site.owner == gated_users_id)
            .expect("gatedUsers gate handoff should exist");
        assert_eq!(gate.owner_signal, gated_users.derived());
        assert!(matches!(gate.plan, HirRuntimeGatePlan::SignalFilter(_)));

        let recurrence = assembly
            .recurrences()
            .iter()
            .find(|node| node.site.owner == retried_id)
            .expect("retried recurrence handoff should exist");
        assert_eq!(recurrence.owner_signal, retried.derived());
        assert_eq!(recurrence.plan.wakeup.kind(), RecurrenceWakeupKind::Timer);

        let runtime: TaskSourceRuntime<i32, hir::SourceDecodeProgram> = assembly
            .instantiate_runtime()
            .expect("assembled sources should register into a runtime");
        assert!(runtime.source_spec(source.spec.instance).is_some());
        assert_eq!(
            runtime.graph().signal_count(),
            assembly.graph().signal_count()
        );
    }

    #[test]
    fn assembles_task_specs_from_task_values() {
        let lowered = lower_text(
            "runtime-hir-adapter-task.aivi",
            r#"
domain Retry over Int
    literal times : Int -> Retry

fun keep value =>
    value

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> keep
     <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "task fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly =
            assemble_hir_runtime(lowered.module()).expect("task fixture should assemble");
        let retried_id = item_id(lowered.module(), "retried");
        let task = assembly
            .task_by_owner(retried_id)
            .expect("retried task binding should exist");

        assert_eq!(task.owner, retried_id);
        assert_eq!(
            task.spec
                .wakeup
                .expect("task wakeup should be preserved")
                .kind(),
            RecurrenceWakeupKind::Backoff
        );

        let runtime: TaskSourceRuntime<i32, hir::SourceDecodeProgram> = assembly
            .instantiate_runtime()
            .expect("assembled tasks should register into a runtime");
        assert!(runtime.task_spec(task.spec.instance).is_some());
        assert_eq!(
            runtime.graph().signal_count(),
            assembly.graph().signal_count()
        );
    }

    #[test]
    fn bodyless_sources_and_scan_signals_keep_runtime_roles_separate() {
        let lowered = lower_text(
            "runtime-hir-adapter-bodyless-source-scan.aivi",
            r#"
fun step:Int n:Int current:Int =>
    n

signal enabled = True

@source http.get "/users" with {
    activeWhen: enabled
}
signal userEvents : Signal Int

signal gated : Signal Int =
    userEvents
     |> scan 0 step
"#,
        );
        assert!(
            !lowered.has_errors(),
            "bodyless source scan fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("bodyless source scan fixture should assemble");
        let user_events_id = item_id(lowered.module(), "userEvents");
        let gated_id = item_id(lowered.module(), "gated");
        let user_events = assembly
            .signal(user_events_id)
            .expect("userEvents signal binding should exist");
        let gated = assembly
            .signal(gated_id)
            .expect("gated signal binding should exist");
        let source = assembly
            .source_by_owner(user_events_id)
            .expect("userEvents source binding should exist");

        let derived = gated
            .derived()
            .expect("scan-derived signal should expose a public derived handle");
        assert_eq!(source.signal, user_events.signal());
        assert_eq!(
            source.input,
            user_events
                .input()
                .expect("raw source signal should stay input-backed")
        );
        assert!(
            gated.source_input.is_none(),
            "derived scan signals should not allocate their own source input handle"
        );
        assert_eq!(
            gated.dependencies(),
            &[user_events.signal()],
            "scan-derived signals should depend on the raw source signal"
        );
        assert_eq!(
            source.spec.active_when,
            Some(
                assembly
                    .signal(item_id(lowered.module(), "enabled"))
                    .expect("enabled signal binding should exist")
                    .signal()
            )
        );
        let recurrence = assembly
            .recurrences()
            .iter()
            .find(|node| node.site.owner == gated_id)
            .expect("gated recurrence handoff should exist");
        assert_eq!(recurrence.owner_signal, Some(derived));
        assert_eq!(
            recurrence.plan.wakeup_signal,
            Some(user_events_id),
            "scan recurrence handoff should preserve its upstream wakeup signal"
        );
    }

    #[test]
    fn lowers_builtin_dbus_source_providers_into_runtime_specs() {
        let lowered = lower_text(
            "runtime-hir-adapter-dbus.aivi",
            r#"
type DbusValue =
  | DbusString Text
  | DbusInt Int
  | DbusBool Bool
  | DbusList (List DbusValue)
  | DbusStruct (List DbusValue)
  | DbusVariant DbusValue

type DbusSignal = {
    path: Text,
    interface: Text,
    member: Text,
    body: List DbusValue
}

@source dbus.signal "/org/aivi/Test" "org.aivi.Test" "Ping"
signal inbound : Signal DbusSignal
"#,
        );
        assert!(
            !lowered.has_errors(),
            "dbus fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = assemble_hir_runtime(lowered.module())
            .expect("dbus fixture should assemble into a runtime");
        let inbound_id = item_id(lowered.module(), "inbound");
        let source = assembly
            .source_by_owner(inbound_id)
            .expect("dbus source binding should exist");

        assert_eq!(
            source.spec.provider,
            RuntimeSourceProvider::builtin(BuiltinSourceProvider::DbusSignal)
        );
    }

    #[test]
    fn reports_unsupported_source_option_expressions_explicitly() {
        let lowered = lower_text(
            "runtime-hir-adapter-unsupported-source-option.aivi",
            r#"
signal enabled = True
signal other = False

@source http.get "/users" with {
    activeWhen: enabled and other
}
signal users : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "unsupported-option fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let users_id = item_id(lowered.module(), "users");
        let errors = assemble_hir_runtime(lowered.module())
            .expect_err("composite activeWhen expressions should stay explicit adapter errors");
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            HirRuntimeAdapterError::UnsupportedSourceOptionSignalExpr {
                owner,
                option_name,
                ..
            } if *owner == users_id && option_name.as_ref() == "activeWhen"
        )));
    }

    #[test]
    fn forwards_blocked_decode_programs_into_adapter_failures() {
        let lowered = lower_text(
            "runtime-hir-adapter-blocked-decode.aivi",
            r#"
@source custom.feed
signal bad : Signal (Signal Int)
"#,
        );
        assert!(
            !lowered.has_errors(),
            "blocked-decode fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let bad_id = item_id(lowered.module(), "bad");
        let errors = assemble_hir_runtime(lowered.module())
            .expect_err("blocked source decode programs should stop assembly");
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            HirRuntimeAdapterError::BlockedSourceDecodeProgram { owner, .. }
            if *owner == bad_id
        )));
    }

    #[test]
    fn preserves_gate_outcomes_without_rederiving_them() {
        let lowered = lower_text(
            "runtime-hir-adapter-gate-preservation.aivi",
            r#"
value maybeOne:Option Int =
    1
     ?|> True
"#,
        );
        assert!(
            !lowered.has_errors(),
            "gate-preservation fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let assembly = HirRuntimeAssemblyBuilder::new(
            lowered.module(),
            &hir::elaborate_source_lifecycles(lowered.module()),
            &hir::generate_source_decode_programs(lowered.module()),
            &hir::elaborate_recurrences(lowered.module()),
            &hir::elaborate_gates(lowered.module()),
        )
        .build()
        .expect("gate-preservation fixture should assemble");
        let gate = assembly
            .gates()
            .iter()
            .find(|gate| gate.site.owner == item_id(lowered.module(), "maybeOne"))
            .expect("ordinary gate handoff should be preserved");
        assert!(matches!(gate.plan, HirRuntimeGatePlan::Ordinary(_)));
    }
}
