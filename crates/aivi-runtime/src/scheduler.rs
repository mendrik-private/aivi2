use std::{
    collections::{BTreeSet, VecDeque},
    iter::repeat_with,
    sync::mpsc,
};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    fn advance(self) -> Self {
        Self(self.0.checked_add(1).expect("input generation overflow"))
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
    sender: mpsc::Sender<Publication<V>>,
}

impl<V> WorkerPublicationSender<V> {
    pub fn publish(&self, publication: Publication<V>) -> Result<(), WorkerSendError<V>> {
        self.sender
            .send(publication)
            .map_err(|err| WorkerSendError { publication: err.0 })
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

pub struct Scheduler<V> {
    graph: SignalGraph,
    owners: Vec<OwnerRuntimeState>,
    inputs: Vec<Option<InputRuntimeState>>,
    signals: Vec<SignalRuntimeState<V>>,
    queue: VecDeque<SchedulerMessage<V>>,
    worker_publication_tx: mpsc::Sender<Publication<V>>,
    worker_publication_rx: mpsc::Receiver<Publication<V>>,
    initialized: bool,
    next_tick: u64,
}

impl<V> Scheduler<V> {
    pub fn new(graph: SignalGraph) -> Self {
        let (worker_publication_tx, worker_publication_rx) = mpsc::channel();
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
            .map(|_| SignalRuntimeState { current: None })
            .collect();

        Self {
            graph,
            owners,
            inputs,
            signals,
            queue: VecDeque::new(),
            worker_publication_tx,
            worker_publication_rx,
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
        }
    }

    pub fn current_value(&self, signal: SignalHandle) -> Result<Option<&V>, SchedulerAccessError> {
        self.validate_signal(signal)?;
        Ok(self.signals[signal.index()].current.as_ref())
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
        self.drain_worker_publications();
        let tick = self.next_tick;
        self.next_tick = self
            .next_tick
            .checked_add(1)
            .expect("tick counter overflow");

        let mut pending = repeat_with(|| PendingValue::Unchanged)
            .take(self.signals.len())
            .collect::<Vec<_>>();
        let messages = self.queue.drain(..).collect::<Vec<_>>();
        let disposed = self.collect_disposed_owners(&messages);
        self.apply_owner_disposals(&disposed, &mut pending);

        let mut dropped = Vec::new();
        let mut publications = repeat_with(|| None::<Publication<V>>)
            .take(self.signals.len())
            .collect::<Vec<_>>();

        for message in messages {
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

        for publication in publications.into_iter().flatten() {
            let (stamp, value) = publication.into_parts();
            pending[stamp.input.index()] = PendingValue::NextSome(value);
        }

        let mut dirty = vec![false; self.signals.len()];
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
                    committed: &self.signals,
                };
                pending[signal.index()] = match evaluator.evaluate(signal, inputs) {
                    Some(value) => PendingValue::NextSome(value),
                    None => PendingValue::NextNone,
                };
            }
        }

        let committed = pending
            .into_iter()
            .enumerate()
            .filter_map(|(index, pending)| {
                let handle = SignalHandle::from_raw(index as u32);
                match pending {
                    PendingValue::Unchanged => None,
                    PendingValue::NextNone => self.signals[index]
                        .current
                        .take()
                        .is_some()
                        .then_some(handle),
                    PendingValue::NextSome(value) => {
                        self.signals[index].current = Some(value);
                        Some(handle)
                    }
                }
            })
            .collect::<Vec<_>>();

        self.initialized = true;
        TickOutcome {
            tick,
            committed: committed.into_boxed_slice(),
            dropped_publications: dropped.into_boxed_slice(),
        }
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
    committed: &'a [SignalRuntimeState<V>],
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
            PendingValue::Unchanged => self.committed[signal.index()].current.as_ref(),
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

struct SignalRuntimeState<V> {
    current: Option<V>,
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

    use crate::{
        graph::SignalGraphBuilder,
        scheduler::{Publication, PublicationDropReason, Scheduler, SchedulerAccessError},
    };

    use super::{DependencyValues, DroppedPublication};

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
}
