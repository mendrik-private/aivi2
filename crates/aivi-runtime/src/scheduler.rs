use std::{
    collections::{BTreeSet, VecDeque},
    convert::Infallible,
    sync::{Arc, mpsc},
};

use aivi_backend::{CommittedValueStore, InlineCommittedValueStore};

use crate::graph::{
    DerivedHandle, InputHandle, OwnerHandle, ReactiveClauseHandle, SignalGraph, SignalHandle,
};
use crate::reactive_program::ReactiveProgram;

pub trait DerivedNodeEvaluator<V> {
    fn evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> DerivedSignalUpdate<V>;

    fn evaluate_reactive_seed(
        &mut self,
        signal: SignalHandle,
        _inputs: DependencyValues<'_, V>,
    ) -> DerivedSignalUpdate<V> {
        missing_reactive_evaluator(signal, None)
    }

    fn evaluate_reactive_guard(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> bool {
        let _ = inputs;
        missing_reactive_evaluator(signal, Some(clause))
    }

    fn evaluate_reactive_body(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> DerivedSignalUpdate<V> {
        let _ = inputs;
        missing_reactive_evaluator(signal, Some(clause))
    }
}

impl<V, F, R> DerivedNodeEvaluator<V> for F
where
    F: for<'a> FnMut(DerivedHandle, DependencyValues<'a, V>) -> R,
    R: Into<DerivedSignalUpdate<V>>,
{
    fn evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> DerivedSignalUpdate<V> {
        self(signal, inputs).into()
    }
}

pub trait TryDerivedNodeEvaluator<V> {
    type Error;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error>;

    fn try_evaluate_reactive_seed(
        &mut self,
        signal: SignalHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        let _ = inputs;
        missing_reactive_evaluator(signal, None)
    }

    fn try_evaluate_reactive_guard(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<bool, Self::Error> {
        let _ = inputs;
        missing_reactive_evaluator(signal, Some(clause))
    }

    fn try_evaluate_reactive_body(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        let _ = inputs;
        missing_reactive_evaluator(signal, Some(clause))
    }
}

impl<V, E, F, R> TryDerivedNodeEvaluator<V> for F
where
    F: for<'a> FnMut(DerivedHandle, DependencyValues<'a, V>) -> Result<R, E>,
    R: Into<DerivedSignalUpdate<V>>,
{
    type Error = E;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        self(signal, inputs).map(Into::into)
    }
}

struct InfallibleDerivedEvaluator<'a, E>(&'a mut E);

impl<V, E> TryDerivedNodeEvaluator<V> for InfallibleDerivedEvaluator<'_, E>
where
    E: DerivedNodeEvaluator<V>,
{
    type Error = Infallible;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        Ok(self.0.evaluate(signal, inputs))
    }

    fn try_evaluate_reactive_seed(
        &mut self,
        signal: SignalHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        Ok(self.0.evaluate_reactive_seed(signal, inputs))
    }

    fn try_evaluate_reactive_guard(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<bool, Self::Error> {
        Ok(self.0.evaluate_reactive_guard(signal, clause, inputs))
    }

    fn try_evaluate_reactive_body(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<DerivedSignalUpdate<V>, Self::Error> {
        Ok(self.0.evaluate_reactive_body(signal, clause, inputs))
    }
}

fn missing_reactive_evaluator(signal: SignalHandle, clause: Option<ReactiveClauseHandle>) -> ! {
    match clause {
        Some(clause) => panic!(
            "signal {:?} owns reactive clause {:?}, but the evaluator does not implement reactive update execution",
            signal, clause
        ),
        None => panic!(
            "signal {:?} is reactive, but the evaluator does not implement reactive update execution",
            signal
        ),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DerivedSignalUpdate<V> {
    Unchanged,
    Clear,
    Value(V),
}

impl<V> From<Option<V>> for DerivedSignalUpdate<V> {
    fn from(value: Option<V>) -> Self {
        match value {
            Some(value) => Self::Value(value),
            None => Self::Clear,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawSlotPlanId(u32);

impl RawSlotPlanId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawBytes {
    Inline {
        bytes: [u8; Self::INLINE_CAPACITY],
        len: u8,
    },
    Heap(Box<[u8]>),
}

impl RawBytes {
    pub const INLINE_CAPACITY: usize = 32;

    pub fn from_slice(bytes: &[u8]) -> Self {
        if bytes.len() <= Self::INLINE_CAPACITY {
            let mut inline = [0_u8; Self::INLINE_CAPACITY];
            inline[..bytes.len()].copy_from_slice(bytes);
            Self::Inline {
                bytes: inline,
                len: bytes.len() as u8,
            }
        } else {
            Self::Heap(bytes.into())
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Inline { len, .. } => *len as usize,
            Self::Heap(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { bytes, len } => &bytes[..*len as usize],
            Self::Heap(bytes) => bytes,
        }
    }
}

impl From<&[u8]> for RawBytes {
    fn from(bytes: &[u8]) -> Self {
        Self::from_slice(bytes)
    }
}

impl<const N: usize> From<[u8; N]> for RawBytes {
    fn from(bytes: [u8; N]) -> Self {
        Self::from_slice(&bytes)
    }
}

impl From<Vec<u8>> for RawBytes {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.len() <= Self::INLINE_CAPACITY {
            Self::from_slice(&bytes)
        } else {
            Self::Heap(bytes.into_boxed_slice())
        }
    }
}

impl From<Box<[u8]>> for RawBytes {
    fn from(bytes: Box<[u8]>) -> Self {
        if bytes.len() <= Self::INLINE_CAPACITY {
            Self::from_slice(&bytes)
        } else {
            Self::Heap(bytes)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSlot<V> {
    plan: RawSlotPlanId,
    bytes: RawBytes,
    value: V,
}

impl<V> RawSlot<V> {
    pub fn plan(&self) -> RawSlotPlanId {
        self.plan
    }

    pub fn bytes(&self) -> &RawBytes {
        &self.bytes
    }

    pub fn value(&self) -> &V {
        &self.value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRawValue<V> {
    plan: RawSlotPlanId,
    bytes: RawBytes,
    value: V,
}

impl<V> PendingRawValue<V> {
    pub fn new(plan: RawSlotPlanId, bytes: impl Into<RawBytes>, value: V) -> Self {
        Self {
            plan,
            bytes: bytes.into(),
            value,
        }
    }

    pub fn plan(&self) -> RawSlotPlanId {
        self.plan
    }

    pub fn bytes(&self) -> &RawBytes {
        &self.bytes
    }

    pub fn value(&self) -> &V {
        &self.value
    }

    fn into_committed(self) -> RawSlot<V> {
        RawSlot {
            plan: self.plan,
            bytes: self.bytes,
            value: self.value,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn advance(self) -> Self {
        // Wrapping is safe: no two live generation values ever compare equal
        // across a wrap boundary in practice at any achievable tick rate.
        // At 60 fps, u64::MAX generations would take ~9.7 billion years.
        Self(self.0.wrapping_add(1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PublicationStamp {
    input: InputHandle,
    generation: Generation,
}

impl PublicationStamp {
    pub fn input(self) -> InputHandle {
        self.input
    }

    pub fn generation(self) -> Generation {
        self.generation
    }
}

#[derive(Debug)]
pub struct Publication<V> {
    stamp: PublicationStamp,
    value: V,
}

impl<V> Publication<V> {
    pub fn new(stamp: PublicationStamp, value: V) -> Self {
        Self { stamp, value }
    }

    pub fn stamp(&self) -> PublicationStamp {
        self.stamp
    }

    pub fn into_parts(self) -> (PublicationStamp, V) {
        (self.stamp, self.value)
    }
}

#[derive(Clone)]
pub struct WorkerPublicationSender<V> {
    sender: mpsc::SyncSender<Publication<V>>,
    notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
}

impl<V> WorkerPublicationSender<V> {
    pub fn publish(&self, publication: Publication<V>) -> Result<(), WorkerSendError<V>> {
        self.sender
            .send(publication)
            .map_err(|err| WorkerSendError { publication: err.0 })?;
        if let Some(notifier) = &self.notifier {
            notifier();
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct WorkerSendError<V> {
    publication: Publication<V>,
}

impl<V> WorkerSendError<V> {
    pub fn into_publication(self) -> Publication<V> {
        self.publication
    }
}

pub enum SchedulerMessage<V> {
    Publish(Publication<V>),
    DisposeOwner(OwnerHandle),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationDropReason {
    StaleGeneration { active: Generation },
    OwnerInactive { owner: OwnerHandle },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DroppedPublication {
    stamp: PublicationStamp,
    reason: PublicationDropReason,
}

impl DroppedPublication {
    pub fn stamp(self) -> PublicationStamp {
        self.stamp
    }

    pub fn reason(self) -> PublicationDropReason {
        self.reason
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickOutcome {
    tick: u64,
    committed: Box<[SignalHandle]>,
    dropped_publications: Box<[DroppedPublication]>,
}

impl TickOutcome {
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn committed(&self) -> &[SignalHandle] {
        &self.committed
    }

    pub fn dropped_publications(&self) -> &[DroppedPublication] {
        &self.dropped_publications
    }

    pub fn is_empty(&self) -> bool {
        self.committed.is_empty() && self.dropped_publications.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerAccessError {
    UnknownSignalHandle { signal: u32 },
    UnknownOwnerHandle { owner: u32 },
    SignalIsNotInput { signal: u32 },
    OwnerInactive { owner: OwnerHandle },
}

/// # Thread Safety
///
/// `Scheduler` is NOT thread-safe. The internal queue (`self.queue`) is a
/// plain `VecDeque` with no synchronization. Worker threads publish via
/// `mpsc::Sender<SchedulerMessage>` (which IS thread-safe), but the scheduler
/// itself MUST be driven from a single thread (the GTK main thread).
///
/// Do NOT call `tick()` or queue-mutation methods from multiple threads.
/// The `WorkerPublicationSender` returned by `worker_sender()` is safe to
/// clone and use from any thread.
pub struct Scheduler<V, S = InlineCommittedValueStore<V>>
where
    S: CommittedValueStore<V>,
{
    graph: SignalGraph,
    storage: S,
    owners: Vec<OwnerRuntimeState>,
    inputs: Vec<Option<InputRuntimeState>>,
    slots: SlotStore<V, S::Slot>,
    queue: VecDeque<SchedulerMessage<V>>,
    queued_messages_scratch: Vec<SchedulerMessage<V>>,
    dirty_scratch: Vec<bool>,
    publications_scratch: Vec<Option<Publication<V>>>,
    dropped_scratch: Vec<DroppedPublication>,
    worker_publication_tx: mpsc::SyncSender<Publication<V>>,
    worker_publication_rx: mpsc::Receiver<Publication<V>>,
    worker_publication_notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    initialized: bool,
    next_tick: u64,
}

#[derive(Clone, Copy)]
enum TickEvaluationOrder<'a> {
    GraphBatches,
    ReactiveProgram(&'a ReactiveProgram),
}

impl<V> Scheduler<V, InlineCommittedValueStore<V>> {
    pub fn new(graph: SignalGraph) -> Self {
        Self::with_value_store(graph, InlineCommittedValueStore::default())
    }
}

impl<V, S> Scheduler<V, S>
where
    S: CommittedValueStore<V>,
{
    pub fn with_value_store(graph: SignalGraph, storage: S) -> Self {
        let (worker_publication_tx, worker_publication_rx) = mpsc::sync_channel(128);
        let signal_count = graph.signal_count();
        let owners = (0..graph.owner_count())
            .map(|_| OwnerRuntimeState { active: true })
            .collect();
        let inputs = (0..graph.signal_count())
            .map(
                |index| match graph.signal(SignalHandle::from_raw(index as u32)) {
                    Some(signal) if signal.is_input() => Some(InputRuntimeState {
                        generation: Generation::default(),
                    }),
                    _ => None,
                },
            )
            .collect();

        Self {
            graph,
            storage,
            owners,
            inputs,
            slots: SlotStore::new(signal_count),
            queue: VecDeque::new(),
            queued_messages_scratch: Vec::new(),
            dirty_scratch: Vec::new(),
            publications_scratch: Vec::new(),
            dropped_scratch: Vec::new(),
            worker_publication_tx,
            worker_publication_rx,
            worker_publication_notifier: None,
            initialized: false,
            next_tick: 0,
        }
    }

    pub fn graph(&self) -> &SignalGraph {
        &self.graph
    }

    pub fn tick_count(&self) -> u64 {
        self.next_tick
    }

    pub fn queued_message_count(&self) -> usize {
        self.queue.len()
    }

    pub fn worker_sender(&self) -> WorkerPublicationSender<V> {
        WorkerPublicationSender {
            sender: self.worker_publication_tx.clone(),
            notifier: self.worker_publication_notifier.clone(),
        }
    }

    pub fn set_worker_publication_notifier(
        &mut self,
        notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) {
        self.worker_publication_notifier = notifier;
    }

    pub fn current_value(&self, signal: SignalHandle) -> Result<Option<&V>, SchedulerAccessError> {
        self.validate_signal(signal)?;
        Ok(self.slots.current_value(signal, &self.storage))
    }

    pub fn is_owner_active(&self, owner: OwnerHandle) -> Result<bool, SchedulerAccessError> {
        self.validate_owner(owner)?;
        Ok(self.owners[owner.index()].active)
    }

    pub fn current_generation(
        &self,
        input: InputHandle,
    ) -> Result<Generation, SchedulerAccessError> {
        Ok(self.validate_input_state(input)?.generation)
    }

    pub fn current_stamp(
        &self,
        input: InputHandle,
    ) -> Result<PublicationStamp, SchedulerAccessError> {
        self.ensure_input_owner_active(input)?;
        Ok(PublicationStamp {
            input,
            generation: self.validate_input_state(input)?.generation,
        })
    }

    pub fn advance_generation(
        &mut self,
        input: InputHandle,
    ) -> Result<PublicationStamp, SchedulerAccessError> {
        self.ensure_input_owner_active(input)?;
        let state = self.validate_input_state_mut(input)?;
        state.generation = state.generation.advance();
        Ok(PublicationStamp {
            input,
            generation: state.generation,
        })
    }

    pub fn queue_message(
        &mut self,
        message: SchedulerMessage<V>,
    ) -> Result<(), SchedulerAccessError> {
        match &message {
            SchedulerMessage::Publish(publication) => {
                self.validate_input(publication.stamp.input)?;
            }
            SchedulerMessage::DisposeOwner(owner) => {
                self.validate_owner(*owner)?;
            }
        }
        // NOTE: `self.queue` is a plain `VecDeque` with no interior locking.
        // This method takes `&mut self`, so Rust's borrow checker enforces exclusive access.
        // The scheduler is intentionally single-threaded: all queue mutations and `tick` calls
        // must happen on the same owning thread (typically the GLib/GTK main thread when used
        // with `GlibSchedulerDriver`). Worker threads must never call this method directly;
        // they communicate via `WorkerPublicationSender` which uses a separate `mpsc` channel
        // drained by `drain_worker_publications` at the start of each `tick`.
        self.queue.push_back(message);
        Ok(())
    }

    pub fn queue_publication(
        &mut self,
        publication: Publication<V>,
    ) -> Result<(), SchedulerAccessError> {
        self.queue_message(SchedulerMessage::Publish(publication))
    }

    pub fn queue_dispose_owner(&mut self, owner: OwnerHandle) -> Result<(), SchedulerAccessError> {
        self.queue_message(SchedulerMessage::DisposeOwner(owner))
    }

    pub fn tick<E>(&mut self, evaluator: &mut E) -> TickOutcome
    where
        E: DerivedNodeEvaluator<V>,
    {
        let mut evaluator = InfallibleDerivedEvaluator(evaluator);
        self.tick_with(&mut evaluator, TickEvaluationOrder::GraphBatches)
            .unwrap_or_else(|never| match never {})
    }

    pub fn try_tick<E>(&mut self, evaluator: &mut E) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        self.tick_with(evaluator, TickEvaluationOrder::GraphBatches)
    }

    pub(crate) fn try_tick_with_reactive_program<E>(
        &mut self,
        program: &ReactiveProgram,
        evaluator: &mut E,
    ) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        self.tick_with(evaluator, TickEvaluationOrder::ReactiveProgram(program))
    }

    fn tick_with<E>(
        &mut self,
        evaluator: &mut E,
        order: TickEvaluationOrder<'_>,
    ) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        self.drain_worker_publications();
        let tick = self.next_tick;
        // Wrapping is safe: the tick counter is used for tracing only; no two
        // in-flight ticks run simultaneously.
        self.next_tick = self.next_tick.wrapping_add(1);

        let mut pending = std::mem::take(&mut self.slots.pending);
        pending.clear();
        pending.resize_with(self.slots.committed.len(), || PendingSlot::Unchanged);

        let mut messages = std::mem::take(&mut self.queued_messages_scratch);
        messages.clear();
        messages.extend(self.queue.drain(..));
        let disposed = self.collect_disposed_owners(&messages);
        self.apply_owner_disposals(&disposed, &mut pending);

        let mut dropped = std::mem::take(&mut self.dropped_scratch);
        dropped.clear();
        let mut publications = std::mem::take(&mut self.publications_scratch);
        publications.clear();
        publications.resize_with(self.slots.committed.len(), || None::<Publication<V>>);

        for message in messages.drain(..) {
            let SchedulerMessage::Publish(publication) = message else {
                continue;
            };

            let input = publication.stamp.input;
            let signal = input.as_signal();
            let owner = self.graph.signal(signal).and_then(|spec| spec.owner());
            if let Some(owner) = owner
                && !self.owners[owner.index()].active
            {
                dropped.push(DroppedPublication {
                    stamp: publication.stamp,
                    reason: PublicationDropReason::OwnerInactive { owner },
                });
                continue;
            }

            let active = self.inputs[input.index()]
                .as_ref()
                .expect("validated input handle must have runtime state")
                .generation;
            if publication.stamp.generation != active {
                dropped.push(DroppedPublication {
                    stamp: publication.stamp,
                    reason: PublicationDropReason::StaleGeneration { active },
                });
                continue;
            }

            publications[input.index()] = Some(publication);
        }
        self.queued_messages_scratch = messages;

        for publication in publications.drain(..).flatten() {
            let (stamp, value) = publication.into_parts();
            pending[stamp.input.index()] = PendingSlot::NextStored(value);
        }
        self.publications_scratch = publications;

        let mut dirty = std::mem::take(&mut self.dirty_scratch);
        dirty.clear();
        dirty.resize(self.slots.committed.len(), false);
        if self.initialized {
            self.mark_dirty_dependents(&pending, &mut dirty);
        } else {
            for batch in self.graph.batches() {
                for &signal in batch.signals() {
                    if self.signal_active(signal) {
                        dirty[signal.index()] = true;
                    }
                }
            }
        }

        self.evaluate_dirty_signals(order, evaluator, &mut pending, &dirty)?;
        self.dirty_scratch = dirty;

        let mut committed = Vec::new();
        for (index, pending_value) in pending.drain(..).enumerate() {
            let handle = SignalHandle::from_raw(index as u32);
            if commit_pending_slot(
                &mut self.storage,
                &mut self.slots.committed[index],
                pending_value,
            ) {
                committed.push(handle);
            }
        }
        self.slots.pending = pending;
        self.collect_committed_values();

        self.initialized = true;
        // Drain `dropped` into a boxed slice for the outcome, then return the
        // now-empty Vec to the scratch field to avoid a fresh allocation next tick.
        let dropped_publications: Box<[DroppedPublication]> = dropped.drain(..).collect();
        self.dropped_scratch = dropped;
        Ok(TickOutcome {
            tick,
            committed: committed.into_boxed_slice(),
            dropped_publications,
        })
    }

    fn collect_committed_values(&mut self) {
        let roots = self
            .slots
            .committed
            .iter()
            .filter_map(|slot| match slot {
                CommittedSlot::Stored(slot) => Some(slot),
                CommittedSlot::Empty | CommittedSlot::Raw(_) => None,
            })
            .collect::<Vec<_>>();
        self.storage.collect(&roots);
    }

    fn evaluate_dirty_signals<E>(
        &self,
        order: TickEvaluationOrder<'_>,
        evaluator: &mut E,
        pending: &mut [PendingSlot<V>],
        dirty: &[bool],
    ) -> Result<(), E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        let committed_values = self
            .slots
            .committed
            .iter()
            .map(|slot| slot.current_value(&self.storage))
            .collect::<Vec<_>>();
        match order {
            TickEvaluationOrder::GraphBatches => {
                for batch in self.graph.batches() {
                    self.evaluate_signals(
                        batch.signals(),
                        evaluator,
                        pending,
                        dirty,
                        &committed_values,
                    )?;
                }
            }
            TickEvaluationOrder::ReactiveProgram(program) => {
                debug_assert_eq!(
                    program.signal_count(),
                    self.graph.signal_count(),
                    "reactive program and signal graph must describe the same scheduler graph",
                );
                for partition in self.dirty_partitions(program, dirty) {
                    self.evaluate_signals(
                        partition.signals(),
                        evaluator,
                        pending,
                        dirty,
                        &committed_values,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn dirty_partitions<'a>(
        &self,
        program: &'a ReactiveProgram,
        dirty: &[bool],
    ) -> Vec<&'a crate::ReactivePartition> {
        program
            .partitions()
            .iter()
            .filter(|partition| {
                partition.signals().iter().copied().any(|signal| {
                    dirty[signal.index()]
                        && self.signal_active(signal)
                        && self
                            .graph
                            .signal(signal)
                            .is_some_and(|spec| !spec.is_input())
                })
            })
            .collect()
    }

    fn evaluate_signals<E>(
        &self,
        signals: &[SignalHandle],
        evaluator: &mut E,
        pending: &mut [PendingSlot<V>],
        dirty: &[bool],
        committed_values: &[Option<&V>],
    ) -> Result<(), E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        for &signal in signals {
            if !dirty[signal.index()] || !self.signal_active(signal) {
                continue;
            }

            let next = match self
                .graph
                .signal(signal)
                .expect("scheduled signals must exist in the graph")
                .kind()
            {
                crate::graph::SignalKind::Input => continue,
                crate::graph::SignalKind::Derived(spec) => {
                    let inputs = DependencyValues {
                        dependencies: spec.dependencies(),
                        pending,
                        committed: committed_values,
                    };
                    pending_slot_from_update(evaluator.try_evaluate(signal.as_derived(), inputs)?)
                }
                crate::graph::SignalKind::Reactive(spec) => self.evaluate_reactive_signal(
                    evaluator,
                    signal,
                    spec,
                    committed_values,
                    pending,
                )?,
            };
            pending[signal.index()] = next;
        }
        Ok(())
    }

    fn collect_disposed_owners(&self, messages: &[SchedulerMessage<V>]) -> BTreeSet<OwnerHandle> {
        let mut disposed = BTreeSet::new();
        let mut worklist = messages
            .iter()
            .filter_map(|message| match message {
                SchedulerMessage::Publish(_) => None,
                SchedulerMessage::DisposeOwner(owner) => Some(*owner),
            })
            .collect::<VecDeque<_>>();

        while let Some(owner) = worklist.pop_front() {
            if !disposed.insert(owner) {
                continue;
            }
            if let Some(spec) = self.graph.owner(owner) {
                for &child in spec.children() {
                    worklist.push_back(child);
                }
            }
        }

        disposed
    }

    fn drain_worker_publications(&mut self) {
        // NOTE: Must only be called from the scheduler's owning thread (see `queue_message`).
        // The `mpsc::Receiver` is not `Sync`, so this is enforced by the type system as long as
        // `Scheduler` is not shared across threads without wrapping in a `Mutex`.
        while let Ok(publication) = self.worker_publication_rx.try_recv() {
            self.queue.push_back(SchedulerMessage::Publish(publication));
        }
    }

    fn apply_owner_disposals(
        &mut self,
        disposed: &BTreeSet<OwnerHandle>,
        pending: &mut [PendingSlot<V>],
    ) {
        for &owner in disposed {
            let state = &mut self.owners[owner.index()];
            if !state.active {
                continue;
            }
            state.active = false;

            let spec = self
                .graph
                .owner(owner)
                .expect("disposed owners are validated on enqueue");
            for &signal in spec.signals() {
                pending[signal.index()] = PendingSlot::Clear;
                if let Some(input) = self.inputs[signal.index()].as_mut() {
                    input.generation = input.generation.advance();
                }
            }
        }
    }

    fn mark_dirty_dependents(&self, pending: &[PendingSlot<V>], dirty: &mut [bool]) {
        let mut worklist = pending
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| {
                if pending.is_unchanged() {
                    None
                } else {
                    Some(SignalHandle::from_raw(index as u32))
                }
            })
            .flat_map(|signal| {
                self.graph
                    .dependents(signal)
                    .expect("pending slots are indexed by graph signals")
                    .iter()
                    .copied()
            })
            .collect::<VecDeque<_>>();

        while let Some(signal) = worklist.pop_front() {
            if dirty[signal.index()] {
                continue;
            }
            if !self.signal_active(signal) && pending[signal.index()].is_unchanged() {
                continue;
            }

            dirty[signal.index()] = true;
            for &dependent in self
                .graph
                .dependents(signal)
                .expect("dirty worklist only contains graph signals")
            {
                worklist.push_back(dependent);
            }
        }
    }

    fn signal_active(&self, signal: SignalHandle) -> bool {
        self.graph
            .signal(signal)
            .and_then(|spec| spec.owner())
            .is_none_or(|owner| self.owners[owner.index()].active)
    }

    fn ensure_input_owner_active(&self, input: InputHandle) -> Result<(), SchedulerAccessError> {
        self.validate_input(input)?;
        let owner = self
            .graph
            .signal(input.as_signal())
            .and_then(|spec| spec.owner());
        if let Some(owner) = owner
            && !self.owners[owner.index()].active
        {
            return Err(SchedulerAccessError::OwnerInactive { owner });
        }
        Ok(())
    }

    fn validate_signal(&self, signal: SignalHandle) -> Result<(), SchedulerAccessError> {
        if self.graph.contains_signal(signal) {
            Ok(())
        } else {
            Err(SchedulerAccessError::UnknownSignalHandle {
                signal: signal.as_raw(),
            })
        }
    }

    fn validate_owner(&self, owner: OwnerHandle) -> Result<(), SchedulerAccessError> {
        if self.graph.contains_owner(owner) {
            Ok(())
        } else {
            Err(SchedulerAccessError::UnknownOwnerHandle {
                owner: owner.as_raw(),
            })
        }
    }

    fn validate_input(&self, input: InputHandle) -> Result<(), SchedulerAccessError> {
        self.graph.validate_input(input).map_err(|err| match err {
            crate::graph::InputValidationError::UnknownHandle { raw } => {
                SchedulerAccessError::UnknownSignalHandle { signal: raw }
            }
            crate::graph::InputValidationError::NotAnInput { raw } => {
                SchedulerAccessError::SignalIsNotInput { signal: raw }
            }
        })
    }

    fn validate_input_state(
        &self,
        input: InputHandle,
    ) -> Result<&InputRuntimeState, SchedulerAccessError> {
        self.validate_input(input)?;
        Ok(self.inputs[input.index()]
            .as_ref()
            .expect("validated input handle must have runtime state"))
    }

    fn validate_input_state_mut(
        &mut self,
        input: InputHandle,
    ) -> Result<&mut InputRuntimeState, SchedulerAccessError> {
        self.validate_input(input)?;
        Ok(self.inputs[input.index()]
            .as_mut()
            .expect("validated input handle must have runtime state"))
    }

    fn evaluate_reactive_signal<E>(
        &self,
        evaluator: &mut E,
        signal: SignalHandle,
        spec: &crate::graph::ReactiveSignalSpec,
        committed: &[Option<&V>],
        pending: &[PendingSlot<V>],
    ) -> Result<PendingSlot<V>, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        let mut next = if self.initialized {
            PendingSlot::Unchanged
        } else {
            pending_slot_from_update(evaluator.try_evaluate_reactive_seed(
                signal,
                DependencyValues {
                    dependencies: spec.seed_dependencies(),
                    pending,
                    committed,
                },
            )?)
        };

        for &clause in spec.clauses() {
            let clause_spec = self
                .graph
                .reactive_clause(clause)
                .expect("reactive signal clauses are built from the same graph");
            if let Some(trigger_signal) = clause_spec.trigger_signal()
                && pending[trigger_signal.index()].is_unchanged()
            {
                continue;
            }
            let should_commit = evaluator.try_evaluate_reactive_guard(
                signal,
                clause,
                DependencyValues {
                    dependencies: clause_spec.guard_dependencies(),
                    pending,
                    committed,
                },
            )?;
            if !should_commit {
                continue;
            }
            let update = pending_slot_from_update(evaluator.try_evaluate_reactive_body(
                signal,
                clause,
                DependencyValues {
                    dependencies: clause_spec.body_dependencies(),
                    pending,
                    committed,
                },
            )?);
            if !update.is_unchanged() {
                next = update;
            }
        }

        Ok(next)
    }
}

fn pending_slot_from_update<V>(update: DerivedSignalUpdate<V>) -> PendingSlot<V> {
    match update {
        DerivedSignalUpdate::Unchanged => PendingSlot::Unchanged,
        DerivedSignalUpdate::Clear => PendingSlot::Clear,
        DerivedSignalUpdate::Value(value) => PendingSlot::NextStored(value),
    }
}

/// Read-only dependency view for one derived evaluation within a transactional scheduler tick.
///
/// Each lookup resolves against the latest value already produced for this tick (`pending`) before
/// falling back to the previous committed snapshot. That guarantees topological, glitch-free reads:
/// downstream evaluators observe the newest stable upstream values for the current tick, never a
/// mixed intermediate state.
pub struct DependencyValues<'a, V> {
    dependencies: &'a [SignalHandle],
    pending: &'a [PendingSlot<V>],
    committed: &'a [Option<&'a V>],
}

impl<'a, V> DependencyValues<'a, V> {
    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    pub fn all_present(&self) -> bool {
        self.dependencies
            .iter()
            .all(|signal| self.resolve(*signal).is_some())
    }

    pub fn get(&self, index: usize) -> Option<DependencyValue<'a, V>> {
        let signal = *self.dependencies.get(index)?;
        Some(DependencyValue {
            signal,
            value: self.resolve(signal),
        })
    }

    pub fn signal(&self, index: usize) -> Option<SignalHandle> {
        self.dependencies.get(index).copied()
    }

    pub fn value(&self, index: usize) -> Option<&'a V> {
        self.get(index)?.value
    }

    pub fn contains_signal(&self, signal: SignalHandle) -> bool {
        self.dependencies.contains(&signal)
    }

    pub fn value_for(&self, signal: SignalHandle) -> Option<&'a V> {
        self.contains_signal(signal)
            .then(|| self.resolve(signal))
            .flatten()
    }

    pub fn updated(&self, index: usize) -> bool {
        let Some(signal) = self.dependencies.get(index).copied() else {
            return false;
        };
        self.pending[signal.index()].is_updated()
    }

    pub fn updated_signal(&self, signal: SignalHandle) -> bool {
        self.contains_signal(signal) && self.pending[signal.index()].is_updated()
    }

    fn resolve(&self, signal: SignalHandle) -> Option<&'a V> {
        match &self.pending[signal.index()] {
            PendingSlot::Unchanged => self.committed[signal.index()],
            PendingSlot::Clear => None,
            PendingSlot::NextRaw(value) => Some(value.value()),
            PendingSlot::NextStored(value) => Some(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DependencyValue<'a, V> {
    signal: SignalHandle,
    value: Option<&'a V>,
}

impl<'a, V> DependencyValue<'a, V> {
    pub fn signal(self) -> SignalHandle {
        self.signal
    }

    pub fn value(self) -> Option<&'a V> {
        self.value
    }
}

struct OwnerRuntimeState {
    active: bool,
}

#[derive(Clone, Copy)]
struct InputRuntimeState {
    generation: Generation,
}

struct SlotStore<V, S> {
    committed: Vec<CommittedSlot<V, S>>,
    pending: Vec<PendingSlot<V>>,
}

impl<V, S> SlotStore<V, S> {
    fn new(signal_count: usize) -> Self {
        let committed = (0..signal_count).map(|_| CommittedSlot::Empty).collect();
        let pending = (0..signal_count).map(|_| PendingSlot::Unchanged).collect();
        Self { committed, pending }
    }

    fn current_value<'a, Store>(&'a self, signal: SignalHandle, storage: &'a Store) -> Option<&'a V>
    where
        Store: CommittedValueStore<V, Slot = S>,
    {
        self.committed[signal.index()].current_value(storage)
    }
}

pub(crate) enum CommittedSlot<V, S> {
    Empty,
    Raw(RawSlot<V>),
    Stored(S),
}

impl<V, S> CommittedSlot<V, S> {
    fn current_value<'a, Store>(&'a self, storage: &'a Store) -> Option<&'a V>
    where
        Store: CommittedValueStore<V, Slot = S>,
    {
        match self {
            Self::Empty => None,
            Self::Raw(slot) => Some(slot.value()),
            Self::Stored(slot) => storage.get(slot),
        }
    }
}

// Phase 4 lands the raw slot state model before Phase 5 wires native evaluators to emit it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum PendingSlot<V> {
    Unchanged,
    Clear,
    NextRaw(PendingRawValue<V>),
    NextStored(V),
}

impl<V> PendingSlot<V> {
    fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }

    fn is_updated(&self) -> bool {
        !self.is_unchanged()
    }
}

fn commit_pending_slot<V, S, Store>(
    storage: &mut Store,
    committed: &mut CommittedSlot<V, S>,
    pending: PendingSlot<V>,
) -> bool
where
    Store: CommittedValueStore<V, Slot = S>,
    S: Default,
{
    match pending {
        PendingSlot::Unchanged => false,
        PendingSlot::Clear => clear_committed_slot(storage, committed),
        PendingSlot::NextRaw(raw) => {
            discard_stored_slot(storage, committed);
            *committed = CommittedSlot::Raw(raw.into_committed());
            true
        }
        PendingSlot::NextStored(value) => {
            let mut slot = match std::mem::replace(committed, CommittedSlot::Empty) {
                CommittedSlot::Stored(slot) => slot,
                CommittedSlot::Empty | CommittedSlot::Raw(_) => S::default(),
            };
            storage.replace(&mut slot, value);
            *committed = CommittedSlot::Stored(slot);
            true
        }
    }
}

fn clear_committed_slot<V, S, Store>(
    storage: &mut Store,
    committed: &mut CommittedSlot<V, S>,
) -> bool
where
    Store: CommittedValueStore<V, Slot = S>,
{
    match std::mem::replace(committed, CommittedSlot::Empty) {
        CommittedSlot::Empty => false,
        CommittedSlot::Raw(_) => true,
        CommittedSlot::Stored(mut slot) => storage.clear(&mut slot),
    }
}

fn discard_stored_slot<V, S, Store>(storage: &mut Store, committed: &mut CommittedSlot<V, S>)
where
    Store: CommittedValueStore<V, Slot = S>,
{
    if let CommittedSlot::Stored(mut slot) = std::mem::replace(committed, CommittedSlot::Empty) {
        let _ = storage.clear(&mut slot);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use aivi_backend::{MovingRuntimeValueStore, RuntimeValue};

    use crate::{
        graph::{
            DerivedHandle, ReactiveClauseBuilderSpec, ReactiveClauseHandle, SignalGraphBuilder,
            SignalHandle,
        },
        scheduler::{Publication, PublicationDropReason, Scheduler, SchedulerAccessError},
    };

    use super::{
        CommittedSlot, DependencyValues, DerivedNodeEvaluator, DerivedSignalUpdate,
        DroppedPublication, PendingRawValue, PendingSlot, RawSlotPlanId, clear_committed_slot,
        commit_pending_slot,
    };

    fn text_ptr(value: &RuntimeValue) -> *const u8 {
        let RuntimeValue::Text(text) = value else {
            panic!("expected text runtime value");
        };
        text.as_ptr()
    }

    struct ReactiveTestEvaluator {
        total: SignalHandle,
        doubled: DerivedHandle,
        left: SignalHandle,
        right: SignalHandle,
        ready: SignalHandle,
        enabled: SignalHandle,
        first_clause: ReactiveClauseHandle,
        second_clause: ReactiveClauseHandle,
    }

    impl DerivedNodeEvaluator<i32> for ReactiveTestEvaluator {
        fn evaluate(
            &mut self,
            signal: DerivedHandle,
            inputs: DependencyValues<'_, i32>,
        ) -> DerivedSignalUpdate<i32> {
            if signal == self.doubled {
                if !inputs.updated_signal(self.total) {
                    return DerivedSignalUpdate::Unchanged;
                }
                return inputs
                    .value_for(self.total)
                    .copied()
                    .map(|value| value * 2)
                    .into();
            }
            DerivedSignalUpdate::Unchanged
        }

        fn evaluate_reactive_seed(
            &mut self,
            signal: SignalHandle,
            _inputs: DependencyValues<'_, i32>,
        ) -> DerivedSignalUpdate<i32> {
            assert_eq!(signal, self.total);
            DerivedSignalUpdate::Value(0)
        }

        fn evaluate_reactive_guard(
            &mut self,
            signal: SignalHandle,
            clause: ReactiveClauseHandle,
            inputs: DependencyValues<'_, i32>,
        ) -> bool {
            assert_eq!(signal, self.total);
            if clause == self.first_clause {
                return inputs.value_for(self.ready).copied() == Some(1);
            }
            if clause == self.second_clause {
                return inputs.value_for(self.ready).copied() == Some(1)
                    && inputs.value_for(self.enabled).copied() == Some(1);
            }
            panic!("unexpected reactive clause {:?}", clause);
        }

        fn evaluate_reactive_body(
            &mut self,
            signal: SignalHandle,
            clause: ReactiveClauseHandle,
            inputs: DependencyValues<'_, i32>,
        ) -> DerivedSignalUpdate<i32> {
            assert_eq!(signal, self.total);
            let left = inputs
                .value_for(self.left)
                .copied()
                .expect("left should be present");
            let right = inputs
                .value_for(self.right)
                .copied()
                .expect("right should be present");
            if clause == self.first_clause {
                return DerivedSignalUpdate::Value(left + right);
            }
            if clause == self.second_clause {
                return DerivedSignalUpdate::Value(left + right + 100);
            }
            panic!("unexpected reactive clause {:?}", clause);
        }
    }

    #[test]
    fn scheduler_uses_latest_tick_snapshot_transactionally() {
        let mut builder = SignalGraphBuilder::new();
        let left = builder.add_input("left", None).unwrap();
        let right = builder.add_input("right", None).unwrap();
        let sum = builder.add_derived("sum", None).unwrap();
        let doubled = builder.add_derived("doubled", None).unwrap();
        builder
            .define_derived(sum, [left.as_signal(), right.as_signal()])
            .unwrap();
        builder.define_derived(doubled, [sum.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let left_stamp = scheduler.current_stamp(left).unwrap();
        let right_stamp = scheduler.current_stamp(right).unwrap();
        scheduler
            .queue_publication(Publication::new(left_stamp, 2_i32))
            .unwrap();
        scheduler
            .queue_publication(Publication::new(right_stamp, 3_i32))
            .unwrap();

        let mut order = Vec::new();
        let outcome = scheduler.tick(&mut |signal, inputs: DependencyValues<'_, i32>| {
            order.push(signal);
            if signal == sum {
                Some(inputs.value(0).copied()? + inputs.value(1).copied()?)
            } else if signal == doubled {
                Some(inputs.value(0).copied()? * 2)
            } else {
                None
            }
        });

        assert_eq!(order, vec![sum, doubled]);
        assert_eq!(
            outcome.committed(),
            &[
                left.as_signal(),
                right.as_signal(),
                sum.as_signal(),
                doubled.as_signal()
            ]
        );
        assert_eq!(
            scheduler.current_value(sum.as_signal()).unwrap().copied(),
            Some(5)
        );
        assert_eq!(
            scheduler
                .current_value(doubled.as_signal())
                .unwrap()
                .copied(),
            Some(10)
        );
    }

    #[test]
    fn scheduler_drains_worker_publications_into_scheduler_owned_queue() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("input", None).unwrap();
        let mirror = builder.add_derived("mirror", None).unwrap();
        builder.define_derived(mirror, [input.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let sender = scheduler.worker_sender();
        let stamp = scheduler.current_stamp(input).unwrap();

        thread::spawn(move || {
            sender.publish(Publication::new(stamp, 9_i32)).unwrap();
        })
        .join()
        .unwrap();

        let outcome =
            scheduler.tick(&mut |_, inputs: DependencyValues<'_, i32>| inputs.value(0).copied());

        assert_eq!(
            outcome.committed(),
            &[input.as_signal(), mirror.as_signal()]
        );
        assert_eq!(
            scheduler.current_value(input.as_signal()).unwrap().copied(),
            Some(9)
        );
        assert_eq!(
            scheduler
                .current_value(mirror.as_signal())
                .unwrap()
                .copied(),
            Some(9)
        );
    }

    #[test]
    fn scheduler_try_tick_propagates_evaluator_errors() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("input", None).unwrap();
        let mirror = builder.add_derived("mirror", None).unwrap();
        builder.define_derived(mirror, [input.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::<i32>::new(graph);
        let error = scheduler
            .try_tick(&mut |signal,
                            _: DependencyValues<'_, i32>|
             -> Result<Option<i32>, &'static str> {
                assert_eq!(signal, mirror);
                Err("boom")
            })
            .expect_err("fallible scheduler tick should surface evaluator errors");
        assert_eq!(error, "boom");
    }

    #[test]
    fn scheduler_executes_reactive_updates_in_source_order() {
        let mut builder = SignalGraphBuilder::new();
        let left = builder.add_input("left", None).unwrap();
        let right = builder.add_input("right", None).unwrap();
        let ready = builder.add_input("ready", None).unwrap();
        let enabled = builder.add_input("enabled", None).unwrap();
        let total = builder.add_reactive("total", None).unwrap();
        let doubled = builder.add_derived("doubled", None).unwrap();
        builder
            .define_reactive(
                total,
                [],
                [
                    ReactiveClauseBuilderSpec::new(
                        [ready.as_signal()],
                        [left.as_signal(), right.as_signal()],
                    ),
                    ReactiveClauseBuilderSpec::new(
                        [ready.as_signal(), enabled.as_signal()],
                        [left.as_signal(), right.as_signal()],
                    ),
                ],
            )
            .unwrap();
        builder.define_derived(doubled, [total]).unwrap();

        let graph = builder.build().unwrap();
        let clauses = graph
            .reactive(total)
            .expect("reactive signal spec should exist")
            .clauses()
            .to_vec();
        let mut scheduler = Scheduler::new(graph);

        for (input, value) in [(left, 2), (right, 3), (ready, 1), (enabled, 1)] {
            let stamp = scheduler.current_stamp(input).unwrap();
            scheduler
                .queue_publication(Publication::new(stamp, value))
                .unwrap();
        }

        let mut evaluator = ReactiveTestEvaluator {
            total,
            doubled,
            left: left.as_signal(),
            right: right.as_signal(),
            ready: ready.as_signal(),
            enabled: enabled.as_signal(),
            first_clause: clauses[0],
            second_clause: clauses[1],
        };
        let outcome = scheduler.tick(&mut evaluator);

        assert_eq!(
            outcome.committed(),
            &[
                left.as_signal(),
                right.as_signal(),
                ready.as_signal(),
                enabled.as_signal(),
                total,
                doubled.as_signal()
            ]
        );
        assert_eq!(scheduler.current_value(total).unwrap().copied(), Some(105));
        assert_eq!(
            scheduler
                .current_value(doubled.as_signal())
                .unwrap()
                .copied(),
            Some(210)
        );
    }

    #[test]
    fn scheduler_keeps_previous_committed_value_when_reactive_guard_is_false() {
        let mut builder = SignalGraphBuilder::new();
        let left = builder.add_input("left", None).unwrap();
        let right = builder.add_input("right", None).unwrap();
        let ready = builder.add_input("ready", None).unwrap();
        let enabled = builder.add_input("enabled", None).unwrap();
        let total = builder.add_reactive("total", None).unwrap();
        let doubled = builder.add_derived("doubled", None).unwrap();
        builder
            .define_reactive(
                total,
                [],
                [
                    ReactiveClauseBuilderSpec::new(
                        [ready.as_signal()],
                        [left.as_signal(), right.as_signal()],
                    ),
                    ReactiveClauseBuilderSpec::new(
                        [ready.as_signal(), enabled.as_signal()],
                        [left.as_signal(), right.as_signal()],
                    ),
                ],
            )
            .unwrap();
        builder.define_derived(doubled, [total]).unwrap();

        let graph = builder.build().unwrap();
        let clauses = graph
            .reactive(total)
            .expect("reactive signal spec should exist")
            .clauses()
            .to_vec();
        let mut scheduler = Scheduler::new(graph);
        let mut evaluator = ReactiveTestEvaluator {
            total,
            doubled,
            left: left.as_signal(),
            right: right.as_signal(),
            ready: ready.as_signal(),
            enabled: enabled.as_signal(),
            first_clause: clauses[0],
            second_clause: clauses[1],
        };

        for (input, value) in [(left, 4), (right, 6), (ready, 1), (enabled, 0)] {
            let stamp = scheduler.current_stamp(input).unwrap();
            scheduler
                .queue_publication(Publication::new(stamp, value))
                .unwrap();
        }
        scheduler.tick(&mut evaluator);
        assert_eq!(scheduler.current_value(total).unwrap().copied(), Some(10));
        assert_eq!(
            scheduler
                .current_value(doubled.as_signal())
                .unwrap()
                .copied(),
            Some(20)
        );

        for (input, value) in [(left, 7), (right, 9), (ready, 0), (enabled, 1)] {
            let stamp = scheduler.current_stamp(input).unwrap();
            scheduler
                .queue_publication(Publication::new(stamp, value))
                .unwrap();
        }
        let outcome = scheduler.tick(&mut evaluator);

        assert_eq!(
            outcome.committed(),
            &[
                left.as_signal(),
                right.as_signal(),
                ready.as_signal(),
                enabled.as_signal()
            ]
        );
        assert_eq!(scheduler.current_value(total).unwrap().copied(), Some(10));
        assert_eq!(
            scheduler
                .current_value(doubled.as_signal())
                .unwrap()
                .copied(),
            Some(20)
        );
    }

    #[test]
    fn scheduler_drops_stale_publications_after_generation_advance() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("input", None).unwrap();
        let mirror = builder.add_derived("mirror", None).unwrap();
        builder.define_derived(mirror, [input.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);

        let stale = scheduler.current_stamp(input).unwrap();
        let fresh = scheduler.advance_generation(input).unwrap();
        scheduler
            .queue_publication(Publication::new(stale, 1_i32))
            .unwrap();
        scheduler
            .queue_publication(Publication::new(fresh, 2_i32))
            .unwrap();

        let outcome =
            scheduler.tick(&mut |_, inputs: DependencyValues<'_, i32>| inputs.value(0).copied());

        assert_eq!(outcome.dropped_publications().len(), 1);
        assert_eq!(
            outcome.dropped_publications()[0],
            DroppedPublication {
                stamp: stale,
                reason: PublicationDropReason::StaleGeneration {
                    active: fresh.generation(),
                },
            }
        );
        assert_eq!(
            scheduler.current_value(input.as_signal()).unwrap().copied(),
            Some(2)
        );
        assert_eq!(
            scheduler
                .current_value(mirror.as_signal())
                .unwrap()
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn disposing_owner_recursively_clears_owned_signals() {
        let mut builder = SignalGraphBuilder::new();
        let session = builder.add_owner("session", None).unwrap();
        let widget = builder.add_owner("widget", Some(session)).unwrap();
        let source = builder.add_input("source", Some(widget)).unwrap();
        let local = builder.add_derived("local", Some(widget)).unwrap();
        let view = builder.add_derived("view", None).unwrap();
        builder.define_derived(local, [source.as_signal()]).unwrap();
        builder.define_derived(view, [local.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let mut evaluator = |signal, inputs: DependencyValues<'_, i32>| {
            if signal == local {
                Some(inputs.value(0).copied()? + 1)
            } else if signal == view {
                Some(inputs.value(0).copied()? * 2)
            } else {
                None
            }
        };

        let source_stamp = scheduler.current_stamp(source).unwrap();
        scheduler
            .queue_publication(Publication::new(source_stamp, 5_i32))
            .unwrap();
        scheduler.tick(&mut evaluator);
        assert_eq!(
            scheduler.current_value(view.as_signal()).unwrap().copied(),
            Some(12)
        );

        scheduler.queue_dispose_owner(session).unwrap();
        let outcome = scheduler.tick(&mut evaluator);

        assert!(!scheduler.is_owner_active(session).unwrap());
        assert!(!scheduler.is_owner_active(widget).unwrap());
        assert_eq!(scheduler.current_value(source.as_signal()).unwrap(), None);
        assert_eq!(scheduler.current_value(local.as_signal()).unwrap(), None);
        assert_eq!(scheduler.current_value(view.as_signal()).unwrap(), None);
        assert_eq!(
            outcome.committed(),
            &[source.as_signal(), local.as_signal(), view.as_signal()]
        );
        assert_eq!(
            scheduler.current_stamp(source),
            Err(SchedulerAccessError::OwnerInactive { owner: widget })
        );

        scheduler
            .queue_publication(Publication::new(source_stamp, 99_i32))
            .unwrap();
        let dropped = scheduler.tick(&mut evaluator);
        assert_eq!(
            dropped.dropped_publications(),
            &[DroppedPublication {
                stamp: source_stamp,
                reason: PublicationDropReason::OwnerInactive { owner: widget },
            }]
        );
    }

    #[test]
    fn adversarial_generation_bursts_and_owner_teardown_preserve_scheduler_invariants() {
        let mut builder = SignalGraphBuilder::new();
        let session = builder.add_owner("session", None).unwrap();
        let widget = builder.add_owner("widget", Some(session)).unwrap();
        let root = builder.add_input("root", None).unwrap();
        let owned = builder.add_input("owned", Some(session)).unwrap();
        let nested = builder.add_input("nested", Some(widget)).unwrap();
        let owned_view = builder.add_derived("owned-view", Some(session)).unwrap();
        let nested_view = builder.add_derived("nested-view", Some(widget)).unwrap();
        let aggregate = builder.add_derived("aggregate", None).unwrap();
        builder
            .define_derived(owned_view, [owned.as_signal()])
            .unwrap();
        builder
            .define_derived(nested_view, [nested.as_signal()])
            .unwrap();
        builder
            .define_derived(
                aggregate,
                [
                    root.as_signal(),
                    owned_view.as_signal(),
                    nested_view.as_signal(),
                ],
            )
            .unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let mut evaluator = |signal, inputs: DependencyValues<'_, i32>| {
            if signal == owned_view {
                Some(inputs.value(0).copied()? + 1)
            } else if signal == nested_view {
                Some(inputs.value(0).copied()? + 10)
            } else if signal == aggregate {
                Some(
                    inputs.value(0).copied()?
                        + inputs.value(1).copied()?
                        + inputs.value(2).copied()?,
                )
            } else {
                None
            }
        };

        scheduler
            .queue_publication(Publication::new(
                scheduler.current_stamp(root).unwrap(),
                0_i32,
            ))
            .unwrap();
        scheduler
            .queue_publication(Publication::new(
                scheduler.current_stamp(owned).unwrap(),
                10_i32,
            ))
            .unwrap();
        scheduler
            .queue_publication(Publication::new(
                scheduler.current_stamp(nested).unwrap(),
                100_i32,
            ))
            .unwrap();
        scheduler.tick(&mut evaluator);

        let mut stale_owned = Vec::new();
        let mut stale_nested = Vec::new();
        for round in 0..24_i32 {
            let stale_owned_stamp = scheduler.current_stamp(owned).unwrap();
            let fresh_owned = scheduler.advance_generation(owned).unwrap();
            stale_owned.push(stale_owned_stamp);

            let stale_nested_stamp = scheduler.current_stamp(nested).unwrap();
            let fresh_nested = scheduler.advance_generation(nested).unwrap();
            stale_nested.push(stale_nested_stamp);

            scheduler
                .queue_publication(Publication::new(
                    scheduler.current_stamp(root).unwrap(),
                    round,
                ))
                .unwrap();
            for (index, stamp) in stale_owned.iter().copied().enumerate() {
                scheduler
                    .queue_publication(Publication::new(stamp, -1_000 - round * 100 - index as i32))
                    .unwrap();
            }
            for (index, stamp) in stale_nested.iter().copied().enumerate() {
                scheduler
                    .queue_publication(Publication::new(
                        stamp,
                        -10_000 - round * 100 - index as i32,
                    ))
                    .unwrap();
            }

            let fresh_owned_value = round * 2 + 1;
            let fresh_nested_value = round * 3 + 2;
            scheduler
                .queue_publication(Publication::new(fresh_owned, fresh_owned_value))
                .unwrap();
            scheduler
                .queue_publication(Publication::new(fresh_nested, fresh_nested_value))
                .unwrap();

            let outcome = scheduler.tick(&mut evaluator);
            let owned_drops = outcome
                .dropped_publications()
                .iter()
                .filter(|publication| publication.stamp().input() == owned)
                .collect::<Vec<_>>();
            let nested_drops = outcome
                .dropped_publications()
                .iter()
                .filter(|publication| publication.stamp().input() == nested)
                .collect::<Vec<_>>();
            assert_eq!(owned_drops.len(), stale_owned.len());
            assert_eq!(nested_drops.len(), stale_nested.len());
            assert!(owned_drops.iter().all(|publication| publication.reason()
                == PublicationDropReason::StaleGeneration {
                    active: fresh_owned.generation(),
                }));
            assert!(nested_drops.iter().all(|publication| publication.reason()
                == PublicationDropReason::StaleGeneration {
                    active: fresh_nested.generation(),
                }));

            assert_eq!(
                scheduler.current_value(root.as_signal()).unwrap().copied(),
                Some(round)
            );
            assert_eq!(
                scheduler.current_value(owned.as_signal()).unwrap().copied(),
                Some(fresh_owned_value)
            );
            assert_eq!(
                scheduler
                    .current_value(nested.as_signal())
                    .unwrap()
                    .copied(),
                Some(fresh_nested_value)
            );
            assert_eq!(
                scheduler
                    .current_value(owned_view.as_signal())
                    .unwrap()
                    .copied(),
                Some(fresh_owned_value + 1)
            );
            assert_eq!(
                scheduler
                    .current_value(nested_view.as_signal())
                    .unwrap()
                    .copied(),
                Some(fresh_nested_value + 10)
            );
            assert_eq!(
                scheduler
                    .current_value(aggregate.as_signal())
                    .unwrap()
                    .copied(),
                Some(round + fresh_owned_value + fresh_nested_value + 11)
            );
        }

        let active_owned = scheduler.current_stamp(owned).unwrap();
        let active_nested = scheduler.current_stamp(nested).unwrap();
        scheduler.queue_dispose_owner(session).unwrap();
        scheduler
            .queue_publication(Publication::new(
                scheduler.current_stamp(root).unwrap(),
                999_i32,
            ))
            .unwrap();
        for (index, stamp) in stale_owned
            .iter()
            .copied()
            .chain(std::iter::once(active_owned))
            .enumerate()
        {
            scheduler
                .queue_publication(Publication::new(stamp, 50_000 + index as i32))
                .unwrap();
        }
        for (index, stamp) in stale_nested
            .iter()
            .copied()
            .chain(std::iter::once(active_nested))
            .enumerate()
        {
            scheduler
                .queue_publication(Publication::new(stamp, 60_000 + index as i32))
                .unwrap();
        }

        let outcome = scheduler.tick(&mut evaluator);
        assert!(!scheduler.is_owner_active(session).unwrap());
        assert!(!scheduler.is_owner_active(widget).unwrap());
        assert_eq!(
            scheduler.current_value(root.as_signal()).unwrap().copied(),
            Some(999)
        );
        assert_eq!(scheduler.current_value(owned.as_signal()).unwrap(), None);
        assert_eq!(scheduler.current_value(nested.as_signal()).unwrap(), None);
        assert_eq!(
            scheduler.current_value(owned_view.as_signal()).unwrap(),
            None
        );
        assert_eq!(
            scheduler.current_value(nested_view.as_signal()).unwrap(),
            None
        );
        assert_eq!(
            scheduler.current_value(aggregate.as_signal()).unwrap(),
            None
        );

        let owned_drops = outcome
            .dropped_publications()
            .iter()
            .filter(|publication| publication.stamp().input() == owned)
            .collect::<Vec<_>>();
        let nested_drops = outcome
            .dropped_publications()
            .iter()
            .filter(|publication| publication.stamp().input() == nested)
            .collect::<Vec<_>>();
        assert_eq!(owned_drops.len(), stale_owned.len() + 1);
        assert_eq!(nested_drops.len(), stale_nested.len() + 1);
        assert!(owned_drops.iter().all(|publication| publication.reason()
            == PublicationDropReason::OwnerInactive { owner: session }));
        assert!(nested_drops.iter().all(|publication| publication.reason()
            == PublicationDropReason::OwnerInactive { owner: widget }));
        assert_eq!(
            scheduler.current_stamp(owned),
            Err(SchedulerAccessError::OwnerInactive { owner: session })
        );
        assert_eq!(
            scheduler.current_stamp(nested),
            Err(SchedulerAccessError::OwnerInactive { owner: widget })
        );
    }

    #[test]
    fn moving_store_relocates_live_values_and_collects_disposed_owner_roots() {
        let mut builder = SignalGraphBuilder::new();
        let owner = builder.add_owner("owner", None).unwrap();
        let input = builder.add_input("input", Some(owner)).unwrap();
        let mirror = builder.add_derived("mirror", Some(owner)).unwrap();
        builder.define_derived(mirror, [input.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::with_value_store(graph, MovingRuntimeValueStore::default());
        let stamp = scheduler.current_stamp(input).unwrap();
        scheduler
            .queue_publication(Publication::new(stamp, RuntimeValue::Text("Ada".into())))
            .unwrap();

        scheduler.tick(&mut |_, inputs: DependencyValues<'_, RuntimeValue>| {
            Some(inputs.value(0)?.clone())
        });

        assert_eq!(scheduler.storage.live_root_count(), 2);
        assert_eq!(scheduler.storage.allocated_value_count(), 2);
        let CommittedSlot::Stored(first_input_slot) = &scheduler.slots.committed[input.index()]
        else {
            panic!("input signal should hold a store-managed slot");
        };
        let first_input_handle = first_input_slot.expect("input signal should hold a GC root");
        let CommittedSlot::Stored(first_mirror_slot) =
            &scheduler.slots.committed[mirror.as_signal().index()]
        else {
            panic!("derived signal should hold a store-managed slot");
        };
        let first_mirror_handle = first_mirror_slot.expect("derived signal should hold a GC root");
        let first_input_ptr = text_ptr(
            scheduler
                .current_value(input.as_signal())
                .unwrap()
                .expect("input value should remain readable"),
        );
        let first_mirror_ptr = text_ptr(
            scheduler
                .current_value(mirror.as_signal())
                .unwrap()
                .expect("derived value should remain readable"),
        );

        let outcome = scheduler.tick(&mut |_, inputs: DependencyValues<'_, RuntimeValue>| {
            Some(inputs.value(0)?.clone())
        });
        assert!(
            outcome.is_empty(),
            "empty ticks should still be valid GC safe points"
        );
        assert_eq!(scheduler.storage.live_root_count(), 2);
        assert_eq!(
            scheduler
                .slots
                .current_value(input.as_signal(), &scheduler.storage),
            scheduler.current_value(input.as_signal()).unwrap(),
            "slot store reads should stay aligned with current_value"
        );
        assert_eq!(
            match &scheduler.slots.committed[input.index()] {
                CommittedSlot::Stored(slot) => *slot,
                CommittedSlot::Empty | CommittedSlot::Raw(_) => None,
            },
            Some(first_input_handle),
            "stable GC handles must survive relocation"
        );
        assert_eq!(
            match &scheduler.slots.committed[mirror.as_signal().index()] {
                CommittedSlot::Stored(slot) => *slot,
                CommittedSlot::Empty | CommittedSlot::Raw(_) => None,
            },
            Some(first_mirror_handle),
            "stable GC handles must survive relocation for derived signals too"
        );
        assert_ne!(
            first_input_ptr,
            text_ptr(
                scheduler
                    .current_value(input.as_signal())
                    .unwrap()
                    .expect("relocated input value should stay readable"),
            ),
            "moving collection must relocate committed input payloads"
        );
        assert_ne!(
            first_mirror_ptr,
            text_ptr(
                scheduler
                    .current_value(mirror.as_signal())
                    .unwrap()
                    .expect("relocated derived value should stay readable"),
            ),
            "moving collection must relocate derived payloads too"
        );

        scheduler.queue_dispose_owner(owner).unwrap();
        scheduler.tick(&mut |_, inputs: DependencyValues<'_, RuntimeValue>| {
            Some(inputs.value(0)?.clone())
        });
        assert_eq!(scheduler.current_value(input.as_signal()).unwrap(), None);
        assert_eq!(scheduler.current_value(mirror.as_signal()).unwrap(), None);
        assert_eq!(scheduler.storage.live_root_count(), 0);
        assert_eq!(
            scheduler.storage.allocated_value_count(),
            0,
            "owner disposal should leave no retained GC objects after the collection safe point"
        );
    }

    #[test]
    fn slot_store_commits_raw_slots_and_clears_them_without_store_state() {
        let mut storage = aivi_backend::InlineCommittedValueStore::<i32>::default();
        let mut committed = CommittedSlot::Empty;
        let raw_plan = RawSlotPlanId::from_raw(7);
        let raw = PendingRawValue::new(raw_plan, 42_i32.to_le_bytes(), 42_i32);

        assert!(commit_pending_slot(
            &mut storage,
            &mut committed,
            PendingSlot::NextRaw(raw),
        ));
        let CommittedSlot::Raw(raw) = &committed else {
            panic!("raw commit should keep the slot in raw form");
        };
        assert_eq!(raw.plan(), raw_plan);
        assert_eq!(raw.bytes().as_slice(), &42_i32.to_le_bytes());
        assert_eq!(committed.current_value(&storage), Some(&42_i32));

        assert!(clear_committed_slot(&mut storage, &mut committed));
        assert!(matches!(committed, CommittedSlot::Empty));
    }

    #[test]
    fn slot_store_transitions_from_raw_to_store_managed_value() {
        let mut storage = aivi_backend::InlineCommittedValueStore::<i32>::default();
        let mut committed = CommittedSlot::Empty;
        assert!(commit_pending_slot(
            &mut storage,
            &mut committed,
            PendingSlot::NextRaw(PendingRawValue::new(
                RawSlotPlanId::from_raw(3),
                7_i32.to_le_bytes(),
                7_i32,
            )),
        ));

        assert!(commit_pending_slot(
            &mut storage,
            &mut committed,
            PendingSlot::NextStored(9_i32),
        ));
        let CommittedSlot::Stored(slot) = &committed else {
            panic!("store-managed commit should replace the raw slot");
        };
        assert_eq!(*slot, Some(9_i32));
        assert_eq!(committed.current_value(&storage), Some(&9_i32));
    }

    #[test]
    fn dependency_values_resolve_raw_and_store_managed_pending_updates_before_committed_values() {
        let raw_signal = SignalHandle::from_raw(0);
        let stored_signal = SignalHandle::from_raw(1);
        let raw_committed = 100_i32;
        let stored_committed = 200_i32;
        let pending = vec![
            PendingSlot::NextRaw(PendingRawValue::new(
                RawSlotPlanId::from_raw(1),
                5_i32.to_le_bytes(),
                5_i32,
            )),
            PendingSlot::NextStored(9_i32),
        ];
        let committed = vec![Some(&raw_committed), Some(&stored_committed)];
        let inputs = DependencyValues {
            dependencies: &[raw_signal, stored_signal],
            pending: &pending,
            committed: &committed,
        };

        assert_eq!(inputs.value_for(raw_signal).copied(), Some(5_i32));
        assert_eq!(inputs.value_for(stored_signal).copied(), Some(9_i32));
        assert!(inputs.updated_signal(raw_signal));
        assert!(inputs.updated_signal(stored_signal));
    }

    #[test]
    fn deep_chains_propagate_without_recursion() {
        let mut builder = SignalGraphBuilder::new();
        let root = builder.add_input("root", None).unwrap();
        let mut previous = root.as_signal();
        let depth = 4_096usize;

        for index in 0..depth {
            let node = builder.add_derived(format!("node-{index}"), None).unwrap();
            builder.define_derived(node, [previous]).unwrap();
            previous = node.as_signal();
        }

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let stamp = scheduler.current_stamp(root).unwrap();
        scheduler
            .queue_publication(Publication::new(stamp, 0usize))
            .unwrap();

        let outcome = scheduler.tick(&mut |_, inputs: DependencyValues<'_, usize>| {
            Some(inputs.value(0).copied()? + 1)
        });

        assert_eq!(
            scheduler.current_value(previous).unwrap().copied(),
            Some(depth)
        );
        assert_eq!(outcome.committed().len(), depth + 1);
    }

    #[test]
    fn diamond_graph_batches_evaluate_once_without_glitches() {
        let mut builder = SignalGraphBuilder::new();
        let root = builder.add_input("root", None).unwrap();
        let join = builder.add_derived("join", None).unwrap();
        let left = builder.add_derived("left", None).unwrap();
        let right = builder.add_derived("right", None).unwrap();
        builder
            .define_derived(join, [left.as_signal(), right.as_signal()])
            .unwrap();
        builder.define_derived(left, [root.as_signal()]).unwrap();
        builder.define_derived(right, [root.as_signal()]).unwrap();

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let stamp = scheduler.current_stamp(root).unwrap();
        scheduler
            .queue_publication(Publication::new(stamp, 4_i32))
            .unwrap();

        let mut order = Vec::new();
        let mut left_evals = 0;
        let mut right_evals = 0;
        let mut join_evals = 0;
        scheduler.tick(&mut |signal, inputs: DependencyValues<'_, i32>| {
            order.push(signal);
            if signal == left {
                left_evals += 1;
                Some(inputs.value(0).copied()? + 1)
            } else if signal == right {
                right_evals += 1;
                Some(inputs.value(0).copied()? * 2)
            } else if signal == join {
                join_evals += 1;
                Some(inputs.value(0).copied()? + inputs.value(1).copied()?)
            } else {
                None
            }
        });

        assert_eq!(order, vec![left, right, join]);
        assert_eq!(left_evals, 1);
        assert_eq!(right_evals, 1);
        assert_eq!(join_evals, 1);
        assert_eq!(
            scheduler.current_value(join.as_signal()).unwrap().copied(),
            Some(13)
        );
    }

    #[derive(Clone, Copy)]
    struct TestRng(u64);

    impl TestRng {
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }

        fn next_usize(&mut self, upper_bound: usize) -> usize {
            (self.next_u32() as usize) % upper_bound
        }

        fn next_i32(&mut self) -> i32 {
            (self.next_u32() % 17) as i32 - 8
        }
    }

    #[test]
    fn randomized_dags_commit_the_same_snapshot_as_their_topological_model() {
        for seed in 0..32_u64 {
            let mut rng = TestRng(seed + 1);
            let mut builder = SignalGraphBuilder::new();
            let mut all_signals = Vec::new();
            let mut inputs = Vec::new();
            for index in 0..3 {
                let input = builder.add_input(format!("input-{index}"), None).unwrap();
                inputs.push(input);
                all_signals.push(input.as_signal());
            }

            let mut derived_specs = Vec::new();
            for index in 0..4 {
                let derived = builder
                    .add_derived(format!("derived-{index}"), None)
                    .unwrap();
                let available = all_signals.len();
                let dep_count = 1 + rng.next_usize(available.min(3));
                let mut selected = std::collections::BTreeSet::new();
                while selected.len() < dep_count {
                    selected.insert(rng.next_usize(available));
                }
                let dependencies = selected
                    .into_iter()
                    .map(|slot| all_signals[slot])
                    .collect::<Vec<_>>();
                builder
                    .define_derived(derived, dependencies.iter().copied())
                    .unwrap();
                let bias = rng.next_i32();
                derived_specs.push((derived, dependencies, bias));
                all_signals.push(derived.as_signal());
            }

            let graph = builder.build().unwrap();
            let mut scheduler = Scheduler::new(graph);
            let mut expected = vec![None::<i32>; all_signals.len()];

            for step in 0..20 {
                let mut changed_inputs = std::collections::BTreeSet::new();
                changed_inputs.insert(rng.next_usize(inputs.len()));
                while changed_inputs.len() < inputs.len() && rng.next_u32().is_multiple_of(3) {
                    changed_inputs.insert(rng.next_usize(inputs.len()));
                }

                for changed in changed_inputs {
                    let input = inputs[changed];
                    let next = rng.next_i32();
                    let stamp = scheduler.current_stamp(input).unwrap();
                    scheduler
                        .queue_publication(Publication::new(stamp, next))
                        .unwrap();
                    expected[input.index()] = Some(next);
                }

                for (derived, dependencies, bias) in &derived_specs {
                    let mut total = *bias;
                    let mut ready = true;
                    for dependency in dependencies {
                        match expected[dependency.index()] {
                            Some(value) => total += value,
                            None => {
                                ready = false;
                                break;
                            }
                        }
                    }
                    expected[derived.as_signal().index()] = ready.then_some(total);
                }

                scheduler.tick(&mut |signal, inputs: DependencyValues<'_, i32>| {
                    let (_, _, bias) = derived_specs
                        .iter()
                        .find(|(derived, _, _)| *derived == signal)
                        .expect(
                            "randomized evaluator should only receive declared derived signals",
                        );
                    let mut total = *bias;
                    for index in 0..inputs.len() {
                        total += inputs.value(index).copied()?;
                    }
                    Some(total)
                });

                for &signal in &all_signals {
                    assert_eq!(
                        scheduler.current_value(signal).unwrap().copied(),
                        expected[signal.index()],
                        "seed {seed} step {step} diverged for signal {:?}",
                        signal
                    );
                }
            }
        }
    }

    #[test]
    fn runtime_high_fanout_signal_graph_no_glitch() {
        const N: usize = 20;
        let mut builder = SignalGraphBuilder::new();
        let source = builder.add_input("source", None).unwrap();
        let mut derived_handles = Vec::with_capacity(N);
        for index in 0..N {
            let node = builder
                .add_derived(format!("derived-{index}"), None)
                .unwrap();
            builder.define_derived(node, [source.as_signal()]).unwrap();
            derived_handles.push(node);
        }

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let stamp = scheduler.current_stamp(source).unwrap();
        scheduler
            .queue_publication(Publication::new(stamp, 100_i32))
            .unwrap();

        let mut eval_counts = [0usize; N];
        let outcome = scheduler.tick(&mut |signal, inputs: DependencyValues<'_, i32>| {
            let idx = derived_handles
                .iter()
                .position(|&h| h == signal)
                .expect("evaluator receives only declared derived signals");
            eval_counts[idx] += 1;
            Some(inputs.value(0).copied()? + idx as i32)
        });

        // Source + all N derived must be committed in one tick.
        assert_eq!(outcome.committed().len(), N + 1);

        // Each derived is evaluated exactly once (no glitch).
        for (idx, count) in eval_counts.iter().enumerate() {
            assert_eq!(*count, 1, "derived-{idx} evaluated more than once");
        }

        // All N derived signals have the expected values.
        for (idx, handle) in derived_handles.iter().enumerate() {
            assert_eq!(
                scheduler
                    .current_value(handle.as_signal())
                    .unwrap()
                    .copied(),
                Some(100 + idx as i32),
                "derived-{idx} has wrong value"
            );
        }
    }

    #[test]
    fn runtime_initial_signal_settle_is_correct() {
        const DEPTH: usize = 10;
        let mut builder = SignalGraphBuilder::new();
        let root = builder.add_input("root", None).unwrap();
        let mut previous = root.as_signal();
        let mut chain = Vec::with_capacity(DEPTH);
        for index in 0..DEPTH {
            let node = builder.add_derived(format!("chain-{index}"), None).unwrap();
            builder.define_derived(node, [previous]).unwrap();
            previous = node.as_signal();
            chain.push(node);
        }

        let graph = builder.build().unwrap();
        let mut scheduler = Scheduler::new(graph);
        let stamp = scheduler.current_stamp(root).unwrap();
        scheduler
            .queue_publication(Publication::new(stamp, 0_i32))
            .unwrap();

        scheduler
            .tick(&mut |_, inputs: DependencyValues<'_, i32>| Some(inputs.value(0).copied()? + 1));

        // The last node in the chain should equal DEPTH (each step adds 1).
        assert_eq!(
            scheduler.current_value(previous).unwrap().copied(),
            Some(DEPTH as i32)
        );
    }
}
