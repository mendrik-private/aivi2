use std::{
    collections::{BTreeSet, VecDeque},
    convert::Infallible,
    sync::{Arc, mpsc},
};

use aivi_backend::{CommittedValueStore, InlineCommittedValueStore};

use crate::graph::{
    DerivedHandle, InputHandle, OwnerHandle, SignalGraph, SignalHandle, SignalKind,
};

pub trait DerivedNodeEvaluator<V> {
    fn evaluate(&mut self, signal: DerivedHandle, inputs: DependencyValues<'_, V>) -> Option<V>;
}

impl<V, F> DerivedNodeEvaluator<V> for F
where
    F: for<'a> FnMut(DerivedHandle, DependencyValues<'a, V>) -> Option<V>,
{
    fn evaluate(&mut self, signal: DerivedHandle, inputs: DependencyValues<'_, V>) -> Option<V> {
        self(signal, inputs)
    }
}

pub trait TryDerivedNodeEvaluator<V> {
    type Error;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<Option<V>, Self::Error>;
}

impl<V, E, F> TryDerivedNodeEvaluator<V> for F
where
    F: for<'a> FnMut(DerivedHandle, DependencyValues<'a, V>) -> Result<Option<V>, E>,
{
    type Error = E;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, V>,
    ) -> Result<Option<V>, Self::Error> {
        self(signal, inputs)
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
    signals: Vec<SignalRuntimeState<S::Slot>>,
    queue: VecDeque<SchedulerMessage<V>>,
    queued_messages_scratch: Vec<SchedulerMessage<V>>,
    pending_scratch: Vec<PendingValue<V>>,
    dirty_scratch: Vec<bool>,
    publications_scratch: Vec<Option<Publication<V>>>,
    dropped_scratch: Vec<DroppedPublication>,
    worker_publication_tx: mpsc::SyncSender<Publication<V>>,
    worker_publication_rx: mpsc::Receiver<Publication<V>>,
    worker_publication_notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    initialized: bool,
    next_tick: u64,
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
        let signals = (0..graph.signal_count())
            .map(|_| SignalRuntimeState {
                current: Default::default(),
            })
            .collect();

        Self {
            graph,
            storage,
            owners,
            inputs,
            signals,
            queue: VecDeque::new(),
            queued_messages_scratch: Vec::new(),
            pending_scratch: Vec::new(),
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
        Ok(self.storage.get(&self.signals[signal.index()].current))
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
        self.tick_with::<_, Infallible>(|signal, inputs| Ok(evaluator.evaluate(signal, inputs)))
            .unwrap_or_else(|never| match never {})
    }

    pub fn try_tick<E>(&mut self, evaluator: &mut E) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        self.tick_with(|signal, inputs| evaluator.try_evaluate(signal, inputs))
    }

    fn tick_with<E, X>(&mut self, mut evaluate: E) -> Result<TickOutcome, X>
    where
        E: FnMut(DerivedHandle, DependencyValues<'_, V>) -> Result<Option<V>, X>,
    {
        self.drain_worker_publications();
        let tick = self.next_tick;
        // Wrapping is safe: the tick counter is used for tracing only; no two
        // in-flight ticks run simultaneously.
        self.next_tick = self.next_tick.wrapping_add(1);

        let mut pending = std::mem::take(&mut self.pending_scratch);
        pending.clear();
        pending.resize_with(self.signals.len(), || PendingValue::Unchanged);

        let mut messages = std::mem::take(&mut self.queued_messages_scratch);
        messages.clear();
        messages.extend(self.queue.drain(..));
        let disposed = self.collect_disposed_owners(&messages);
        self.apply_owner_disposals(&disposed, &mut pending);

        let mut dropped = std::mem::take(&mut self.dropped_scratch);
        dropped.clear();
        let mut publications = std::mem::take(&mut self.publications_scratch);
        publications.clear();
        publications.resize_with(self.signals.len(), || None::<Publication<V>>);

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
            pending[stamp.input.index()] = PendingValue::NextSome(value);
        }
        self.publications_scratch = publications;

        let mut dirty = std::mem::take(&mut self.dirty_scratch);
        dirty.clear();
        dirty.resize(self.signals.len(), false);
        if self.initialized {
            self.mark_dirty_dependents(&pending, &mut dirty);
        } else {
            for batch in self.graph.batches() {
                for &signal in batch.signals() {
                    if self.signal_active(signal.as_signal()) {
                        dirty[signal.index()] = true;
                    }
                }
            }
        }

        {
            let committed_values = self
                .signals
                .iter()
                .map(|state| self.storage.get(&state.current))
                .collect::<Vec<_>>();
            for batch in self.graph.batches() {
                for &signal in batch.signals() {
                    if !dirty[signal.index()] || !self.signal_active(signal.as_signal()) {
                        continue;
                    }

                    let dependencies = self
                        .graph
                        .dependencies(signal)
                        .expect("topology batches only contain derived signals");
                    let inputs = DependencyValues {
                        dependencies,
                        pending: &pending,
                        committed: &committed_values,
                    };
                    pending[signal.index()] = match evaluate(signal, inputs)? {
                        Some(value) => PendingValue::NextSome(value),
                        None => PendingValue::NextNone,
                    };
                }
            }
        }
        self.dirty_scratch = dirty;

        let mut committed = Vec::new();
        for (index, pending_value) in pending.drain(..).enumerate() {
            let handle = SignalHandle::from_raw(index as u32);
            match pending_value {
                PendingValue::Unchanged => {}
                PendingValue::NextNone => {
                    if self.storage.clear(&mut self.signals[index].current) {
                        committed.push(handle);
                    }
                }
                PendingValue::NextSome(value) => {
                    self.storage
                        .replace(&mut self.signals[index].current, value);
                    committed.push(handle);
                }
            }
        }
        self.pending_scratch = pending;
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
            .signals
            .iter()
            .map(|state| &state.current)
            .collect::<Vec<_>>();
        self.storage.collect(&roots);
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
        pending: &mut [PendingValue<V>],
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
                pending[signal.index()] = PendingValue::NextNone;
                if let Some(input) = self.inputs[signal.index()].as_mut() {
                    input.generation = input.generation.advance();
                }
            }
        }
    }

    fn mark_dirty_dependents(&self, pending: &[PendingValue<V>], dirty: &mut [bool]) {
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
            if !self.signal_active(signal.as_signal()) && pending[signal.index()].is_unchanged() {
                continue;
            }

            dirty[signal.index()] = true;
            for &dependent in self
                .graph
                .dependents(signal.as_signal())
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
        self.validate_signal(input.as_signal())?;
        match self
            .graph
            .signal(input.as_signal())
            .map(|signal| signal.kind())
        {
            Some(SignalKind::Input) => Ok(()),
            Some(SignalKind::Derived(_)) => Err(SchedulerAccessError::SignalIsNotInput {
                signal: input.as_raw(),
            }),
            None => Err(SchedulerAccessError::UnknownSignalHandle {
                signal: input.as_raw(),
            }),
        }
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
}

/// Read-only dependency view for one derived evaluation within a transactional scheduler tick.
///
/// Each lookup resolves against the latest value already produced for this tick (`pending`) before
/// falling back to the previous committed snapshot. That guarantees topological, glitch-free reads:
/// downstream evaluators observe the newest stable upstream values for the current tick, never a
/// mixed intermediate state.
pub struct DependencyValues<'a, V> {
    dependencies: &'a [SignalHandle],
    pending: &'a [PendingValue<V>],
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

    fn resolve(&self, signal: SignalHandle) -> Option<&'a V> {
        match &self.pending[signal.index()] {
            PendingValue::Unchanged => self.committed[signal.index()],
            PendingValue::NextNone => None,
            PendingValue::NextSome(value) => Some(value),
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

struct SignalRuntimeState<S> {
    current: S,
}

enum PendingValue<V> {
    Unchanged,
    NextNone,
    NextSome(V),
}

impl<V> PendingValue<V> {
    fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use aivi_backend::{MovingRuntimeValueStore, RuntimeValue};

    use crate::{
        graph::SignalGraphBuilder,
        scheduler::{Publication, PublicationDropReason, Scheduler, SchedulerAccessError},
    };

    use super::{DependencyValues, DroppedPublication};

    fn text_ptr(value: &RuntimeValue) -> *const u8 {
        let RuntimeValue::Text(text) = value else {
            panic!("expected text runtime value");
        };
        text.as_ptr()
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

        assert_eq!(scheduler.is_owner_active(session).unwrap(), false);
        assert_eq!(scheduler.is_owner_active(widget).unwrap(), false);
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
        assert_eq!(scheduler.is_owner_active(session).unwrap(), false);
        assert_eq!(scheduler.is_owner_active(widget).unwrap(), false);
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
        let first_input_handle = scheduler.signals[input.index()]
            .current
            .expect("input signal should hold a GC root");
        let first_mirror_handle = scheduler.signals[mirror.as_signal().index()]
            .current
            .expect("derived signal should hold a GC root");
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
            scheduler.signals[input.index()].current,
            Some(first_input_handle),
            "stable GC handles must survive relocation"
        );
        assert_eq!(
            scheduler.signals[mirror.as_signal().index()].current,
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
                while changed_inputs.len() < inputs.len() && rng.next_u32() % 3 == 0 {
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
}
