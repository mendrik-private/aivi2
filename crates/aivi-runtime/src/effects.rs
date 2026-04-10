use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

// Re-export the backend's canonical `SourceInstanceId` so the runtime and
// backend share one definition instead of two independent macro-generated
// newtypes that could diverge (I2).
pub use aivi_backend::SourceInstanceId;
use aivi_backend::{CommittedValueStore, InlineCommittedValueStore};
use aivi_typing::{
    BuiltinSourceProvider, RecurrenceWakeupPlan, SourceCancellationPolicy, SourceOptionWakeupCause,
};

use crate::{
    graph::{InputHandle, OwnerHandle, SignalGraph, SignalHandle},
    reactive_program::ReactiveProgram,
    scheduler::{
        DerivedNodeEvaluator, Generation, Publication, PublicationStamp, Scheduler,
        SchedulerAccessError, TickOutcome, TryDerivedNodeEvaluator, WorkerPublicationSender,
    },
};

macro_rules! define_runtime_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub const fn from_raw(raw: u32) -> Self {
                Self(raw)
            }

            pub const fn as_raw(self) -> u32 {
                self.0
            }
        }
    };
}

define_runtime_id!(TaskInstanceId);

/// Runtime-facing provider identity carried forward from source elaboration/lowering.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RuntimeSourceProvider {
    Builtin(BuiltinSourceProvider),
    Custom(Box<str>),
}

impl RuntimeSourceProvider {
    pub const fn builtin(provider: BuiltinSourceProvider) -> Self {
        Self::Builtin(provider)
    }

    pub fn custom(provider: impl Into<Box<str>>) -> Self {
        Self::Custom(provider.into())
    }

    pub fn builtin_provider(&self) -> Option<BuiltinSourceProvider> {
        match self {
            Self::Builtin(provider) => Some(*provider),
            Self::Custom(_) => None,
        }
    }
}

/// Reconfiguration contract retained from the current source-lifecycle handoff.
///
/// Only `DisposeSupersededBeforePublish` exists today.  A second variant
/// (`HoldSupersededUntilPublish`) is anticipated when sources need to
/// complete in-flight work before a replacement takes over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceReplacementPolicy {
    /// Dispose superseded source work immediately before the replacement's
    /// first publication is applied to the scheduler state.
    DisposeSupersededBeforePublish,
}

/// Stale-work contract retained from the current source-lifecycle handoff.
///
/// Only `DropStalePublications` exists today.  A second variant
/// (`QueueStalePublications`) is anticipated for sources that must not lose
/// results even when a reconfiguration races ahead of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceStaleWorkPolicy {
    /// Discard any publication whose generation stamp predates the current
    /// scheduler generation for that source slot.
    DropStalePublications,
}

/// Runtime-facing source contract ready to accept lowered compiler handoffs.
///
/// `D` intentionally stays generic so later lowering can store either the compiler's decode
/// program directly or a lowered runtime-local decode reference without changing the scheduler
/// boundary again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRuntimeSpec<D = ()> {
    pub instance: SourceInstanceId,
    pub input: InputHandle,
    pub provider: RuntimeSourceProvider,
    pub reconfiguration_dependencies: Box<[SignalHandle]>,
    pub explicit_triggers: Box<[SignalHandle]>,
    pub active_when: Option<SignalHandle>,
    pub cancellation: SourceCancellationPolicy,
    pub replacement: SourceReplacementPolicy,
    pub stale_work: SourceStaleWorkPolicy,
    pub wakeup: Option<RecurrenceWakeupPlan>,
    pub decode: Option<D>,
}

impl<D> SourceRuntimeSpec<D> {
    pub fn new(
        instance: SourceInstanceId,
        input: InputHandle,
        provider: RuntimeSourceProvider,
    ) -> Self {
        let cancellation = provider
            .builtin_provider()
            .map(|provider| provider.contract().lifecycle().cancellation())
            .unwrap_or(SourceCancellationPolicy::ProviderManaged);
        Self {
            instance,
            input,
            provider,
            reconfiguration_dependencies: Vec::new().into_boxed_slice(),
            explicit_triggers: Vec::new().into_boxed_slice(),
            active_when: None,
            cancellation,
            replacement: SourceReplacementPolicy::DisposeSupersededBeforePublish,
            stale_work: SourceStaleWorkPolicy::DropStalePublications,
            wakeup: None,
            decode: None,
        }
    }
}

/// Runtime-facing task execution contract.
///
/// The current slice chooses the narrowest coherent scheduler contract: one live run per task
/// instance. Starting the same task again supersedes the older run by cancelling it and advancing
/// the scheduler generation on the task's sink input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRuntimeSpec {
    pub instance: TaskInstanceId,
    pub input: InputHandle,
    pub dependencies: Box<[SignalHandle]>,
    pub wakeup: Option<RecurrenceWakeupPlan>,
}

impl TaskRuntimeSpec {
    pub fn new(instance: TaskInstanceId, input: InputHandle) -> Self {
        Self {
            instance,
            input,
            dependencies: Vec::new().into_boxed_slice(),
            wakeup: None,
        }
    }
}

/// Read-only worker-side cancellation view.
///
/// Workers may observe cancellation but cannot directly mutate scheduler-owned task/source state.
#[derive(Clone, Debug, Default)]
pub struct CancellationObserver {
    state: Arc<AtomicBool>,
}

impl CancellationObserver {
    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Default)]
struct CancellationHandle {
    state: Arc<AtomicBool>,
}

impl CancellationHandle {
    fn new() -> Self {
        Self::default()
    }

    fn observer(&self) -> CancellationObserver {
        CancellationObserver {
            state: self.state.clone(),
        }
    }

    fn cancel(&self) {
        self.state.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicationPortError<V> {
    Cancelled { stamp: PublicationStamp, value: V },
    Disconnected { stamp: PublicationStamp, value: V },
}

/// Cloneable worker-side publication port for long-lived sources.
#[derive(Clone)]
pub struct SourcePublicationPort<V> {
    sender: WorkerPublicationSender<V>,
    stamp: PublicationStamp,
    cancellation: CancellationObserver,
}

impl<V> SourcePublicationPort<V> {
    pub fn stamp(&self) -> PublicationStamp {
        self.stamp
    }

    pub fn cancellation(&self) -> CancellationObserver {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn publish(&self, value: V) -> Result<(), PublicationPortError<V>> {
        publish_with_contract(&self.sender, self.stamp, &self.cancellation, value)
    }
}

/// One-shot worker-side completion port for `Task` execution.
pub struct TaskCompletionPort<V> {
    sender: WorkerPublicationSender<V>,
    stamp: PublicationStamp,
    cancellation: CancellationObserver,
}

impl<V> TaskCompletionPort<V> {
    pub fn stamp(&self) -> PublicationStamp {
        self.stamp
    }

    pub fn cancellation(&self) -> CancellationObserver {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn complete(self, value: V) -> Result<(), PublicationPortError<V>> {
        publish_with_contract(&self.sender, self.stamp, &self.cancellation, value)
    }
}

fn publish_with_contract<V>(
    sender: &WorkerPublicationSender<V>,
    stamp: PublicationStamp,
    cancellation: &CancellationObserver,
    value: V,
) -> Result<(), PublicationPortError<V>> {
    if cancellation.is_cancelled() {
        return Err(PublicationPortError::Cancelled { stamp, value });
    }

    sender
        .publish(Publication::new(stamp, value))
        .map_err(|err| {
            let (stamp, value) = err.into_publication().into_parts();
            PublicationPortError::Disconnected { stamp, value }
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderRuntimeContractViolation {
    ActiveWhenNotSupported,
    TooManyExplicitTriggers {
        supported: usize,
        actual: usize,
    },
    CancellationPolicyMismatch {
        expected: SourceCancellationPolicy,
        actual: SourceCancellationPolicy,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskSourceRuntimeError {
    UnknownSignalHandle {
        signal: u32,
    },
    SignalIsNotInput {
        signal: u32,
    },
    DuplicateManagedInput {
        input: u32,
    },
    DuplicateSourceInstance {
        instance: SourceInstanceId,
    },
    DuplicateTaskInstance {
        instance: TaskInstanceId,
    },
    UnknownSourceInstance {
        instance: SourceInstanceId,
    },
    UnknownTaskInstance {
        instance: TaskInstanceId,
    },
    OwnerPendingDisposal {
        owner: OwnerHandle,
    },
    SourceProviderContractViolation {
        instance: SourceInstanceId,
        provider: RuntimeSourceProvider,
        violation: SourceProviderRuntimeContractViolation,
    },
    Scheduler(SchedulerAccessError),
}

impl From<SchedulerAccessError> for TaskSourceRuntimeError {
    fn from(value: SchedulerAccessError) -> Self {
        Self::Scheduler(value)
    }
}

/// Evaluates one source's `activeWhen` gate against the latest stable signal value committed for
/// the current scheduler tick.
///
/// The runtime keeps the lifecycle decision itself generic because the full runtime value model is
/// not frozen yet. Callers provide the one narrow piece of interpretation the RFC requires here:
/// whether a source should be active for the current tick once its gate signal has settled.
pub trait SourceActiveWhenEvaluator<V, D = ()> {
    fn is_active_when(
        &mut self,
        instance: SourceInstanceId,
        spec: &SourceRuntimeSpec<D>,
        value: Option<&V>,
    ) -> bool;
}

impl<V, D, F> SourceActiveWhenEvaluator<V, D> for F
where
    F: FnMut(SourceInstanceId, &SourceRuntimeSpec<D>, Option<&V>) -> bool,
{
    fn is_active_when(
        &mut self,
        instance: SourceInstanceId,
        spec: &SourceRuntimeSpec<D>,
        value: Option<&V>,
    ) -> bool {
        self(instance, spec, value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLifecycleActionKind {
    Activate,
    Reconfigure,
    Suspend,
}

#[derive(Clone)]
pub enum SourceLifecycleAction<V> {
    Activate {
        instance: SourceInstanceId,
        port: SourcePublicationPort<V>,
    },
    Reconfigure {
        instance: SourceInstanceId,
        port: SourcePublicationPort<V>,
    },
    Suspend {
        instance: SourceInstanceId,
    },
}

impl<V> SourceLifecycleAction<V> {
    pub fn instance(&self) -> SourceInstanceId {
        match self {
            Self::Activate { instance, .. }
            | Self::Reconfigure { instance, .. }
            | Self::Suspend { instance } => *instance,
        }
    }

    pub fn kind(&self) -> SourceLifecycleActionKind {
        match self {
            Self::Activate { .. } => SourceLifecycleActionKind::Activate,
            Self::Reconfigure { .. } => SourceLifecycleActionKind::Reconfigure,
            Self::Suspend { .. } => SourceLifecycleActionKind::Suspend,
        }
    }

    pub fn port(&self) -> Option<&SourcePublicationPort<V>> {
        match self {
            Self::Activate { port, .. } | Self::Reconfigure { port, .. } => Some(port),
            Self::Suspend { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct TaskSourceTickOutcome<V> {
    scheduler: TickOutcome,
    source_actions: Box<[SourceLifecycleAction<V>]>,
}

impl<V> TaskSourceTickOutcome<V> {
    pub fn scheduler(&self) -> &TickOutcome {
        &self.scheduler
    }

    pub fn source_actions(&self) -> &[SourceLifecycleAction<V>] {
        &self.source_actions
    }

    pub fn into_scheduler(self) -> TickOutcome {
        self.scheduler
    }
}

pub struct TaskSourceRuntime<V, D = (), S = InlineCommittedValueStore<V>>
where
    S: CommittedValueStore<V>,
{
    scheduler: Scheduler<V, S>,
    sources: BTreeMap<SourceInstanceId, SourceSlot<D>>,
    tasks: BTreeMap<TaskInstanceId, TaskSlot>,
    claimed_inputs: BTreeMap<u32, ManagedInputKind>,
    pending_owner_disposals: BTreeSet<OwnerHandle>,
}

impl<V, D> TaskSourceRuntime<V, D, InlineCommittedValueStore<V>> {
    pub fn new(graph: SignalGraph) -> Self {
        Self::from_scheduler(Scheduler::new(graph))
    }
}

impl<V, D, S> TaskSourceRuntime<V, D, S>
where
    S: CommittedValueStore<V>,
{
    pub fn with_value_store(graph: SignalGraph, storage: S) -> Self {
        Self::from_scheduler(Scheduler::with_value_store(graph, storage))
    }

    pub fn from_scheduler(scheduler: Scheduler<V, S>) -> Self {
        Self {
            scheduler,
            sources: BTreeMap::new(),
            tasks: BTreeMap::new(),
            claimed_inputs: BTreeMap::new(),
            pending_owner_disposals: BTreeSet::new(),
        }
    }

    pub fn graph(&self) -> &SignalGraph {
        self.scheduler.graph()
    }

    pub fn worker_sender(&self) -> WorkerPublicationSender<V> {
        self.scheduler.worker_sender()
    }

    pub fn set_worker_publication_notifier(
        &mut self,
        notifier: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) {
        self.scheduler.set_worker_publication_notifier(notifier);
    }

    pub fn tick_count(&self) -> u64 {
        self.scheduler.tick_count()
    }

    pub fn queued_message_count(&self) -> usize {
        self.scheduler.queued_message_count()
    }

    pub fn current_value(
        &self,
        signal: SignalHandle,
    ) -> Result<Option<&V>, TaskSourceRuntimeError> {
        self.scheduler.current_value(signal).map_err(Into::into)
    }

    pub fn current_generation(
        &self,
        input: InputHandle,
    ) -> Result<Generation, TaskSourceRuntimeError> {
        self.scheduler.current_generation(input).map_err(Into::into)
    }

    pub fn current_stamp(
        &self,
        input: InputHandle,
    ) -> Result<PublicationStamp, TaskSourceRuntimeError> {
        self.scheduler.current_stamp(input).map_err(Into::into)
    }

    pub fn advance_generation(
        &mut self,
        input: InputHandle,
    ) -> Result<PublicationStamp, TaskSourceRuntimeError> {
        self.scheduler.advance_generation(input).map_err(Into::into)
    }

    pub fn is_owner_active(&self, owner: OwnerHandle) -> Result<bool, TaskSourceRuntimeError> {
        self.scheduler.is_owner_active(owner).map_err(Into::into)
    }

    pub fn source_spec(&self, instance: SourceInstanceId) -> Option<&SourceRuntimeSpec<D>> {
        self.sources.get(&instance).map(|slot| &slot.spec)
    }

    pub fn task_spec(&self, instance: TaskInstanceId) -> Option<&TaskRuntimeSpec> {
        self.tasks.get(&instance).map(|slot| &slot.spec)
    }

    pub fn is_source_active(&self, instance: SourceInstanceId) -> bool {
        self.sources
            .get(&instance)
            .is_some_and(|slot| slot.active.is_some())
    }

    pub fn is_task_active(&self, instance: TaskInstanceId) -> bool {
        self.tasks
            .get(&instance)
            .is_some_and(|slot| slot.active.is_some())
    }

    pub fn register_source(
        &mut self,
        spec: SourceRuntimeSpec<D>,
    ) -> Result<(), TaskSourceRuntimeError> {
        self.validate_input_handle(spec.input)?;
        for &dependency in &spec.reconfiguration_dependencies {
            self.validate_signal_handle(dependency)?;
        }
        for &trigger in &spec.explicit_triggers {
            self.validate_signal_handle(trigger)?;
        }
        if let Some(active_when) = spec.active_when {
            self.validate_signal_handle(active_when)?;
        }
        if self.sources.contains_key(&spec.instance) {
            return Err(TaskSourceRuntimeError::DuplicateSourceInstance {
                instance: spec.instance,
            });
        }
        self.validate_source_provider_contract(&spec)?;
        self.claim_input(spec.input)?;

        let owner = self.input_owner(spec.input);
        self.sources.insert(
            spec.instance,
            SourceSlot {
                spec,
                owner,
                active: None,
            },
        );
        Ok(())
    }

    pub fn register_task(&mut self, spec: TaskRuntimeSpec) -> Result<(), TaskSourceRuntimeError> {
        self.validate_input_handle(spec.input)?;
        for &dependency in &spec.dependencies {
            self.validate_signal_handle(dependency)?;
        }
        if self.tasks.contains_key(&spec.instance) {
            return Err(TaskSourceRuntimeError::DuplicateTaskInstance {
                instance: spec.instance,
            });
        }
        self.claim_input(spec.input)?;

        let owner = self.input_owner(spec.input);
        self.tasks.insert(
            spec.instance,
            TaskSlot {
                spec,
                owner,
                active: None,
            },
        );
        Ok(())
    }

    pub fn activate_source(
        &mut self,
        instance: SourceInstanceId,
    ) -> Result<SourcePublicationPort<V>, TaskSourceRuntimeError> {
        let (owner, input, active) = {
            let slot = self
                .sources
                .get(&instance)
                .ok_or(TaskSourceRuntimeError::UnknownSourceInstance { instance })?;
            (slot.owner, slot.spec.input, slot.active.clone())
        };
        self.ensure_owner_ready(owner)?;

        if let Some(active) = active {
            return Ok(active.source_port(self.scheduler.worker_sender()));
        }

        let next = ActiveLease::new(self.scheduler.current_stamp(input)?);
        let port = next.source_port(self.scheduler.worker_sender());
        self.sources
            .get_mut(&instance)
            .expect("validated source slot disappeared")
            .active = Some(next);
        Ok(port)
    }

    pub fn reconfigure_source(
        &mut self,
        instance: SourceInstanceId,
    ) -> Result<SourcePublicationPort<V>, TaskSourceRuntimeError> {
        let (owner, input, active) = {
            let slot = self
                .sources
                .get(&instance)
                .ok_or(TaskSourceRuntimeError::UnknownSourceInstance { instance })?;
            (slot.owner, slot.spec.input, slot.active.clone())
        };
        self.ensure_owner_ready(owner)?;

        let stamp = if let Some(active) = active {
            active.cancellation.cancel();
            self.scheduler.advance_generation(input)?
        } else {
            self.scheduler.current_stamp(input)?
        };
        let next = ActiveLease::new(stamp);
        let port = next.source_port(self.scheduler.worker_sender());
        self.sources
            .get_mut(&instance)
            .expect("validated source slot disappeared")
            .active = Some(next);
        Ok(port)
    }

    pub fn suspend_source(
        &mut self,
        instance: SourceInstanceId,
    ) -> Result<(), TaskSourceRuntimeError> {
        let (input, active) = {
            let slot = self
                .sources
                .get(&instance)
                .ok_or(TaskSourceRuntimeError::UnknownSourceInstance { instance })?;
            (slot.spec.input, slot.active.clone())
        };

        if let Some(active) = active {
            active.cancellation.cancel();
            self.scheduler.advance_generation(input)?;
            self.sources
                .get_mut(&instance)
                .expect("validated source slot disappeared")
                .active = None;
        }
        Ok(())
    }

    pub fn start_task(
        &mut self,
        instance: TaskInstanceId,
    ) -> Result<TaskCompletionPort<V>, TaskSourceRuntimeError> {
        let (owner, input, active) = {
            let slot = self
                .tasks
                .get(&instance)
                .ok_or(TaskSourceRuntimeError::UnknownTaskInstance { instance })?;
            (slot.owner, slot.spec.input, slot.active.clone())
        };
        self.ensure_owner_ready(owner)?;

        let stamp = if let Some(active) = active {
            active.cancellation.cancel();
            self.scheduler.advance_generation(input)?
        } else {
            self.scheduler.current_stamp(input)?
        };
        let next = ActiveLease::new(stamp);
        let port = next.task_port(self.scheduler.worker_sender());
        self.tasks
            .get_mut(&instance)
            .expect("validated task slot disappeared")
            .active = Some(next);
        Ok(port)
    }

    pub fn cancel_task(&mut self, instance: TaskInstanceId) -> Result<(), TaskSourceRuntimeError> {
        let (input, active) = {
            let slot = self
                .tasks
                .get(&instance)
                .ok_or(TaskSourceRuntimeError::UnknownTaskInstance { instance })?;
            (slot.spec.input, slot.active.clone())
        };

        if let Some(active) = active {
            active.cancellation.cancel();
            self.scheduler.advance_generation(input)?;
            self.tasks
                .get_mut(&instance)
                .expect("validated task slot disappeared")
                .active = None;
        }
        Ok(())
    }

    pub fn dispose_owner(&mut self, owner: OwnerHandle) -> Result<(), TaskSourceRuntimeError> {
        self.scheduler.is_owner_active(owner)?;
        let subtree = self.collect_owner_subtree(owner);
        self.pending_owner_disposals.extend(subtree.iter().copied());
        self.cancel_sources_in_subtree(&subtree);
        self.cancel_tasks_in_subtree(&subtree);
        self.scheduler.queue_dispose_owner(owner)?;
        Ok(())
    }

    pub fn tick<E>(&mut self, evaluator: &mut E) -> TickOutcome
    where
        E: DerivedNodeEvaluator<V>,
    {
        let outcome = self.scheduler.tick(evaluator);
        self.pending_owner_disposals.clear();
        outcome
    }

    pub fn try_tick<E>(&mut self, evaluator: &mut E) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        let outcome = self.scheduler.try_tick(evaluator)?;
        self.pending_owner_disposals.clear();
        Ok(outcome)
    }

    pub(crate) fn try_tick_with_reactive_program<E>(
        &mut self,
        program: &ReactiveProgram,
        evaluator: &mut E,
    ) -> Result<TickOutcome, E::Error>
    where
        E: TryDerivedNodeEvaluator<V>,
    {
        let outcome = self
            .scheduler
            .try_tick_with_reactive_program(program, evaluator)?;
        self.pending_owner_disposals.clear();
        Ok(outcome)
    }

    /// Runs one transactional scheduler tick and derives deterministic source lifecycle actions
    /// from the tick's latest stable signal values.
    ///
    /// Lifecycle decisions are applied at most once per source per tick with the following
    /// precedence:
    /// 1. `activeWhen = False` suspends an active source,
    /// 2. an inactive source whose gate now allows activation emits one activation action,
    /// 3. otherwise, active sources whose reactive config or trigger signals committed this tick
    ///    emit one reconfiguration action.
    ///
    /// This keeps source activation/reconfiguration glitch-free: the runtime observes only the
    /// fully committed scheduler state for the tick, never mixed intermediate values.
    pub fn tick_with_source_lifecycle<E, A>(
        &mut self,
        evaluator: &mut E,
        active_when: &mut A,
    ) -> Result<TaskSourceTickOutcome<V>, TaskSourceRuntimeError>
    where
        E: DerivedNodeEvaluator<V>,
        A: SourceActiveWhenEvaluator<V, D>,
    {
        let scheduler = self.scheduler.tick(evaluator);
        let source_actions = self.collect_source_lifecycle_actions(&scheduler, active_when);
        self.pending_owner_disposals.clear();
        source_actions.map(|source_actions| TaskSourceTickOutcome {
            scheduler,
            source_actions,
        })
    }

    pub fn queue_publication(
        &mut self,
        publication: Publication<V>,
    ) -> Result<(), TaskSourceRuntimeError> {
        self.scheduler
            .queue_publication(publication)
            .map_err(Into::into)
    }

    fn claim_input(&mut self, input: InputHandle) -> Result<(), TaskSourceRuntimeError> {
        if self
            .claimed_inputs
            .insert(input.as_raw(), ManagedInputKind::TaskOrSource)
            .is_some()
        {
            return Err(TaskSourceRuntimeError::DuplicateManagedInput {
                input: input.as_raw(),
            });
        }
        Ok(())
    }

    fn validate_source_provider_contract(
        &self,
        spec: &SourceRuntimeSpec<D>,
    ) -> Result<(), TaskSourceRuntimeError> {
        let Some(provider) = spec.provider.builtin_provider() else {
            return Ok(());
        };
        let contract = provider.contract();

        if spec.active_when.is_some() && contract.option("activeWhen").is_none() {
            return Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: spec.instance,
                provider: spec.provider.clone(),
                violation: SourceProviderRuntimeContractViolation::ActiveWhenNotSupported,
            });
        }

        let supported_triggers = builtin_explicit_trigger_slots(provider);
        if spec.explicit_triggers.len() > supported_triggers {
            return Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: spec.instance,
                provider: spec.provider.clone(),
                violation: SourceProviderRuntimeContractViolation::TooManyExplicitTriggers {
                    supported: supported_triggers,
                    actual: spec.explicit_triggers.len(),
                },
            });
        }

        let expected_cancellation = contract.lifecycle().cancellation();
        if spec.cancellation != expected_cancellation {
            return Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: spec.instance,
                provider: spec.provider.clone(),
                violation: SourceProviderRuntimeContractViolation::CancellationPolicyMismatch {
                    expected: expected_cancellation,
                    actual: spec.cancellation,
                },
            });
        }

        Ok(())
    }

    fn validate_signal_handle(&self, signal: SignalHandle) -> Result<(), TaskSourceRuntimeError> {
        if self.scheduler.graph().signal(signal).is_some() {
            Ok(())
        } else {
            Err(TaskSourceRuntimeError::UnknownSignalHandle {
                signal: signal.as_raw(),
            })
        }
    }

    fn validate_input_handle(&self, input: InputHandle) -> Result<(), TaskSourceRuntimeError> {
        match self.scheduler.graph().signal(input.as_signal()) {
            Some(spec) if spec.is_input() => Ok(()),
            Some(_) => Err(TaskSourceRuntimeError::SignalIsNotInput {
                signal: input.as_raw(),
            }),
            None => Err(TaskSourceRuntimeError::UnknownSignalHandle {
                signal: input.as_raw(),
            }),
        }
    }

    fn input_owner(&self, input: InputHandle) -> Option<OwnerHandle> {
        self.scheduler
            .graph()
            .signal(input.as_signal())
            .and_then(|spec| spec.owner())
    }

    fn ensure_owner_ready(&self, owner: Option<OwnerHandle>) -> Result<(), TaskSourceRuntimeError> {
        let Some(owner) = owner else {
            return Ok(());
        };

        if self.owner_pending_disposal(owner) {
            return Err(TaskSourceRuntimeError::OwnerPendingDisposal { owner });
        }

        if !self.scheduler.is_owner_active(owner)? {
            return Err(TaskSourceRuntimeError::Scheduler(
                SchedulerAccessError::OwnerInactive { owner },
            ));
        }
        Ok(())
    }

    fn owner_pending_disposal(&self, owner: OwnerHandle) -> bool {
        let mut current = Some(owner);
        while let Some(owner) = current {
            if self.pending_owner_disposals.contains(&owner) {
                return true;
            }
            current = self
                .scheduler
                .graph()
                .owner(owner)
                .and_then(|spec| spec.parent());
        }
        false
    }

    fn collect_owner_subtree(&self, owner: OwnerHandle) -> BTreeSet<OwnerHandle> {
        let mut subtree = BTreeSet::new();
        let mut worklist = VecDeque::from([owner]);

        while let Some(owner) = worklist.pop_front() {
            if !subtree.insert(owner) {
                continue;
            }
            if let Some(spec) = self.scheduler.graph().owner(owner) {
                for &child in spec.children() {
                    worklist.push_back(child);
                }
            }
        }

        subtree
    }

    fn collect_source_lifecycle_actions<A>(
        &mut self,
        outcome: &TickOutcome,
        active_when: &mut A,
    ) -> Result<Box<[SourceLifecycleAction<V>]>, TaskSourceRuntimeError>
    where
        A: SourceActiveWhenEvaluator<V, D>,
    {
        let mut committed = vec![false; self.scheduler.graph().signal_count()];
        for &signal in outcome.committed() {
            committed[signal.index()] = true;
        }

        let mut planned = Vec::new();
        for (&instance, slot) in &self.sources {
            if !self.owner_available_for_lifecycle(slot.owner)? {
                continue;
            }

            let should_be_active =
                self.source_should_be_active(instance, &slot.spec, active_when)?;
            if !should_be_active {
                if slot.active.is_some() {
                    planned.push((instance, PlannedSourceLifecycleAction::Suspend));
                }
                continue;
            }

            if slot.active.is_none() {
                planned.push((instance, PlannedSourceLifecycleAction::Activate));
                continue;
            }

            let dependency_changed = slot
                .spec
                .reconfiguration_dependencies
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            let trigger_changed = slot
                .spec
                .explicit_triggers
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            if dependency_changed || trigger_changed {
                planned.push((instance, PlannedSourceLifecycleAction::Reconfigure));
            }
        }

        let mut actions = Vec::with_capacity(planned.len());
        for (instance, action) in planned {
            match action {
                PlannedSourceLifecycleAction::Activate => {
                    let port = self.activate_source(instance)?;
                    actions.push(SourceLifecycleAction::Activate { instance, port });
                }
                PlannedSourceLifecycleAction::Reconfigure => {
                    let port = self.reconfigure_source(instance)?;
                    actions.push(SourceLifecycleAction::Reconfigure { instance, port });
                }
                PlannedSourceLifecycleAction::Suspend => {
                    self.suspend_source(instance)?;
                    actions.push(SourceLifecycleAction::Suspend { instance });
                }
            }
        }

        Ok(actions.into_boxed_slice())
    }

    fn source_should_be_active<A>(
        &self,
        instance: SourceInstanceId,
        spec: &SourceRuntimeSpec<D>,
        active_when: &mut A,
    ) -> Result<bool, TaskSourceRuntimeError>
    where
        A: SourceActiveWhenEvaluator<V, D>,
    {
        let Some(signal) = spec.active_when else {
            return Ok(true);
        };
        let value = self.scheduler.current_value(signal)?;
        Ok(active_when.is_active_when(instance, spec, value))
    }

    fn owner_available_for_lifecycle(
        &self,
        owner: Option<OwnerHandle>,
    ) -> Result<bool, TaskSourceRuntimeError> {
        let Some(owner) = owner else {
            return Ok(true);
        };
        if self.owner_pending_disposal(owner) {
            return Ok(false);
        }
        self.scheduler.is_owner_active(owner).map_err(Into::into)
    }

    fn cancel_sources_in_subtree(&mut self, subtree: &BTreeSet<OwnerHandle>) {
        for slot in self.sources.values_mut() {
            if slot.owner.is_some_and(|owner| subtree.contains(&owner))
                && let Some(active) = slot.active.take()
            {
                active.cancellation.cancel();
            }
        }
    }

    fn cancel_tasks_in_subtree(&mut self, subtree: &BTreeSet<OwnerHandle>) {
        for slot in self.tasks.values_mut() {
            if slot.owner.is_some_and(|owner| subtree.contains(&owner))
                && let Some(active) = slot.active.take()
            {
                active.cancellation.cancel();
            }
        }
    }
}

struct SourceSlot<D> {
    spec: SourceRuntimeSpec<D>,
    owner: Option<OwnerHandle>,
    active: Option<ActiveLease>,
}

struct TaskSlot {
    spec: TaskRuntimeSpec,
    owner: Option<OwnerHandle>,
    active: Option<ActiveLease>,
}

#[derive(Clone)]
struct ActiveLease {
    stamp: PublicationStamp,
    cancellation: CancellationHandle,
}

impl ActiveLease {
    fn new(stamp: PublicationStamp) -> Self {
        Self {
            stamp,
            cancellation: CancellationHandle::new(),
        }
    }

    fn source_port<V>(&self, sender: WorkerPublicationSender<V>) -> SourcePublicationPort<V> {
        SourcePublicationPort {
            sender,
            stamp: self.stamp,
            cancellation: self.cancellation.observer(),
        }
    }

    fn task_port<V>(&self, sender: WorkerPublicationSender<V>) -> TaskCompletionPort<V> {
        TaskCompletionPort {
            sender,
            stamp: self.stamp,
            cancellation: self.cancellation.observer(),
        }
    }
}

enum ManagedInputKind {
    TaskOrSource,
}

#[derive(Clone, Copy)]
enum PlannedSourceLifecycleAction {
    Activate,
    Reconfigure,
    Suspend,
}

fn builtin_explicit_trigger_slots(provider: BuiltinSourceProvider) -> usize {
    let contract = provider.contract();
    contract
        .options()
        .iter()
        .filter(|option| {
            matches!(
                contract
                    .wakeup_option(option.name())
                    .map(|wakeup| wakeup.cause()),
                Some(SourceOptionWakeupCause::TriggerSignal)
            )
        })
        .count()
}

#[cfg(test)]
mod tests {
    use aivi_typing::{NonSourceWakeupCause, RecurrenceWakeupEvidence};

    use crate::{
        effects::{
            PublicationPortError, RuntimeSourceProvider, SourceInstanceId,
            SourceLifecycleActionKind, SourceProviderRuntimeContractViolation,
            SourceReplacementPolicy, SourceRuntimeSpec, SourceStaleWorkPolicy, TaskInstanceId,
            TaskRuntimeSpec, TaskSourceRuntime, TaskSourceRuntimeError,
        },
        graph::SignalGraphBuilder,
        scheduler::{DependencyValues, Publication, PublicationDropReason, SchedulerAccessError},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestValue {
        Bool(bool),
        Int(i32),
    }

    #[test]
    fn source_runtime_spec_defaults_builtin_cancellation_from_provider_contract() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("users", None).unwrap();

        let http = SourceRuntimeSpec::<()>::new(
            SourceInstanceId::from_raw(1),
            input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
        );
        let timer = SourceRuntimeSpec::<()>::new(
            SourceInstanceId::from_raw(2),
            input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::TimerEvery),
        );
        let custom = SourceRuntimeSpec::<()>::new(
            SourceInstanceId::from_raw(3),
            input,
            RuntimeSourceProvider::custom("custom.feed"),
        );

        assert_eq!(
            http.cancellation,
            aivi_typing::SourceCancellationPolicy::CancelInFlight
        );
        assert_eq!(
            timer.cancellation,
            aivi_typing::SourceCancellationPolicy::ProviderManaged
        );
        assert_eq!(
            custom.cancellation,
            aivi_typing::SourceCancellationPolicy::ProviderManaged
        );
    }

    #[test]
    fn register_source_rejects_active_when_for_unsupported_builtin_provider() {
        let mut builder = SignalGraphBuilder::new();
        let source_input = builder.add_input("process-events", None).unwrap();
        let enabled = builder.add_input("enabled", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(4),
            source_input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::ProcessSpawn),
        );
        source.active_when = Some(enabled.as_signal());

        assert_eq!(
            runtime.register_source(source),
            Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: SourceInstanceId::from_raw(4),
                provider: RuntimeSourceProvider::builtin(
                    aivi_typing::BuiltinSourceProvider::ProcessSpawn
                ),
                violation: SourceProviderRuntimeContractViolation::ActiveWhenNotSupported,
            })
        );
    }

    #[test]
    fn register_source_rejects_more_explicit_triggers_than_provider_contract_allows() {
        let mut builder = SignalGraphBuilder::new();
        let source_input = builder.add_input("process-events", None).unwrap();
        let restart = builder.add_input("restart", None).unwrap();
        let extra = builder.add_input("extra", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(5),
            source_input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::ProcessSpawn),
        );
        source.explicit_triggers = vec![restart.as_signal(), extra.as_signal()].into_boxed_slice();

        assert_eq!(
            runtime.register_source(source),
            Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: SourceInstanceId::from_raw(5),
                provider: RuntimeSourceProvider::builtin(
                    aivi_typing::BuiltinSourceProvider::ProcessSpawn
                ),
                violation: SourceProviderRuntimeContractViolation::TooManyExplicitTriggers {
                    supported: 1,
                    actual: 2,
                },
            })
        );
    }

    #[test]
    fn register_source_rejects_builtin_cancellation_policy_mismatches() {
        let mut builder = SignalGraphBuilder::new();
        let source_input = builder.add_input("users", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(6),
            source_input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
        );
        source.cancellation = aivi_typing::SourceCancellationPolicy::ProviderManaged;

        assert_eq!(
            runtime.register_source(source),
            Err(TaskSourceRuntimeError::SourceProviderContractViolation {
                instance: SourceInstanceId::from_raw(6),
                provider: RuntimeSourceProvider::builtin(
                    aivi_typing::BuiltinSourceProvider::HttpGet
                ),
                violation: SourceProviderRuntimeContractViolation::CancellationPolicyMismatch {
                    expected: aivi_typing::SourceCancellationPolicy::CancelInFlight,
                    actual: aivi_typing::SourceCancellationPolicy::ProviderManaged,
                },
            })
        );
    }

    #[test]
    fn source_reconfiguration_cancels_old_resource_and_drops_stale_publications() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("users", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32, &'static str> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(7),
            input,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
        );
        source.cancellation = aivi_typing::SourceCancellationPolicy::CancelInFlight;
        source.replacement = SourceReplacementPolicy::DisposeSupersededBeforePublish;
        source.stale_work = SourceStaleWorkPolicy::DropStalePublications;
        source.decode = Some("users.decode");
        runtime.register_source(source).unwrap();

        assert_eq!(
            runtime
                .source_spec(SourceInstanceId::from_raw(7))
                .unwrap()
                .decode,
            Some("users.decode")
        );

        let first = runtime
            .activate_source(SourceInstanceId::from_raw(7))
            .unwrap();
        assert!(!first.is_cancelled());
        first.publish(1).unwrap();
        runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);
        assert_eq!(
            runtime.current_value(input.as_signal()).unwrap().copied(),
            Some(1)
        );

        let stale_stamp = first.stamp();
        let second = runtime
            .reconfigure_source(SourceInstanceId::from_raw(7))
            .unwrap();
        assert!(first.is_cancelled());
        assert_eq!(
            first.publish(9),
            Err(PublicationPortError::Cancelled {
                stamp: stale_stamp,
                value: 9,
            })
        );

        runtime
            .worker_sender()
            .publish(Publication::new(stale_stamp, 99))
            .unwrap();
        second.publish(2).unwrap();
        let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);

        assert_eq!(outcome.dropped_publications().len(), 1);
        assert_eq!(
            outcome.dropped_publications()[0].reason(),
            PublicationDropReason::StaleGeneration {
                active: second.stamp().generation(),
            }
        );
        assert_eq!(
            runtime.current_value(input.as_signal()).unwrap().copied(),
            Some(2)
        );
    }

    #[test]
    fn task_cancellation_suppresses_old_completions_and_retains_wakeup_handoff() {
        let mut builder = SignalGraphBuilder::new();
        let input = builder.add_input("task-result", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);

        let wakeup =
            aivi_typing::RecurrenceWakeupPlan::from_evidence(RecurrenceWakeupEvidence::NonSource {
                cause: NonSourceWakeupCause::ExplicitTimer,
            });
        let mut task = TaskRuntimeSpec::new(TaskInstanceId::from_raw(3), input);
        task.wakeup = Some(wakeup);
        runtime.register_task(task).unwrap();

        assert_eq!(
            runtime
                .task_spec(TaskInstanceId::from_raw(3))
                .unwrap()
                .wakeup,
            Some(wakeup)
        );

        let first = runtime.start_task(TaskInstanceId::from_raw(3)).unwrap();
        let stale_stamp = first.stamp();
        runtime.cancel_task(TaskInstanceId::from_raw(3)).unwrap();
        assert!(first.is_cancelled());
        assert_eq!(
            first.complete(5),
            Err(PublicationPortError::Cancelled {
                stamp: stale_stamp,
                value: 5,
            })
        );

        runtime
            .worker_sender()
            .publish(Publication::new(stale_stamp, 13))
            .unwrap();
        let second = runtime.start_task(TaskInstanceId::from_raw(3)).unwrap();
        let second_stamp = second.stamp();
        second.complete(21).unwrap();
        let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);

        assert_eq!(outcome.dropped_publications().len(), 1);
        assert_eq!(
            outcome.dropped_publications()[0].reason(),
            PublicationDropReason::StaleGeneration {
                active: second_stamp.generation(),
            }
        );
        assert_eq!(
            runtime.current_value(input.as_signal()).unwrap().copied(),
            Some(21)
        );
    }

    #[test]
    fn owner_disposal_cancels_registered_work_and_blocks_new_runs_until_tick() {
        let mut builder = SignalGraphBuilder::new();
        let owner = builder.add_owner("view", None).unwrap();
        let source_input = builder.add_input("source", Some(owner)).unwrap();
        let task_input = builder.add_input("task", Some(owner)).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);

        runtime
            .register_source(SourceRuntimeSpec::new(
                SourceInstanceId::from_raw(1),
                source_input,
                RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::TimerEvery),
            ))
            .unwrap();
        runtime
            .register_task(TaskRuntimeSpec::new(
                TaskInstanceId::from_raw(2),
                task_input,
            ))
            .unwrap();

        let source = runtime
            .activate_source(SourceInstanceId::from_raw(1))
            .unwrap();
        let task = runtime.start_task(TaskInstanceId::from_raw(2)).unwrap();

        runtime.dispose_owner(owner).unwrap();
        assert!(source.is_cancelled());
        assert!(task.is_cancelled());
        assert!(matches!(
            runtime.start_task(TaskInstanceId::from_raw(2)),
            Err(TaskSourceRuntimeError::OwnerPendingDisposal { owner: actual_owner })
                if actual_owner == owner
        ));
        assert!(matches!(
            runtime.activate_source(SourceInstanceId::from_raw(1)),
            Err(TaskSourceRuntimeError::OwnerPendingDisposal { owner: actual_owner })
                if actual_owner == owner
        ));

        runtime
            .worker_sender()
            .publish(Publication::new(source.stamp(), 1))
            .unwrap();
        runtime
            .worker_sender()
            .publish(Publication::new(task.stamp(), 2))
            .unwrap();
        let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);

        assert!(!runtime.is_owner_active(owner).unwrap());
        assert_eq!(outcome.dropped_publications().len(), 2);
        assert!(
            outcome
                .dropped_publications()
                .iter()
                .all(|publication| publication.reason()
                    == PublicationDropReason::OwnerInactive { owner })
        );
        assert!(matches!(
            runtime.start_task(TaskInstanceId::from_raw(2)),
            Err(TaskSourceRuntimeError::Scheduler(
                SchedulerAccessError::OwnerInactive { owner: actual_owner }
            )) if actual_owner == owner
        ));
    }

    #[test]
    fn source_reconfiguration_bursts_and_parent_teardown_drop_every_stale_publication() {
        let mut builder = SignalGraphBuilder::new();
        let session = builder.add_owner("session", None).unwrap();
        let widget = builder.add_owner("widget", Some(session)).unwrap();
        let input = builder.add_input("users", Some(widget)).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);
        let instance = SourceInstanceId::from_raw(21);
        runtime
            .register_source(SourceRuntimeSpec::new(
                instance,
                input,
                RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
            ))
            .unwrap();

        let mut active = runtime.activate_source(instance).unwrap();
        let mut stale_stamps = Vec::new();
        for round in 0..24_i32 {
            stale_stamps.push(active.stamp());
            let next = runtime.reconfigure_source(instance).unwrap();
            assert!(active.is_cancelled());
            assert_eq!(
                active.publish(-1),
                Err(PublicationPortError::Cancelled {
                    stamp: stale_stamps.last().copied().unwrap(),
                    value: -1,
                })
            );

            for (index, stamp) in stale_stamps.iter().copied().enumerate() {
                runtime
                    .worker_sender()
                    .publish(Publication::new(
                        stamp,
                        -10_000 - round * 100 - index as i32,
                    ))
                    .unwrap();
            }
            let fresh_value = round * 11 + 7;
            next.publish(fresh_value).unwrap();

            let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);
            let drops = outcome
                .dropped_publications()
                .iter()
                .filter(|publication| publication.stamp().input() == input)
                .collect::<Vec<_>>();
            assert_eq!(drops.len(), stale_stamps.len());
            assert!(drops.iter().all(|publication| publication.reason()
                == PublicationDropReason::StaleGeneration {
                    active: next.stamp().generation(),
                }));
            assert_eq!(
                runtime.current_value(input.as_signal()).unwrap().copied(),
                Some(fresh_value)
            );
            active = next;
        }

        runtime.dispose_owner(session).unwrap();
        assert!(active.is_cancelled());
        assert!(matches!(
            runtime.activate_source(instance),
            Err(TaskSourceRuntimeError::OwnerPendingDisposal { owner }) if owner == widget
        ));
        for stamp in stale_stamps
            .iter()
            .copied()
            .chain(std::iter::once(active.stamp()))
        {
            runtime
                .worker_sender()
                .publish(Publication::new(stamp, 99))
                .unwrap();
        }

        let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);
        assert!(!runtime.is_owner_active(session).unwrap());
        assert!(!runtime.is_owner_active(widget).unwrap());
        assert_eq!(runtime.current_value(input.as_signal()).unwrap(), None);
        assert_eq!(outcome.dropped_publications().len(), stale_stamps.len() + 1);
        assert!(
            outcome
                .dropped_publications()
                .iter()
                .all(|publication| publication.reason()
                    == PublicationDropReason::OwnerInactive { owner: widget })
        );
        assert!(matches!(
            runtime.activate_source(instance),
            Err(TaskSourceRuntimeError::Scheduler(
                SchedulerAccessError::OwnerInactive { owner }
            )) if owner == widget
        ));
    }

    #[test]
    fn task_cancellation_bursts_and_parent_teardown_drop_every_stale_completion() {
        let mut builder = SignalGraphBuilder::new();
        let session = builder.add_owner("session", None).unwrap();
        let widget = builder.add_owner("widget", Some(session)).unwrap();
        let input = builder.add_input("task-result", Some(widget)).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<i32> = TaskSourceRuntime::new(graph);
        let instance = TaskInstanceId::from_raw(22);
        runtime
            .register_task(TaskRuntimeSpec::new(instance, input))
            .unwrap();

        let mut latest_fresh = None;
        let mut stale_stamps = Vec::new();
        for round in 0..24_i32 {
            if let Some(previous) = latest_fresh.take() {
                stale_stamps.push(previous);
            }

            let cancelled = runtime.start_task(instance).unwrap();
            let cancelled_stamp = cancelled.stamp();
            runtime.cancel_task(instance).unwrap();
            assert!(cancelled.is_cancelled());
            assert_eq!(
                cancelled.complete(-1),
                Err(PublicationPortError::Cancelled {
                    stamp: cancelled_stamp,
                    value: -1,
                })
            );
            stale_stamps.push(cancelled_stamp);

            let fresh = runtime.start_task(instance).unwrap();
            let fresh_stamp = fresh.stamp();
            for (index, stamp) in stale_stamps.iter().copied().enumerate() {
                runtime
                    .worker_sender()
                    .publish(Publication::new(
                        stamp,
                        -20_000 - round * 100 - index as i32,
                    ))
                    .unwrap();
            }
            let fresh_value = round * 13 + 5;
            fresh.complete(fresh_value).unwrap();

            let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);
            let drops = outcome
                .dropped_publications()
                .iter()
                .filter(|publication| publication.stamp().input() == input)
                .collect::<Vec<_>>();
            assert_eq!(drops.len(), stale_stamps.len());
            assert!(drops.iter().all(|publication| publication.reason()
                == PublicationDropReason::StaleGeneration {
                    active: fresh_stamp.generation(),
                }));
            assert_eq!(
                runtime.current_value(input.as_signal()).unwrap().copied(),
                Some(fresh_value)
            );
            latest_fresh = Some(fresh_stamp);
        }

        runtime.dispose_owner(session).unwrap();
        assert!(matches!(
            runtime.start_task(instance),
            Err(TaskSourceRuntimeError::OwnerPendingDisposal { owner }) if owner == widget
        ));
        for stamp in stale_stamps.iter().copied().chain(std::iter::once(
            latest_fresh.expect("loop should record one live task generation"),
        )) {
            runtime
                .worker_sender()
                .publish(Publication::new(stamp, 77))
                .unwrap();
        }

        let outcome = runtime.tick(&mut |_, _: DependencyValues<'_, i32>| None);
        assert!(!runtime.is_owner_active(session).unwrap());
        assert!(!runtime.is_owner_active(widget).unwrap());
        assert_eq!(runtime.current_value(input.as_signal()).unwrap(), None);
        assert_eq!(outcome.dropped_publications().len(), stale_stamps.len() + 1);
        assert!(
            outcome
                .dropped_publications()
                .iter()
                .all(|publication| publication.reason()
                    == PublicationDropReason::OwnerInactive { owner: widget })
        );
        assert!(matches!(
            runtime.start_task(instance),
            Err(TaskSourceRuntimeError::Scheduler(
                SchedulerAccessError::OwnerInactive { owner }
            )) if owner == widget
        ));
    }

    #[test]
    fn source_lifecycle_tick_activates_and_reconfigures_from_committed_dependencies() {
        let mut builder = SignalGraphBuilder::new();
        let config = builder.add_input("config", None).unwrap();
        let refresh = builder.add_input("refresh", None).unwrap();
        let users = builder.add_input("users", None).unwrap();
        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<TestValue> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(7),
            users,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
        );
        source.reconfiguration_dependencies = vec![config.as_signal()].into_boxed_slice();
        source.explicit_triggers = vec![refresh.as_signal()].into_boxed_slice();
        runtime.register_source(source).unwrap();

        let initial = runtime
            .tick_with_source_lifecycle(
                &mut |_, _: DependencyValues<'_, TestValue>| None,
                &mut |_, _: &SourceRuntimeSpec<()>, _: Option<&TestValue>| false,
            )
            .unwrap();
        assert!(initial.scheduler().is_empty());
        assert_eq!(initial.source_actions().len(), 1);
        assert_eq!(
            initial.source_actions()[0].kind(),
            SourceLifecycleActionKind::Activate
        );
        let first = initial.source_actions()[0]
            .port()
            .expect("activation should expose a publication port")
            .clone();
        assert!(runtime.is_source_active(SourceInstanceId::from_raw(7)));

        let config_stamp = runtime.current_stamp(config).unwrap();
        runtime
            .queue_publication(Publication::new(config_stamp, TestValue::Int(1)))
            .unwrap();
        let reconfigured = runtime
            .tick_with_source_lifecycle(
                &mut |_, _: DependencyValues<'_, TestValue>| None,
                &mut |_, _: &SourceRuntimeSpec<()>, _: Option<&TestValue>| false,
            )
            .unwrap();
        assert_eq!(reconfigured.scheduler().committed(), &[config.as_signal()]);
        assert_eq!(reconfigured.source_actions().len(), 1);
        assert_eq!(
            reconfigured.source_actions()[0].kind(),
            SourceLifecycleActionKind::Reconfigure
        );
        let second = reconfigured.source_actions()[0]
            .port()
            .expect("reconfiguration should expose a fresh publication port")
            .clone();
        assert!(first.is_cancelled());
        assert_eq!(
            first.publish(TestValue::Int(9)),
            Err(PublicationPortError::Cancelled {
                stamp: first.stamp(),
                value: TestValue::Int(9),
            })
        );
        assert!(second.stamp().generation().as_raw() > first.stamp().generation().as_raw());

        let refresh_stamp = runtime.current_stamp(refresh).unwrap();
        runtime
            .queue_publication(Publication::new(refresh_stamp, TestValue::Int(2)))
            .unwrap();
        let triggered = runtime
            .tick_with_source_lifecycle(
                &mut |_, _: DependencyValues<'_, TestValue>| None,
                &mut |_, _: &SourceRuntimeSpec<()>, _: Option<&TestValue>| false,
            )
            .unwrap();
        assert_eq!(triggered.scheduler().committed(), &[refresh.as_signal()]);
        assert_eq!(triggered.source_actions().len(), 1);
        assert_eq!(
            triggered.source_actions()[0].kind(),
            SourceLifecycleActionKind::Reconfigure
        );
    }

    #[test]
    fn source_lifecycle_tick_uses_latest_active_when_value_and_suppresses_stale_work() {
        let mut builder = SignalGraphBuilder::new();
        let enabled_input = builder.add_input("enabled-input", None).unwrap();
        let enabled = builder.add_derived("enabled", None).unwrap();
        let users = builder.add_input("users", None).unwrap();
        builder
            .define_derived(enabled, [enabled_input.as_signal()])
            .unwrap();

        let graph = builder.build().unwrap();
        let mut runtime: TaskSourceRuntime<TestValue> = TaskSourceRuntime::new(graph);

        let mut source = SourceRuntimeSpec::new(
            SourceInstanceId::from_raw(9),
            users,
            RuntimeSourceProvider::builtin(aivi_typing::BuiltinSourceProvider::HttpGet),
        );
        source.active_when = Some(enabled.as_signal());
        runtime.register_source(source).unwrap();

        let mut evaluator = |signal, inputs: DependencyValues<'_, TestValue>| {
            if signal == enabled {
                match inputs.value(0)? {
                    TestValue::Bool(value) => Some(TestValue::Bool(*value)),
                    TestValue::Int(_) => None,
                }
            } else {
                None
            }
        };
        let mut active_when =
            |_: SourceInstanceId, _: &SourceRuntimeSpec<()>, value: Option<&TestValue>| {
                matches!(value, Some(TestValue::Bool(true)))
            };

        let initial = runtime
            .tick_with_source_lifecycle(&mut evaluator, &mut active_when)
            .unwrap();
        assert!(initial.source_actions().is_empty());
        assert!(!runtime.is_source_active(SourceInstanceId::from_raw(9)));

        let enabled_stamp = runtime.current_stamp(enabled_input).unwrap();
        runtime
            .queue_publication(Publication::new(enabled_stamp, TestValue::Bool(true)))
            .unwrap();
        let activated = runtime
            .tick_with_source_lifecycle(&mut evaluator, &mut active_when)
            .unwrap();
        assert_eq!(
            activated.scheduler().committed(),
            &[enabled_input.as_signal(), enabled.as_signal()]
        );
        assert_eq!(activated.source_actions().len(), 1);
        assert_eq!(
            activated.source_actions()[0].kind(),
            SourceLifecycleActionKind::Activate
        );
        let first = activated.source_actions()[0]
            .port()
            .expect("activation should expose a publication port")
            .clone();
        assert!(runtime.is_source_active(SourceInstanceId::from_raw(9)));

        let enabled_stamp = runtime.current_stamp(enabled_input).unwrap();
        runtime
            .queue_publication(Publication::new(enabled_stamp, TestValue::Bool(false)))
            .unwrap();
        let suspended = runtime
            .tick_with_source_lifecycle(&mut evaluator, &mut active_when)
            .unwrap();
        assert_eq!(suspended.source_actions().len(), 1);
        assert_eq!(
            suspended.source_actions()[0].kind(),
            SourceLifecycleActionKind::Suspend
        );
        assert!(first.is_cancelled());
        assert!(!runtime.is_source_active(SourceInstanceId::from_raw(9)));

        runtime
            .worker_sender()
            .publish(Publication::new(first.stamp(), TestValue::Int(13)))
            .unwrap();
        let enabled_stamp = runtime.current_stamp(enabled_input).unwrap();
        runtime
            .queue_publication(Publication::new(enabled_stamp, TestValue::Bool(true)))
            .unwrap();
        let resumed = runtime
            .tick_with_source_lifecycle(&mut evaluator, &mut active_when)
            .unwrap();
        assert_eq!(resumed.source_actions().len(), 1);
        assert_eq!(
            resumed.source_actions()[0].kind(),
            SourceLifecycleActionKind::Activate
        );
        let second = resumed.source_actions()[0]
            .port()
            .expect("resumed activation should expose a fresh publication port")
            .clone();
        assert_eq!(resumed.scheduler().dropped_publications().len(), 1);
        assert_eq!(
            resumed.scheduler().dropped_publications()[0].reason(),
            PublicationDropReason::StaleGeneration {
                active: second.stamp().generation(),
            }
        );
        assert!(second.stamp().generation().as_raw() > first.stamp().generation().as_raw());
    }
}
