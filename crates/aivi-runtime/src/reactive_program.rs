use std::collections::BTreeMap;

use aivi_base::SourceSpan;
use aivi_hir as hir;

use crate::{
    SourceInstanceId, TaskInstanceId,
    graph::{
        InputHandle, OwnerHandle, ReactiveClauseHandle, SignalGraph, SignalHandle, SignalKind,
    },
    hir_adapter::{
        HirReactiveUpdateBinding, HirRecurrenceBinding, HirSignalBinding, HirSourceBinding,
        HirTaskBinding,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveProgram {
    signals: Box<[ReactiveSignalNode]>,
    clauses: Box<[ReactiveClauseNode]>,
    partitions: Box<[ReactivePartition]>,
    topo_order: Box<[SignalHandle]>,
}

impl ReactiveProgram {
    pub fn signal_count(&self) -> usize {
        self.signals.len()
    }

    pub fn signal(&self, handle: SignalHandle) -> Option<&ReactiveSignalNode> {
        self.signals.get(handle.index())
    }

    pub fn signals(&self) -> impl ExactSizeIterator<Item = (SignalHandle, &ReactiveSignalNode)> {
        self.signals
            .iter()
            .enumerate()
            .map(|(index, node)| (SignalHandle::from_raw(index as u32), node))
    }

    pub fn reactive_clause(&self, handle: ReactiveClauseHandle) -> Option<&ReactiveClauseNode> {
        self.clauses.get(handle.index())
    }

    pub fn reactive_clauses(
        &self,
    ) -> impl ExactSizeIterator<Item = (ReactiveClauseHandle, &ReactiveClauseNode)> {
        self.clauses
            .iter()
            .enumerate()
            .map(|(index, clause)| (ReactiveClauseHandle::from_raw(index as u32), clause))
    }

    pub fn partitions(&self) -> &[ReactivePartition] {
        &self.partitions
    }

    pub fn partition(&self, id: ReactivePartitionId) -> Option<&ReactivePartition> {
        self.partitions.get(id.index())
    }

    pub fn topo_order(&self) -> &[SignalHandle] {
        &self.topo_order
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveSignalNode {
    handle: SignalHandle,
    name: Box<str>,
    owner: Option<OwnerHandle>,
    item: Option<hir::ItemId>,
    kind: ReactiveSignalNodeKind,
    dependencies: Box<[SignalHandle]>,
    dependents: Box<[SignalHandle]>,
    root_signals: Box<[SignalHandle]>,
    topo_index: usize,
    partition: ReactivePartitionId,
}

impl ReactiveSignalNode {
    pub fn handle(&self) -> SignalHandle {
        self.handle
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn owner(&self) -> Option<OwnerHandle> {
        self.owner
    }

    pub fn item(&self) -> Option<hir::ItemId> {
        self.item
    }

    pub fn kind(&self) -> &ReactiveSignalNodeKind {
        &self.kind
    }

    pub fn dependencies(&self) -> &[SignalHandle] {
        &self.dependencies
    }

    pub fn dependents(&self) -> &[SignalHandle] {
        &self.dependents
    }

    pub fn root_signals(&self) -> &[SignalHandle] {
        &self.root_signals
    }

    pub fn topo_index(&self) -> usize {
        self.topo_index
    }

    pub fn partition(&self) -> ReactivePartitionId {
        self.partition
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactiveSignalNodeKind {
    Input(ReactiveInputNode),
    Derived(ReactiveDerivedNode),
    Reactive(ReactiveReactiveNode),
}

impl ReactiveSignalNodeKind {
    pub fn as_input(&self) -> Option<&ReactiveInputNode> {
        match self {
            Self::Input(node) => Some(node),
            Self::Derived(_) | Self::Reactive(_) => None,
        }
    }

    pub fn as_derived(&self) -> Option<&ReactiveDerivedNode> {
        match self {
            Self::Input(_) | Self::Reactive(_) => None,
            Self::Derived(node) => Some(node),
        }
    }

    pub fn as_reactive(&self) -> Option<&ReactiveReactiveNode> {
        match self {
            Self::Input(_) | Self::Derived(_) => None,
            Self::Reactive(node) => Some(node),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReactiveInputNode {
    source_owner: Option<hir::ItemId>,
    source_instance: Option<SourceInstanceId>,
    task_owner: Option<hir::ItemId>,
    task_instance: Option<TaskInstanceId>,
    temporal_owner: Option<hir::ItemId>,
}

impl ReactiveInputNode {
    pub fn source_owner(&self) -> Option<hir::ItemId> {
        self.source_owner
    }

    pub fn source_instance(&self) -> Option<SourceInstanceId> {
        self.source_instance
    }

    pub fn task_owner(&self) -> Option<hir::ItemId> {
        self.task_owner
    }

    pub fn task_instance(&self) -> Option<TaskInstanceId> {
        self.task_instance
    }

    pub fn temporal_owner(&self) -> Option<hir::ItemId> {
        self.temporal_owner
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveDerivedNode {
    source_input: Option<InputHandle>,
    temporal_helpers: Box<[InputHandle]>,
    has_recurrence: bool,
}

impl ReactiveDerivedNode {
    pub fn source_input(&self) -> Option<InputHandle> {
        self.source_input
    }

    pub fn temporal_helpers(&self) -> &[InputHandle] {
        &self.temporal_helpers
    }

    pub fn has_recurrence(&self) -> bool {
        self.has_recurrence
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveReactiveNode {
    source_input: Option<InputHandle>,
    seed_dependencies: Box<[SignalHandle]>,
    clauses: Box<[ReactiveClauseHandle]>,
}

impl ReactiveReactiveNode {
    pub fn source_input(&self) -> Option<InputHandle> {
        self.source_input
    }

    pub fn seed_dependencies(&self) -> &[SignalHandle] {
        &self.seed_dependencies
    }

    pub fn clauses(&self) -> &[ReactiveClauseHandle] {
        &self.clauses
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveClauseNode {
    handle: ReactiveClauseHandle,
    owner_signal: SignalHandle,
    owner_item: hir::ItemId,
    source_order: usize,
    span: SourceSpan,
    keyword_span: SourceSpan,
    target_span: SourceSpan,
    body_mode: hir::ReactiveUpdateBodyMode,
    trigger_signal: Option<SignalHandle>,
    guard_dependencies: Box<[SignalHandle]>,
    body_dependencies: Box<[SignalHandle]>,
}

impl ReactiveClauseNode {
    pub fn handle(&self) -> ReactiveClauseHandle {
        self.handle
    }

    pub fn owner_signal(&self) -> SignalHandle {
        self.owner_signal
    }

    pub fn owner_item(&self) -> hir::ItemId {
        self.owner_item
    }

    pub fn source_order(&self) -> usize {
        self.source_order
    }

    pub fn span(&self) -> SourceSpan {
        self.span
    }

    pub fn keyword_span(&self) -> SourceSpan {
        self.keyword_span
    }

    pub fn target_span(&self) -> SourceSpan {
        self.target_span
    }

    pub fn body_mode(&self) -> hir::ReactiveUpdateBodyMode {
        self.body_mode
    }

    pub fn trigger_signal(&self) -> Option<SignalHandle> {
        self.trigger_signal
    }

    pub fn guard_dependencies(&self) -> &[SignalHandle] {
        &self.guard_dependencies
    }

    pub fn body_dependencies(&self) -> &[SignalHandle] {
        &self.body_dependencies
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReactivePartitionId(u32);

impl ReactivePartitionId {
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactivePartition {
    id: ReactivePartitionId,
    batch_index: usize,
    signals: Box<[SignalHandle]>,
}

impl ReactivePartition {
    pub fn id(&self) -> ReactivePartitionId {
        self.id
    }

    pub fn batch_index(&self) -> usize {
        self.batch_index
    }

    pub fn signals(&self) -> &[SignalHandle] {
        &self.signals
    }
}

struct ClauseBindingInfo<'a> {
    owner: hir::ItemId,
    binding: &'a HirReactiveUpdateBinding,
}

pub(crate) fn build_reactive_program(
    graph: &SignalGraph,
    signal_bindings: &[HirSignalBinding],
    source_bindings: &[HirSourceBinding],
    task_bindings: &[HirTaskBinding],
    recurrence_bindings: &[HirRecurrenceBinding],
) -> ReactiveProgram {
    let signal_item_by_handle = signal_bindings
        .iter()
        .map(|binding| (binding.signal(), binding.item))
        .collect::<BTreeMap<_, _>>();
    let binding_by_signal = signal_bindings
        .iter()
        .map(|binding| (binding.signal(), binding))
        .collect::<BTreeMap<_, _>>();
    let source_input_by_handle = source_bindings
        .iter()
        .map(|binding| (binding.input, binding))
        .collect::<BTreeMap<_, _>>();
    let task_input_by_handle = task_bindings
        .iter()
        .map(|binding| (binding.input, binding))
        .collect::<BTreeMap<_, _>>();
    let temporal_owner_by_input = signal_bindings
        .iter()
        .flat_map(|binding| {
            binding
                .temporal_helper_inputs()
                .iter()
                .copied()
                .map(move |input| (input, binding.item))
        })
        .collect::<BTreeMap<_, _>>();
    let recurrence_by_signal = recurrence_bindings
        .iter()
        .filter_map(|binding| {
            binding
                .owner_signal
                .map(|signal| (signal.as_signal(), binding))
        })
        .collect::<BTreeMap<_, _>>();
    let clause_binding_by_handle = signal_bindings
        .iter()
        .flat_map(|binding| {
            binding.reactive_updates().iter().map(move |clause| {
                (
                    clause.clause,
                    ClauseBindingInfo {
                        owner: binding.item,
                        binding: clause,
                    },
                )
            })
        })
        .collect::<BTreeMap<_, _>>();

    let mut partition_by_signal = vec![ReactivePartitionId::from_raw(0); graph.signal_count()];
    let mut topo_order = Vec::with_capacity(graph.signal_count());
    let input_signals = graph
        .signals()
        .filter_map(|(signal, spec)| spec.is_input().then_some(signal))
        .collect::<Vec<_>>();
    let input_partition_offset = usize::from(!input_signals.is_empty());
    let mut partitions = Vec::with_capacity(graph.batches().len() + input_partition_offset);
    if !input_signals.is_empty() {
        let id = ReactivePartitionId::from_raw(0);
        for &signal in &input_signals {
            partition_by_signal[signal.index()] = id;
            topo_order.push(signal);
        }
        partitions.push(ReactivePartition {
            id,
            batch_index: 0,
            signals: input_signals.into_boxed_slice(),
        });
    }
    for (batch_index, batch) in graph.batches().iter().enumerate() {
        let id = ReactivePartitionId::from_raw((batch_index + input_partition_offset) as u32);
        for &signal in batch.signals() {
            partition_by_signal[signal.index()] = id;
            topo_order.push(signal);
        }
        partitions.push(ReactivePartition {
            id,
            batch_index: batch_index + input_partition_offset,
            signals: batch.signals().to_vec().into_boxed_slice(),
        });
    }
    let partitions = partitions.into_boxed_slice();

    let mut clause_nodes = vec![None; clause_binding_by_handle.len()];
    for (&handle, info) in &clause_binding_by_handle {
        let spec = graph
            .reactive_clause(handle)
            .expect("assembly reactive clauses should resolve in the signal graph");
        clause_nodes[handle.index()] = Some(ReactiveClauseNode {
            handle,
            owner_signal: spec.target(),
            owner_item: info.owner,
            source_order: spec.source_order(),
            span: info.binding.span,
            keyword_span: info.binding.keyword_span,
            target_span: info.binding.target_span,
            body_mode: info.binding.body_mode,
            trigger_signal: spec.trigger_signal(),
            guard_dependencies: spec.guard_dependencies().to_vec().into_boxed_slice(),
            body_dependencies: spec.body_dependencies().to_vec().into_boxed_slice(),
        });
    }
    let clauses = clause_nodes
        .into_iter()
        .map(|clause| clause.expect("reactive clause handles should stay dense"))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let mut root_signals = vec![Vec::<SignalHandle>::new(); graph.signal_count()];
    let mut signal_nodes = vec![None; graph.signal_count()];
    for (topo_index, signal) in topo_order.iter().copied().enumerate() {
        let spec = graph
            .signal(signal)
            .expect("topological signal order should only reference live graph signals");
        let dependencies = graph
            .signal_dependencies(signal)
            .expect("live graph signal should resolve dependencies")
            .to_vec();
        let dependents = graph
            .dependents(signal)
            .expect("live graph signal should resolve dependents")
            .to_vec();
        let roots = if dependencies.is_empty() {
            vec![signal]
        } else {
            let mut roots = Vec::new();
            for dependency in &dependencies {
                for &root in &root_signals[dependency.index()] {
                    push_unique_signal(&mut roots, root);
                }
            }
            roots
        };
        root_signals[signal.index()] = roots.clone();
        let item = signal_item_by_handle.get(&signal).copied();
        let kind = match spec.kind() {
            SignalKind::Input => {
                let input = signal.as_input();
                let source_binding = source_input_by_handle.get(&input).copied();
                let task_binding = task_input_by_handle.get(&input).copied();
                ReactiveSignalNodeKind::Input(ReactiveInputNode {
                    source_owner: source_binding.map(|binding| binding.owner),
                    source_instance: source_binding.map(|binding| binding.spec.instance),
                    task_owner: task_binding.map(|binding| binding.owner),
                    task_instance: task_binding.map(|binding| binding.spec.instance),
                    temporal_owner: temporal_owner_by_input.get(&input).copied(),
                })
            }
            SignalKind::Derived(_) => {
                let binding = binding_by_signal.get(&signal).copied();
                let temporal_helpers = binding
                    .map(|binding| binding.temporal_helper_inputs().to_vec().into_boxed_slice())
                    .unwrap_or_else(|| Vec::new().into_boxed_slice());
                ReactiveSignalNodeKind::Derived(ReactiveDerivedNode {
                    source_input: binding.and_then(|binding| binding.source_input),
                    temporal_helpers,
                    has_recurrence: recurrence_by_signal.contains_key(&signal),
                })
            }
            SignalKind::Reactive(spec) => {
                let binding = binding_by_signal.get(&signal).copied();
                ReactiveSignalNodeKind::Reactive(ReactiveReactiveNode {
                    source_input: binding.and_then(|binding| binding.source_input),
                    seed_dependencies: spec.seed_dependencies().to_vec().into_boxed_slice(),
                    clauses: spec.clauses().to_vec().into_boxed_slice(),
                })
            }
        };

        signal_nodes[signal.index()] = Some(ReactiveSignalNode {
            handle: signal,
            name: spec.name().into(),
            owner: spec.owner(),
            item,
            kind,
            dependencies: dependencies.into_boxed_slice(),
            dependents: dependents.into_boxed_slice(),
            root_signals: roots.into_boxed_slice(),
            topo_index,
            partition: partition_by_signal[signal.index()],
        });
    }

    ReactiveProgram {
        signals: signal_nodes
            .into_iter()
            .map(|node| node.expect("signal handles should stay dense"))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        clauses,
        partitions,
        topo_order: topo_order.into_boxed_slice(),
    }
}

fn push_unique_signal(target: &mut Vec<SignalHandle>, signal: SignalHandle) {
    if !target.contains(&signal) {
        target.push(signal);
    }
}
