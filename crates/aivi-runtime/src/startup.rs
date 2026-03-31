use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    thread::{self, JoinHandle},
};

use aivi_backend::{
    DetachedRuntimeValue, EvaluationError, GateStage as BackendGateStage, ItemId as BackendItemId,
    ItemKind as BackendItemKind, KernelEvaluator, KernelId, LayoutKind, MovingRuntimeValueStore,
    PipelineId as BackendPipelineId, Program as BackendProgram, RuntimeCallable,
    RuntimeDbConnection, RuntimeRecordField, RuntimeSumValue, RuntimeValue,
    SourceId as BackendSourceId, StageKind as BackendStageKind,
};
use aivi_core as core;
use aivi_hir as hir;

use crate::{
    DerivedSignalUpdate, InputHandle, Publication, PublicationPortError, RuntimeSourceProvider,
    SourceInstanceId, SourceLifecycleActionKind, SourcePublicationPort, TaskCompletionPort,
    TaskInstanceId, TaskSourceRuntime, TaskSourceRuntimeError, TickOutcome,
    TryDerivedNodeEvaluator,
    graph::{DerivedHandle, OwnerHandle, ReactiveClauseHandle, SignalHandle},
    hir_adapter::{HirCompiledRuntimeExpr, HirRuntimeAssembly, HirRuntimeInstantiationError},
    scheduler::DependencyValues,
    task_executor::{RuntimeDbCommitInvalidation, execute_runtime_value_with_stdio_effects},
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
    let mut linked_runtime = BackendLinkedRuntime {
        assembly,
        runtime,
        backend,
        signal_items_by_handle: linked.signal_items_by_handle,
        runtime_signal_by_item: linked.runtime_signal_by_item,
        derived_signals: linked.derived_signals,
        reactive_signals: linked.reactive_signals,
        reactive_clauses: linked.reactive_clauses,
        linked_recurrence_signals: linked.linked_recurrence_signals,
        source_bindings: linked.source_bindings,
        task_bindings: linked.task_bindings,
        db_changed_routes: linked.db_changed_routes,
        db_commit_invalidation_sink: None,
    };
    linked_runtime.prime_db_changed_routes();
    Ok(linked_runtime)
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
    reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    db_changed_routes: Box<[LinkedDbChangedRoute]>,
    db_commit_invalidation_sink: Option<DbCommitInvalidationSink>,
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

    pub fn signal_graph(&self) -> &crate::SignalGraph {
        self.runtime.graph()
    }

    pub fn derived_signal(&self, signal: DerivedHandle) -> Option<&LinkedDerivedSignal> {
        self.derived_signals.get(&signal)
    }

    pub fn source_binding(&self, instance: SourceInstanceId) -> Option<&LinkedSourceBinding> {
        self.source_bindings.get(&instance)
    }

    pub fn source_bindings(&self) -> impl ExactSizeIterator<Item = &LinkedSourceBinding> {
        self.source_bindings.values()
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

    pub(crate) fn set_db_commit_invalidation_sink(
        &mut self,
        sink: Option<DbCommitInvalidationSink>,
    ) {
        self.db_commit_invalidation_sink = sink;
    }

    fn prime_db_changed_routes(&mut self) {
        let mut seeded = BTreeSet::new();
        for route in &self.db_changed_routes {
            if !seeded.insert(route.changed_input) {
                continue;
            }
            let Ok(stamp) = self.runtime.current_stamp(route.changed_input) else {
                continue;
            };
            let _ = self
                .runtime
                .queue_publication(Publication::new(stamp, RuntimeValue::Unit));
        }
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

    pub fn evaluate_task_value_by_owner(
        &self,
        owner: hir::ItemId,
    ) -> Result<DetachedRuntimeValue, BackendRuntimeError> {
        let binding = self
            .task_by_owner(owner)
            .cloned()
            .ok_or(BackendRuntimeError::UnknownTaskOwner { owner })?;
        let (kernel, required_signals) = match &binding.execution {
            LinkedTaskExecutionBinding::Ready {
                kernel,
                required_signals,
            } => (*kernel, required_signals.clone()),
            LinkedTaskExecutionBinding::Blocked(blocker) => {
                return Err(BackendRuntimeError::TaskExecutionBlocked {
                    instance: binding.instance,
                    owner: binding.owner,
                    blocker: blocker.clone(),
                });
            }
        };
        let snapshots = self.committed_signal_snapshots()?;
        let globals = self.required_task_globals(
            binding.instance,
            kernel,
            required_signals.as_ref(),
            &snapshots,
        )?;
        let runtime_globals = materialize_detached_globals(&globals);
        let mut evaluator = KernelEvaluator::new(self.backend.as_ref());
        let value = evaluator
            .evaluate_item(binding.backend_item, &runtime_globals)
            .map_err(|error| BackendRuntimeError::EvaluateTaskBody {
                instance: binding.instance,
                owner: binding.owner,
                backend_item: binding.backend_item,
                error,
            })?;
        Ok(DetachedRuntimeValue::from_runtime_owned(value))
    }

    pub fn tick(&mut self) -> Result<TickOutcome, BackendRuntimeError> {
        let committed = self.committed_signal_snapshots()?;
        let runtime_committed = materialize_detached_globals(&committed);
        let mut evaluator = LinkedDerivedEvaluator {
            backend: self.backend.as_ref(),
            signal_items_by_handle: &self.signal_items_by_handle,
            derived_signals: &self.derived_signals,
            reactive_signals: &self.reactive_signals,
            reactive_clauses: &self.reactive_clauses,
            linked_recurrence_signals: &self.linked_recurrence_signals,
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
                let public_value = match value {
                    RuntimeValue::Signal(_) => value.clone(),
                    other => RuntimeValue::Signal(Box::new(other.clone())),
                };
                snapshots.insert(item, DetachedRuntimeValue::from_runtime_owned(public_value));
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
            db_commit_invalidation_sink: self.db_commit_invalidation_sink.clone(),
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

    pub(crate) fn invalidate_db_commit(
        &mut self,
        invalidation: &RuntimeDbCommitInvalidation,
    ) -> bool {
        let snapshots = match self.committed_signal_snapshots() {
            Ok(snapshots) => snapshots,
            Err(_) => return false,
        };
        let mut changed_inputs = BTreeSet::new();
        for route in &self.db_changed_routes {
            let Some(identity) = self.db_changed_route_identity(route, &snapshots) else {
                continue;
            };
            if identity.connection.database != invalidation.connection.database {
                continue;
            }
            if !invalidation
                .changed_tables
                .contains(identity.table_name.as_ref())
            {
                continue;
            }
            changed_inputs.insert(route.changed_input);
        }

        let mut invalidated = false;
        for input in changed_inputs {
            match self.runtime.advance_generation(input) {
                Ok(stamp) => {
                    if self
                        .runtime
                        .queue_publication(Publication::new(stamp, RuntimeValue::Unit))
                        .is_ok()
                    {
                        invalidated = true;
                    }
                }
                Err(TaskSourceRuntimeError::Scheduler(
                    crate::SchedulerAccessError::OwnerInactive { .. },
                )) => {}
                Err(_) => {}
            }
        }
        invalidated
    }

    fn db_changed_route_identity(
        &self,
        route: &LinkedDbChangedRoute,
        snapshots: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Option<RuntimeDbTableIdentity> {
        let value = match &route.table {
            LinkedDbChangedRouteTable::Signal { signal } => {
                self.runtime.current_value(*signal).ok().flatten()?.clone()
            }
            LinkedDbChangedRouteTable::Value {
                backend_item,
                required_signals,
                changed_signal_item,
                ..
            } => {
                let globals = self.required_db_route_globals(
                    required_signals,
                    *changed_signal_item,
                    snapshots,
                )?;
                let runtime_globals = materialize_detached_globals(&globals);
                let mut evaluator = KernelEvaluator::new(self.backend.as_ref());
                evaluator
                    .evaluate_item(*backend_item, &runtime_globals)
                    .ok()?
            }
        };
        runtime_db_table_identity(&value)
    }

    fn required_db_route_globals(
        &self,
        required: &[BackendItemId],
        changed_signal_item: Option<BackendItemId>,
        snapshots: &BTreeMap<BackendItemId, DetachedRuntimeValue>,
    ) -> Option<BTreeMap<BackendItemId, DetachedRuntimeValue>> {
        let mut globals = BTreeMap::new();
        for item in required {
            if let Some(value) = snapshots.get(item) {
                globals.insert(*item, value.clone());
                continue;
            }
            if Some(*item) == changed_signal_item {
                globals.insert(
                    *item,
                    DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Signal(Box::new(
                        RuntimeValue::Unit,
                    ))),
                );
                continue;
            }
            return None;
        }
        Some(globals)
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

fn signal_global_value(value: &RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(_) => value.clone(),
        other => RuntimeValue::Signal(Box::new(other.clone())),
    }
}

fn signal_payload_value(value: &RuntimeValue) -> RuntimeValue {
    match value {
        RuntimeValue::Signal(inner) => inner.as_ref().clone(),
        other => other.clone(),
    }
}

fn stage_subject_value(
    backend: &BackendProgram,
    layout: aivi_backend::LayoutId,
    value: &RuntimeValue,
) -> RuntimeValue {
    match (&backend.layouts()[layout].kind, value) {
        (LayoutKind::Signal { .. }, RuntimeValue::Signal(_)) => value.clone(),
        (LayoutKind::Signal { .. }, other) => RuntimeValue::Signal(Box::new(other.clone())),
        (_, RuntimeValue::Signal(inner)) => inner.as_ref().clone(),
        _ => value.clone(),
    }
}

fn unwrap_signal_layout_result(
    backend: &BackendProgram,
    layout: aivi_backend::LayoutId,
    value: RuntimeValue,
) -> RuntimeValue {
    match (&backend.layouts()[layout].kind, value) {
        (LayoutKind::Signal { .. }, RuntimeValue::Signal(inner)) => *inner,
        (_, value) => value,
    }
}

fn runtime_db_table_identity(value: &RuntimeValue) -> Option<RuntimeDbTableIdentity> {
    let RuntimeValue::Record(fields) = strip_runtime_signal(value) else {
        return None;
    };
    let table_name = record_text_field(fields, "name")?;
    let connection = runtime_db_connection_value(record_field(fields, "conn")?)?;
    Some(RuntimeDbTableIdentity {
        connection,
        table_name: table_name.into(),
    })
}

fn runtime_db_connection_value(value: &RuntimeValue) -> Option<RuntimeDbConnection> {
    let RuntimeValue::Record(fields) = strip_runtime_signal(value) else {
        return None;
    };
    Some(RuntimeDbConnection {
        database: record_text_field(fields, "database")?.into(),
    })
}

fn record_field<'a>(fields: &'a [RuntimeRecordField], label: &str) -> Option<&'a RuntimeValue> {
    fields
        .iter()
        .find(|field| field.label.as_ref() == label)
        .map(|field| &field.value)
}

fn record_text_field<'a>(fields: &'a [RuntimeRecordField], label: &str) -> Option<&'a str> {
    strip_runtime_signal(record_field(fields, label)?).as_text()
}

fn strip_runtime_signal(value: &RuntimeValue) -> &RuntimeValue {
    let mut current = value;
    while let RuntimeValue::Signal(inner) = current {
        current = inner.as_ref();
    }
    current
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedDerivedSignal {
    pub item: hir::ItemId,
    pub signal: DerivedHandle,
    pub backend_item: BackendItemId,
    pub dependency_items: Box<[BackendItemId]>,
    pub source_input: Option<InputHandle>,
    /// Backend pipeline IDs that must be applied to the body result in order.
    pub pipeline_ids: Box<[BackendPipelineId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedRecurrenceSignal {
    pub item: hir::ItemId,
    pub signal: DerivedHandle,
    pub backend_item: BackendItemId,
    pub wakeup_dependency_index: usize,
    pub seed_kernel: KernelId,
    pub step_kernels: Box<[KernelId]>,
    pub dependency_items: Box<[BackendItemId]>,
    pub pipeline_ids: Box<[BackendPipelineId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedReactiveSignal {
    pub item: hir::ItemId,
    pub signal: SignalHandle,
    pub backend_item: BackendItemId,
    pub has_seed_body: bool,
    pub pipeline_signals: Box<[SignalHandle]>,
    pub pipeline_ids: Box<[BackendPipelineId]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedReactiveClause {
    pub owner: hir::ItemId,
    pub target: SignalHandle,
    pub clause: ReactiveClauseHandle,
    pub pipeline_ids: Box<[BackendPipelineId]>,
    pub body_mode: hir::ReactiveUpdateBodyMode,
    pub compiled_guard: HirCompiledRuntimeExpr,
    pub compiled_body: HirCompiledRuntimeExpr,
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

pub(crate) type DbCommitInvalidationSink =
    Arc<dyn Fn(RuntimeDbCommitInvalidation) + Send + Sync + 'static>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinkedDbChangedRoute {
    changed_input: InputHandle,
    table: LinkedDbChangedRouteTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LinkedDbChangedRouteTable {
    Signal {
        signal: SignalHandle,
    },
    Value {
        owner: hir::ItemId,
        backend_item: BackendItemId,
        required_signals: Box<[BackendItemId]>,
        changed_signal_item: Option<BackendItemId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeDbTableIdentity {
    connection: RuntimeDbConnection,
    table_name: Box<str>,
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
    TaskExecution {
        instance: TaskInstanceId,
        owner: hir::ItemId,
        backend_item: BackendItemId,
        error: crate::task_executor::RuntimeTaskExecutionError,
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
            Self::TaskExecution {
                instance,
                owner,
                backend_item,
                error,
            } => write!(
                f,
                "task instance {} for owner {owner} failed while executing the task plan produced by backend item item{backend_item}: {error}",
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
    #[cfg(test)]
    pub(crate) fn from_source_port(inner: SourcePublicationPort<RuntimeValue>) -> Self {
        Self { inner }
    }

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
    db_commit_invalidation_sink: Option<DbCommitInvalidationSink>,
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
        db_commit_invalidation_sink,
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
    let outcome = execute_runtime_value_with_stdio_effects(value).map_err(|error| {
        LinkedTaskWorkerError::TaskExecution {
            instance,
            owner,
            backend_item,
            error,
        }
    })?;
    if let Some(invalidation) = outcome.commit_invalidation
        && let Some(sink) = db_commit_invalidation_sink
    {
        sink(invalidation);
    }
    match completion.complete(DetachedRuntimeValue::from_runtime_owned(outcome.value)) {
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
    MissingRecurrenceWakeupDependency {
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
                "signal {item} still mixes source publication with a body-backed runtime path"
            ),
            Self::MissingRecurrenceWakeupDependency { item } => write!(
                f,
                "signal {item} has recurrence lowering but no runtime wakeup dependency"
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
    UnknownReactiveSignal {
        signal: SignalHandle,
    },
    UnknownReactiveClause {
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
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
    EvaluateTaskBody {
        instance: TaskInstanceId,
        owner: hir::ItemId,
        backend_item: BackendItemId,
        error: EvaluationError,
    },
    EvaluateDerivedSignal {
        signal: DerivedHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    EvaluateReactiveSeed {
        signal: SignalHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    EvaluateReactiveGuard {
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    EvaluateReactiveBody {
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    },
    ReactiveBodyReturnedNonOption {
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        value: RuntimeValue,
    },
    ReactiveGuardReturnedNonBool {
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        value: RuntimeValue,
    },
    EvaluateRecurrenceSignal {
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
            Self::UnknownReactiveSignal { signal } => {
                write!(
                    f,
                    "startup linker does not know reactive signal {:?}",
                    signal
                )
            }
            Self::UnknownReactiveClause { signal, clause } => write!(
                f,
                "startup linker does not know reactive clause {:?} for signal {:?}",
                clause, signal
            ),
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
            Self::EvaluateTaskBody {
                instance,
                owner,
                backend_item,
                error,
            } => write!(
                f,
                "task instance {} for owner {owner} failed while evaluating backend item item{backend_item}: {error}",
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
            Self::EvaluateReactiveSeed {
                signal,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate reactive seed for signal {:?} / item {item}: {error}",
                signal
            ),
            Self::EvaluateReactiveGuard {
                signal,
                clause,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate reactive guard {:?} for signal {:?} / item {item}: {error}",
                clause, signal
            ),
            Self::EvaluateReactiveBody {
                signal,
                clause,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate reactive body {:?} for signal {:?} / item {item}: {error}",
                clause, signal
            ),
            Self::ReactiveBodyReturnedNonOption {
                signal,
                clause,
                item,
                value,
            } => write!(
                f,
                "reactive body {:?} for signal {:?} / item {item} returned non-option value {value:?}",
                clause, signal
            ),
            Self::ReactiveGuardReturnedNonBool {
                signal,
                clause,
                item,
                value,
            } => write!(
                f,
                "reactive guard {:?} for signal {:?} / item {item} returned non-Bool value {:?}",
                clause, signal, value
            ),
            Self::EvaluateRecurrenceSignal {
                signal,
                item,
                error,
            } => write!(
                f,
                "failed to evaluate recurrence signal {:?} for item {item}: {error}",
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
    signal_items_by_handle: &'a BTreeMap<SignalHandle, BackendItemId>,
    derived_signals: &'a BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    reactive_signals: &'a BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: &'a BTreeMap<crate::ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: &'a BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    committed_signals: &'a BTreeMap<BackendItemId, RuntimeValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReactivePipelineContext {
    Seed,
    Body(ReactiveClauseHandle),
}

impl TryDerivedNodeEvaluator<RuntimeValue> for LinkedDerivedEvaluator<'_> {
    type Error = BackendRuntimeError;

    fn try_evaluate(
        &mut self,
        signal: DerivedHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        // Check recurrence signals first.
        if let Some(binding) = self.linked_recurrence_signals.get(&signal) {
            return self.try_evaluate_recurrence(signal, binding, inputs);
        }

        let binding = self
            .derived_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownDerivedSignal { signal })?;
        let expected_inputs =
            binding.dependency_items.len() + usize::from(binding.source_input.is_some());
        if inputs.len() != expected_inputs {
            return Err(BackendRuntimeError::DerivedDependencyArityMismatch {
                signal,
                expected: expected_inputs,
                found: inputs.len(),
            });
        }

        // If no upstream dependency changed this tick and the signal already has a committed
        // value, the output of this pure function is guaranteed to be the same — skip the
        // (potentially expensive) kernel re-evaluation.
        let any_updated = (0..expected_inputs).any(|i| inputs.updated(i));
        if !any_updated && self.committed_signals.contains_key(&binding.backend_item) {
            return Ok(DerivedSignalUpdate::Unchanged);
        }

        let mut globals = self.committed_signals.clone();
        globals.remove(&binding.backend_item);
        for (index, dependency) in binding.dependency_items.iter().copied().enumerate() {
            let Some(value) = inputs.value(index) else {
                return Ok(DerivedSignalUpdate::Clear);
            };
            globals.insert(dependency, RuntimeValue::Signal(Box::new(value.clone())));
        }

        let mut evaluator = KernelEvaluator::new(self.backend);
        let value = evaluator
            .evaluate_item(binding.backend_item, &globals)
            .map_err(|error| self.derived_eval_error(signal, binding.item, error))?;

        self.apply_pipelines(
            signal,
            binding.item,
            &binding.pipeline_ids,
            value,
            &globals,
            &mut evaluator,
        )
    }

    fn try_evaluate_reactive_seed(
        &mut self,
        signal: SignalHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        let binding = self
            .reactive_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownReactiveSignal { signal })?;
        if !binding.has_seed_body || !inputs.all_present() {
            return Ok(DerivedSignalUpdate::Clear);
        }

        let globals = self.build_signal_globals(binding.backend_item, &inputs);
        let mut evaluator = KernelEvaluator::new(self.backend);
        let value = evaluator
            .evaluate_item(binding.backend_item, &globals)
            .map_err(|error| self.reactive_seed_eval_error(signal, binding.item, error))?;
        self.apply_reactive_pipelines(
            signal,
            binding.item,
            ReactivePipelineContext::Seed,
            &binding.pipeline_ids,
            value,
            &globals,
            &mut evaluator,
        )
    }

    fn try_evaluate_reactive_guard(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<bool, Self::Error> {
        if !inputs.all_present() {
            return Ok(false);
        }
        let binding = self.reactive_clause(signal, clause)?;
        let Some(globals) = self.build_fragment_globals(&binding.compiled_guard, &inputs) else {
            return Ok(false);
        };
        let mut evaluator = KernelEvaluator::new(binding.compiled_guard.backend.as_ref());
        let value = evaluator
            .evaluate_item(binding.compiled_guard.entry_item, &globals)
            .map_err(|error| {
                self.reactive_guard_eval_error(signal, clause, binding.owner, error)
            })?;
        match value {
            RuntimeValue::Bool(value) => Ok(value),
            RuntimeValue::Signal(inner) => match *inner {
                RuntimeValue::Bool(value) => Ok(value),
                other => Err(BackendRuntimeError::ReactiveGuardReturnedNonBool {
                    signal,
                    clause,
                    item: binding.owner,
                    value: RuntimeValue::Signal(Box::new(other)),
                }),
            },
            other => Err(BackendRuntimeError::ReactiveGuardReturnedNonBool {
                signal,
                clause,
                item: binding.owner,
                value: other,
            }),
        }
    }

    fn try_evaluate_reactive_body(
        &mut self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, Self::Error> {
        if !inputs.all_present() {
            return Ok(DerivedSignalUpdate::Clear);
        }
        let signal_binding = self
            .reactive_signals
            .get(&signal)
            .ok_or(BackendRuntimeError::UnknownReactiveSignal { signal })?;
        let clause_binding = self.reactive_clause(signal, clause)?;
        let Some(fragment_globals) =
            self.build_fragment_globals(&clause_binding.compiled_body, &inputs)
        else {
            return Ok(DerivedSignalUpdate::Clear);
        };
        let mut fragment_evaluator =
            KernelEvaluator::new(clause_binding.compiled_body.backend.as_ref());
        let value = fragment_evaluator
            .evaluate_item(clause_binding.compiled_body.entry_item, &fragment_globals)
            .map_err(|error| {
                self.reactive_body_eval_error(signal, clause, clause_binding.owner, error)
            })?;
        let value = match clause_binding.body_mode {
            hir::ReactiveUpdateBodyMode::Payload => value,
            hir::ReactiveUpdateBodyMode::OptionalPayload => match value {
                RuntimeValue::OptionSome(value) => *value,
                RuntimeValue::OptionNone => return Ok(DerivedSignalUpdate::Unchanged),
                RuntimeValue::Signal(inner) => match *inner {
                    RuntimeValue::OptionSome(value) => *value,
                    RuntimeValue::OptionNone => return Ok(DerivedSignalUpdate::Unchanged),
                    other => {
                        return Err(BackendRuntimeError::ReactiveBodyReturnedNonOption {
                            signal,
                            clause,
                            item: clause_binding.owner,
                            value: RuntimeValue::Signal(Box::new(other)),
                        });
                    }
                },
                other => {
                    return Err(BackendRuntimeError::ReactiveBodyReturnedNonOption {
                        signal,
                        clause,
                        item: clause_binding.owner,
                        value: other,
                    });
                }
            },
        };

        let globals = self.build_signal_globals(signal_binding.backend_item, &inputs);
        let mut evaluator = KernelEvaluator::new(self.backend);
        self.apply_reactive_pipelines(
            signal,
            clause_binding.owner,
            ReactivePipelineContext::Body(clause),
            &clause_binding.pipeline_ids,
            value,
            &globals,
            &mut evaluator,
        )
    }
}

impl LinkedDerivedEvaluator<'_> {
    fn reactive_clause(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
    ) -> Result<&LinkedReactiveClause, BackendRuntimeError> {
        let binding = self
            .reactive_clauses
            .get(&clause)
            .ok_or(BackendRuntimeError::UnknownReactiveClause { signal, clause })?;
        if binding.target != signal {
            return Err(BackendRuntimeError::UnknownReactiveClause { signal, clause });
        }
        Ok(binding)
    }

    fn build_signal_globals(
        &self,
        target_item: BackendItemId,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> BTreeMap<BackendItemId, RuntimeValue> {
        let mut globals = self.committed_signals.clone();
        globals.remove(&target_item);
        for index in 0..inputs.len() {
            let Some(signal) = inputs.signal(index) else {
                continue;
            };
            let Some(&dependency) = self.signal_items_by_handle.get(&signal) else {
                continue;
            };
            match inputs.value(index) {
                Some(value) => {
                    globals.insert(dependency, signal_global_value(value));
                }
                None => {
                    globals.remove(&dependency);
                }
            }
        }
        globals
    }

    fn build_fragment_globals(
        &self,
        fragment: &HirCompiledRuntimeExpr,
        inputs: &DependencyValues<'_, RuntimeValue>,
    ) -> Option<BTreeMap<BackendItemId, RuntimeValue>> {
        let mut globals = BTreeMap::new();
        for required in fragment.required_signals.iter() {
            let value = self.fragment_signal_value_for_inputs(inputs, required.signal)?;
            globals.insert(required.backend_item, value);
        }
        Some(globals)
    }

    fn fragment_signal_value_for_inputs(
        &self,
        inputs: &DependencyValues<'_, RuntimeValue>,
        signal: SignalHandle,
    ) -> Option<RuntimeValue> {
        if inputs.contains_signal(signal) {
            return inputs.value_for(signal).cloned();
        }
        let backend_item = self.signal_items_by_handle.get(&signal)?;
        self.committed_signals
            .get(backend_item)
            .map(signal_payload_value)
    }

    fn derived_eval_error(
        &self,
        signal: DerivedHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateDerivedSignal {
            signal,
            item,
            error,
        }
    }

    fn reactive_seed_eval_error(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveSeed {
            signal,
            item,
            error,
        }
    }

    fn reactive_guard_eval_error(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveGuard {
            signal,
            clause,
            item,
            error,
        }
    }

    fn reactive_body_eval_error(
        &self,
        signal: SignalHandle,
        clause: ReactiveClauseHandle,
        item: hir::ItemId,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        BackendRuntimeError::EvaluateReactiveBody {
            signal,
            clause,
            item,
            error,
        }
    }

    fn reactive_pipeline_eval_error(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        error: EvaluationError,
    ) -> BackendRuntimeError {
        match context {
            ReactivePipelineContext::Seed => self.reactive_seed_eval_error(signal, item, error),
            ReactivePipelineContext::Body(clause) => {
                self.reactive_body_eval_error(signal, clause, item, error)
            }
        }
    }

    fn apply_pipelines(
        &self,
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline_ids: &[BackendPipelineId],
        mut value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut KernelEvaluator<'_>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        for &pipeline_id in pipeline_ids {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate,
                        emits_negative_update,
                        ..
                    }) => {
                        let pred = evaluator
                            .evaluate_kernel(*predicate, Some(&value), &[], globals)
                            .map_err(|error| self.derived_eval_error(signal, item, error))?;
                        if matches!(pred, RuntimeValue::Bool(false)) && !emits_negative_update {
                            return Ok(DerivedSignalUpdate::Clear);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {
                        // Carrier metadata only; the body kernel already computed
                        // the branch result, so we pass the value through unchanged.
                    }
                    BackendStageKind::Fanout(fanout) => {
                        value = self
                            .apply_fanout_stage(signal, item, fanout, value, globals, evaluator)?;
                    }
                    _ => unreachable!(
                        "unsupported pipeline stage kind should have been blocked during linking"
                    ),
                }
            }
        }
        Ok(DerivedSignalUpdate::Value(value))
    }

    fn apply_reactive_pipelines(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        pipeline_ids: &[BackendPipelineId],
        mut value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut KernelEvaluator<'_>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        for &pipeline_id in pipeline_ids {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate,
                        emits_negative_update,
                        ..
                    }) => {
                        let pred = evaluator
                            .evaluate_kernel(*predicate, Some(&value), &[], globals)
                            .map_err(|error| {
                                self.reactive_pipeline_eval_error(signal, item, context, error)
                            })?;
                        if matches!(pred, RuntimeValue::Bool(false)) && !emits_negative_update {
                            return Ok(DerivedSignalUpdate::Clear);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {}
                    BackendStageKind::Fanout(fanout) => {
                        value = self.apply_reactive_fanout_stage(
                            signal, item, context, fanout, value, globals, evaluator,
                        )?;
                    }
                    _ => unreachable!(
                        "unsupported pipeline stage kind should have been blocked during linking"
                    ),
                }
            }
        }
        Ok(DerivedSignalUpdate::Value(value))
    }

    fn apply_fanout_stage(
        &self,
        signal: DerivedHandle,
        item: hir::ItemId,
        fanout: &aivi_backend::FanoutStage,
        value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut KernelEvaluator<'_>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let current = match value {
            RuntimeValue::Signal(inner) => *inner,
            other => other,
        };
        let RuntimeValue::List(elements) = current else {
            return Ok(current);
        };

        let mut mapped = Vec::with_capacity(elements.len());
        'elements: for element in elements {
            let mapped_value = evaluator
                .evaluate_kernel(fanout.map, Some(&element), &[], globals)
                .map_err(|error| self.derived_eval_error(signal, item, error))?;
            for filter in &fanout.filters {
                let predicate = evaluator
                    .evaluate_kernel(filter.predicate, Some(&mapped_value), &[], globals)
                    .map_err(|error| self.derived_eval_error(signal, item, error))?;
                if !matches!(predicate, RuntimeValue::Bool(true)) {
                    continue 'elements;
                }
            }
            mapped.push(mapped_value);
        }

        let mapped_collection = RuntimeValue::List(mapped);
        match &fanout.join {
            Some(join) => {
                let subject =
                    stage_subject_value(self.backend, join.input_layout, &mapped_collection);
                let joined = evaluator
                    .evaluate_kernel(join.kernel, Some(&subject), &[], globals)
                    .map_err(|error| self.derived_eval_error(signal, item, error))?;
                Ok(unwrap_signal_layout_result(
                    self.backend,
                    join.result_layout,
                    joined,
                ))
            }
            None => Ok(mapped_collection),
        }
    }

    fn apply_reactive_fanout_stage(
        &self,
        signal: SignalHandle,
        item: hir::ItemId,
        context: ReactivePipelineContext,
        fanout: &aivi_backend::FanoutStage,
        value: RuntimeValue,
        globals: &BTreeMap<BackendItemId, RuntimeValue>,
        evaluator: &mut KernelEvaluator<'_>,
    ) -> Result<RuntimeValue, BackendRuntimeError> {
        let current = match value {
            RuntimeValue::Signal(inner) => *inner,
            other => other,
        };
        let RuntimeValue::List(elements) = current else {
            return Ok(current);
        };

        let mut mapped = Vec::with_capacity(elements.len());
        'elements: for element in elements {
            let mapped_value = evaluator
                .evaluate_kernel(fanout.map, Some(&element), &[], globals)
                .map_err(|error| self.reactive_pipeline_eval_error(signal, item, context, error))?;
            for filter in &fanout.filters {
                let predicate = evaluator
                    .evaluate_kernel(filter.predicate, Some(&mapped_value), &[], globals)
                    .map_err(|error| {
                        self.reactive_pipeline_eval_error(signal, item, context, error)
                    })?;
                if !matches!(predicate, RuntimeValue::Bool(true)) {
                    continue 'elements;
                }
            }
            mapped.push(mapped_value);
        }

        let mapped_collection = RuntimeValue::List(mapped);
        match &fanout.join {
            Some(join) => {
                let subject =
                    stage_subject_value(self.backend, join.input_layout, &mapped_collection);
                let joined = evaluator
                    .evaluate_kernel(join.kernel, Some(&subject), &[], globals)
                    .map_err(|error| {
                        self.reactive_pipeline_eval_error(signal, item, context, error)
                    })?;
                Ok(unwrap_signal_layout_result(
                    self.backend,
                    join.result_layout,
                    joined,
                ))
            }
            None => Ok(mapped_collection),
        }
    }

    fn try_evaluate_recurrence(
        &self,
        signal: DerivedHandle,
        binding: &LinkedRecurrenceSignal,
        inputs: DependencyValues<'_, RuntimeValue>,
    ) -> Result<DerivedSignalUpdate<RuntimeValue>, BackendRuntimeError> {
        let mut globals = self.committed_signals.clone();
        for (index, &dep_item) in binding.dependency_items.iter().enumerate() {
            if let Some(value) = inputs.value(index) {
                globals.insert(dep_item, RuntimeValue::Signal(Box::new(value.clone())));
            }
        }

        let wakeup_fired = inputs.updated(binding.wakeup_dependency_index);

        let previous = self.committed_signals.get(&binding.backend_item).cloned();

        let mut evaluator = KernelEvaluator::new(self.backend);

        if previous.is_none() {
            // First tick: evaluate the seed kernel (no input subject).
            let seed_value = evaluate_kernel_coercing_zero_arity(
                &mut evaluator,
                binding.seed_kernel,
                None,
                &globals,
            )
            .map_err(|error| BackendRuntimeError::EvaluateRecurrenceSignal {
                signal,
                item: binding.item,
                error,
            })?;
            return Ok(DerivedSignalUpdate::Value(seed_value));
        }

        if !wakeup_fired {
            // Non-wakeup dependencies (for example a separate direction signal captured
            // by an accumulate step) may change between firings. Preserve the last committed
            // accumulator snapshot until the wakeup dependency actually fires.
            return Ok(DerivedSignalUpdate::Unchanged);
        }

        // Wakeup fired: evaluate step kernel with previous value as subject.
        let prev_value = previous.unwrap();
        let actual_prev = match &prev_value {
            RuntimeValue::Signal(inner) => inner.as_ref(),
            other => other,
        };
        let mut result = actual_prev.clone();
        for &step_kernel in binding.step_kernels.iter() {
            result = evaluator
                .evaluate_kernel(step_kernel, Some(&result), &[], &globals)
                .map_err(|error| BackendRuntimeError::EvaluateRecurrenceSignal {
                    signal,
                    item: binding.item,
                    error,
                })?;
        }

        Ok(DerivedSignalUpdate::Value(result))
    }
}

/// Evaluate a kernel, applying zero-arity sum constructors when the kernel returns a Callable
/// where a fully-applied Sum value is expected.
///
/// This is needed for seed expressions that are zero-arity sum constructors (e.g., `Direction.Right`).
/// The kernel evaluator always returns `Callable(SumConstructor)` for constructor references, even
/// when the constructor has no fields. This helper detects that case and applies the constructor to
/// produce the expected `Sum` value.
fn evaluate_kernel_coercing_zero_arity(
    evaluator: &mut KernelEvaluator<'_>,
    kernel_id: KernelId,
    input_subject: Option<&RuntimeValue>,
    globals: &std::collections::BTreeMap<BackendItemId, RuntimeValue>,
) -> Result<RuntimeValue, EvaluationError> {
    match evaluator.evaluate_kernel(kernel_id, input_subject, &[], globals) {
        Ok(value) => Ok(value),
        Err(EvaluationError::KernelResultLayoutMismatch {
            expected, found, ..
        }) => {
            // If the value is a zero-arity sum constructor callable, apply it to get
            // the actual Sum value.
            if let RuntimeValue::Callable(RuntimeCallable::SumConstructor {
                ref handle,
                ref bound_arguments,
            }) = found
            {
                if handle.field_count == 0 && bound_arguments.is_empty() {
                    return Ok(RuntimeValue::Sum(RuntimeSumValue {
                        item: handle.item,
                        type_name: handle.type_name.clone(),
                        variant_name: handle.variant_name.clone(),
                        fields: Vec::new(),
                    }));
                }
            }
            Err(EvaluationError::KernelResultLayoutMismatch {
                kernel: kernel_id,
                expected,
                found,
            })
        }
        Err(other) => Err(other),
    }
}

struct LinkArtifacts {
    signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    db_changed_routes: Box<[LinkedDbChangedRoute]>,
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
    reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    db_changed_routes: Vec<LinkedDbChangedRoute>,
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
            reactive_signals: BTreeMap::new(),
            reactive_clauses: BTreeMap::new(),
            linked_recurrence_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            task_bindings: BTreeMap::new(),
            db_changed_routes: Vec::new(),
        }
    }

    fn build(&mut self) -> Result<LinkArtifacts, BackendRuntimeLinkErrors> {
        self.index_origins();
        self.index_signal_items();
        self.link_sources();
        self.link_tasks();
        self.link_reactive_signals();
        self.link_db_changed_routes();
        self.link_derived_signals();
        if self.errors.is_empty() {
            Ok(LinkArtifacts {
                signal_items_by_handle: std::mem::take(&mut self.signal_items_by_handle),
                runtime_signal_by_item: std::mem::take(&mut self.runtime_signal_by_item),
                derived_signals: std::mem::take(&mut self.derived_signals),
                reactive_signals: std::mem::take(&mut self.reactive_signals),
                reactive_clauses: std::mem::take(&mut self.reactive_clauses),
                linked_recurrence_signals: std::mem::take(&mut self.linked_recurrence_signals),
                source_bindings: std::mem::take(&mut self.source_bindings),
                task_bindings: std::mem::take(&mut self.task_bindings),
                db_changed_routes: std::mem::take(&mut self.db_changed_routes).into_boxed_slice(),
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
                .filter(|option| {
                    !matches!(
                        option.option_name.as_ref(),
                        "decode" | "refreshOn" | "reloadOn" | "activeWhen"
                    )
                })
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
            debug_assert!(
                item.parameters.is_empty(),
                "runtime task bindings currently originate from parameterless top-level values"
            );
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

    fn link_reactive_signals(&mut self) {
        for binding in self.assembly.signals() {
            let Some(reactive) = binding.reactive_signal() else {
                continue;
            };
            let Some(&backend_item) = self.hir_to_backend.get(&binding.item) else {
                self.errors
                    .push(BackendRuntimeLinkError::MissingBackendItem { item: binding.item });
                continue;
            };
            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(_) = &item.kind else {
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
            let pipeline_ids = item
                .pipelines
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let has_seed_body = item.body.is_some();
            let pipeline_signals = self
                .collect_pipeline_signal_handles(binding.item, pipeline_ids.as_ref())
                .into_boxed_slice();
            self.reactive_signals.insert(
                reactive,
                LinkedReactiveSignal {
                    item: binding.item,
                    signal: reactive,
                    backend_item,
                    has_seed_body,
                    pipeline_signals,
                    pipeline_ids: pipeline_ids.clone(),
                },
            );
            for clause in binding.reactive_updates() {
                self.reactive_clauses.insert(
                    clause.clause,
                    LinkedReactiveClause {
                        owner: binding.item,
                        target: reactive,
                        clause: clause.clause,
                        pipeline_ids: pipeline_ids.clone(),
                        body_mode: clause.body_mode,
                        compiled_guard: clause.compiled_guard.clone(),
                        compiled_body: clause.compiled_body.clone(),
                    },
                );
            }
        }
    }

    fn link_db_changed_routes(&mut self) {
        for binding in self.assembly.db_changed_bindings() {
            let Some(changed_input) = self
                .assembly
                .signal(binding.changed_signal)
                .and_then(|signal| signal.input())
            else {
                continue;
            };
            let table = if let Some(signal) = self.assembly.signal(binding.table_item) {
                LinkedDbChangedRouteTable::Signal {
                    signal: signal.signal(),
                }
            } else {
                let Some(&backend_item) = self.hir_to_backend.get(&binding.table_item) else {
                    continue;
                };
                let Some(item) = self.backend.items().get(backend_item) else {
                    continue;
                };
                let Some(body) = item.body else {
                    continue;
                };
                if !item.parameters.is_empty() {
                    continue;
                }
                LinkedDbChangedRouteTable::Value {
                    owner: binding.table_item,
                    backend_item,
                    required_signals: self.collect_required_signal_items(binding.table_item, body),
                    changed_signal_item: self.hir_to_backend.get(&binding.changed_signal).copied(),
                }
            };
            self.db_changed_routes.push(LinkedDbChangedRoute {
                changed_input,
                table,
            });
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
            let item = &self.backend.items()[backend_item];
            let BackendItemKind::Signal(info) = &item.kind else {
                self.errors
                    .push(BackendRuntimeLinkError::BackendItemNotSignal {
                        item: binding.item,
                        backend_item,
                    });
                continue;
            };
            if let Some(pipeline_id) = item
                .pipelines
                .iter()
                .copied()
                .find(|&pid| self.backend.pipelines()[pid].recurrence.is_some())
            {
                let Some(recurrence_binding) = self
                    .assembly
                    .recurrences()
                    .iter()
                    .find(|recurrence| recurrence.site.owner == binding.item)
                else {
                    self.errors
                        .push(BackendRuntimeLinkError::SignalPipelinesNotYetLinked {
                            item: binding.item,
                            count: item.pipelines.len(),
                        });
                    continue;
                };
                let Some(wakeup_dependency_index) =
                    self.recurrence_wakeup_dependency_index(binding, &recurrence_binding.plan)
                else {
                    self.errors
                        .push(BackendRuntimeLinkError::MissingRecurrenceWakeupDependency {
                            item: binding.item,
                        });
                    continue;
                };

                let recurrence = self.backend.pipelines()[pipeline_id]
                    .recurrence
                    .as_ref()
                    .expect("selected recurrence pipeline should carry recurrence metadata");
                let seed_kernel = recurrence.seed;
                let mut step_kernels = Vec::with_capacity(1 + recurrence.steps.len());
                step_kernels.push(recurrence.start.kernel);
                step_kernels.extend(recurrence.steps.iter().map(|step| step.kernel));
                let step_kernels = step_kernels.into_boxed_slice();
                let all_kernels: Vec<KernelId> = std::iter::once(seed_kernel)
                    .chain(step_kernels.iter().copied())
                    .collect();
                let required = self.collect_recurrence_signal_items(binding.item, &all_kernels);
                let dependency_items = binding
                    .dependencies()
                    .iter()
                    .filter_map(|signal| self.signal_items_by_handle.get(signal).copied())
                    .collect::<Vec<_>>();
                if !same_items(&required, &dependency_items) {
                    self.errors
                        .push(BackendRuntimeLinkError::SignalRequirementMismatch {
                            item: binding.item,
                            declared: dependency_items.clone().into_boxed_slice(),
                            required,
                        });
                    continue;
                }

                self.linked_recurrence_signals.insert(
                    derived,
                    LinkedRecurrenceSignal {
                        item: binding.item,
                        signal: derived,
                        backend_item,
                        wakeup_dependency_index,
                        seed_kernel,
                        step_kernels,
                        dependency_items: dependency_items.into_boxed_slice(),
                        pipeline_ids: item
                            .pipelines
                            .iter()
                            .copied()
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    },
                );
                continue;
            }
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
                .collect::<Vec<_>>();
            let mut expected_runtime_dependencies = backend_dependencies.clone();
            if let Some(source_input) = binding.source_input {
                expected_runtime_dependencies.push(source_input.as_signal());
            }
            if expected_runtime_dependencies.as_slice() != binding.dependencies() {
                self.errors
                    .push(BackendRuntimeLinkError::SignalDependencyMismatch {
                        item: binding.item,
                        runtime: binding.dependencies().to_vec().into_boxed_slice(),
                        backend: expected_runtime_dependencies.into_boxed_slice(),
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
                    source_input: binding.source_input,
                    pipeline_ids: item
                        .pipelines
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                },
            );
        }
    }

    fn recurrence_wakeup_dependency_index(
        &self,
        binding: &crate::hir_adapter::HirSignalBinding,
        plan: &hir::RecurrenceNodePlan,
    ) -> Option<usize> {
        if let Some(wakeup_signal) = plan.wakeup_signal {
            let wakeup_handle = self.assembly.signal(wakeup_signal)?.signal();
            return binding
                .dependencies()
                .iter()
                .position(|&signal| signal == wakeup_handle);
        }
        let source_input = binding.source_input?;
        binding
            .dependencies()
            .iter()
            .position(|&signal| signal == source_input.as_signal())
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

    fn collect_recurrence_signal_items(
        &mut self,
        owner: hir::ItemId,
        kernels: &[KernelId],
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernel_queue = kernels.to_vec();
        let mut visited_items = BTreeSet::new();
        while let Some(kernel_id) = kernel_queue.pop() {
            let kernel = &self.backend.kernels()[kernel_id];
            for &item_id in &kernel.global_items {
                if !visited_items.insert(item_id) {
                    continue;
                }
                let item = &self.backend.items()[item_id];
                match item.kind {
                    BackendItemKind::Signal(_) => {
                        required.insert(item_id);
                    }
                    _ => {
                        if let Some(body) = item.body {
                            kernel_queue.push(body);
                        } else {
                            self.errors
                                .push(BackendRuntimeLinkError::MissingItemBodyForGlobal {
                                    owner,
                                    item: item_id,
                                });
                        }
                    }
                }
            }
        }
        required.into_iter().collect::<Vec<_>>().into_boxed_slice()
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

    fn collect_required_signal_items_for_kernels(
        &mut self,
        owner: hir::ItemId,
        kernels: Vec<KernelId>,
    ) -> Box<[BackendItemId]> {
        let mut required = BTreeSet::new();
        let mut kernels = kernels;
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

    fn collect_pipeline_signal_handles(
        &mut self,
        owner: hir::ItemId,
        pipeline_ids: &[BackendPipelineId],
    ) -> Vec<SignalHandle> {
        let mut kernels = Vec::new();
        for &pipeline_id in pipeline_ids {
            let pipeline = &self.backend.pipelines()[pipeline_id];
            for stage in &pipeline.stages {
                match &stage.kind {
                    BackendStageKind::Gate(BackendGateStage::Ordinary {
                        when_true,
                        when_false,
                    }) => {
                        kernels.push(*when_true);
                        kernels.push(*when_false);
                    }
                    BackendStageKind::Gate(BackendGateStage::SignalFilter {
                        predicate, ..
                    }) => {
                        kernels.push(*predicate);
                    }
                    BackendStageKind::Fanout(fanout) => {
                        kernels.push(fanout.map);
                        kernels.extend(fanout.filters.iter().map(|filter| filter.predicate));
                        if let Some(join) = &fanout.join {
                            kernels.push(join.kernel);
                        }
                    }
                    BackendStageKind::TruthyFalsy(_) => {}
                }
            }
        }
        self.collect_required_signal_items_for_kernels(owner, kernels)
            .iter()
            .filter_map(|dependency| self.runtime_signal_for_backend_item(owner, *dependency))
            .collect()
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
                            BackendStageKind::TruthyFalsy(_)
                                | BackendStageKind::Gate(BackendGateStage::SignalFilter { .. })
                                | BackendStageKind::Fanout(_)
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
            reactive_signals: BTreeMap::new(),
            reactive_clauses: BTreeMap::new(),
            linked_recurrence_signals: BTreeMap::new(),
            source_bindings: BTreeMap::new(),
            task_bindings: BTreeMap::from([(instance, binding)]),
            db_changed_routes: Vec::new().into_boxed_slice(),
            db_commit_invalidation_sink: None,
        }
    }

    fn signal_handle(
        linked: &BackendLinkedRuntime,
        module: &hir::Module,
        name: &str,
    ) -> SignalHandle {
        linked
            .assembly()
            .signal(item_id(module, name))
            .unwrap_or_else(|| panic!("signal binding should exist for {name}"))
            .signal()
    }

    fn activation_port_for_owner(
        linked: &BackendLinkedRuntime,
        module: &hir::Module,
        outcome: &LinkedSourceTickOutcome,
        owner_name: &str,
    ) -> DetachedRuntimePublicationPort {
        let instance = linked
            .source_by_owner(item_id(module, owner_name))
            .unwrap_or_else(|| panic!("source binding should exist for {owner_name}"))
            .instance;
        outcome
            .source_actions()
            .iter()
            .find_map(|action| match action {
                LinkedSourceLifecycleAction::Activate {
                    instance: action_instance,
                    port,
                    ..
                } if *action_instance == instance => Some(port.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected activation for source {owner_name}"))
    }

    fn user_value(active: bool, email: &str) -> DetachedRuntimeValue {
        DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Record(vec![
            aivi_backend::RuntimeRecordField {
                label: "active".into(),
                value: RuntimeValue::Bool(active),
            },
            aivi_backend::RuntimeRecordField {
                label: "email".into(),
                value: RuntimeValue::Text(email.into()),
            },
        ]))
    }

    #[test]
    fn linked_runtime_ticks_simple_signals_and_evaluates_source_config() {
        let lowered = lower_text(
            "runtime-startup-basic.aivi",
            r#"
value prefix = "https://example.com/"

signal id = 7
signal next = id + 1
signal enabled = True
signal label = "Ada"

@source http.get "{prefix}{id}" with {
    activeWhen: enabled
}
signal users : Signal Text
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
        assert!(
            config.options.is_empty(),
            "scheduler-owned lifecycle options should not leak into provider config"
        );
    }

    #[test]
    fn linked_runtime_relocates_committed_signal_values_between_ticks() {
        let lowered = lower_text(
            "runtime-startup-moving-gc.aivi",
            r#"
signal label = "Ada"
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
signal apiHost : Signal Text

@source http.get "{apiHost}/users"
signal users : Signal Text
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
signal apiHost : Signal Text

@source http.get "{apiHost}/users"
signal users : Signal Text
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
value answer = 42
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
    fn linked_runtime_task_workers_execute_runtime_task_plans_before_publication() {
        let lowered = lower_text(
            "runtime-startup-manual-task-plan-success.aivi",
            r#"
value answer : Task Text Int = pure 42
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
value total:Int = 1 / 0
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
            Err(LinkedTaskWorkerError::Evaluation { instance, owner, .. })
                if instance == binding.instance && owner == binding.owner
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
value answer = 42
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
    literal times : Int -> Retry

fun step:Int n:Int =>
    n

@recur.backoff 3times
value retried : Task Int Int =
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
fun choose:Text maybeName:(Option Text) =>
    maybeName
     ||> Some name -> name
     ||> None -> "guest"

signal maybeName = Some "Ada"
signal label = choose maybeName
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
fun greetSelected:Signal Text prefix:Text fallback:Text selected:Signal (Option Text) =>
    selected
     ||> Some name -> "{prefix}:{name}"
     ||> None -> "{prefix}:{fallback}"

signal selectedUser : Signal (Option Text) = Some "Ada"

signal greeting : Signal Text =
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
fun renderStatus:Signal Text prefix:Text readyText:Text waitText:Text statusReady:Signal Bool =>
    statusReady
     T|> "{prefix}:{readyText}"
     F|> "{prefix}:{waitText}"

signal ready : Signal Bool = True

signal status : Signal Text =
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

signal ready : Signal Bool = True

signal maybeUser : Signal (Option User) = Some { name: "Ada" }

signal loaded : Signal (Result LoadError User) = Err { message: "offline" }

signal submitted : Signal (Validation FormError User) = Valid { name: "Grace" }

signal readyText : Signal Text =
    ready
     T|> "start"
     F|> "wait"

signal greeting : Signal Text =
    maybeUser
     T|> .name
     F|> "guest"

signal rendered : Signal Text =
    loaded
     T|> .name
     F|> .message

signal summary : Signal Text =
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

signal session : Signal Session = { user: { name: "Ada" } }

signal label : Signal Text =
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

signal current : Signal Status =
    Failed "503" "offline"

signal label : Signal Text =
    current
     ||> Idle -> "idle"
     ||> Ready name -> name
     ||> Failed code message -> "{code}:{message}"
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

value seed:User = { active: True, email: "ada@example.com" }

signal sessions : Signal Session = { user: seed }

signal activeUsers : Signal User =
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
    fn linked_runtime_executes_signal_filter_gate_pipelines_without_prefix_body() {
        let lowered = lower_text(
            "runtime-startup-direct-signal-gate-head-only.aivi",
            r#"
type User = {
    active: Bool,
    email: Text
}

signal users : Signal User = { active: True, email: "ada@example.com" }

signal activeUsers : Signal User =
    users
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
        .expect("head-only signal filter pipelines should now link successfully");

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
    fn linked_runtime_executes_signal_fanout_map_and_join_pipelines() {
        let lowered = lower_text(
            "runtime-startup-signal-fanout.aivi",
            r#"
type User = {
    active: Bool,
    email: Text
}

fun joinEmails:Text items:List Text =>
    "joined"

signal liveUsers : Signal (List User) = [
    { active: True, email: "ada@example.com" }
]

signal liveEmails : Signal (List Text) =
    liveUsers
     *|> .email

signal liveJoinedEmails : Signal Text =
    liveUsers
     *|> .email
     <|* joinEmails
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("signal fanout pipelines should now link successfully");

        let outcome = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");

        let live_emails_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "liveEmails"))
            .expect("liveEmails signal binding should exist")
            .signal();
        let live_joined_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "liveJoinedEmails"))
            .expect("liveJoinedEmails signal binding should exist")
            .signal();

        assert_eq!(
            linked.runtime().current_value(live_emails_signal).unwrap(),
            Some(&RuntimeValue::List(vec![RuntimeValue::Text(
                "ada@example.com".into()
            )]))
        );
        assert_eq!(
            linked.runtime().current_value(live_joined_signal).unwrap(),
            Some(&RuntimeValue::Text("joined".into()))
        );
        assert!(outcome.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_links_source_backed_body_signals_without_recurrence() {
        let lowered = lower_text(
            "runtime-startup-source-body-trigger.aivi",
            r#"
provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal status : Signal Text =
    "ready"
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("source-backed body signals should now link successfully");

        let first = linked
            .tick_with_source_lifecycle()
            .expect("source activation tick should succeed");
        assert_eq!(first.source_actions().len(), 1);
        let port = match &first.source_actions()[0] {
            LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
            _ => panic!("expected source activation"),
        };
        let status_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "status"))
            .expect("status signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(status_signal).unwrap(),
            Some(&RuntimeValue::Text("ready".into())),
            "source-backed body signals should commit their body immediately"
        );

        port.publish(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Text("ignored".into()),
        ))
        .expect("source publication should queue");
        let second = linked
            .tick_with_source_lifecycle()
            .expect("source publication tick should succeed");
        assert_eq!(
            linked.runtime().current_value(status_signal).unwrap(),
            Some(&RuntimeValue::Text("ready".into()))
        );
        assert!(second.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_executes_reactive_when_clauses_end_to_end() {
        let lowered = lower_text(
            "runtime-startup-reactive-when.aivi",
            r#"
provider custom.ready
    wakeup: providerTrigger

provider custom.enabled
    wakeup: providerTrigger

@source custom.ready
signal ready : Signal Bool

@source custom.enabled
signal enabled : Signal Bool

signal left = 20
signal right = 22
signal total : Signal Int = 0

when ready => total <- left + right
when ready and enabled => total <- left + right + 1
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("reactive when clauses should now link successfully");

        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial reactive tick should succeed");
        assert_eq!(first.source_actions().len(), 2);
        let ready_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "ready");
        let enabled_port =
            activation_port_for_owner(&linked, lowered.hir.module(), &first, "enabled");
        let total_signal = signal_handle(&linked, lowered.hir.module(), "total");
        assert_eq!(
            linked.runtime().current_value(total_signal).unwrap(),
            Some(&RuntimeValue::Int(0)),
            "reactive signals should commit their seed body before any when clause fires"
        );

        ready_port
            .publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(true),
            ))
            .expect("ready publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("ready-only reactive tick should succeed");
        assert_eq!(
            linked.runtime().current_value(total_signal).unwrap(),
            Some(&RuntimeValue::Int(42)),
            "the first when clause should fire once ready becomes true"
        );

        enabled_port
            .publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(true),
            ))
            .expect("enabled publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("overlapping reactive tick should succeed");
        assert_eq!(
            linked.runtime().current_value(total_signal).unwrap(),
            Some(&RuntimeValue::Int(43)),
            "later firing when clauses should win by source order"
        );

        ready_port
            .publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(false),
            ))
            .expect("ready reset should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("guard-false reactive tick should succeed");
        assert_eq!(
            linked.runtime().current_value(total_signal).unwrap(),
            Some(&RuntimeValue::Int(43)),
            "false guards should preserve the previously committed reactive value"
        );
    }

    #[test]
    fn linked_runtime_executes_pattern_armed_reactive_updates_end_to_end() {
        let lowered = lower_text(
            "runtime-startup-pattern-reactive-when.aivi",
            r#"
type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down
signal heading : Signal Direction = Up
signal tickSeen : Signal Bool = False

when event
  ||> Turn dir => heading <- dir
  ||> Tick => tickSeen <- True
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build for pattern-armed reactive updates");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("pattern-armed reactive updates should link successfully");

        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial pattern-armed reactive tick should succeed");
        assert!(
            first.source_actions().is_empty(),
            "constant pattern-armed fixture should not request source actions"
        );

        let heading_signal = signal_handle(&linked, lowered.hir.module(), "heading");
        let tick_seen_signal = signal_handle(&linked, lowered.hir.module(), "tickSeen");
        let Some(RuntimeValue::Sum(heading)) =
            linked.runtime().current_value(heading_signal).unwrap()
        else {
            panic!("heading signal should hold a sum value after the reactive tick");
        };
        assert_eq!(&*heading.type_name, "Direction");
        assert_eq!(&*heading.variant_name, "Down");
        assert!(heading.fields.is_empty());
        assert_eq!(
            linked.runtime().current_value(tick_seen_signal).unwrap(),
            Some(&RuntimeValue::Bool(false)),
            "non-matching pattern arms should leave the other target untouched"
        );
    }

    #[test]
    fn linked_runtime_applies_target_pipelines_to_reactive_when_bodies() {
        let lowered = lower_text(
            "runtime-startup-reactive-when-pipeline.aivi",
            r#"
provider custom.ready
    wakeup: providerTrigger

provider custom.user
    wakeup: providerTrigger

type User = {
    active: Bool,
    email: Text
}

@source custom.ready
signal ready : Signal Bool

@source custom.user
signal incoming : Signal User

signal seed : Signal User = { active: True, email: "seed@example.com" }

signal current : Signal User =
    seed
     ?|> .active

when ready => current <- incoming
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("reactive when targets with supported pipelines should link successfully");

        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial reactive pipeline tick should succeed");
        assert_eq!(first.source_actions().len(), 2);
        let ready_port = activation_port_for_owner(&linked, lowered.hir.module(), &first, "ready");
        let incoming_port =
            activation_port_for_owner(&linked, lowered.hir.module(), &first, "incoming");
        let current_signal = signal_handle(&linked, lowered.hir.module(), "current");
        assert_eq!(
            linked.runtime().current_value(current_signal).unwrap(),
            Some(&RuntimeValue::Record(vec![
                aivi_backend::RuntimeRecordField {
                    label: "active".into(),
                    value: RuntimeValue::Bool(true),
                },
                aivi_backend::RuntimeRecordField {
                    label: "email".into(),
                    value: RuntimeValue::Text("seed@example.com".into()),
                },
            ])),
            "the seed body should still flow through the target pipeline"
        );

        ready_port
            .publish(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Bool(true),
            ))
            .expect("ready publication should queue");
        incoming_port
            .publish(user_value(false, "inactive@example.com"))
            .expect("inactive user publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("inactive reactive pipeline tick should succeed");
        assert!(
            linked
                .runtime()
                .current_value(current_signal)
                .unwrap()
                .is_none(),
            "reactive bodies should still run through the target signal pipeline"
        );

        incoming_port
            .publish(user_value(true, "active@example.com"))
            .expect("active user publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("active reactive pipeline tick should succeed");
        assert_eq!(
            linked.runtime().current_value(current_signal).unwrap(),
            Some(&RuntimeValue::Record(vec![
                aivi_backend::RuntimeRecordField {
                    label: "active".into(),
                    value: RuntimeValue::Bool(true),
                },
                aivi_backend::RuntimeRecordField {
                    label: "email".into(),
                    value: RuntimeValue::Text("active@example.com".into()),
                },
            ])),
            "reactive bodies should commit values that satisfy the target pipeline"
        );
    }

    #[test]
    fn linked_runtime_applies_all_source_recurrence_steps() {
        let lowered = lower_text(
            "runtime-startup-source-recurrence-steps.aivi",
            r#"
fun bump:Int n:Int =>
    n + 1

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal counter : Signal Int =
    0
     @|> bump
     <|@ bump
     <|@ bump
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("source-backed recurrences should now link successfully");

        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial recurrence tick should succeed");
        assert_eq!(first.source_actions().len(), 1);
        let port = match &first.source_actions()[0] {
            LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
            _ => panic!("expected source activation"),
        };
        let counter_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "counter"))
            .expect("counter signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(counter_signal).unwrap(),
            Some(&RuntimeValue::Int(0))
        );

        port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            1,
        )))
        .expect("source publication should queue");
        let second = linked
            .tick_with_source_lifecycle()
            .expect("recurrence publication tick should succeed");
        assert_eq!(
            linked.runtime().current_value(counter_signal).unwrap(),
            Some(&RuntimeValue::Int(3))
        );
        assert!(second.source_actions().is_empty());
    }

    #[test]
    fn linked_runtime_task_values_lower_to_zero_parameter_backend_items() {
        let lowered = lower_text(
            "runtime-startup-task-parameters-invariant.aivi",
            r#"
domain Retry over Int
    literal times : Int -> Retry

fun keep:Int n:Int =>
    n

@recur.backoff 3times
value retried : Task Int Int =
    0
     @|> keep
     <|@ keep
"#,
        );
        let backend_item = backend_item_id(&lowered.backend, "retried");
        assert!(
            lowered.backend.items()[backend_item].parameters.is_empty(),
            "startup-linked tasks currently come from parameterless top-level values"
        );
    }

    #[test]
    fn linked_runtime_applies_accumulate_steps_once_per_wakeup() {
        let lowered = lower_text(
            "runtime-startup-accumulate-signal.aivi",
            r#"
fun step:Int next:Int current:Int =>
    current + next

provider custom.feed
    wakeup: providerTrigger

@source custom.feed
signal next : Signal Int

signal counter : Signal Int =
    next
     +|> 0 step
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("accumulate signals should now link successfully");
        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial accumulate tick should succeed");
        assert_eq!(first.source_actions().len(), 1);
        let port = match &first.source_actions()[0] {
            LinkedSourceLifecycleAction::Activate { port, .. } => port.clone(),
            _ => panic!("expected source activation"),
        };
        let counter_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "counter"))
            .expect("counter signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(counter_signal).unwrap(),
            Some(&RuntimeValue::Int(0))
        );

        port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            2,
        )))
        .expect("first source publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("first accumulate publication tick should succeed");
        assert_eq!(
            linked.runtime().current_value(counter_signal).unwrap(),
            Some(&RuntimeValue::Int(2))
        );

        port.publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            3,
        )))
        .expect("second source publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("second accumulate publication tick should succeed");
        assert_eq!(
            linked.runtime().current_value(counter_signal).unwrap(),
            Some(&RuntimeValue::Int(5))
        );
    }

    #[test]
    fn linked_runtime_keeps_recurrence_value_when_only_non_wakeup_dependencies_change() {
        let lowered = lower_text(
            "runtime-startup-recurrence-non-wakeup-deps.aivi",
            r#"
fun setDirection:Int next:Int current:Int =>
    next

fun stepGame:Int tick:Int current:Int =>
    current + direction

provider custom.turn
    wakeup: providerTrigger

provider custom.tick
    wakeup: providerTrigger

@source custom.turn
signal turn : Signal Int

signal direction : Signal Int =
    turn
     +|> 1 setDirection

@source custom.tick
signal tick : Signal Int

signal game : Signal Int =
    tick
     +|> 0 stepGame
"#,
        );
        let assembly = crate::assemble_hir_runtime(lowered.hir.module())
            .expect("runtime assembly should build");
        let mut linked = link_backend_runtime(
            assembly,
            &lowered.core,
            std::sync::Arc::new(lowered.backend.clone()),
        )
        .expect("recurrence with non-wakeup dependencies should link successfully");
        let first = linked
            .tick_with_source_lifecycle()
            .expect("initial recurrence tick should succeed");
        assert_eq!(first.source_actions().len(), 2);
        let turn_instance = linked
            .source_by_owner(item_id(lowered.hir.module(), "turn"))
            .expect("turn source binding should exist")
            .instance;
        let tick_instance = linked
            .source_by_owner(item_id(lowered.hir.module(), "tick"))
            .expect("tick source binding should exist")
            .instance;
        let mut turn_port = None;
        let mut tick_port = None;
        for action in first.source_actions() {
            let LinkedSourceLifecycleAction::Activate {
                instance,
                port,
                config: _,
            } = action
            else {
                panic!("initial source lifecycle should only activate providers");
            };
            match instance {
                instance if *instance == turn_instance => {
                    turn_port = Some(port.clone());
                }
                instance if *instance == tick_instance => {
                    tick_port = Some(port.clone());
                }
                _ => {}
            }
        }
        let turn_port = turn_port.expect("turn source should activate");
        let tick_port = tick_port.expect("tick source should activate");
        let game_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "game"))
            .expect("game signal binding should exist")
            .signal();
        let direction_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "direction"))
            .expect("direction signal binding should exist")
            .signal();
        assert_eq!(
            linked.runtime().current_value(game_signal).unwrap(),
            Some(&RuntimeValue::Int(0))
        );
        assert_eq!(
            linked.runtime().current_value(direction_signal).unwrap(),
            Some(&RuntimeValue::Int(1))
        );

        turn_port
            .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
                5,
            )))
            .expect("direction publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("direction-only tick should succeed");
        assert_eq!(
            linked.runtime().current_value(direction_signal).unwrap(),
            Some(&RuntimeValue::Int(5))
        );
        assert_eq!(
            linked.runtime().current_value(game_signal).unwrap(),
            Some(&RuntimeValue::Int(0)),
            "non-wakeup dependency changes must preserve the current recurrence snapshot"
        );

        tick_port
            .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
                1,
            )))
            .expect("tick publication should queue");
        linked
            .tick_with_source_lifecycle()
            .expect("tick publication should advance the recurrence");
        assert_eq!(
            linked.runtime().current_value(game_signal).unwrap(),
            Some(&RuntimeValue::Int(5)),
            "the next wakeup should apply the latest direction"
        );
    }
}
