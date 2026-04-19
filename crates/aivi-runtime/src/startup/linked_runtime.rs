impl BackendLinkedRuntime {
    pub fn assembly(&self) -> &HirRuntimeAssembly {
        &self.assembly
    }

    pub fn backend(&self) -> &BackendProgram {
        self.backend
            .as_program()
            .expect("full backend program is unavailable in frozen bundle runtimes")
            .as_ref()
    }

    pub fn backend_arc(&self) -> Arc<BackendProgram> {
        self.backend
            .as_program()
            .expect("full backend program is unavailable in frozen bundle runtimes")
            .clone()
    }

    pub fn backend_payload(&self) -> &BackendRuntimePayload {
        &self.backend
    }

    pub fn backend_view(&self) -> BackendRuntimeView<'_> {
        self.backend.runtime_view()
    }

    fn executable_program(&self) -> aivi_backend::BackendExecutableProgram<'_> {
        self.backend
            .executable_program(self.native_kernels.as_ref())
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

    pub fn set_execution_context(&mut self, context: SourceProviderContext) {
        self.execution_context = context;
    }

    pub fn signal_graph(&self) -> &crate::SignalGraph {
        self.runtime.graph()
    }

    pub fn reactive_program(&self) -> &crate::ReactiveProgram {
        self.assembly.reactive_program()
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

    /// Build a [`RuntimeSourceMap`] for error rendering, enriched with
    /// pipeline IDs from the linked runtime.
    pub fn build_source_map(&self) -> crate::source_map::RuntimeSourceMap {
        let mut map = crate::source_map::RuntimeSourceMap::from_assembly(&self.assembly);

        // Enrich with pipeline IDs from derived signals.
        for (handle, linked) in &self.derived_signals {
            map.set_signal_pipeline_ids(handle.as_signal(), linked.pipeline_ids.clone());
        }
        // Enrich with pipeline IDs from reactive signals.
        for linked in self.reactive_signals.values() {
            map.set_signal_pipeline_ids(linked.signal, linked.pipeline_ids.clone());
        }
        // Enrich with pipeline IDs from recurrence-backed derived signals.
        for linked in self.linked_recurrence_signals.values() {
            map.set_signal_pipeline_ids(linked.signal.as_signal(), linked.pipeline_ids.clone());
        }

        map
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
        let value = self
            .executable_program()
            .with_execution_options(aivi_backend::BackendExecutionOptions {
                prefer_interpreter: true,
                ..Default::default()
            })
            .create_engine()
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
        let mut temporal_states = self.temporal_states.clone();
        let mut pending_temporal_schedules = Vec::new();
        let mut evaluator = LinkedDerivedEvaluator {
            backend: &self.backend,
            native_kernels: self.native_kernels.as_ref(),
            signal_items_by_handle: &self.signal_items_by_handle,
            derived_signals: &self.derived_signals,
            reactive_signals: &self.reactive_signals,
            reactive_clauses: &self.reactive_clauses,
            linked_recurrence_signals: &self.linked_recurrence_signals,
            committed_signals: &runtime_committed,
            temporal_states: &mut temporal_states,
            pending_temporal_schedules: &mut pending_temporal_schedules,
        };
        let reactive_program = self.assembly.reactive_program();
        let outcome = if reactive_program.signal_count() == self.runtime.graph().signal_count() {
            self.runtime
                .try_tick_with_reactive_program(reactive_program, &mut evaluator)?
        } else {
            self.runtime.try_tick(&mut evaluator)?
        };
        if self.temporal_triggers_bootstrapped {
            let committed = self.committed_signal_snapshots()?;
            let runtime_committed = materialize_detached_globals(&committed);
            let mut trigger_evaluator = LinkedDerivedEvaluator {
                backend: &self.backend,
                native_kernels: self.native_kernels.as_ref(),
                signal_items_by_handle: &self.signal_items_by_handle,
                derived_signals: &self.derived_signals,
                reactive_signals: &self.reactive_signals,
                reactive_clauses: &self.reactive_clauses,
                linked_recurrence_signals: &self.linked_recurrence_signals,
                committed_signals: &runtime_committed,
                temporal_states: &mut temporal_states,
                pending_temporal_schedules: &mut pending_temporal_schedules,
            };
            trigger_evaluator.schedule_temporal_triggered_signals(outcome.committed())?;
        } else {
            self.temporal_triggers_bootstrapped = true;
        }
        self.temporal_states = temporal_states;
        self.arm_pending_temporal_schedules(pending_temporal_schedules)?;
        Ok(outcome)
    }

    fn arm_pending_temporal_schedules(
        &mut self,
        schedules: Vec<PendingTemporalSchedule>,
    ) -> Result<(), BackendRuntimeError> {
        for schedule in schedules {
            let binding = self.derived_signals.get(&schedule.signal).ok_or(
                BackendRuntimeError::UnknownDerivedSignal {
                    signal: schedule.signal,
                },
            )?;
            let helper = binding.temporal_helper(schedule.key).ok_or(
                BackendRuntimeError::MissingTemporalHelper {
                    signal: schedule.signal,
                    item: schedule.item,
                    pipeline: schedule.key.pipeline,
                    stage_index: schedule.key.stage_index,
                },
            )?;
            let stamp = self.runtime.advance_generation(helper.input)?;
            let worker =
                self.ensure_temporal_worker(schedule.signal, schedule.item, schedule.key)?;
            worker
                .schedule(TemporalWorkerSchedule {
                    stamp,
                    value: schedule.value,
                    kind: schedule.kind,
                })
                .map_err(|_| BackendRuntimeError::SpawnTemporalWorker {
                    signal: schedule.signal,
                    item: schedule.item,
                    pipeline: schedule.key.pipeline,
                    stage_index: schedule.key.stage_index,
                    message: "temporal worker command channel closed".into(),
                })?;
        }
        Ok(())
    }

    fn ensure_temporal_worker(
        &mut self,
        signal: DerivedHandle,
        item: hir::ItemId,
        key: TemporalStageKey,
    ) -> Result<&TemporalWorkerHandle, BackendRuntimeError> {
        if !self.temporal_workers.contains_key(&key) {
            let sender = self.runtime.worker_sender();
            let handle = spawn_temporal_worker(signal, item, key, sender).map_err(|message| {
                BackendRuntimeError::SpawnTemporalWorker {
                    signal,
                    item,
                    pipeline: key.pipeline,
                    stage_index: key.stage_index,
                    message: message.into_boxed_str(),
                }
            })?;
            self.temporal_workers.insert(key, handle);
        }
        Ok(self
            .temporal_workers
            .get(&key)
            .expect("temporal worker should exist after insertion"))
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
        let mut engine = self.executable_program().create_engine();
        let mut arguments = Vec::with_capacity(binding.arguments.len());
        for (index, argument) in binding.arguments.iter().enumerate() {
            let globals = self.required_signal_globals(
                instance,
                argument.kernel,
                &argument.required_signals,
                &snapshots,
            )?;
            let runtime_globals = materialize_detached_globals(&globals);
            let value = engine
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
            let value = engine
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
            native_kernels: self.native_kernels.clone(),
            globals,
            completion,
            db_commit_invalidation_sink: self.db_commit_invalidation_sink.clone(),
            execution_context: self.execution_context.clone(),
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
                let mut engine = self.executable_program().create_engine();
                engine.evaluate_item(*backend_item, &runtime_globals).ok()?
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

impl Drop for BackendLinkedRuntime {
    fn drop(&mut self) {
        // Remove all thread-local kernel plans for this runtime so stale compiled code cannot
        // survive into a new runtime that happens to occupy the same `BackendProgram` address.
        let program_addr = self.backend.cache_identity();
        // Also collect addresses for reactive-clause backends (which may differ from the main one).
        let mut addrs = std::collections::BTreeSet::new();
        addrs.insert(program_addr);
        // Also collect addresses for reactive-clause backends (which may differ from the main one).
        for clause in self.reactive_clauses.values() {
            addrs.insert(clause.compiled_guard.backend.cache_identity());
            addrs.insert(clause.compiled_body.backend.cache_identity());
        }
        NATIVE_KERNEL_PLAN_CACHE.with(|cell| {
            cell.borrow_mut()
                .retain(|(addr, _), _| !addrs.contains(addr));
        });
        for worker in self.temporal_workers.values_mut() {
            worker.stop();
        }
    }
}
