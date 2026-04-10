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
        temporal_states: BTreeMap::new(),
        temporal_workers: BTreeMap::new(),
        db_commit_invalidation_sink: None,
        execution_context: SourceProviderContext::current(),
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
    temporal_states: BTreeMap<TemporalStageKey, RuntimeValue>,
    temporal_workers: BTreeMap<TemporalStageKey, TemporalWorkerHandle>,
    db_commit_invalidation_sink: Option<DbCommitInvalidationSink>,
    execution_context: SourceProviderContext,
}
