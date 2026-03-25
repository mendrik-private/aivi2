use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    thread::{self, JoinHandle},
};

use glib::MainContext;

use aivi_backend::{
    DetachedRuntimeValue, EvaluationError, GateStage as BackendGateStage,
    ItemId as BackendItemId, ItemKind as BackendItemKind, KernelEvaluator, KernelId,
    MovingRuntimeValueStore, PipelineId as BackendPipelineId, Program as BackendProgram,
    RuntimeValue, SourceId as BackendSourceId, StageKind as BackendStageKind,
};
use aivi_core as core;
use aivi_hir as hir;

use crate::{
    InputHandle, PublicationPortError, RuntimeSourceProvider, SourceInstanceId,
    SourceLifecycleActionKind, SourcePublicationPort, TaskCompletionPort, TaskInstanceId,
    TaskSourceRuntime, TaskSourceRuntimeError, TickOutcome, TryDerivedNodeEvaluator,
    graph::{DerivedHandle, OwnerHandle, SignalHandle},
    hir_adapter::{HirRuntimeAssembly, HirRuntimeInstantiationError},
    scheduler::DependencyValues,
};

pub fn link_backend_runtime(
    assembly: HirRuntimeAssembly,
    core: &core::Module,
    backend: Arc<BackendProgram>,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    let runtime = assembly
        .instantiate_runtime_with_value_store::<RuntimeValue, _>(MovingRuntimeValueStore::default())
        .map_err(|error| {
            BackendRuntimeLinkErrors::new(vec![BackendRuntimeLinkError::InstantiateRuntime {
                error,
            }])
        })?;
    let mut builder = LinkBuilder::new(&assembly, core, &backend);
    let linked = builder.build()?;
    Ok(BackendLinkedRuntime {
        assembly,
        runtime,
        backend,
        signal_items_by_handle: linked.signal_items_by_handle,
        runtime_signal_by_item: linked.runtime_signal_by_item,
        derived_signals: linked.derived_signals,
        source_bindings: linked.source_bindings,
        task_bindings: linked.task_bindings,
    })
}

/// A fully linked runtime pairing a compiled backend program with the
/// scheduler/assembly required to tick signals.
///
/// Owns an `Arc<BackendProgram>` so the runtime is `'static` and can be
/// stored in `Arc`, sent across threads, or used at async `await` points
/// without a lifetime coupling to the stack that produced the program (M5).
pub struct BackendLinkedRuntime {
    assembly: HirRuntimeAssembly,
    runtime: TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram, MovingRuntimeValueStore>,
    backend: Arc<BackendProgram>,
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
}

impl BackendLinkedRuntime {
    pub fn assembly(&self) -> &HirRuntimeAssembly {
        &self.assembly
    }

    pub fn backend(&self) -> &BackendProgram {
        self.backend.as_ref()
    }

    pub fn runtime(
        &self,
    ) -> &TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram, MovingRuntimeValueStore> {
        &self.runtime
    }

    pub fn runtime_mut(
        &mut self,
    ) -> &mut TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram, MovingRuntimeValueStore>
    {
        &mut self.runtime
    }

    pub fn derived_signal(&self, signal: DerivedHandle) -> Option<&LinkedDerivedSignal> {
        self.derived_signals.get(&signal)
    }

    pub fn source_binding(&self, instance: SourceInstanceId) -> Option<&LinkedSourceBinding> {
        self.source_bindings.get(&instance)
    }

    pub fn source_by_owner(&self, owner: hir::ItemId) -> Option<&LinkedSourceBinding> {
        self.source_bindings
            .values()
            .find(|binding| binding.owner == owner)
    }

    pub fn task_binding(&self, instance: TaskInstanceId) -> Option<&LinkedTaskBinding> {
        self.task_bindings.get(&instance)
    }

    pub fn task_by_owner(&self, owner: hir::ItemId) -> Option<&LinkedTaskBinding> {
        self.task_bindings
            .values()
            .find(|binding| binding.owner == owner)
    }

    pub fn queued_message_count(&self) -> usize {
        self.runtime.queued_message_count()
    }

    pub fn current_signal_globals(
        &self,
    ) -> Result<BTreeMap<BackendItemId, DetachedRuntimeValue>, BackendRuntimeError> {
        self.committed_signal_snapshots()
    }

    pub fn spawn_task_worker(
        &mut self,
        instance: TaskInstanceId,
    ) -> Result<
        JoinHandle<Result<LinkedTaskWorkerOutcome, LinkedTaskWorkerError>>,
        BackendRuntimeError,
    > {
        let task = self.prepare_task_execution(instance)?;
        thread::Builder::new()
            .name(format!("aivi-task-{}", instance.as_raw()))
            .spawn(move || execute_task_plan(task))
            .map_err(|error| BackendRuntimeError::SpawnTaskWorker {
                instance,
                message: error.to_string().into(),
            })
    }

    pub fn spawn_task_worker_by_owner(
        &mut self,
        owner: hir::ItemId,
    ) -> Result<
        JoinHandle<Result<LinkedTaskWorkerOutcome, LinkedTaskWorkerError>>,
        BackendRuntimeError,
    > {
        let instance = self
            .task_by_owner(owner)
            .map(|binding| binding.instance)
            .ok_or(BackendRuntimeError::UnknownTaskOwner { owner })?;
        self.spawn_task_worker(instance)
    }

    pub fn tick(&mut self) -> Result<TickOutcome, BackendRuntimeError> {
        // Assert that we are running on the GLib/GTK main thread. The scheduler's internal
        // `VecDeque` queue is not protected by a lock, so ticking from a background thread would
        // race with publications from worker threads and corrupt the scheduler state.
        assert!(
            MainContext::default().is_owner(),
            "BackendLinkedRuntime::tick must be called from the GLib main thread; \
             use GlibLinkedRuntime to drive the runtime from worker or async contexts"
        );
        let committed = self.committed_signal_snapshots()?;
        let runtime_committed = materialize_detached_globals(&committed);
        let mut evaluator = LinkedDerivedEvaluator {
            backend: self.backend.as_ref(),
            derived_signals: &self.derived_signals,
            committed_signals: &runtime_committed,
        };
        self.runtime.try_tick(&mut evaluator)
    }

    pub fn tick_with_source_lifecycle(
        &mut self,
    ) -> Result<LinkedSourceTickOutcome, BackendRuntimeError> {
        let scheduler = self.tick()?;
        let mut committed = vec![false; self.runtime.graph().signal_count()];
        for &signal in scheduler.committed() {
            committed[signal.index()] = true;
        }

        let instances = self.source_bindings.keys().copied().collect::<Vec<_>>();
        let mut actions = Vec::new();
        for instance in instances {
            let binding = self
                .source_bindings
                .get(&instance)
                .expect("linked source binding should exist");
            if !self.runtime.is_owner_active(binding.owner_handle)? {
                continue;
            }

            let spec = self
                .runtime
                .source_spec(instance)
                .expect("linked runtime should preserve registered source specs");
            let should_be_active = match spec.active_when {
                Some(signal) => {
                    let value = self.runtime.current_value(signal)?;
                    active_when_value(instance, value)?
                }
                None => true,
            };
            if !should_be_active {
                if self.runtime.is_source_active(instance) {
                    self.runtime.suspend_source(instance)?;
                    actions.push(LinkedSourceLifecycleAction::Suspend { instance });
                }
                continue;
            }

            if !self.runtime.is_source_active(instance) {
                let config = self.evaluate_source_config(instance)?;
                let port = DetachedRuntimePublicationPort {
                    inner: self.runtime.activate_source(instance)?,
                };
                actions.push(LinkedSourceLifecycleAction::Activate {
                    instance,
                    port,
                    config,
                });
                continue;
            }

            let dependency_changed = spec
                .reconfiguration_dependencies
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            let trigger_changed = spec
                .explicit_triggers
                .iter()
                .copied()
                .any(|signal| committed[signal.index()]);
            if dependency_changed || trigger_changed {
                let config = self.evaluate_source_config(instance)?;
                let port = DetachedRuntimePublicationPort {
                    inner: self.runtime.reconfigure_source(instance)?,
                };
                actions.push(LinkedSourceLifecycleAction::Reconfigure {
                    instance,
                    port,
                    config,
                });
            }
        }

        Ok(LinkedSourceTickOutcome {
            scheduler,
            source_actions: actions.into_boxed_slice(),
        })
    }

    pub fn evaluate_source_config(
        &self,
        instance: SourceInstanceId,
    ) -> Result<EvaluatedSourceConfig, BackendRuntimeError> {
        let binding = self
            .source_bindings
            .get(&instance)
            .ok_or(BackendRuntimeError::UnknownSourceInstance { instance })?;
        let snapshots = self.committed_signal_snapshots()?;
        let mut evaluator = KernelEvaluator::new(self.backend.as_ref());
        let mut arguments = Vec::with_capacity(binding.arguments.len());
        for (index, argument) in binding.arguments.iter().enumerate() {
            let globals = self.required_signal_globals(
                instance,
                argument.kernel,
                &argument.required_signals,
                &snapshots,
            )?;
            let runtime_globals = materialize_detached_globals(&globals);
            let value = evaluator
                .evaluate_kernel(argument.kernel, None, &[], &runtime_globals)
                .map_err(|error| BackendRuntimeError::EvaluateSourceArgument {
                    instance,
                    index,
                    error,
                })?;
            arguments.push(DetachedRuntimeValue::from_runtime_owned(value));
        }
        let mut options = Vec::with_capacity(binding.options.len());
        for option in &binding.options {
            let globals = self.required_signal_globals(
                instance,
                option.kernel,
                &option.required_signals,
                &snapshots,
            )?;
            let runtime_globals = materialize_detached_globals(&globals);
            let value = evaluator
                .evaluate_kernel(option.kernel, None, &[], &runtime_globals)
                .map_err(|error| BackendRuntimeError::EvaluateSourceOption {
                    instance,
                    option_name: option.option_name.clone(),
                    error,
                })?;
            options.push(EvaluatedSourceOption {
                option_name: option.option_name.clone(),
                value: DetachedRuntimeValue::from_runtime_owned(value),
            });
        }

        Ok(EvaluatedSourceConfig {
            owner: binding.owner,
            instance,
            source: binding.backend_source,
            provider: self
                .runtime
                .source_spec(instance)
                .expect("linked runtime should preserve registered source specs")
                .provider
                .clone(),
            decode: self
                .runtime
                .source_spec(instance)
                .expect("linked runtime should preserve registered source specs")
                .decode
                .clone(),
            arguments: arguments.into_boxed_slice(),
            options: options.into_boxed_slice(),
        })
    }

    fn committed_signal_snapshots(
        &self,
    ) -> Result<BTreeMap<BackendItemId, DetachedRuntimeValue>, BackendRuntimeError> {
        let mut snapshots = BTreeMap::new();
        for (&signal, &item) in &self.signal_items_by_handle {
            if let Some(value) = self.runtime.current_value(signal)? {
                snapshots.insert(
                    item,
                    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Signal(Box::new(
                        value.clone(),
                    ))),
                );
            }
        }
        Ok(snapshots)
    }

    fn required_signal_globals(
        &self,
        instance: SourceInstanceId,
        kernel: KernelId,
        required: &[BackendItemId],
        snapshots: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Result<BTreeMap<BackendItemId, DetachedRuntimeValue>, BackendRuntimeError> {
        let mut globals = BTreeMap::new();
        for item in required {
            let signal = self.runtime_signal_by_item.get(item).copied().ok_or(
                BackendRuntimeError::MissingSignalItemMapping {
                    instance,
                    kernel,
                    item: *item,
                },
            )?;
            let value = snapshots.get(item).cloned().ok_or(
                BackendRuntimeError::MissingCommittedSignalSnapshot {
                    instance,
                    kernel,
                    signal,
                },
            )?;
            globals.insert(*item, value);
        }
        Ok(globals)
    }

    fn prepare_task_execution(
        &mut self,
        instance: TaskInstanceId,
    ) -> Result<PreparedTaskExecution, BackendRuntimeError> {
        let binding = self
            .task_bindings
            .get(&instance)
            .cloned()
            .ok_or(BackendRuntimeError::UnknownTaskInstance { instance })?;
        let (kernel, required_signals) = match &binding.execution {
            LinkedTaskExecutionBinding::Ready {
                kernel,
                required_signals,
            } => (*kernel, required_signals.clone()),
            LinkedTaskExecutionBinding::Blocked(blocker) => {
                return Err(BackendRuntimeError::TaskExecutionBlocked {
                    instance,
                    owner: binding.owner,
                    blocker: blocker.clone(),
                });
            }
        };
        let snapshots = self.committed_signal_snapshots()?;
        let globals =
            self.required_task_globals(instance, kernel, required_signals.as_ref(), &snapshots)?;
        let completion = DetachedRuntimeCompletionPort {
            inner: self.runtime.start_task(instance)?,
        };
        Ok(PreparedTaskExecution {
            instance,
            owner: binding.owner,
            backend_item: binding.backend_item,
            backend: self.backend.clone(),
            globals,
            completion,
        })
    }

    fn required_task_globals(
        &self,
        instance: TaskInstanceId,
        kernel: KernelId,
        required: &[BackendItemId],
        snapshots: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Result<BTreeMap<BackendItemId, DetachedRuntimeValue>, BackendRuntimeError> {
        let mut globals = BTreeMap::new();
        for item in required {
            let signal = self.runtime_signal_by_item.get(item).copied().ok_or(
                BackendRuntimeError::MissingTaskSignalItemMapping {
                    instance,
                    kernel,
                    item: *item,
                },
            )?;
            let value = snapshots.get(item).cloned().ok_or(
                BackendRuntimeError::MissingCommittedTaskSignalSnapshot {
                    instance,
                    kernel,
                    signal,
                },
            )?;
            globals.insert(*item, value);
        }
        Ok(globals)
    }
}

fn materialize_detached_globals(
    globals: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
) -> BTreeMap<BackendItemId, RuntimeValue> {
    globals
        .iter()
        .map(|(&item, value)| (item, value.to_runtime()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedDerivedSignal {
    pub item: hir::ItemId,
    pub signal: DerivedHandle,
    pub backend_item: BackendItemId,
    pub dependency_items: Box<[BackendItemId]>,
    /// Backend pipeline IDs that must be applied to the body result in order.
    pub pipeline_ids: Box<[BackendPipelineId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceBinding {
    pub owner: hir::ItemId,
    pub owner_handle: OwnerHandle,
    pub signal: SignalHandle,
    pub input: InputHandle,
    pub instance: SourceInstanceId,
    pub backend_owner: BackendItemId,
    pub backend_source: BackendSourceId,
    pub arguments: Box<[LinkedSourceArgument]>,
    pub options: Box<[LinkedSourceOption]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceArgument {
    pub kernel: KernelId,
    pub required_signals: Box<[BackendItemId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSourceOption {
    pub option_name: Box<str>,
    pub kernel: KernelId,
    pub required_signals: Box<[BackendItemId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedTaskBinding {
    pub owner: hir::ItemId,
    pub owner_handle: OwnerHandle,
    pub input: InputHandle,
    pub instance: TaskInstanceId,
    pub backend_item: BackendItemId,
    pub execution: LinkedTaskExecutionBinding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedTaskExecutionBinding {
    Ready {
        kernel: KernelId,
        required_signals: Box<[BackendItemId]>,
    },
    Blocked(LinkedTaskExecutionBlocker),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedTaskExecutionBlocker {
    MissingLoweredBody,
    UnsupportedParameters { parameter_count: usize },
}

impl fmt::Display for LinkedTaskExecutionBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLoweredBody => {
                f.write_str("the current compiler slice did not lower a backend task body")
            }
            Self::UnsupportedParameters { parameter_count } => write!(
                f,
                "task items with {parameter_count} parameter(s) are not directly schedulable yet"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedTaskWorkerOutcome {
    Published,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedTaskWorkerError {
    Evaluation {
        instance: TaskInstanceId,
        owner: hir::ItemId,
        backend_item: BackendItemId,
        error: EvaluationError,
    },
    Disconnected {
        instance: TaskInstanceId,
        owner: hir::ItemId,
        stamp: crate::PublicationStamp,
        value: DetachedRuntimeValue,
    },
}

impl fmt::Display for LinkedTaskWorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation {
                instance,
                owner,
                backend_item,
                error,
            } => write!(
                f,
                "task instance {} for owner {owner} failed while evaluating backend item item{backend_item}: {error}",
                instance.as_raw()
            ),
            Self::Disconnected {
                instance, stamp, ..
            } => write!(
                f,
                "task instance {} could not publish completion for stamp {:?} because the runtime disconnected",
                instance.as_raw(),
                stamp
            ),
        }
    }
}

impl std::error::Error for LinkedTaskWorkerError {}

#[derive(Clone)]
pub struct DetachedRuntimePublicationPort {
    inner: SourcePublicationPort<RuntimeValue>,
}

impl DetachedRuntimePublicationPort {
    pub fn stamp(&self) -> crate::PublicationStamp {
        self.inner.stamp()
    }

    pub fn cancellation(&self) -> crate::CancellationObserver {
        self.inner.cancellation()
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub fn publish(
        &self,
        value: DetachedRuntimeValue,
    ) -> Result<(), PublicationPortError<DetachedRuntimeValue>> {
        self.inner
            .publish(value.into_runtime())
            .map_err(map_detached_publication_port_error)
    }
}

pub struct DetachedRuntimeCompletionPort {
    inner: TaskCompletionPort<RuntimeValue>,
}

impl DetachedRuntimeCompletionPort {
    pub fn stamp(&self) -> crate::PublicationStamp {
        self.inner.stamp()
    }

    pub fn cancellation(&self) -> crate::CancellationObserver {
        self.inner.cancellation()
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub fn complete(
        self,
        value: DetachedRuntimeValue,
    ) -> Result<(), PublicationPortError<DetachedRuntimeValue>> {
        self.inner
            .complete(value.into_runtime())
            .map_err(map_detached_publication_port_error)
    }
}

fn map_detached_publication_port_error(
    error: PublicationPortError<RuntimeValue>,
) -> PublicationPortError<DetachedRuntimeValue> {
    match error {
        PublicationPortError::Cancelled { stamp, value } => PublicationPortError::Cancelled {
            stamp,
            value: DetachedRuntimeValue::from_runtime_owned(value),
        },
        PublicationPortError::Disconnected { stamp, value } => PublicationPortError::Disconnected {
            stamp,
            value: DetachedRuntimeValue::from_runtime_owned(value),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedSourceConfig {
    pub owner: hir::ItemId,
    pub instance: SourceInstanceId,
    pub source: BackendSourceId,
    pub provider: RuntimeSourceProvider,
    pub decode: Option<hir::SourceDecodeProgram>,
    pub arguments: Box<[DetachedRuntimeValue]>,
    pub options: Box<[EvaluatedSourceOption]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedSourceOption {
    pub option_name: Box<str>,
    pub value: DetachedRuntimeValue,
}

pub struct LinkedSourceTickOutcome {
    scheduler: TickOutcome,
    source_actions: Box<[LinkedSourceLifecycleAction]>,
}

impl LinkedSourceTickOutcome {
    pub fn scheduler(&self) -> &TickOutcome {
        &self.scheduler
    }

    pub fn source_actions(&self) -> &[LinkedSourceLifecycleAction] {
        &self.source_actions
    }
}

#[derive(Clone)]
pub enum LinkedSourceLifecycleAction {
    Activate {
        instance: SourceInstanceId,
        port: DetachedRuntimePublicationPort,
        config: EvaluatedSourceConfig,
    },
    Reconfigure {
        instance: SourceInstanceId,
        port: DetachedRuntimePublicationPort,
        config: EvaluatedSourceConfig,
    },
    Suspend {
        instance: SourceInstanceId,
    },
}

impl LinkedSourceLifecycleAction {
    pub const fn kind(&self) -> SourceLifecycleActionKind {
        match self {
            Self::Activate { .. } => SourceLifecycleActionKind::Activate,
            Self::Reconfigure { .. } => SourceLifecycleActionKind::Reconfigure,
            Self::Suspend { .. } => SourceLifecycleActionKind::Suspend,
        }
    }

    pub const fn instance(&self) -> SourceInstanceId {
        match self {
            Self::Activate { instance, .. }
            | Self::Reconfigure { instance, .. }
            | Self::Suspend { instance } => *instance,
        }
    }

    pub fn config(&self) -> Option<&EvaluatedSourceConfig> {
        match self {
            Self::Activate { config, .. } | Self::Reconfigure { config, .. } => Some(config),
            Self::Suspend { .. } => None,
        }
    }
}

struct PreparedTaskExecution {
    instance: TaskInstanceId,
    owner: hir::ItemId,
    backend_item: BackendItemId,
    backend: Arc<BackendProgram>,
    globals: BTreeMap<BackendItemId, DetachedRuntimeValue>,
    completion: DetachedRuntimeCompletionPort,
}

fn execute_task_plan(
    task: PreparedTaskExecution,
) -> Result<LinkedTaskWorkerOutcome, LinkedTaskWorkerError> {
    let PreparedTaskExecution {
        instance,
        owner,
        backend_item,
        backend,
        globals,
        completion,
    } = task;
    if completion.is_cancelled() {
        return Ok(LinkedTaskWorkerOutcome::Cancelled);
    }
    let mut evaluator = KernelEvaluator::new(backend.as_ref());
    let runtime_globals = materialize_detached_globals(&globals);
    let value = evaluator
        .evaluate_item(backend_item, &runtime_globals)
        .map_err(|error| LinkedTaskWorkerError::Evaluation {
            instance,
            owner,
            backend_item,
            error,
        })?;
    match completion.complete(DetachedRuntimeValue::from_runtime_owned(value)) {
        Ok(()) => Ok(LinkedTaskWorkerOutcome::Published),
        Err(PublicationPortError::Cancelled { .. }) => Ok(LinkedTaskWorkerOutcome::Cancelled),
        Err(PublicationPortError::Disconnected { stamp, value }) => {
            Err(LinkedTaskWorkerError::Disconnected {
                instance,
                owner,
                stamp,
                value,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendRuntimeLinkError {
    InstantiateRuntime {
        error: HirRuntimeInstantiationError,
    },
    MissingCoreItemOrigin {
        backend_item: BackendItemId,
        core_item: core::ItemId,
    },
    DuplicateBackendOrigin {
        item: hir::ItemId,
        first: BackendItemId,
        second: BackendItemId,
    },
    MissingBackendItem {
        item: hir::ItemId,
    },
    BackendItemNotSignal {
        item: hir::ItemId,
        backend_item: BackendItemId,
    },
    MissingRuntimeOwner {
        owner: hir::ItemId,
    },
    MissingBackendSource {
        owner: hir::ItemId,
        backend_item: BackendItemId,
    },
    SourceInstanceMismatch {
        owner: hir::ItemId,
        runtime: SourceInstanceId,
        backend: aivi_backend::SourceInstanceId,
    },
    SourceBackedBodySignalNotYetLinked {
        item: hir::ItemId,
    },
    SignalPipelinesNotYetLinked {
        item: hir::ItemId,
        count: usize,
    },
    MissingSignalBody {
        item: hir::ItemId,
        backend_item: BackendItemId,
    },
    UnsupportedInlinePipeKernel {
        owner: hir::ItemId,
        kernel: KernelId,
    },
    MissingItemBodyForGlobal {
        owner: hir::ItemId,
        item: BackendItemId,
    },
    MissingRuntimeSignalDependency {
        owner: hir::ItemId,
        dependency: BackendItemId,
    },
    SignalRequirementMismatch {
        item: hir::ItemId,
        declared: Box<[BackendItemId]>,
        required: Box<[BackendItemId]>,
    },
    SignalDependencyMismatch {
        item: hir::ItemId,
        runtime: Box<[SignalHandle]>,
        backend: Box<[SignalHandle]>,
    },
}

impl fmt::Display for BackendRuntimeLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstantiateRuntime { error } => {
                write!(f, "runtime instantiation failed: {error}")
            }
            Self::MissingCoreItemOrigin {
                backend_item,
                core_item,
            } => write!(
                f,
                "backend item item{backend_item} points at missing core item {core_item}"
            ),
            Self::DuplicateBackendOrigin {
                item,
                first,
                second,
            } => write!(
                f,
                "HIR item {item} lowered to multiple backend items: item{first} and item{second}"
            ),
            Self::MissingBackendItem { item } => {
                write!(f, "HIR runtime item {item} has no linked backend item")
            }
            Self::BackendItemNotSignal { item, backend_item } => write!(
                f,
                "HIR signal {item} lowered to non-signal backend item item{backend_item}"
            ),
            Self::MissingRuntimeOwner { owner } => {
                write!(
                    f,
                    "runtime assembly is missing an owner binding for item {owner}"
                )
            }
            Self::MissingBackendSource {
                owner,
                backend_item,
            } => write!(
                f,
                "runtime source owner {owner} has no linked backend source on item{backend_item}"
            ),
            Self::SourceInstanceMismatch {
                owner,
                runtime,
                backend,
            } => write!(
                f,
                "runtime source instance {} for owner {owner} does not match backend source {}",
                runtime.as_raw(),
                backend.as_raw()
            ),
            Self::SourceBackedBodySignalNotYetLinked { item } => write!(
                f,
                "source-backed body signal {item} still crosses the explicit publication-to-body gap"
            ),
            Self::SignalPipelinesNotYetLinked { item, count } => write!(
                f,
                "signal {item} still has {count} backend pipeline handoff(s) that startup does not execute yet"
            ),
            Self::MissingSignalBody { item, backend_item } => write!(
                f,
                "linked derived signal {item} has no backend body kernel on item{backend_item}"
            ),
            Self::UnsupportedInlinePipeKernel { owner, kernel } => write!(
                f,
                "owner {owner} still depends on inline-pipe kernel{kernel}, which startup cannot evaluate yet"
            ),
            Self::MissingItemBodyForGlobal { owner, item } => write!(
                f,
                "owner {owner} references non-signal global item{item} without a backend body kernel"
            ),
            Self::MissingRuntimeSignalDependency { owner, dependency } => write!(
                f,
                "owner {owner} depends on backend signal item{dependency} with no runtime signal binding"
            ),
            Self::SignalRequirementMismatch {
                item,
                declared,
                required,
            } => write!(
                f,
                "signal {item} declares backend dependencies {:?}, but its reachable body requires {:?}",
                declared, required
            ),
            Self::SignalDependencyMismatch {
                item,
                runtime,
                backend,
            } => write!(
                f,
                "signal {item} runtime dependencies {:?} do not match backend dependencies {:?}",
                runtime, backend
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendRuntimeLinkErrors {
    errors: Box<[BackendRuntimeLinkError]>,
}

impl BackendRuntimeLinkErrors {
    pub fn new(errors: Vec<BackendRuntimeLinkError>) -> Self {
        debug_assert!(!errors.is_empty());
        Self {
            errors: errors.into_boxed_slice(),
        }
    }

    pub fn errors(&self) -> &[BackendRuntimeLinkError] {
        &self.errors
    }
}

impl fmt::Display for BackendRuntimeLinkErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BackendRuntimeLinkErrors {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendRuntimeError {
    Runtime(TaskSourceRuntimeError),
    UnknownDerivedSignal {
        signal: DerivedHandle,
    },
    DerivedDependencyArityMismatch {
        signal: DerivedHandle,
        expected: usize,
        found: usize,
    },
    UnknownSourceInstance {
        instance: SourceInstanceId,
    },
    UnknownTaskInstance {
        instance: TaskInstanceId,
    },
    UnknownTaskOwner {
        owner: hir::ItemId,
    },
    MissingCommittedSignalSnapshot {
        instance: SourceInstanceId,
        kernel: KernelId,
        signal: SignalHandle,
    },
    MissingSignalItemMapping {
        instance: SourceInstanceId,
        kernel: KernelId,
        item: BackendItemId,
    },
    MissingCommittedTaskSignalSnapshot {
        instance: TaskInstanceId,
        kernel: KernelId,
        signal: SignalHandle,
    },
    MissingTaskSignalItemMapping {
        instance: TaskInstanceId,
        kernel: KernelId,
        item: BackendItemId,
    },
    TaskExecutionBlocked {
        instance: TaskInstanceId,
        owner: hir::ItemId,
        blocker: LinkedTaskExecutionBlocker,
    },
    SpawnTaskWorker {
        instance: TaskInstanceId,
        message: Box<str>,
    },
    EvaluateDerivedSignal {
        signal: DerivedHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    EvaluateSourceArgument {
        instance: SourceInstanceId,
        index: usize,
        error: EvaluationError,
    },
    EvaluateSourceOption {
        instance: SourceInstanceId,
        option_name: Box<str>,
        error: EvaluationError,
    },
    InvalidActiveWhenValue {
        instance: SourceInstanceId,
        value: RuntimeValue,
    },
}

impl From<TaskSourceRuntimeError> for BackendRuntimeError {
    fn from(value: TaskSourceRuntimeError) -> Self {
        Self::Runtime(value)
    }
}

impl fmt::Display for BackendRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => write!(f, "runtime access failed: {error:?}"),
            Self::UnknownDerivedSignal { signal } => {
                write!(
                    f,
                    "startup linker does not know derived signal {:?}",
                    signal
                )
            }
            Self::DerivedDependencyArityMismatch {
                signal,
                expected,
                found,
            } => write!(
                f,
                "derived signal {:?} expected {expected} runtime dependencies, found {found}",
                signal
            ),
            Self::UnknownSourceInstance { instance } => {
                write!(
                    f,
                    "startup linker does not know source instance {}",
                    instance.as_raw()
                )
            }
            Self::UnknownTaskInstance { instance } => {
                write!(
                    f,
                    "startup linker does not know task instance {}",
                    instance.as_raw()
                )
            }
            Self::UnknownTaskOwner { owner } => {
                write!(f, "startup linker does not know task owner {owner}")
            }
            Self::MissingCommittedSignalSnapshot {
                instance,
                kernel,
                signal,
            } => write!(
                f,
                "source instance {} requires committed snapshot for signal {:?} while evaluating kernel{kernel}",
                instance.as_raw(),
                signal
            ),
            Self::MissingSignalItemMapping {
                instance,
                kernel,
                item,
            } => write!(
                f,
                "source instance {} could not map backend item {item} to a runtime signal while evaluating kernel{kernel}",
                instance.as_raw()
            ),
            Self::MissingCommittedTaskSignalSnapshot {
                instance,
                kernel,
                signal,
            } => write!(
                f,
                "task instance {} requires committed snapshot for signal {:?} while evaluating kernel{kernel}",
                instance.as_raw(),
                signal
            ),
            Self::MissingTaskSignalItemMapping {
                instance,
                kernel,
                item,
            } => write!(
                f,
                "task instance {} could not map backend item {item} to a runtime signal while evaluating kernel{kernel}",
                instance.as_raw()
            ),
            Self::TaskExecutionBlocked {
                instance,
                owner,
                blocker,
            } => write!(
                f,
                "task instance {} for owner {owner} cannot execute yet: {blocker}",
                instance.as_raw()
            ),
            Self::SpawnTaskWorker { instance, message } => write!(
                f,
                "failed to spawn worker thread for task instance {}: {message}",
                instance.as_raw()
            ),
            Self::EvaluateDerivedSignal {
                signal,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate derived signal {:?} for item {item}: {error}",
                signal
            ),
            Self::EvaluateSourceArgument {
                instance,
                index,
                error,
            } => write!(
                f,
                "failed to evaluate source argument {index} for instance {}: {error}",
                instance.as_raw()
            ),
            Self::EvaluateSourceOption {
                instance,
                option_name,
                error,
            } => write!(
                f,
                "failed to evaluate source option {option_name} for instance {}: {error}",
                instance.as_raw()
            ),
            Self::InvalidActiveWhenValue { instance, value } => write!(
                f,
                "source instance {} produced non-Bool activeWhen value {:?}",
                instance.as_raw(),
                value
            ),
        }
    }
}

impl std::error::Error for BackendRuntimeError {}

struct LinkedDerivedEvaluator<'a> {
    backend: &'a BackendProgram,
    derived_signals: &'a BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    committed_signals: &'a BTreeMap<BackendItemId, RuntimeValue>,
}

impl TryDerivedNodeEvaluator<RuntimeValue> for LinkedDerivedEvaluator<'_> {
    type Error = BackendRuntimeError;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<Option<RuntimeValue>, Self::Error> {
        let binding = self
            .derived_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownDerivedSignal { signal })?;
        if inputs.len() != binding.dependency_items.len() {
            return Err(BackendRuntimeError::DerivedDependencyArityMismatch {
                signal,
                expected: binding.dependency_items.len(),
                found: inputs.len(),
            });
        }

        let mut globals = self.committed_signals.clone();
        for (index, dependency) in binding.dependency_items.iter().copied().enumerate() {
            let Some(value) = inputs.value(index) else {
                return Ok(None);
            };
            globals.insert(dependency, RuntimeValue::Signal(Box::new(value.clone())));
        }

        let mut evaluator = KernelEvaluator::new(self.backend);
        let value = evaluator
            .evaluate_item(binding.backend_item, &globals)
            .map_err(|error| BackendRuntimeError::EvaluateDerivedSignal {
                signal,
                item: binding.item,
                error,
            })?;

        // Apply pipeline stages in order.
        for &pipeline_id in binding.pipeline_ids.iter() {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate,
                        emits_negative_update,
                        ..
                    }) => {
                        let pred = evaluator
                            .evaluate_kernel(*predicate, Some(&value), &[], &globals)
                            .map_err(|error| BackendRuntimeError::EvaluateDerivedSignal {
                                signal,
                                item: binding.item,
                                error,
                            })?;
                        // Suppress the update when the predicate is false and no
                        // negative update is expected by downstream consumers.
                        if matches!(pred, RuntimeValue::Bool(false)) && !emits_negative_update {
                            return Ok(None);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {
                        // Carrier metadata only; the body kernel already computed
                        // the branch result, so we pass the value through unchanged.
                    }
                    _ => unreachable!(
                        "unsupported pipeline stage kind should have been blocked during linking"
                    ),
                }
            }
        }

        Ok(Some(value))
    }
}

struct LinkArtifacts {
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
}

struct LinkBuilder<'a> {
    assembly: &'a HirRuntimeAssembly,
    core: &'a core::Module,
    backend: &'a BackendProgram,
    errors: Vec<BackendRuntimeLinkError>,
    core_to_hir: BTreeMap<core::ItemId, hir::ItemId>,
    hir_to_backend: BTreeMap<hir::ItemId, BackendItemId>,
    backend_to_hir: BTreeMap<BackendItemId, hir::ItemId>,
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
}

impl<'a> LinkBuilder<'a> {
    fn new(
        assembly: &'a HirRuntimeAssembly,
        core: &'a core::Module,
        backend: &'a BackendProgram,
    ) -> Self {
        Self {
            assembly,
            core,
            backend,
            errors: Vec::new(),
            core_to_hir: BTreeMap::new(),
            hir_to_backend: BTreeMap::new(),
            backend_to_hir: BTreeMap::new(),
            signal_items_by_handle: BTreeMap::new(),
            runtime_signal_by_item: BTreeMap::new(),
            derived_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            task_bindings: BTreeMap::new(),
        }
    }

    fn build(&mut self) -> Result<LinkArtifacts, BackendRuntimeLinkErrors> {
        self.index_origins();
        self.index_signal_items();
        self.link_sources();
        self.link_tasks();
        self.link_derived_signals();
        if self.errors.is_empty() {
            Ok(LinkArtifacts {
                signal_items_by_handle: std::mem::take(&mut self.signal_items_by_handle),
                runtime_signal_by_item: std::mem::take(&mut self.runtime_signal_by_item),
                derived_signals: std::mem::take(&mut self.derived_signals),
                source_bindings: std::mem::take(&mut self.source_bindings),
                task_bindings: std::mem::take(&mut self.task_bindings),
            })
        } else {
            Err(BackendRuntimeLinkErrors::new(std::mem::take(
                &mut self.errors,
            )))
        }
    }

    fn index_origins(&mut self) {
        for (core_id, item) in self.core.items().iter() {
            self.core_to_hir.insert(core_id, item.origin);
        }
        for (backend_item, item) in self.backend.items().iter() {
            let Some(&hir_item) = self.core_to_hir.get(&item.origin) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingCoreItemOrigin {
                        backend_item,
                        core_item: item.origin,
                    });
                continue;
            };
            if let Some(previous) = self.hir_to_backend.insert(hir_item, backend_item) {
                self.errors
                    .push(BackendRuntimeLinkError::DuplicateBackendOrigin {
                        item: hir_item,
                        first: previous,
                        second: backend_item,
                    });
            }
            self.backend_to_hir.insert(backend_item, hir_item);
        }
    }

    fn index_signal_items(&mut self) {
        for binding in self.assembly.signals() {
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            self.signal_items_by_handle
                .insert(binding.signal(), backend_item);
            self.runtime_signal_by_item
                .insert(backend_item, binding.signal());
        }
    }

    fn link_sources(&mut self) {
        for source in self.assembly.sources() {
            let Some(&backend_owner) = self.hir_to_backend.get(&source.owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let Some(owner_handle) = self
                .assembly
                .owner(source.owner)
                .map(|binding| binding.handle)
            else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingRuntimeOwner {
                        owner: source.owner,
                    });
                continue;
            };
            let Some(backend_item) = self.backend.items().get(backend_owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: source.owner });
                continue;
            };
            let BackendItemKind::Signal(info) = &backend_item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let Some(backend_source_id) = info.source else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendSource {
                        owner: source.owner,
                        backend_item: backend_owner,
                    });
                continue;
            };
            let backend_source = &self.backend.sources()[backend_source_id];
            if backend_source.instance.as_raw() != source.spec.instance.as_raw() {
                self.errors
                    .push(BackendRuntimeLinkError::SourceInstanceMismatch {
                        owner: source.owner,
                        runtime: source.spec.instance,
                        backend: backend_source.instance,
                    });
                continue;
            }

            let arguments = backend_source
                .arguments
                .iter()
                .map(|argument| LinkedSourceArgument {
                    kernel: argument.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, argument.kernel),
                })
                .collect::<Vec<_>>();
            let options = backend_source
                .options
                .iter()
                .map(|option| LinkedSourceOption {
                    option_name: option.option_name.clone(),
                    kernel: option.kernel,
                    required_signals: self
                        .collect_required_signal_items(source.owner, option.kernel),
                })
                .collect::<Vec<_>>();

            self.source_bindings.insert(
                source.spec.instance,
                LinkedSourceBinding {
                    owner: source.owner,
                    owner_handle,
                    signal: source.signal,
                    input: source.input,
                    instance: source.spec.instance,
                    backend_owner,
                    backend_source: backend_source_id,
                    arguments: arguments.into_boxed_slice(),
                    options: options.into_boxed_slice(),
                },
            );
        }
    }

    fn link_tasks(&mut self) {
        for task in self.assembly.tasks() {
            let Some(&backend_item) = self.hir_to_backend.get(&task.owner) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: task.owner });
                continue;
            };
            let Some(owner_handle) = self
                .assembly
                .owner(task.owner)
                .map(|binding| binding.handle)
            else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingRuntimeOwner { owner: task.owner });
                continue;
            };
            let Some(item) = self.backend.items().get(backend_item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: task.owner });
                continue;
            };
            let execution = if !item.parameters.is_empty() {
                LinkedTaskExecutionBinding::Blocked(
                    LinkedTaskExecutionBlocker::UnsupportedParameters {
                        parameter_count: item.parameters.len(),
                    },
                )
            } else if let Some(kernel) = item.body {
                LinkedTaskExecutionBinding::Ready {
                    kernel,
                    required_signals: self.collect_required_signal_items(task.owner, kernel),
                }
            } else {
                LinkedTaskExecutionBinding::Blocked(LinkedTaskExecutionBlocker::MissingLoweredBody)
            };
            self.task_bindings.insert(
                task.spec.instance,
                LinkedTaskBinding {
                    owner: task.owner,
                    owner_handle,
                    input: task.input,
                    instance: task.spec.instance,
                    backend_item,
                    execution,
                },
            );
        }
    }

    fn link_derived_signals(&mut self) {
        for binding in self.assembly.signals() {
            let Some(derived) = binding.derived() else {
                continue;
            };
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            if binding.source_input.is_some() {
                self.errors.push(
                    BackendRuntimeLinkError::SourceBackedBodySignalNotYetLinked {
                        item: binding.item,
                    },
                );
                continue;
            }

            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(info) = &item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };
            if !item.pipelines.is_empty() && !self.supported_body_backed_signal_pipelines(item) {
                self.errors
                    .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                        item: binding.item,
                        count: item.pipelines.len(),
                    });
                continue;
            }
            let Some(body) = item.body else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingSignalBody {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };

            let required = self.collect_required_signal_items(binding.item, body);
            let declared = info.dependencies.clone().into_boxed_slice();
            if !same_items(&required, &declared) {
                self.errors
                    .push(BackendRuntimeLinkError::SignalRequirementMismatch {
                        item: binding.item,
                        declared,
                        required,
                    });
                continue;
            }

            let backend_dependencies = info
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    self.runtime_signal_for_backend_item(binding.item, *dependency)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            if backend_dependencies.as_ref() != binding.dependencies() {
                self.errors
                    .push(BackendRuntimeLinkError::SignalDependencyMismatch {
                        item: binding.item,
                        runtime: binding.dependencies().to_vec().into_boxed_slice(),
                        backend: backend_dependencies,
                    });
                continue;
            }

            self.derived_signals.insert(
                derived,
                LinkedDerivedSignal {
                    item: binding.item,
                    signal: derived,
                    backend_item,
                    dependency_items: info.dependencies.clone().into_boxed_slice(),
                    pipeline_ids: item.pipelines.iter().copied().collect::<Vec<_>>().into_boxed_slice(),
                },
            );
        }
    }

    fn runtime_signal_for_backend_item(
        &mut self,
        owner: hir::ItemId,
        dependency: BackendItemId,
    ) -> Option<SignalHandle> {
        if let Some(signal) = self.runtime_signal_by_item.get(&dependency).copied() {
            return Some(signal);
        }
        self.errors
            .push(BackendRuntimeLinkError::MissingRuntimeSignalDependency { owner, dependency });
        None
    }

    fn collect_required_signal_items(
        &mut self,
        owner: hir::ItemId,
        root: KernelId,
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernels = vec![root];
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernels.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for item_id in &kernel.global_items {
                if !visited_items.insert(*item_id) {
                    continue;
                }
                let item = &self.backend.items()[*item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(*item_id);
                    }
                    _ => {
                        let Some(body) = item.body else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: *item_id,
                                });
                            continue;
                        };
                        kernels.push(body);
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }

    fn supported_body_backed_signal_pipelines(&self, item: &aivi_backend::Item) -> bool {
        item.body.is_some()
            && item.pipelines.iter().copied().all(|pipeline_id| {
                let pipeline = &self.backend.pipelines()[pipeline_id];
                // Recurrence pipelines require scheduler wakeup infrastructure
                // that is not yet wired; keep them as an explicit boundary.
                pipeline.recurrence.is_none()
                    && pipeline.stages.iter().all(|stage| {
                        matches!(
                            stage.kind,
                            // TruthyFalsy carrier metadata: body kernel already handles
                            // the branching computation.
                            BackendStageKind::TruthyFalsy(_)
                                // SignalFilter gates: predicate kernel is executable.
                                | BackendStageKind::Gate(BackendGateStage::SignalFilter { .. })
                        )
                    })
            })
    }
}

fn same_items(left: &[BackendItemId], right: &[BackendItemId]) -> bool {
    left.len() == right.len()
        && left.iter().copied().collect::<BTreeSet<_>>()
            == right.iter().copied().collect::<BTreeSet<_>>()
}

fn active_when_value(
    instance: SourceInstanceId,
    value: Option<&RuntimeValue>,
) -> Result<bool, BackendRuntimeError> {
    match value {
        None => Ok(false),
        Some(RuntimeValue::Bool(value)) => Ok(*value),
        Some(RuntimeValue::Signal(value)) => match value.as_ref() {
            RuntimeValue::Bool(value) => Ok(*value),
            other => Err(BackendRuntimeError::InvalidActiveWhenValue {
                instance,
                value: other.clone(),
            }),
        },
        Some(other) => Err(BackendRuntimeError::InvalidActiveWhenValue {
            instance,
            value: other.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module as lower_hir_module};
    use aivi_lambda::lower_module as lower_lambda_module;
    use aivi_syntax::parse_module;

    use super::*;
    use crate::{SignalGraphBuilder, TaskRuntimeSpec, TaskSourceRuntime};

    struct LoweredStack {
        hir: hir::LoweringResult,
        core: core::Module,
        backend: BackendProgram,
    }

    fn lower_text(path: &str, text: &str) -> LoweredStack {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let hir = lower_hir_module(&parsed.module);
        assert!(
            !hir.has_errors(),
            "fixture {path} should lower to HIR: {:?}",
            hir.diagnostics()
        );
        let core = core::lower_module(hir.module()).expect("typed-core lowering should succeed");
        let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
        let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
        LoweredStack { hir, core, backend }
    }

    fn item_id(module: &hir::Module, name: &str) -> hir::ItemId {
        module
            .items()
            .iter()
            .find_map(|(item_id, item)| match item {
                Item::Value(item) if item.name.text() == name => Some(item_id),
                Item::Function(item) if item.name.text() == name => Some(item_id),
                Item::Signal(item) if item.name.text() == name => Some(item_id),
                Item::Type(item) if item.name.text() == name => Some(item_id),
                Item::Class(item) if item.name.text() == name => Some(item_id),
                Item::Domain(item) if item.name.text() == name => Some(item_id),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected item named {name}"))
    }

    fn backend_item_id(program: &BackendProgram, name: &str) -> BackendItemId {
        program
            .items()
            .iter()
            .find_map(|(item_id, item)| (item.name.as_ref() == name).then_some(item_id))
            .unwrap_or_else(|| panic!("expected backend item named {name}"))
    }

    fn text_ptr(value: &RuntimeValue) -> *const u8 {
        let RuntimeValue::Text(text) = value else {
            panic!("expected text runtime value");
        };
        text.as_ptr()
    }

    fn manual_task_linked_runtime(
        lowered: &LoweredStack,
        owner_name: &str,
    ) -> BackendLinkedRuntime {
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("manual task fixture should assemble");
        let owner = item_id(lowered.hir.module(), owner_name);
        let backend_item = backend_item_id(&lowered.backend, owner_name);

        let mut graph = SignalGraphBuilder::new();
        let owner_handle = graph
            .add_owner(owner_name, None)
            .expect("task owner should allocate");
        let input = graph
            .add_input(format!("{owner_name}#task"), Some(owner_handle))
            .expect("task input should allocate");
        let graph = graph.build().expect("task graph should build");

        let mut runtime: TaskSourceRuntime<
            RuntimeValue,
            hir::SourceDecodeProgram,
            MovingRuntimeValueStore,
        > = TaskSourceRuntime::with_value_store(graph, MovingRuntimeValueStore::default());
        let instance = TaskInstanceId::from_raw(owner.as_raw());
        runtime
            .register_task(TaskRuntimeSpec::new(instance, input))
            .expect("task spec should register");

        let kernel = lowered.backend.items()[backend_item]
            .body
            .expect("manual task fixture should have a lowered backend body");
        let binding = LinkedTaskBinding {
            owner,
            owner_handle,
            input,
            instance,
            backend_item,
            execution: LinkedTaskExecutionBinding::Ready {
                kernel,
                required_signals: Vec::new().into_boxed_slice(),
            },
        };

        BackendLinkedRuntime {
            assembly,
            runtime,
            backend: Arc::new(lowered.backend.clone()),
            signal_items_by_handle: BTreeMap::new(),
            runtime_signal_by_item: BTreeMap::new(),
            derived_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            task_bindings: BTreeMap::from([(instance, binding)]),
        }
    }

    #[test]
    fn linked_runtime_ticks_simple_signals_and_evaluates_source_config() {
        let lowered = lower_text(
            "runtime-startup-basic.aivi",
            r#"
val prefix = "https://example.com/"

sig id = 7
sig next = id + 1
sig enabled = True
sig label = "Ada"

@source http.get "{prefix}{id}" with {
    activeWhen: enabled
}
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");

        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let next_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "next"))
            .expect("next signal binding should exist")
            .signal();
        let id_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "id"))
            .expect("id signal binding should exist")
            .signal();
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(id_signal).unwrap(),
            Some(&RuntimeValue::Int(7))
        );
        assert_eq!(
            linked.runtime().current_value(next_signal).unwrap(),
            Some(&RuntimeValue::Int(8))
        );
        let label_value = linked
            .runtime()
            .current_value(label_signal)
            .unwrap()
            .expect("label signal should commit");
        let globals = linked
            .current_signal_globals()
            .expect("signal globals should snapshot committed values");
        let label_item = backend_item_id(&lowered.backend, "label");
        let label_snapshot = globals
            .get(&label_item)
            .expect("signal snapshot should carry label value");
        let RuntimeValue::Text(committed_label) = label_value else {
            panic!("label signal should carry text")
        };
        let RuntimeValue::Signal(snapshot_inner) = label_snapshot.as_runtime() else {
            panic!("signal snapshot should preserve wrapped signal shape")
        };
        let RuntimeValue::Text(snapshot_label) = snapshot_inner.as_ref() else {
            panic!("signal snapshot should carry text payload")
        };
        assert_ne!(
            committed_label.as_ptr(),
            snapshot_label.as_ptr(),
            "committed signal snapshots must detach boundary storage from scheduler-owned values"
        );
        assert_eq!(outcome.source_actions().len(), 1);
        let action = &outcome.source_actions()[0];
        assert_eq!(action.kind(), SourceLifecycleActionKind::Activate);
        let config = action.config().expect("activation should carry config");
        assert_eq!(
            config.arguments.as_ref(),
            &[RuntimeValue::Text("https://example.com/7".into())]
        );
        assert_eq!(config.options.len(), 1);
        assert_eq!(config.options[0].option_name.as_ref(), "activeWhen");
        assert_eq!(
            config.options[0].value,
            RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true)))
        );
    }

    #[test]
    fn linked_runtime_relocates_committed_signal_values_between_ticks() {
        let lowered = lower_text(
            "runtime-startup-moving-gc.aivi",
            r#"
sig label = "Ada"
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();

        linked
            .tick()
            .expect("initial linked runtime tick should succeed");
        let first = linked
            .runtime()
            .current_value(label_signal)
            .unwrap()
            .expect("label signal should commit on the first tick");
        let first_ptr = text_ptr(first);

        let outcome = linked
            .tick()
            .expect("second linked runtime tick should succeed");
        assert!(
            outcome.is_empty(),
            "empty linked-runtime ticks should still serve as moving-GC safe points"
        );
        let second = linked
            .runtime()
            .current_value(label_signal)
            .unwrap()
            .expect("label signal should stay committed after relocation");
        assert_eq!(second, &RuntimeValue::Text("Ada".into()));
        assert_ne!(
            first_ptr,
            text_ptr(second),
            "linked runtime must expose relocated committed text storage on the next tick"
        );
    }

    #[test]
    fn linked_runtime_reports_missing_signal_snapshots_for_source_config() {
        let lowered = lower_text(
            "runtime-startup-missing-snapshot.aivi",
            r#"
@source http.get "/host"
sig apiHost : Signal Text

@source http.get "{apiHost}/users"
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let users = item_id(lowered.hir.module(), "users");
        let instance = linked
            .source_by_owner(users)
            .expect("users source binding should exist")
            .instance;
        let error = linked
            .evaluate_source_config(instance)
            .expect_err("missing signal snapshots should be reported");
        assert!(matches!(
            error,
            BackendRuntimeError::MissingCommittedSignalSnapshot { instance: found, .. } if found == instance
        ));
    }

    #[test]
    fn linked_runtime_reports_missing_signal_item_mappings_for_source_config() {
        let lowered = lower_text(
            "runtime-startup-missing-signal-mapping.aivi",
            r#"
@source http.get "/host"
sig apiHost : Signal Text

@source http.get "{apiHost}/users"
sig users : Signal Text
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let users = item_id(lowered.hir.module(), "users");
        let binding = linked
            .source_by_owner(users)
            .expect("users source binding should exist")
            .clone();
        let required_item = binding.arguments[0].required_signals[0];
        linked.runtime_signal_by_item.remove(&required_item);

        let error = linked
            .evaluate_source_config(binding.instance)
            .expect_err("missing signal-item mappings should be reported explicitly");
        assert!(matches!(
            error,
            BackendRuntimeError::MissingSignalItemMapping {
                instance,
                item,
                ..
            } if instance == binding.instance && item == required_item
        ));
    }

    #[test]
    fn linked_runtime_spawns_task_workers_and_commits_publications() {
        let lowered = lower_text(
            "runtime-startup-manual-task-success.aivi",
            r#"
val answer = 42
"#,
        );
        let mut linked = manual_task_linked_runtime(&lowered, "answer");
        let binding = linked
            .task_by_owner(item_id(lowered.hir.module(), "answer"))
            .expect("manual task binding should exist")
            .clone();

        let handle = linked
            .spawn_task_worker(binding.instance)
            .expect("task worker should spawn");
        assert_eq!(
            handle
                .join()
                .expect("task worker thread should join cleanly"),
            Ok(LinkedTaskWorkerOutcome::Published)
        );

        let outcome = linked.tick().expect("task publication tick should succeed");
        assert!(!outcome.is_empty());
        assert_eq!(
            linked
                .runtime()
                .current_value(binding.input.as_signal())
                .expect("task sink should be readable"),
            Some(&RuntimeValue::Int(42))
        );
    }

    #[test]
    fn linked_runtime_reports_task_worker_evaluation_failures_explicitly() {
        let lowered = lower_text(
            "runtime-startup-manual-task-error.aivi",
            r#"
val total = 1.0 + 2.0
"#,
        );
        let mut linked = manual_task_linked_runtime(&lowered, "total");
        let binding = linked
            .task_by_owner(item_id(lowered.hir.module(), "total"))
            .expect("manual task binding should exist")
            .clone();

        let handle = linked
            .spawn_task_worker(binding.instance)
            .expect("task worker should spawn");
        let result = handle
            .join()
            .expect("task worker thread should join cleanly");
        assert!(matches!(
            result,
            Err(LinkedTaskWorkerError::Evaluation { instance, .. }) if instance == binding.instance
        ));
        let outcome = linked
            .tick()
            .expect("failed task should still allow empty ticks");
        assert!(outcome.is_empty());
        assert_eq!(
            linked
                .runtime()
                .current_value(binding.input.as_signal())
                .expect("task sink should be readable"),
            None
        );
    }

    #[test]
    fn linked_runtime_task_execution_respects_cancellation_and_owner_teardown() {
        let lowered = lower_text(
            "runtime-startup-manual-task-cancel.aivi",
            r#"
val answer = 42
"#,
        );
        let mut linked = manual_task_linked_runtime(&lowered, "answer");
        let binding = linked
            .task_by_owner(item_id(lowered.hir.module(), "answer"))
            .expect("manual task binding should exist")
            .clone();

        let prepared = linked
            .prepare_task_execution(binding.instance)
            .expect("task execution should prepare");
        linked
            .runtime_mut()
            .cancel_task(binding.instance)
            .expect("task cancellation should succeed");
        assert_eq!(
            execute_task_plan(prepared).expect("cancelled task execution should not error"),
            LinkedTaskWorkerOutcome::Cancelled
        );
        let outcome = linked.tick().expect("cancelled task tick should succeed");
        assert!(outcome.is_empty());
        assert_eq!(
            linked
                .runtime()
                .current_value(binding.input.as_signal())
                .expect("task sink should be readable"),
            None
        );

        let prepared = linked
            .prepare_task_execution(binding.instance)
            .expect("task execution should prepare again");
        linked
            .runtime_mut()
            .dispose_owner(binding.owner_handle)
            .expect("owner disposal should succeed");
        assert_eq!(
            execute_task_plan(prepared).expect("disposed task execution should not error"),
            LinkedTaskWorkerOutcome::Cancelled
        );
        linked.tick().expect("owner-disposal tick should succeed");
        assert_eq!(
            linked
                .runtime()
                .is_owner_active(binding.owner_handle)
                .expect("task owner should be queryable"),
            false
        );
    }

    #[test]
    fn linked_runtime_keeps_recurrent_task_body_gap_explicit() {
        let lowered = lower_text(
            "runtime-startup-task-body-gap.aivi",
            r#"
domain Retry over Int
    literal x : Int -> Retry

fun step:Int #value:Int =>
    value

@recur.backoff 3x
val retried : Task Int Int =
    0
     @|> step
     <|@ step
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("task-body-gap runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("task-body-gap startup link should succeed");
        let binding = linked
            .task_by_owner(item_id(lowered.hir.module(), "retried"))
            .expect("recurrent task binding should exist");
        assert!(matches!(
            &binding.execution,
            LinkedTaskExecutionBinding::Blocked(LinkedTaskExecutionBlocker::MissingLoweredBody)
        ));
        let instance = binding.instance;
        assert!(matches!(
            linked.spawn_task_worker(instance),
            Err(BackendRuntimeError::TaskExecutionBlocked {
                instance: blocked_instance,
                ..
            }) if blocked_instance == instance
        ));
    }

    #[test]
    fn linked_runtime_evaluates_helper_kernels_with_inline_case_pipes() {
        let lowered = lower_text(
            "runtime-startup-inline-case-helper.aivi",
            r#"
fun choose:Text #maybeName:(Option Text) =>
    maybeName
     ||> Some name => name
     ||> None => "guest"

sig maybeName = Some "Ada"
sig label = choose maybeName
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(label_signal).unwrap(),
            Some(&RuntimeValue::Text("Ada".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_evaluates_signal_inline_case_kernels_against_committed_snapshots() {
        let lowered = lower_text(
            "runtime-startup-signal-inline-case.aivi",
            r#"
fun greetSelected:Signal Text #prefix:Text #fallback:Text #selected:Signal (Option Text) =>
    selected
     ||> Some name => "{prefix}:{name}"
     ||> None => "{prefix}:{fallback}"

sig selectedUser : Signal (Option Text) = Some "Ada"

sig greeting : Signal Text =
    greetSelected "user" "guest" selectedUser
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let greeting_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "greeting"))
            .expect("greeting signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(greeting_signal).unwrap(),
            Some(&RuntimeValue::Text("user:Ada".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_evaluates_signal_truthy_falsy_kernels_against_committed_snapshots() {
        let lowered = lower_text(
            "runtime-startup-signal-inline-truthy-falsy.aivi",
            r#"
fun renderStatus:Signal Text #prefix:Text #readyText:Text #waitText:Text #statusReady:Signal Bool =>
    statusReady
     T|> "{prefix}:{readyText}"
     F|> "{prefix}:{waitText}"

sig ready : Signal Bool = True

sig status : Signal Text =
    renderStatus "state" "go" "wait" ready
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let status_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "status"))
            .expect("status signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(status_signal).unwrap(),
            Some(&RuntimeValue::Text("state:go".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_evaluates_direct_signal_truthy_falsy_bodies() {
        let lowered = lower_text(
            "runtime-startup-direct-signal-truthy-falsy.aivi",
            r#"
type User = {
    name: Text
}

type LoadError = {
    message: Text
}

type FormError = {
    message: Text
}

sig ready : Signal Bool = True

sig maybeUser : Signal (Option User) = Some { name: "Ada" }

sig loaded : Signal (Result LoadError User) = Err { message: "offline" }

sig submitted : Signal (Validation FormError User) = Valid { name: "Grace" }

sig readyText : Signal Text =
    ready
     T|> "start"
     F|> "wait"

sig greeting : Signal Text =
    maybeUser
     T|> .name
     F|> "guest"

sig rendered : Signal Text =
    loaded
     T|> .name
     F|> .message

sig summary : Signal Text =
    submitted
     T|> .name
     F|> .message
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        for (name, expected) in [
            ("readyText", RuntimeValue::Text("start".into())),
            ("greeting", RuntimeValue::Text("Ada".into())),
            ("rendered", RuntimeValue::Text("offline".into())),
            ("summary", RuntimeValue::Text("Grace".into())),
        ] {
            let signal = linked
                .assembly()
                .signal(item_id(lowered.hir.module(), name))
                .unwrap_or_else(|| panic!("signal binding should exist for {name}"))
                .signal();
            assert_eq!(
                linked.runtime().current_value(signal).unwrap(),
                Some(&expected),
                "signal {name} should commit the expected truthy/falsy result"
            );
        }
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_evaluates_direct_signal_transform_bodies() {
        let lowered = lower_text(
            "runtime-startup-direct-signal-transform.aivi",
            r#"
type User = {
    name: Text
}

type Session = {
    user: User
}

sig session : Signal Session = { user: { name: "Ada" } }

sig label : Signal Text =
    session
     |> .user
     |> .name
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(label_signal).unwrap(),
            Some(&RuntimeValue::Text("Ada".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_evaluates_direct_signal_case_bodies_with_same_module_sum_patterns() {
        let lowered = lower_text(
            "runtime-startup-direct-signal-inline-case.aivi",
            r#"
type Status =
  | Idle
  | Ready Text
  | Failed Text Text

sig current : Signal Status =
    Failed "503" "offline"

sig label : Signal Text =
    current
     ||> Idle => "idle"
     ||> Ready name => name
     ||> Failed code message => "{code}:{message}"
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("startup link should succeed");
        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let label_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "label"))
            .expect("label signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(label_signal).unwrap(),
            Some(&RuntimeValue::Text("503:offline".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_executes_signal_filter_gate_pipelines() {
        let lowered = lower_text(
            "runtime-startup-direct-signal-gate.aivi",
            r#"
type User = {
    active: Bool,
    email: Text
}

type Session = {
    user: User
}

val seed:User = { active: True, email: "ada@example.com" }

sig sessions : Signal Session = { user: seed }

sig activeUsers : Signal User =
    sessions
     |> .user
     ?|> .active
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("signal filter gate pipelines should now link successfully");

        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");

        let active_users_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "activeUsers"))
            .expect("activeUsers signal binding should exist")
            .signal();

        assert!(
            linked
                .runtime()
                .current_value(active_users_signal)
                .unwrap()
                .is_some(),
            "activeUsers should commit a value because user.active is True"
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_rejects_source_backed_body_signals() {
        let lowered = lower_text(
            "runtime-startup-source-body-gap.aivi",
            r#"
fun step:Int #value:Int =>
    value

sig enabled = True

@source http.get "/users" with {
    activeWhen: enabled
}
sig gated : Signal Int =
    0
     @|> step
     <|@ step
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let errors = match link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        ) {
            Ok(_) => panic!("source-backed body signals should stay an explicit startup gap"),
            Err(errors) => errors,
        };
        assert!(errors.errors().iter().any(|error| matches!(
            error,
            BackendRuntimeLinkError::SourceBackedBodySignalNotYetLinked { item }
                if *item == item_id(lowered.hir.module(), "gated")
        )));
    }
}
