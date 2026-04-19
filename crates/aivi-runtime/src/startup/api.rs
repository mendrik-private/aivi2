#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendRuntimeLinkSeed {
    pub hir_to_backend: Box<[(hir::ItemId, BackendItemId)]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendLinkedRuntimeTables {
    pub signal_items_by_handle: BTreeMap<SignalHandle, BackendItemId>,
    pub runtime_signal_by_item: BTreeMap<BackendItemId, SignalHandle>,
    pub derived_signals: BTreeMap<DerivedHandle, LinkedDerivedSignal>,
    pub reactive_signals: BTreeMap<SignalHandle, LinkedReactiveSignal>,
    pub reactive_clauses: BTreeMap<ReactiveClauseHandle, LinkedReactiveClause>,
    pub linked_recurrence_signals: BTreeMap<DerivedHandle, LinkedRecurrenceSignal>,
    pub source_bindings: BTreeMap<SourceInstanceId, LinkedSourceBinding>,
    pub task_bindings: BTreeMap<TaskInstanceId, LinkedTaskBinding>,
    pub db_changed_routes: Box<[LinkedDbChangedRoute]>,
}

pub fn derive_backend_runtime_link_seed(
    core: &core::Module,
    backend: &BackendProgram,
) -> Result<BackendRuntimeLinkSeed, BackendRuntimeLinkErrors> {
    let mut errors = Vec::new();
    let mut core_to_hir = BTreeMap::new();
    let mut hir_to_backend = BTreeMap::new();
    for (core_id, item) in core.items().iter() {
        core_to_hir.insert(core_id, item.origin);
    }
    for (backend_item, item) in backend.items().iter() {
        let Some(&hir_item) = core_to_hir.get(&item.origin) else {
            errors.push(BackendRuntimeLinkError::MissingCoreItemOrigin {
                backend_item,
                core_item: item.origin,
            });
            continue;
        };
        if let Some(previous) = hir_to_backend.insert(hir_item, backend_item) {
            errors.push(BackendRuntimeLinkError::DuplicateBackendOrigin {
                item: hir_item,
                first: previous,
                second: backend_item,
            });
        }
    }
    if errors.is_empty() {
        Ok(BackendRuntimeLinkSeed {
            hir_to_backend: hir_to_backend
                .into_iter()
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    } else {
        Err(BackendRuntimeLinkErrors::new(errors))
    }
}

pub fn link_backend_runtime(
    assembly: HirRuntimeAssembly,
    core: &core::Module,
    backend: Arc<BackendProgram>,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    let seed = derive_backend_runtime_link_seed(core, &backend)?;
    link_backend_runtime_with_seed(assembly, backend, &seed)
}

pub fn link_backend_runtime_with_seed(
    assembly: HirRuntimeAssembly,
    backend: Arc<BackendProgram>,
    seed: &BackendRuntimeLinkSeed,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    link_backend_runtime_with_seed_and_native_kernels(
        assembly,
        backend,
        std::sync::Arc::new(aivi_backend::NativeKernelArtifactSet::default()),
        seed,
    )
}

pub fn link_backend_runtime_with_seed_and_native_kernels(
    assembly: HirRuntimeAssembly,
    backend: Arc<BackendProgram>,
    native_kernels: std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
    seed: &BackendRuntimeLinkSeed,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    link_backend_runtime_with_seed_and_native_kernels_from_payload(
        assembly,
        BackendRuntimePayload::Program(backend),
        native_kernels,
        seed,
    )
}

pub fn link_backend_runtime_with_seed_and_native_kernels_from_payload(
    assembly: HirRuntimeAssembly,
    backend: BackendRuntimePayload,
    native_kernels: std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
    seed: &BackendRuntimeLinkSeed,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    let runtime = assembly
        .instantiate_runtime_with_value_store::<RuntimeValue, _>(MovingRuntimeValueStore::default())
        .map_err(|error| {
            BackendRuntimeLinkErrors::new(vec![BackendRuntimeLinkError::InstantiateRuntime {
                error,
            }])
        })?;
    let mut builder = LinkBuilder::new(&assembly, backend.runtime_view(), native_kernels.as_ref(), seed);
    let linked = builder.build()?;
    Ok(link_backend_runtime_with_tables_from_parts(
        assembly,
        runtime,
        backend,
        native_kernels,
        linked,
    ))
}

pub fn derive_backend_linked_runtime_tables_with_seed_and_native_kernels_from_payload(
    assembly: &HirRuntimeAssembly,
    backend: &BackendRuntimePayload,
    native_kernels: &std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
    seed: &BackendRuntimeLinkSeed,
) -> Result<BackendLinkedRuntimeTables, BackendRuntimeLinkErrors> {
    let mut builder = LinkBuilder::new(assembly, backend.runtime_view(), native_kernels.as_ref(), seed);
    builder.build()
}

pub fn link_backend_runtime_with_tables_and_native_kernels_from_payload(
    assembly: HirRuntimeAssembly,
    backend: BackendRuntimePayload,
    native_kernels: std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
    tables: BackendLinkedRuntimeTables,
) -> Result<BackendLinkedRuntime, BackendRuntimeLinkErrors> {
    let runtime = assembly
        .instantiate_runtime_with_value_store::<RuntimeValue, _>(MovingRuntimeValueStore::default())
        .map_err(|error| {
            BackendRuntimeLinkErrors::new(vec![BackendRuntimeLinkError::InstantiateRuntime {
                error,
            }])
        })?;
    Ok(link_backend_runtime_with_tables_from_parts(
        assembly,
        runtime,
        backend,
        native_kernels,
        tables,
    ))
}

fn link_backend_runtime_with_tables_from_parts(
    assembly: HirRuntimeAssembly,
    runtime: TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram, MovingRuntimeValueStore>,
    backend: BackendRuntimePayload,
    native_kernels: std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
    linked: BackendLinkedRuntimeTables,
) -> BackendLinkedRuntime {
    let mut linked_runtime = BackendLinkedRuntime {
        assembly,
        runtime,
        backend,
        native_kernels,
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
        temporal_triggers_bootstrapped: false,
        db_commit_invalidation_sink: None,
        execution_context: SourceProviderContext::current(),
    };
    linked_runtime.prime_db_changed_routes();
    linked_runtime
}

/// A fully linked runtime pairing a compiled backend program with the
/// scheduler/assembly required to tick signals.
///
/// Owns an `Arc<BackendProgram>` plus bundled native-kernel sidecars so the
/// runtime is `'static` and can be stored in `Arc`, sent across threads, or
/// used at async `await` points without a lifetime coupling to the stack that
/// produced the program (M5).
pub struct BackendLinkedRuntime {
    assembly: HirRuntimeAssembly,
    runtime: TaskSourceRuntime<RuntimeValue, hir::SourceDecodeProgram, MovingRuntimeValueStore>,
    backend: BackendRuntimePayload,
    native_kernels: std::sync::Arc<aivi_backend::NativeKernelArtifactSet>,
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
    temporal_triggers_bootstrapped: bool,
    db_commit_invalidation_sink: Option<DbCommitInvalidationSink>,
    execution_context: SourceProviderContext,
}
