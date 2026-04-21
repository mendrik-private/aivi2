const RUN_ARTIFACT_FORMAT: &str = "aivi.run-artifact";
const RUN_ARTIFACT_VERSION: u32 = 4;
const RUN_ARTIFACT_FILE_NAME: &str = "run-artifact.bin";
const RUN_ARTIFACT_PAYLOAD_DIR: &str = "payloads";
const FROZEN_RUN_IMAGE_FORMAT: &str = "aivi.frozen-run-image";
const FROZEN_RUN_IMAGE_VERSION: u32 = 2;
const FROZEN_RUN_IMAGE_FILE_NAME: &str = "frozen-run-image.bin";
const FROZEN_BACKEND_CATALOG_FORMAT: &str = "aivi.frozen-backend-catalog";
const FROZEN_BACKEND_CATALOG_VERSION: u32 = 1;
const ENCODED_BACKEND_PAYLOAD_FORMAT: &str = "aivi.encoded-backend-payload";
const ENCODED_BACKEND_PAYLOAD_VERSION: u32 = 1;
const SOURCE_RUN_CACHE_FORMAT: &str = "aivi.source-run-cache";
const SOURCE_RUN_CACHE_VERSION: u32 = 3;
const SOURCE_RUN_CACHE_NAMESPACE_REVISION: &str = "4";
const SOURCE_RUN_CACHE_DIR: &str = "run-cache";
const SOURCE_RUN_CACHE_METADATA_FILE_NAME: &str = "source-run-cache.json";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SourceRunCacheManifest {
    format: Box<str>,
    version: u32,
    entry_path: Box<str>,
    requested_view: Option<Box<str>>,
    dependencies: Box<[SourceRunCacheDependency]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SourceRunCacheDependency {
    path: Box<str>,
    fingerprint: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SerializedRunArtifact {
    format: Box<str>,
    version: u32,
    view_name: Box<str>,
    kind: SerializedRunArtifactKind,
    required_signal_globals: Box<[RequiredSignalGlobalWire]>,
    runtime_assembly: HirRuntimeAssemblyWire,
    runtime_link: aivi_runtime::BackendRuntimeLinkSeed,
    backend: BackendPayloadWire,
    stub_signal_defaults: Box<[StubSignalDefaultWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum SerializedRunArtifactKind {
    Gtk(SerializedRunGtkArtifact),
    HeadlessTask { task_owner: HirItemId },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SerializedRunGtkArtifact {
    patterns: Box<[RunPatternEntryWire]>,
    bridge: GtkBridgeGraphWire,
    hydration_inputs: Box<[RunInputEntryWire]>,
    event_handlers: Box<[RunEventHandlerEntryWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RunPatternEntryWire {
    id: HirPatternId,
    pattern: RunPattern,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RunInputEntryWire {
    input: RuntimeInputHandle,
    compiled: CompiledRunInputWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum CompiledRunInputWire {
    Expr(CompiledRunFragmentWire),
    Text(CompiledRunTextWire),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledRunFragmentWire {
    expr: HirExprId,
    parameters: Vec<RunFragmentParameter>,
    execution: BackendPayloadWire,
    item: BackendItemId,
    required_signal_globals: Vec<CompiledRunSignalGlobal>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledRunTextWire {
    segments: Box<[CompiledRunTextSegmentWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum CompiledRunTextSegmentWire {
    Text(Box<str>),
    Interpolation(CompiledRunFragmentWire),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RequiredSignalGlobalWire {
    item: BackendItemId,
    name: Box<str>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RunEventHandlerEntryWire {
    handler: HirExprId,
    resolved: ResolvedRunEventHandler,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct StubSignalDefaultWire {
    input: RuntimeInputHandle,
    value: DetachedRuntimeValue,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct BackendPayloadWire {
    program_path: Box<str>,
    native_kernels: Box<[NativeKernelPayloadWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct NativeKernelPayloadWire {
    kernel: aivi_backend::KernelId,
    artifact_path: Box<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
struct FrozenBackendHandle(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
struct FrozenEntryHandle(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FrozenBackendPayloadRefWire {
    handle: FrozenBackendHandle,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenRunImage {
    format: Box<str>,
    version: u32,
    artifact: FrozenSerializedRunArtifact,
    backends: Box<[FrozenBackendPayloadWire]>,
    entries: Box<[FrozenEntryWire]>,
}

struct FrozenPreparedRunImage {
    bytes: Vec<u8>,
    artifact: RunArtifact,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenBackendPayloadWire {
    backend_catalog: Vec<u8>,
    native_kernels: Box<[FrozenNativeKernelPayloadWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EncodedFrozenBackendCatalog {
    format: Box<str>,
    version: u32,
    catalog: aivi_backend::FrozenBackendCatalog,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct EncodedBackendPayload {
    format: Box<str>,
    version: u32,
    payload: EncodedBackendPayloadKind,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum EncodedBackendPayloadKind {
    Program(aivi_backend::Program),
    Meta(aivi_backend::BackendRuntimeMeta),
    FrozenCatalog(aivi_backend::FrozenBackendCatalog),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenNativeKernelPayloadWire {
    kernel: aivi_backend::KernelId,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenEntryWire {
    backend: FrozenBackendHandle,
    item: BackendItemId,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenSerializedRunArtifact {
    format: Box<str>,
    version: u32,
    view_name: Box<str>,
    kind: FrozenSerializedRunArtifactKind,
    required_signal_globals: Box<[RequiredSignalGlobalWire]>,
    runtime_assembly: FrozenHirRuntimeAssemblyWire,
    runtime_tables: FrozenLinkedRuntimeTablesWire,
    backend: FrozenBackendPayloadRefWire,
    stub_signal_defaults: Box<[StubSignalDefaultWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenLinkedRuntimeTablesWire {
    signal_items_by_handle: Box<[(aivi_runtime::SignalHandle, BackendItemId)]>,
    runtime_signal_by_item: Box<[(BackendItemId, aivi_runtime::SignalHandle)]>,
    derived_signals: Box<[(aivi_runtime::DerivedHandle, aivi_runtime::LinkedDerivedSignal)]>,
    reactive_signals: Box<[(aivi_runtime::SignalHandle, aivi_runtime::LinkedReactiveSignal)]>,
    reactive_clauses: Box<[FrozenLinkedReactiveClauseEntryWire]>,
    linked_recurrence_signals:
        Box<[(aivi_runtime::DerivedHandle, aivi_runtime::LinkedRecurrenceSignal)]>,
    source_bindings:
        Box<[(aivi_runtime::SourceInstanceId, aivi_runtime::LinkedSourceBinding)]>,
    task_bindings: Box<[(aivi_runtime::TaskInstanceId, aivi_runtime::LinkedTaskBinding)]>,
    db_changed_routes: Box<[aivi_runtime::LinkedDbChangedRoute]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenLinkedReactiveClauseEntryWire {
    handle: aivi_runtime::ReactiveClauseHandle,
    clause: FrozenLinkedReactiveClauseWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenLinkedReactiveClauseWire {
    owner: HirItemId,
    target: aivi_runtime::SignalHandle,
    clause: aivi_runtime::ReactiveClauseHandle,
    pipeline_ids: Box<[aivi_backend::PipelineId]>,
    body_mode: aivi_hir::ReactiveUpdateBodyMode,
    guard_eval_lane: aivi_runtime::startup::LinkedEvalLane,
    body_eval_lane: aivi_runtime::startup::LinkedEvalLane,
    compiled_guard: FrozenHirCompiledRuntimeExprWire,
    compiled_body: FrozenHirCompiledRuntimeExprWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum FrozenSerializedRunArtifactKind {
    Gtk(FrozenSerializedRunGtkArtifact),
    HeadlessTask { task_owner: HirItemId },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenSerializedRunGtkArtifact {
    patterns: Box<[RunPatternEntryWire]>,
    bridge: GtkBridgeGraphWire,
    hydration_inputs: Box<[FrozenRunInputEntryWire]>,
    event_handlers: Box<[RunEventHandlerEntryWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenRunInputEntryWire {
    input: RuntimeInputHandle,
    compiled: FrozenCompiledRunInputWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum FrozenCompiledRunInputWire {
    Expr(FrozenCompiledRunFragmentWire),
    Text(FrozenCompiledRunTextWire),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenCompiledRunFragmentWire {
    expr: HirExprId,
    parameters: Vec<RunFragmentParameter>,
    entry: FrozenEntryHandle,
    required_signal_globals: Vec<CompiledRunSignalGlobal>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenCompiledRunTextWire {
    segments: Box<[FrozenCompiledRunTextSegmentWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum FrozenCompiledRunTextSegmentWire {
    Text(Box<str>),
    Interpolation(FrozenCompiledRunFragmentWire),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenHirRuntimeAssemblyWire {
    graph: aivi_runtime::SignalGraphParts,
    reactive_program: aivi_runtime::ReactiveProgramParts,
    owners: Box<[aivi_runtime::HirOwnerBinding]>,
    signals: Box<[FrozenHirSignalBindingWire]>,
    sources: Box<[aivi_runtime::HirSourceBinding]>,
    tasks: Box<[aivi_runtime::HirTaskBinding]>,
    gates: Box<[aivi_runtime::HirGateStageBinding]>,
    recurrences: Box<[aivi_runtime::HirRecurrenceBinding]>,
    db_changed_bindings: Box<[aivi_runtime::HirDbChangedBinding]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenHirSignalBindingWire {
    item: HirItemId,
    span: SourceSpan,
    name: Box<str>,
    owner: aivi_runtime::OwnerHandle,
    kind: FrozenHirSignalBindingKindWire,
    source_input: Option<RuntimeInputHandle>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum FrozenHirSignalBindingKindWire {
    Input {
        signal: RuntimeInputHandle,
    },
    Derived {
        signal: aivi_runtime::DerivedHandle,
        dependencies: Box<[aivi_runtime::SignalHandle]>,
        temporal_trigger_dependencies: Box<[aivi_runtime::SignalHandle]>,
        temporal_helpers: Box<[RuntimeInputHandle]>,
    },
    Reactive {
        signal: aivi_runtime::SignalHandle,
        dependencies: Box<[aivi_runtime::SignalHandle]>,
        seed_dependencies: Box<[aivi_runtime::SignalHandle]>,
        clauses: Box<[FrozenHirReactiveUpdateBindingWire]>,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenHirReactiveUpdateBindingWire {
    span: SourceSpan,
    keyword_span: SourceSpan,
    target_span: SourceSpan,
    guard: HirExprId,
    body: HirExprId,
    body_mode: aivi_hir::ReactiveUpdateBodyMode,
    clause: aivi_runtime::ReactiveClauseHandle,
    trigger_signal: Option<aivi_runtime::SignalHandle>,
    guard_dependencies: Box<[aivi_runtime::SignalHandle]>,
    body_dependencies: Box<[aivi_runtime::SignalHandle]>,
    compiled_guard: FrozenHirCompiledRuntimeExprWire,
    compiled_body: FrozenHirCompiledRuntimeExprWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct FrozenHirCompiledRuntimeExprWire {
    entry: FrozenEntryHandle,
    parameter_signals: Box<[aivi_runtime::SignalHandle]>,
    required_signals: Box<[aivi_runtime::hir_adapter::HirCompiledRuntimeExprSignal]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HirRuntimeAssemblyWire {
    graph: aivi_runtime::SignalGraphParts,
    reactive_program: aivi_runtime::ReactiveProgramParts,
    owners: Box<[aivi_runtime::HirOwnerBinding]>,
    signals: Box<[HirSignalBindingWire]>,
    sources: Box<[aivi_runtime::HirSourceBinding]>,
    tasks: Box<[aivi_runtime::HirTaskBinding]>,
    gates: Box<[aivi_runtime::HirGateStageBinding]>,
    recurrences: Box<[aivi_runtime::HirRecurrenceBinding]>,
    db_changed_bindings: Box<[aivi_runtime::HirDbChangedBinding]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HirSignalBindingWire {
    item: HirItemId,
    span: SourceSpan,
    name: Box<str>,
    owner: aivi_runtime::OwnerHandle,
    kind: HirSignalBindingKindWire,
    source_input: Option<RuntimeInputHandle>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum HirSignalBindingKindWire {
    Input {
        signal: RuntimeInputHandle,
    },
    Derived {
        signal: aivi_runtime::DerivedHandle,
        dependencies: Box<[aivi_runtime::SignalHandle]>,
        temporal_trigger_dependencies: Box<[aivi_runtime::SignalHandle]>,
        temporal_helpers: Box<[RuntimeInputHandle]>,
    },
    Reactive {
        signal: aivi_runtime::SignalHandle,
        dependencies: Box<[aivi_runtime::SignalHandle]>,
        seed_dependencies: Box<[aivi_runtime::SignalHandle]>,
        clauses: Box<[HirReactiveUpdateBindingWire]>,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HirReactiveUpdateBindingWire {
    span: SourceSpan,
    keyword_span: SourceSpan,
    target_span: SourceSpan,
    guard: HirExprId,
    body: HirExprId,
    body_mode: aivi_hir::ReactiveUpdateBodyMode,
    clause: aivi_runtime::ReactiveClauseHandle,
    trigger_signal: Option<aivi_runtime::SignalHandle>,
    guard_dependencies: Box<[aivi_runtime::SignalHandle]>,
    body_dependencies: Box<[aivi_runtime::SignalHandle]>,
    compiled_guard: HirCompiledRuntimeExprWire,
    compiled_body: HirCompiledRuntimeExprWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HirCompiledRuntimeExprWire {
    backend: BackendPayloadWire,
    entry_item: BackendItemId,
    parameter_signals: Box<[aivi_runtime::SignalHandle]>,
    required_signals: Box<[aivi_runtime::hir_adapter::HirCompiledRuntimeExprSignal]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct GtkBridgeGraphWire {
    assembly: aivi_gtk::WidgetRuntimeAssemblyParts,
    root: GtkBridgeNodeRef,
    nodes: Box<[GtkBridgeNodeWire]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct GtkBridgeNodeWire {
    plan: aivi_gtk::PlanNodeId,
    stable_id: aivi_gtk::StableNodeId,
    span: SourceSpan,
    owner: aivi_runtime::OwnerHandle,
    parent: Option<GtkBridgeNodeRef>,
    kind: GtkBridgeNodeKindWire,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum GtkBridgeNodeKindWire {
    Widget(GtkWidgetNodeWire),
    Group(GtkGroupNodeWire),
    Show(aivi_gtk::GtkShowNode),
    Each(aivi_gtk::GtkEachNode),
    Empty(aivi_gtk::GtkEmptyNode),
    Match(aivi_gtk::GtkMatchNode),
    Case(aivi_gtk::GtkCaseNode),
    Fragment(aivi_gtk::GtkFragmentNode),
    With(aivi_gtk::GtkWithNode),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct GtkWidgetNodeWire {
    widget: aivi_hir::NamePath,
    properties: Box<[aivi_gtk::RuntimePropertyBinding]>,
    event_hooks: Box<[aivi_gtk::RuntimeEventBinding]>,
    default_group_descriptor: Option<Box<str>>,
    default_children: GtkChildGroup,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct GtkGroupNodeWire {
    widget: aivi_hir::NamePath,
    descriptor: Box<str>,
    body: GtkChildGroup,
}

#[allow(dead_code)]
struct ArtifactPayloadRegistry {
    include_native_kernels: bool,
    entries: BTreeMap<Box<str>, RegisteredBackendPayload>,
}

impl Default for ArtifactPayloadRegistry {
    fn default() -> Self {
        Self::new(true)
    }
}

#[allow(dead_code)]
struct RegisteredBackendPayload {
    backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
    native_kernels: Box<[RegisteredNativeKernelPayload]>,
}

#[allow(dead_code)]
struct RegisteredNativeKernelPayload {
    kernel: aivi_backend::KernelId,
    artifact_path: Box<str>,
    artifact: aivi_backend::NativeKernelArtifact,
}

#[derive(Clone)]
struct LoadedBackendPayload {
    backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
    native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
}

struct FrozenRegisteredBackendPayload {
    key: u64,
    backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
    native_kernels: Box<[RegisteredNativeKernelPayload]>,
}

struct FrozenPayloadRegistry {
    include_native_kernels: bool,
    freeze_backends: bool,
    handles_by_key: BTreeMap<u64, FrozenBackendHandle>,
    entries: Vec<FrozenRegisteredBackendPayload>,
    entry_handles: BTreeMap<(FrozenBackendHandle, BackendItemId), FrozenEntryHandle>,
    frozen_entries: Vec<FrozenEntryWire>,
}

struct FrozenPayloadLoader {
    backends: Vec<LoadedBackendPayload>,
    entries: Vec<FrozenEntryWire>,
}

#[allow(dead_code)]
impl ArtifactPayloadRegistry {
    fn new(include_native_kernels: bool) -> Self {
        Self {
            include_native_kernels,
            entries: BTreeMap::new(),
        }
    }

    fn register_payload(
        &mut self,
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
        native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    ) -> Result<BackendPayloadWire, String> {
        match &backend {
            aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                let key = compute_program_fingerprint(program.as_ref());
                let meta = Arc::new(aivi_backend::BackendRuntimeMeta::from(program.as_ref()));
                self.register_keyed_payload(key, backend.clone(), meta, native_kernels)
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
                let key = compute_runtime_meta_fingerprint(meta.as_ref())?;
                self.register_keyed_payload(key, backend.clone(), meta.clone(), native_kernels)
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                let meta = Arc::new(catalog.as_ref().to_runtime_meta());
                let key = compute_runtime_meta_fingerprint(meta.as_ref())?;
                self.register_keyed_payload(key, backend.clone(), meta, native_kernels)
            }
        }
    }

    fn register_keyed_payload(
        &mut self,
        key: u64,
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
        meta: Arc<aivi_backend::BackendRuntimeMeta>,
        native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    ) -> Result<BackendPayloadWire, String> {
        let program_path =
            format!("{RUN_ARTIFACT_PAYLOAD_DIR}/backend-{key:016x}.bin").into_boxed_str();
        let entry = match self.entries.entry(program_path.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let native_kernels = if self.include_native_kernels {
                    collect_native_kernel_payloads(
                        key,
                        &backend,
                        meta.as_ref(),
                        native_kernels.as_ref(),
                        true,
                    )?
                } else {
                    Box::default()
                };
                entry.insert(RegisteredBackendPayload {
                    backend,
                    native_kernels,
                })
            }
        };
        Ok(BackendPayloadWire {
            program_path,
            native_kernels: entry
                .native_kernels
                .iter()
                .map(|native| NativeKernelPayloadWire {
                    kernel: native.kernel,
                    artifact_path: native.artifact_path.clone(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    fn write_all(&self, root: &Path) -> Result<(), String> {
        for (relative_path, payload) in &self.entries {
            let path = root.join(relative_path.as_ref());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("failed to create {}: {error}", parent.display())
                })?;
            }
            let bytes = encode_backend_runtime_meta_bytes(relative_path.as_ref(), &payload.backend)?;
            fs::write(&path, bytes)
                .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
            for native in payload.native_kernels.iter() {
                let artifact_path = root.join(native.artifact_path.as_ref());
                if let Some(parent) = artifact_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("failed to create {}: {error}", parent.display())
                    })?;
                }
                let bytes = aivi_backend::encode_native_kernel_artifact_binary(&native.artifact);
                fs::write(&artifact_path, bytes).map_err(|error| {
                    format!("failed to write {}: {error}", artifact_path.display())
                })?;
            }
        }
        Ok(())
    }

}

impl FrozenPayloadRegistry {
    fn new_with_mode(include_native_kernels: bool, freeze_backends: bool) -> Self {
        Self {
            include_native_kernels,
            freeze_backends,
            handles_by_key: BTreeMap::new(),
            entries: Vec::new(),
            entry_handles: BTreeMap::new(),
            frozen_entries: Vec::new(),
        }
    }

    fn register_payload(
        &mut self,
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
        native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    ) -> Result<FrozenBackendPayloadRefWire, String> {
        let (key, meta) = match &backend {
            aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                let meta = Arc::new(aivi_backend::BackendRuntimeMeta::from(program.as_ref()));
                (compute_runtime_meta_fingerprint(meta.as_ref())?, meta)
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
                (compute_runtime_meta_fingerprint(meta.as_ref())?, meta.clone())
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                let meta = Arc::new(catalog.as_ref().to_runtime_meta());
                (compute_runtime_meta_fingerprint(meta.as_ref())?, meta)
            }
        };
        if let Some(&handle) = self.handles_by_key.get(&key) {
            return Ok(FrozenBackendPayloadRefWire { handle });
        }
        let handle = FrozenBackendHandle(self.entries.len().try_into().map_err(|_| {
            "frozen backend table exceeded maximum entry count".to_owned()
        })?);
        let native_kernels = if self.include_native_kernels {
            collect_native_kernel_payloads(
                key,
                &backend,
                meta.as_ref(),
                native_kernels.as_ref(),
                self.freeze_backends,
            )?
        } else {
            Box::default()
        };
        self.entries.push(FrozenRegisteredBackendPayload {
            key,
            backend,
            native_kernels,
        });
        self.handles_by_key.insert(key, handle);
        Ok(FrozenBackendPayloadRefWire { handle })
    }

    fn register_entry(
        &mut self,
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
        native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
        item: BackendItemId,
    ) -> Result<FrozenEntryHandle, String> {
        let backend = self.register_payload(backend, native_kernels)?.handle;
        if self.include_native_kernels && self.freeze_backends {
            self.ensure_item_entry_abi(backend, item)?;
        }
        if let Some(&handle) = self.entry_handles.get(&(backend, item)) {
            return Ok(handle);
        }
        let handle =
            FrozenEntryHandle(self.frozen_entries.len().try_into().map_err(|_| {
                "frozen entry table exceeded maximum entry count".to_owned()
            })?);
        self.frozen_entries.push(FrozenEntryWire { backend, item });
        self.entry_handles.insert((backend, item), handle);
        Ok(handle)
    }

    fn payload(&self, handle: FrozenBackendHandle) -> Result<&FrozenRegisteredBackendPayload, String> {
        self.entries
            .get(handle.0 as usize)
            .ok_or_else(|| format!("unknown frozen backend handle {}", handle.0))
    }

    fn ensure_item_entry_abi(
        &self,
        handle: FrozenBackendHandle,
        item: BackendItemId,
    ) -> Result<(), String> {
        let payload = self.payload(handle)?;
        let runtime_view = match &payload.backend {
            aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                aivi_backend::BackendRuntimeView::Program(program.as_ref())
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
                aivi_backend::BackendRuntimeView::Meta(meta.as_ref())
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                aivi_backend::BackendRuntimeView::FrozenCatalog(catalog.as_ref())
            }
        };
        let item_meta = runtime_view.item(item).ok_or_else(|| {
            format!(
                "missing frozen entry item {} in backend {:016x}",
                item.as_raw(),
                payload.key
            )
        })?;
        let kernel = item_meta.body.ok_or_else(|| {
            format!(
                "item {} in backend {:016x} has no compiled body; built bundles are strict compiled-only",
                item.as_raw(),
                payload.key
            )
        })?;
        self.ensure_kernel_entry_abi(handle, kernel, "entry item")
    }

    fn ensure_kernel_entry_abi(
        &self,
        handle: FrozenBackendHandle,
        kernel: aivi_backend::KernelId,
        context: &str,
    ) -> Result<(), String> {
        let payload = self.payload(handle)?;
        let artifact = payload
            .native_kernels
            .iter()
            .find(|native| native.kernel == kernel)
            .ok_or_else(|| match &payload.backend {
                aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                    match aivi_backend::compile_native_kernel_artifact(program.as_ref(), kernel) {
                        Ok(Some(_)) => format!(
                            "missing native backend payload for {context} kernel {} in frozen backend {:016x}",
                            kernel.as_raw(),
                            payload.key
                        ),
                        Ok(None) => match aivi_backend::compile_kernel(program.as_ref(), kernel) {
                            Ok(_) => {
                                let detail = match aivi_backend::diagnose_native_kernel_artifact_miss(
                                    program.as_ref(),
                                    kernel,
                                ) {
                                    Ok(Some(reason)) => format!(": {reason}"),
                                    Ok(None) => String::new(),
                                    Err(error) => {
                                        format!(": failed to diagnose cache artifact miss: {error}")
                                    }
                                };
                                format!(
                                    "{context} kernel {} in frozen backend {:016x} still cannot produce a cached native artifact{detail}; built bundles are strict compiled-only",
                                    kernel.as_raw(),
                                    payload.key
                                )
                            }
                            Err(error) => format!(
                                "{context} kernel {} in frozen backend {:016x} fails backend compilation: {error}",
                                kernel.as_raw(),
                                payload.key
                            ),
                        },
                        Err(error) => format!(
                            "{context} kernel {} in frozen backend {:016x} failed native compilation: {error}",
                            kernel.as_raw(),
                            payload.key
                        ),
                    }
                }
                _ => format!(
                    "missing native backend payload for {context} kernel {} in frozen backend {:016x}",
                    kernel.as_raw(),
                    payload.key
                ),
            })?;
        if !artifact.artifact.has_frozen_requested_entry_abi() {
            let meta = match &payload.backend {
                aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                    aivi_backend::BackendRuntimeMeta::from(program.as_ref())
                }
                aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => meta.as_ref().clone(),
                aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                    catalog.as_ref().to_runtime_meta()
                }
            };
            return match aivi_backend::validate_frozen_requested_kernel_abi(&meta, kernel) {
                Ok(()) => Err(format!(
                    "{context} kernel {} in backend {:016x} still lacks a full frozen entry ABI; built bundles are strict compiled-only",
                    kernel.as_raw(),
                    payload.key
                )),
                Err(error) => Err(format!(
                    "{context} kernel {} in backend {:016x} cannot freeze a compiled-only entry ABI: {error}",
                    kernel.as_raw(),
                    payload.key
                )),
            };
        }
        let native_kernels = self.attached_native_kernels(handle)?;
        let aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) =
            self.frozen_backend_payload(handle)?
        else {
            unreachable!("frozen backend payload conversion should always produce a catalog");
        };
        if let Err(detail) = aivi_backend::diagnose_frozen_native_kernel_plan(
            catalog.as_ref(),
            native_kernels.as_ref(),
            kernel,
        ) {
            return Err(format!(
                "{context} kernel {} in backend {:016x} cannot materialize a frozen native execution plan: {detail}; built bundles are strict compiled-only",
                kernel.as_raw(),
                payload.key
            ));
        }
        Ok(())
    }

    fn attached_native_kernels(
        &self,
        handle: FrozenBackendHandle,
    ) -> Result<Arc<aivi_backend::NativeKernelArtifactSet>, String> {
        let payload = self.payload(handle)?;
        let runtime_view = match &payload.backend {
            aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                aivi_backend::BackendRuntimeView::Program(program.as_ref())
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
                aivi_backend::BackendRuntimeView::Meta(meta.as_ref())
            }
            aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                aivi_backend::BackendRuntimeView::FrozenCatalog(catalog.as_ref())
            }
        };
        let mut native_kernels = aivi_backend::NativeKernelArtifactSet::default();
        for native in payload.native_kernels.iter() {
            let fingerprint = runtime_view
                .kernel(native.kernel)
                .ok_or_else(|| {
                    format!(
                        "missing kernel {} while rebuilding frozen native payload set for backend {:016x}",
                        native.kernel.as_raw(),
                        payload.key
                    )
                })?
                .fingerprint;
            native_kernels.insert(fingerprint, native.artifact.clone());
        }
        Ok(Arc::new(native_kernels))
    }

    fn frozen_backend_payload(
        &self,
        handle: FrozenBackendHandle,
    ) -> Result<aivi_runtime::hir_adapter::BackendRuntimePayload, String> {
        let payload = self.payload(handle)?;
        Ok(if self.freeze_backends {
            match &payload.backend {
                aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(Arc::new(
                        aivi_backend::FrozenBackendCatalog::from(program.as_ref()),
                    ))
                }
                aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(Arc::new(
                        aivi_backend::FrozenBackendCatalog::from(meta.as_ref()),
                    ))
                }
                aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog.clone())
                }
            }
        } else {
            payload.backend.clone()
        })
    }

    fn collect_backends(&self) -> Result<Box<[FrozenBackendPayloadWire]>, String> {
        self.entries
            .iter()
            .map(|payload| {
                Ok(FrozenBackendPayloadWire {
                    backend_catalog: if self.freeze_backends {
                        encode_frozen_backend_catalog_bytes("frozen-image", &payload.backend)?
                    } else {
                        encode_backend_payload_bytes("frozen-image", &payload.backend)?
                    },
                    native_kernels: payload
                        .native_kernels
                        .iter()
                        .map(|native| FrozenNativeKernelPayloadWire {
                            kernel: native.kernel,
                            bytes: aivi_backend::encode_native_kernel_artifact_binary(
                                &native.artifact,
                            ),
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .map(Vec::into_boxed_slice)
    }

    fn collect_entries(&self) -> Box<[FrozenEntryWire]> {
        self.frozen_entries.clone().into_boxed_slice()
    }
}

struct ArtifactPayloadLoader {
    entries: BTreeMap<Box<str>, LoadedBackendPayload>,
    read_payload: Box<dyn FnMut(&str) -> Result<Vec<u8>, String>>,
}

impl ArtifactPayloadLoader {
    fn new(read_payload: Box<dyn FnMut(&str) -> Result<Vec<u8>, String>>) -> Self {
        Self {
            entries: BTreeMap::new(),
            read_payload,
        }
    }

    fn load(&mut self, payload: &BackendPayloadWire) -> Result<LoadedBackendPayload, String> {
        if let Some(program) = self.entries.get(&payload.program_path).cloned() {
            return Ok(program);
        }
        let bytes = (self.read_payload)(payload.program_path.as_ref())?;
        let backend = decode_backend_payload_bytes(&bytes, payload.program_path.as_ref())?;
        let mut native_kernels = aivi_backend::NativeKernelArtifactSet::default();
        for native in payload.native_kernels.iter() {
            let bytes = (self.read_payload)(native.artifact_path.as_ref())?;
            let artifact = aivi_backend::decode_native_kernel_artifact_binary(&bytes).ok_or_else(
                || format!("failed to decode native backend payload {}", native.artifact_path),
            )?;
            if artifact.requested_kernel() != native.kernel {
                return Err(format!(
                    "native backend payload {} targets kernel {} but manifest expects {}",
                    native.artifact_path,
                    artifact.requested_kernel().as_raw(),
                    native.kernel.as_raw()
                ));
            }
            let fingerprint = match &backend {
                aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                    aivi_backend::compute_kernel_fingerprint(program.as_ref(), native.kernel)
                }
                aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => meta
                    .kernels()
                    .get(native.kernel)
                    .ok_or_else(|| {
                        format!(
                            "backend payload {} is missing kernel {} required by {}",
                            payload.program_path,
                            native.kernel.as_raw(),
                            native.artifact_path
                        )
                    })?
                    .fingerprint,
                aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => catalog
                    .kernels()
                    .get(native.kernel)
                    .ok_or_else(|| {
                        format!(
                            "backend payload {} is missing kernel {} required by {}",
                            payload.program_path,
                            native.kernel.as_raw(),
                            native.artifact_path
                        )
                    })?
                    .fingerprint,
            };
            native_kernels.insert(
                fingerprint,
                artifact,
            );
        }
        let loaded = LoadedBackendPayload {
            backend: backend.clone(),
            native_kernels: Arc::new(native_kernels),
        };
        self.entries
            .insert(payload.program_path.clone(), loaded.clone());
        Ok(loaded)
    }
}

impl FrozenPayloadLoader {
    fn new(
        backends: Box<[FrozenBackendPayloadWire]>,
        entries: Box<[FrozenEntryWire]>,
    ) -> Result<Self, String> {
        let mut loaded = Vec::with_capacity(backends.len());
        for payload in backends.into_vec() {
            let backend = decode_frozen_backend_payload_bytes(
                &payload.backend_catalog,
                "frozen-image backend catalog",
            )?;
            let mut native_kernels = aivi_backend::NativeKernelArtifactSet::default();
            for native in payload.native_kernels.iter() {
                let artifact = aivi_backend::decode_native_kernel_artifact_binary(&native.bytes)
                    .ok_or_else(|| {
                        format!(
                            "failed to decode frozen native backend payload for kernel {}",
                            native.kernel.as_raw()
                        )
                    })?;
                if artifact.requested_kernel() != native.kernel {
                    return Err(format!(
                        "frozen native backend payload targets kernel {} but image expects {}",
                        artifact.requested_kernel().as_raw(),
                        native.kernel.as_raw()
                    ));
                }
                let fingerprint = match &backend {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
                    aivi_backend::compute_kernel_fingerprint(program.as_ref(), native.kernel)
                }
                aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => meta
                    .kernels()
                        .get(native.kernel)
                        .ok_or_else(|| {
                            format!(
                                "frozen backend payload missing kernel {} required by bundled native artifact",
                                native.kernel.as_raw()
                            )
                    })?
                    .fingerprint,
                aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
                    catalog
                        .kernels()
                        .get(native.kernel)
                        .ok_or_else(|| {
                            format!(
                                "frozen backend payload missing kernel {} required by bundled native artifact",
                                native.kernel.as_raw()
                            )
                        })?
                        .fingerprint
                }
            };
            native_kernels.insert(fingerprint, artifact);
        }
            loaded.push(LoadedBackendPayload {
                backend,
                native_kernels: Arc::new(native_kernels),
            });
        }
        Ok(Self {
            backends: loaded,
            entries: entries.into_vec(),
        })
    }

    fn load(&self, payload: FrozenBackendPayloadRefWire) -> Result<LoadedBackendPayload, String> {
        self.backends
            .get(payload.handle.0 as usize)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "frozen backend handle {} is out of range for image payload table",
                    payload.handle.0
                )
            })
    }

    fn load_entry(
        &self,
        entry: FrozenEntryHandle,
    ) -> Result<(LoadedBackendPayload, BackendItemId), String> {
        let entry = self.entries.get(entry.0 as usize).ok_or_else(|| {
            format!(
                "frozen entry handle {} is out of range for image entry table",
                entry.0
            )
        })?;
        let payload = self.load(FrozenBackendPayloadRefWire {
            handle: entry.backend,
        })?;
        Ok((payload, entry.item))
    }
}

fn compute_runtime_meta_fingerprint(meta: &aivi_backend::BackendRuntimeMeta) -> Result<u64, String> {
    use std::hash::{Hash, Hasher};

    let bytes = bincode::serialize(meta)
        .map_err(|error| format!("failed to encode backend runtime metadata for fingerprinting: {error}"))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

fn collect_native_kernel_payloads(
    key: u64,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
    meta: &aivi_backend::BackendRuntimeMeta,
    provided: &aivi_backend::NativeKernelArtifactSet,
    compile_missing: bool,
) -> Result<Box<[RegisteredNativeKernelPayload]>, String> {
    let mut native_kernels = Vec::new();
    for (kernel, kernel_meta) in meta.kernels().iter() {
        let artifact = if let Some(artifact) = provided.get_for_kernel(kernel_meta.fingerprint, kernel) {
            artifact.clone()
        } else if compile_missing {
            let Some(program) = backend.as_program() else {
                return Err(format!(
                    "missing native backend payload for kernel {} in frozen backend {key:016x}",
                    kernel.as_raw()
                ));
            };
            let Some(artifact) =
                aivi_backend::compile_native_kernel_artifact(program.as_ref(), kernel).map_err(
                    |error| {
                        format!(
                            "failed to compile native backend payload for kernel {} in backend {key:016x}: {error}",
                            kernel.as_raw()
                        )
                    },
                )?
            else {
                continue;
            };
            artifact
        } else {
            continue;
        };
        let artifact = aivi_backend::attach_frozen_native_kernel_abi(meta, &artifact).map_err(
            |error| {
                format!(
                    "failed to freeze native ABI for kernel {} in backend {key:016x}: {error}",
                    kernel.as_raw()
                )
            },
        )?;
        let fingerprint = kernel_meta.fingerprint;
        native_kernels.push(RegisteredNativeKernelPayload {
            kernel,
            artifact_path: format!(
                "{RUN_ARTIFACT_PAYLOAD_DIR}/native-{key:016x}-{:08x}-{:016x}.bin",
                kernel.as_raw(),
                fingerprint.as_raw()
            )
            .into_boxed_str(),
            artifact,
        });
    }
    Ok(native_kernels.into_boxed_slice())
}

#[allow(dead_code)]
fn encode_backend_runtime_meta_bytes(
    relative_path: &str,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
) -> Result<Vec<u8>, String> {
    match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
            let meta = aivi_backend::BackendRuntimeMeta::from(program.as_ref());
            bincode::serialize(&meta).map_err(|error| {
                format!(
                    "failed to encode backend runtime metadata {relative_path} as binary: {error}"
                )
            })
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
            bincode::serialize(meta.as_ref()).map_err(|error| {
                format!(
                    "failed to encode backend runtime metadata {relative_path} as binary: {error}"
                )
            })
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
            let meta = catalog.as_ref().to_runtime_meta();
            bincode::serialize(&meta).map_err(|error| {
                format!(
                    "failed to encode backend runtime metadata {relative_path} as binary: {error}"
                )
            })
        }
    }
}

fn encode_backend_payload_bytes(
    relative_path: &str,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
) -> Result<Vec<u8>, String> {
    let payload = match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
            EncodedBackendPayloadKind::Program(program.as_ref().clone())
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
            EncodedBackendPayloadKind::Meta(meta.as_ref().clone())
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
            EncodedBackendPayloadKind::FrozenCatalog(catalog.as_ref().clone())
        }
    };
    let encoded = EncodedBackendPayload {
        format: ENCODED_BACKEND_PAYLOAD_FORMAT.into(),
        version: ENCODED_BACKEND_PAYLOAD_VERSION,
        payload,
    };
    bincode::serialize(&encoded).map_err(|error| {
        format!("failed to encode backend payload {relative_path} as tagged binary: {error}")
    })
}

fn encode_frozen_backend_catalog_bytes(
    relative_path: &str,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
) -> Result<Vec<u8>, String> {
    let catalog = match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
            aivi_backend::FrozenBackendCatalog::from(program.as_ref())
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
            aivi_backend::FrozenBackendCatalog::from(meta.as_ref())
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
            catalog.as_ref().clone()
        }
    };
    let encoded = EncodedFrozenBackendCatalog {
        format: FROZEN_BACKEND_CATALOG_FORMAT.into(),
        version: FROZEN_BACKEND_CATALOG_VERSION,
        catalog,
    };
    bincode::serialize(&encoded).map_err(|error| {
        format!("failed to encode frozen backend catalog {relative_path} as binary: {error}")
    })
}

#[allow(dead_code)]
fn write_serialized_run_artifact_bundle(
    root: &Path,
    artifact: &RunArtifact,
) -> Result<PathBuf, String> {
    let mut payloads = ArtifactPayloadRegistry::default();
    let serialized = serialize_run_artifact(artifact, &mut payloads)?;
    payloads.write_all(root)?;
    let artifact_path = root.join(RUN_ARTIFACT_FILE_NAME);
    let bytes = bincode::serialize(&serialized)
        .map_err(|error| format!("failed to encode run artifact as binary: {error}"))?;
    fs::write(&artifact_path, bytes)
        .map_err(|error| format!("failed to write {}: {error}", artifact_path.display()))?;
    Ok(artifact_path)
}

fn encode_frozen_run_image_bytes_with_options(
    artifact: &RunArtifact,
    include_native_kernels: bool,
    freeze_backends: bool,
) -> Result<Vec<u8>, String> {
    let mut payloads = FrozenPayloadRegistry::new_with_mode(include_native_kernels, freeze_backends);
    let serialized = serialize_frozen_run_artifact(artifact, &mut payloads)?;
    let image = FrozenRunImage {
        format: FROZEN_RUN_IMAGE_FORMAT.into(),
        version: FROZEN_RUN_IMAGE_VERSION,
        artifact: serialized,
        backends: payloads.collect_backends()?,
        entries: payloads.collect_entries(),
    };
    bincode::serialize(&image).map_err(|error| {
        format!("failed to encode {FROZEN_RUN_IMAGE_FILE_NAME} as binary: {error}")
    })
}

fn freeze_run_artifact(artifact: &RunArtifact) -> Result<FrozenPreparedRunImage, String> {
    let bytes = encode_frozen_run_image_bytes_with_options(artifact, true, false)?;
    let mut frozen = load_frozen_run_image_from_bytes(&bytes, Some(artifact.view_name.as_ref()))?;
    frozen.sources = artifact.sources.clone();
    Ok(FrozenPreparedRunImage {
        bytes,
        artifact: frozen,
    })
}

fn write_frozen_run_image_bundle(root: &Path, artifact: &RunArtifact) -> Result<PathBuf, String> {
    write_frozen_run_image_bundle_with_options(root, artifact, true)
}

fn write_frozen_run_image_bundle_without_native_kernels(
    root: &Path,
    artifact: &RunArtifact,
) -> Result<PathBuf, String> {
    write_frozen_run_image_bundle_with_options(root, artifact, false)
}

fn write_frozen_run_image_bundle_with_options(
    root: &Path,
    artifact: &RunArtifact,
    include_native_kernels: bool,
) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    let image_path = root.join(FROZEN_RUN_IMAGE_FILE_NAME);
    let bytes = encode_frozen_run_image_bytes_with_options(artifact, include_native_kernels, true)?;
    fs::write(&image_path, bytes)
        .map_err(|error| format!("failed to write {}: {error}", image_path.display()))?;
    Ok(image_path)
}

#[cfg(test)]
fn write_serialized_run_artifact_bundle_without_native_kernels(
    root: &Path,
    artifact: &RunArtifact,
) -> Result<PathBuf, String> {
    let mut payloads = ArtifactPayloadRegistry::new(false);
    let serialized = serialize_run_artifact(artifact, &mut payloads)?;
    fs::create_dir_all(root)
        .map_err(|error| format!("failed to create {}: {error}", root.display()))?;
    payloads.write_all(root)?;
    let artifact_path = root.join(RUN_ARTIFACT_FILE_NAME);
    let bytes = bincode::serialize(&serialized).map_err(|error| {
        format!(
            "failed to encode {} as binary: {error}",
            artifact_path.display()
        )
    })?;
    fs::write(&artifact_path, bytes)
        .map_err(|error| format!("failed to write {}: {error}", artifact_path.display()))?;
    Ok(artifact_path)
}

fn maybe_load_serialized_run_artifact(
    path: &Path,
    requested_view: Option<&str>,
) -> Result<Option<RunArtifact>, String> {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return Ok(None);
    };
    if extension != "json" && extension != "bin" {
        return Ok(None);
    }
    if extension == "bin" {
        return match load_frozen_run_image(path, requested_view) {
            Ok(artifact) => Ok(Some(artifact)),
            Err(frozen_error) => load_serialized_run_artifact(path, requested_view)
                .map(Some)
                .map_err(|serialized_error| {
                    format!(
                        "failed to decode {} as frozen run image ({frozen_error}) or serialized run artifact ({serialized_error})",
                        path.display()
                    )
                }),
        };
    }
    load_serialized_run_artifact(path, requested_view).map(Some)
}

fn load_cached_source_run_artifact(
    cache_home: &Path,
    entry_path: &Path,
    requested_view: Option<&str>,
) -> Option<RunArtifact> {
    let cache_dir = source_run_cache_dir(cache_home, entry_path, requested_view);
    let manifest_path = cache_dir.join(SOURCE_RUN_CACHE_METADATA_FILE_NAME);
    let manifest = read_source_run_cache_manifest(&manifest_path).ok()?;
    validate_source_run_cache_manifest(&manifest, entry_path, requested_view).ok()?;
    if !source_run_cache_dependencies_match(&manifest) {
        return None;
    }
    let frozen_image_path = cache_dir.join(FROZEN_RUN_IMAGE_FILE_NAME);
    if frozen_image_path.is_file() {
        return load_frozen_run_image(&frozen_image_path, requested_view).ok();
    }
    let artifact_path = cache_dir.join(RUN_ARTIFACT_FILE_NAME);
    load_serialized_run_artifact(&artifact_path, requested_view).ok()
}

#[allow(dead_code)]
fn store_cached_source_run_artifact(
    cache_home: &Path,
    entry_path: &Path,
    requested_view: Option<&str>,
    snapshot: &WorkspaceHirSnapshot,
    artifact: &RunArtifact,
) -> Result<(), String> {
    let frozen = freeze_run_artifact(artifact)?;
    store_cached_frozen_run_image(cache_home, entry_path, requested_view, snapshot, &frozen.bytes)
}

fn store_cached_frozen_run_image(
    cache_home: &Path,
    entry_path: &Path,
    requested_view: Option<&str>,
    snapshot: &WorkspaceHirSnapshot,
    frozen_image_bytes: &[u8],
) -> Result<(), String> {
    let cache_dir = source_run_cache_dir(cache_home, entry_path, requested_view);
    fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("failed to create {}: {error}", cache_dir.display()))?;
    let image_path = cache_dir.join(FROZEN_RUN_IMAGE_FILE_NAME);
    fs::write(&image_path, frozen_image_bytes)
        .map_err(|error| format!("failed to write {}: {error}", image_path.display()))?;
    let manifest = build_source_run_cache_manifest(entry_path, requested_view, snapshot)?;
    let manifest_path = cache_dir.join(SOURCE_RUN_CACHE_METADATA_FILE_NAME);
    let bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("failed to encode source run cache manifest: {error}"))?;
    fs::write(&manifest_path, bytes)
        .map_err(|error| format!("failed to write {}: {error}", manifest_path.display()))?;
    let legacy_path = cache_dir.join(RUN_ARTIFACT_FILE_NAME);
    let _ = fs::remove_file(legacy_path);
    Ok(())
}

fn load_serialized_run_artifact(
    path: &Path,
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let root = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    load_serialized_run_artifact_from_bytes(
        &bytes,
        requested_view,
        Box::new(move |relative_path| {
            let payload_path = root.join(relative_path);
            fs::read(&payload_path)
                .map_err(|error| format!("failed to read {}: {error}", payload_path.display()))
        }),
    )
}

fn load_serialized_run_artifact_from_bytes(
    bytes: &[u8],
    requested_view: Option<&str>,
    payload_reader: Box<dyn FnMut(&str) -> Result<Vec<u8>, String>>,
) -> Result<RunArtifact, String> {
    let serialized: SerializedRunArtifact =
        bincode::deserialize(bytes).or_else(|binary_error| {
            serde_json::from_slice(bytes).map_err(|json_error| {
                format!(
                    "failed to decode run artifact as binary ({binary_error}) or JSON ({json_error})"
                )
            })
        })?;
    if serialized.format.as_ref() != RUN_ARTIFACT_FORMAT {
        return Err(format!(
            "artifact is not an AIVI run artifact (expected format `{RUN_ARTIFACT_FORMAT}`)"
        ));
    }
    if serialized.version != RUN_ARTIFACT_VERSION {
        return Err(format!(
            "artifact uses run artifact format version {} but this runtime expects {}",
            serialized.version,
            RUN_ARTIFACT_VERSION
        ));
    }
    if let Some(requested_view) = requested_view
        && requested_view != serialized.view_name.as_ref()
    {
        return Err(format!(
            "run artifact bundles GTK view `{}`; requested `--view {requested_view}` does not match",
            serialized.view_name
        ));
    }
    deserialize_run_artifact(serialized, payload_reader)
}

fn source_run_cache_dir(
    cache_home: &Path,
    entry_path: &Path,
    requested_view: Option<&str>,
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    SOURCE_RUN_CACHE_FORMAT.hash(&mut hasher);
    SOURCE_RUN_CACHE_VERSION.hash(&mut hasher);
    SOURCE_RUN_CACHE_NAMESPACE_REVISION.hash(&mut hasher);
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    normalize_source_run_cache_path(entry_path)
        .to_string_lossy()
        .hash(&mut hasher);
    requested_view.hash(&mut hasher);
    cache_home
        .join("aivi")
        .join(SOURCE_RUN_CACHE_DIR)
        .join(format!("{:016x}", hasher.finish()))
}

fn normalize_source_run_cache_path(path: &Path) -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    fs::canonicalize(path)
        .or_else(|_| fs::canonicalize(cwd.join(path)))
        .unwrap_or_else(|_| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            }
        })
}

fn build_source_run_cache_manifest(
    entry_path: &Path,
    requested_view: Option<&str>,
    snapshot: &WorkspaceHirSnapshot,
) -> Result<SourceRunCacheManifest, String> {
    let mut dependencies = snapshot
        .files
        .iter()
        .map(|file| {
            let path = normalize_source_run_cache_path(&file.path(&snapshot.frontend.db));
            let fingerprint = source_run_cache_file_fingerprint(&path)?;
            Ok(SourceRunCacheDependency {
                path: path.to_string_lossy().into_owned().into_boxed_str(),
                fingerprint,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    dependencies.sort_by(|left, right| left.path.cmp(&right.path));
    dependencies.dedup_by(|left, right| left.path == right.path);
    Ok(SourceRunCacheManifest {
        format: SOURCE_RUN_CACHE_FORMAT.into(),
        version: SOURCE_RUN_CACHE_VERSION,
        entry_path: normalize_source_run_cache_path(entry_path)
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
        requested_view: requested_view.map(|view| view.into()),
        dependencies: dependencies.into_boxed_slice(),
    })
}

fn read_source_run_cache_manifest(path: &Path) -> Result<SourceRunCacheManifest, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))
}

fn validate_source_run_cache_manifest(
    manifest: &SourceRunCacheManifest,
    entry_path: &Path,
    requested_view: Option<&str>,
) -> Result<(), String> {
    if manifest.format.as_ref() != SOURCE_RUN_CACHE_FORMAT {
        return Err(format!(
            "source run cache format mismatch: expected `{SOURCE_RUN_CACHE_FORMAT}`, got `{}`",
            manifest.format
        ));
    }
    if manifest.version != SOURCE_RUN_CACHE_VERSION {
        return Err(format!(
            "source run cache version mismatch: expected {}, got {}",
            SOURCE_RUN_CACHE_VERSION, manifest.version
        ));
    }
    if manifest.entry_path.as_ref()
        != normalize_source_run_cache_path(entry_path).to_string_lossy()
    {
        return Err("source run cache entry path mismatch".to_owned());
    }
    if manifest.requested_view.as_deref() != requested_view {
        return Err("source run cache view mismatch".to_owned());
    }
    Ok(())
}

fn source_run_cache_dependencies_match(manifest: &SourceRunCacheManifest) -> bool {
    manifest.dependencies.iter().all(|dependency| {
        source_run_cache_file_fingerprint(Path::new(dependency.path.as_ref()))
            .is_ok_and(|fingerprint| fingerprint == dependency.fingerprint)
    })
}

fn source_run_cache_file_fingerprint(path: &Path) -> Result<u64, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

#[allow(dead_code)]
fn load_frozen_run_image(path: &Path, requested_view: Option<&str>) -> Result<RunArtifact, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    load_frozen_run_image_from_bytes(&bytes, requested_view)
}

fn load_frozen_run_image_from_bytes(
    bytes: &[u8],
    requested_view: Option<&str>,
) -> Result<RunArtifact, String> {
    let image: FrozenRunImage =
        bincode::deserialize(bytes).map_err(|error| format!("failed to decode frozen run image as binary: {error}"))?;
    if image.format.as_ref() != FROZEN_RUN_IMAGE_FORMAT {
        return Err(format!(
            "artifact is not an AIVI frozen run image (expected format `{FROZEN_RUN_IMAGE_FORMAT}`)"
        ));
    }
    if image.version != FROZEN_RUN_IMAGE_VERSION {
        return Err(format!(
            "artifact uses frozen run image format version {} but this runtime expects {}",
            image.version,
            FROZEN_RUN_IMAGE_VERSION
        ));
    }
    let payloads = FrozenPayloadLoader::new(image.backends, image.entries)?;
    deserialize_frozen_run_artifact(image.artifact, &payloads)
    .and_then(|artifact| {
        if let Some(requested_view) = requested_view
            && requested_view != artifact.view_name.as_ref()
        {
            return Err(format!(
                "frozen run image bundles GTK view `{}`; requested `--view {requested_view}` does not match",
                artifact.view_name
            ));
        }
        Ok(artifact)
    })
}

fn decode_backend_payload_bytes(
    bytes: &[u8],
    payload_path: &str,
) -> Result<aivi_runtime::hir_adapter::BackendRuntimePayload, String> {
    if let Ok(encoded) = bincode::deserialize::<EncodedBackendPayload>(bytes) {
        if encoded.format.as_ref() == ENCODED_BACKEND_PAYLOAD_FORMAT {
            if encoded.version != ENCODED_BACKEND_PAYLOAD_VERSION {
                return Err(format!(
                    "failed to decode backend payload {payload_path}: expected version {}, got {}",
                    ENCODED_BACKEND_PAYLOAD_VERSION,
                    encoded.version
                ));
            }
            return Ok(match encoded.payload {
                EncodedBackendPayloadKind::Program(program) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::Program(Arc::new(program))
                }
                EncodedBackendPayloadKind::Meta(meta) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(Arc::new(meta))
                }
                EncodedBackendPayloadKind::FrozenCatalog(catalog) => {
                    aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(Arc::new(catalog))
                }
            });
        }
    }
    if let Ok(meta) = bincode::deserialize::<aivi_backend::BackendRuntimeMeta>(bytes)
        && bincode::serialize(&meta).ok().as_deref() == Some(bytes)
    {
        return Ok(aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(Arc::new(meta)));
    }
    if let Ok(program) = aivi_backend::decode_program_binary(bytes)
        && aivi_backend::encode_program_binary(&program).ok().as_deref() == Some(bytes)
    {
        return Ok(aivi_runtime::hir_adapter::BackendRuntimePayload::Program(Arc::new(program)));
    }
    aivi_backend::decode_program_binary(bytes)
        .map(|program| aivi_runtime::hir_adapter::BackendRuntimePayload::Program(Arc::new(program)))
        .or_else(|binary_error| {
            aivi_backend::decode_program_json(bytes)
                .map(|program| aivi_runtime::hir_adapter::BackendRuntimePayload::Program(Arc::new(program)))
                .map_err(|json_error| {
                format!(
                    "failed to decode backend payload {payload_path} as runtime-meta binary, backend binary ({binary_error}) or JSON ({json_error})"
                )
            })
        })
}

fn decode_frozen_backend_payload_bytes(
    bytes: &[u8],
    payload_path: &str,
) -> Result<aivi_runtime::hir_adapter::BackendRuntimePayload, String> {
    if let Ok(encoded) = bincode::deserialize::<EncodedFrozenBackendCatalog>(bytes) {
        if encoded.format.as_ref() == FROZEN_BACKEND_CATALOG_FORMAT {
            if encoded.version != FROZEN_BACKEND_CATALOG_VERSION {
                return Err(format!(
                    "failed to decode frozen backend payload {payload_path}: expected version {}, got {}",
                    FROZEN_BACKEND_CATALOG_VERSION,
                    encoded.version
                ));
            }
            return Ok(aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(
                Arc::new(encoded.catalog),
            ));
        }
    }
    if let Ok(catalog) = bincode::deserialize::<aivi_backend::FrozenBackendCatalog>(bytes) {
        return Ok(aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(
            Arc::new(catalog),
        ));
    }
    if let Ok(meta) = bincode::deserialize::<aivi_backend::BackendRuntimeMeta>(bytes) {
        return Ok(aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(
            Arc::new(aivi_backend::FrozenBackendCatalog::from(&meta)),
        ));
    }
    decode_backend_payload_bytes(bytes, payload_path)
}

#[allow(dead_code)]
fn serialize_run_artifact(
    artifact: &RunArtifact,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<SerializedRunArtifact, String> {
    let kind = match &artifact.kind {
        RunArtifactKind::Gtk(surface) => SerializedRunArtifactKind::Gtk(SerializedRunGtkArtifact {
            patterns: surface
                .patterns
                .patterns
                .iter()
                .map(|(&id, pattern)| RunPatternEntryWire {
                    id,
                    pattern: pattern.clone(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            bridge: gtk_bridge_graph_to_wire(surface.bridge.clone()),
            hydration_inputs: surface
                .hydration_inputs
                .iter()
                .map(|(&input, compiled)| -> Result<RunInputEntryWire, String> {
                    Ok(RunInputEntryWire {
                        input,
                        compiled: compiled_run_input_to_wire(compiled, payloads)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
            event_handlers: surface
                .event_handlers
                .iter()
                .map(|(&handler, resolved)| RunEventHandlerEntryWire {
                    handler,
                    resolved: resolved.clone(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }),
        RunArtifactKind::HeadlessTask { task_owner } => {
            SerializedRunArtifactKind::HeadlessTask {
                task_owner: *task_owner,
            }
        }
    };
    Ok(SerializedRunArtifact {
        format: RUN_ARTIFACT_FORMAT.into(),
        version: RUN_ARTIFACT_VERSION,
        view_name: artifact.view_name.clone(),
        kind,
        required_signal_globals: artifact
            .required_signal_globals
            .iter()
            .map(|(&item, name)| RequiredSignalGlobalWire {
                item,
                name: name.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        runtime_assembly: hir_runtime_assembly_to_wire(artifact.runtime_assembly.clone(), payloads)?,
        runtime_link: artifact.runtime_link.clone(),
        backend: payloads.register_payload(
            artifact.backend.clone(),
            artifact.backend_native_kernels.clone(),
        )?,
        stub_signal_defaults: artifact
            .stub_signal_defaults
            .iter()
            .map(|(input, value)| StubSignalDefaultWire {
                input: *input,
                value: value.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

fn freeze_runtime_assembly_native_kernels(
    assembly: &HirRuntimeAssembly,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<HirRuntimeAssembly, String> {
    fn freeze_expr_native_kernels(
        expr: &aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
        payloads: &mut FrozenPayloadRegistry,
    ) -> Result<aivi_runtime::hir_adapter::HirCompiledRuntimeExpr, String> {
        let payload = payloads.register_payload(expr.backend.clone(), expr.native_kernels.clone())?;
        if payloads.include_native_kernels && payloads.freeze_backends {
            payloads.ensure_item_entry_abi(payload.handle, expr.entry_item)?;
        }
        Ok(aivi_runtime::hir_adapter::HirCompiledRuntimeExpr {
            backend: payloads.frozen_backend_payload(payload.handle)?,
            native_kernels: payloads.attached_native_kernels(payload.handle)?,
            entry_item: expr.entry_item,
            parameter_signals: expr.parameter_signals.clone(),
            required_signals: expr.required_signals.clone(),
        })
    }

    let mut parts = assembly.clone().into_parts();
    for signal in parts.signals.iter_mut() {
        if let aivi_runtime::HirSignalBindingKind::Reactive { clauses, .. } = &mut signal.kind {
            for clause in clauses.iter_mut() {
                clause.compiled_guard =
                    freeze_expr_native_kernels(&clause.compiled_guard, payloads)?;
                clause.compiled_body =
                    freeze_expr_native_kernels(&clause.compiled_body, payloads)?;
            }
        }
    }
    Ok(HirRuntimeAssembly::from_parts(parts))
}

fn serialize_frozen_run_artifact(
    artifact: &RunArtifact,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenSerializedRunArtifact, String> {
    let runtime_tables =
        aivi_runtime::derive_backend_linked_runtime_tables_with_seed_and_native_kernels_from_payload(
            &artifact.runtime_assembly,
            &artifact.backend,
            &artifact.backend_native_kernels,
            &artifact.runtime_link,
        )
        .map_err(|errors| {
            let joined = errors
                .errors()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            format!("failed to prelink frozen runtime tables: {joined}")
        })?;
    let frozen_runtime_assembly =
        freeze_runtime_assembly_native_kernels(&artifact.runtime_assembly, payloads)?;
    let backend = payloads.register_payload(
        artifact.backend.clone(),
        artifact.backend_native_kernels.clone(),
    )?;
    if payloads.include_native_kernels && payloads.freeze_backends {
        ensure_frozen_runtime_tables_compiled_only(payloads, backend.handle, &runtime_tables)?;
    }
    let kind = match &artifact.kind {
        RunArtifactKind::Gtk(surface) => {
            FrozenSerializedRunArtifactKind::Gtk(FrozenSerializedRunGtkArtifact {
                patterns: surface
                    .patterns
                    .patterns
                    .iter()
                    .map(|(&id, pattern)| RunPatternEntryWire {
                        id,
                        pattern: pattern.clone(),
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                bridge: gtk_bridge_graph_to_wire(surface.bridge.clone()),
                hydration_inputs: surface
                    .hydration_inputs
                    .iter()
                    .map(|(&input, compiled)| -> Result<FrozenRunInputEntryWire, String> {
                        Ok(FrozenRunInputEntryWire {
                            input,
                            compiled: frozen_compiled_run_input_to_wire(compiled, payloads)?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
                event_handlers: surface
                    .event_handlers
                    .iter()
                    .map(|(&handler, resolved)| RunEventHandlerEntryWire {
                        handler,
                        resolved: resolved.clone(),
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
        }
        RunArtifactKind::HeadlessTask { task_owner } => {
            FrozenSerializedRunArtifactKind::HeadlessTask {
                task_owner: *task_owner,
            }
        }
    };
    Ok(FrozenSerializedRunArtifact {
        format: FROZEN_RUN_IMAGE_FORMAT.into(),
        version: FROZEN_RUN_IMAGE_VERSION,
        view_name: artifact.view_name.clone(),
        kind,
        required_signal_globals: artifact
            .required_signal_globals
            .iter()
            .map(|(&item, name)| RequiredSignalGlobalWire {
                item,
                name: name.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        runtime_assembly: frozen_hir_runtime_assembly_to_wire(
            frozen_runtime_assembly,
            payloads,
        )?,
        runtime_tables: frozen_linked_runtime_tables_to_wire(&runtime_tables, payloads)?,
        backend,
        stub_signal_defaults: artifact
            .stub_signal_defaults
            .iter()
            .map(|(input, value)| StubSignalDefaultWire {
                input: *input,
                value: value.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

fn deserialize_run_artifact(
    serialized: SerializedRunArtifact,
    payload_reader: Box<dyn FnMut(&str) -> Result<Vec<u8>, String>>,
) -> Result<RunArtifact, String> {
    let mut payloads = ArtifactPayloadLoader::new(payload_reader);
    let backend = payloads.load(&serialized.backend)?;
    let kind = match serialized.kind {
        SerializedRunArtifactKind::Gtk(surface) => RunArtifactKind::Gtk(RunGtkArtifact {
            patterns: RunPatternTable {
                patterns: surface
                    .patterns
                    .into_vec()
                    .into_iter()
                    .map(|entry| (entry.id, entry.pattern))
                    .collect(),
            },
            bridge: gtk_bridge_graph_from_wire(surface.bridge)?,
            hydration_inputs: surface
                .hydration_inputs
                .into_vec()
                .into_iter()
                .map(|entry| {
                    compiled_run_input_from_wire(entry.compiled, &mut payloads)
                        .map(|compiled| (entry.input, compiled))
                })
                .collect::<Result<_, _>>()?,
            event_handlers: surface
                .event_handlers
                .into_vec()
                .into_iter()
                .map(|entry| (entry.handler, entry.resolved))
                .collect(),
        }),
        SerializedRunArtifactKind::HeadlessTask { task_owner } => {
            RunArtifactKind::HeadlessTask { task_owner }
        }
    };
    let mut artifact = RunArtifact {
        view_name: serialized.view_name,
        kind,
        required_signal_globals: serialized
            .required_signal_globals
            .into_vec()
            .into_iter()
            .map(|entry| (entry.item, entry.name))
            .collect(),
        sources: None,
        runtime_assembly: hir_runtime_assembly_from_wire(serialized.runtime_assembly, &mut payloads)?,
        runtime_link: serialized.runtime_link,
        runtime_tables: None,
        backend: backend.backend,
        backend_native_kernels: backend.native_kernels,
        stub_signal_defaults: serialized
            .stub_signal_defaults
            .into_vec()
            .into_iter()
            .map(|entry| (entry.input, entry.value))
            .collect(),
    };
    backfill_fragment_opaque_layout_variants(&mut artifact);
    Ok(artifact)
}

fn deserialize_frozen_run_artifact(
    serialized: FrozenSerializedRunArtifact,
    payloads: &FrozenPayloadLoader,
) -> Result<RunArtifact, String> {
    let backend = payloads.load(serialized.backend)?;
    let runtime_tables = frozen_linked_runtime_tables_from_wire(serialized.runtime_tables, payloads)?;
    let kind = match serialized.kind {
        FrozenSerializedRunArtifactKind::Gtk(surface) => RunArtifactKind::Gtk(RunGtkArtifact {
            patterns: RunPatternTable {
                patterns: surface
                    .patterns
                    .into_vec()
                    .into_iter()
                    .map(|entry| (entry.id, entry.pattern))
                    .collect(),
            },
            bridge: gtk_bridge_graph_from_wire(surface.bridge)?,
            hydration_inputs: surface
                .hydration_inputs
                .into_vec()
                .into_iter()
                .map(|entry| {
                    frozen_compiled_run_input_from_wire(entry.compiled, payloads)
                        .map(|compiled| (entry.input, compiled))
                })
                .collect::<Result<_, _>>()?,
            event_handlers: surface
                .event_handlers
                .into_vec()
                .into_iter()
                .map(|entry| (entry.handler, entry.resolved))
                .collect(),
        }),
        FrozenSerializedRunArtifactKind::HeadlessTask { task_owner } => {
            RunArtifactKind::HeadlessTask { task_owner }
        }
    };
    let mut artifact = RunArtifact {
        view_name: serialized.view_name,
        kind,
        required_signal_globals: serialized
            .required_signal_globals
            .into_vec()
            .into_iter()
            .map(|entry| (entry.item, entry.name))
            .collect(),
        sources: None,
        runtime_assembly: frozen_hir_runtime_assembly_from_wire(serialized.runtime_assembly, payloads)?,
        runtime_link: aivi_runtime::BackendRuntimeLinkSeed {
            hir_to_backend: Box::new([]),
        },
        runtime_tables: Some(runtime_tables),
        backend: backend.backend,
        backend_native_kernels: backend.native_kernels,
        stub_signal_defaults: serialized
            .stub_signal_defaults
            .into_vec()
            .into_iter()
            .map(|entry| (entry.input, entry.value))
            .collect(),
    };
    backfill_fragment_opaque_layout_variants(&mut artifact);
    Ok(artifact)
}

fn frozen_linked_runtime_tables_to_wire(
    tables: &aivi_runtime::BackendLinkedRuntimeTables,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenLinkedRuntimeTablesWire, String> {
    Ok(FrozenLinkedRuntimeTablesWire {
        signal_items_by_handle: tables
            .signal_items_by_handle
            .iter()
            .map(|(&signal, &item)| (signal, item))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        runtime_signal_by_item: tables
            .runtime_signal_by_item
            .iter()
            .map(|(&item, &signal)| (item, signal))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        derived_signals: tables
            .derived_signals
            .iter()
            .map(|(&handle, signal)| (handle, signal.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        reactive_signals: tables
            .reactive_signals
            .iter()
            .map(|(&handle, signal)| (handle, signal.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        reactive_clauses: tables
            .reactive_clauses
            .iter()
            .map(|(&handle, clause)| {
                Ok::<_, String>(FrozenLinkedReactiveClauseEntryWire {
                    handle,
                    clause: frozen_linked_reactive_clause_to_wire(clause.clone(), payloads)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        linked_recurrence_signals: tables
            .linked_recurrence_signals
            .iter()
            .map(|(&handle, signal)| (handle, signal.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        source_bindings: tables
            .source_bindings
            .iter()
            .map(|(&instance, binding)| (instance, binding.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        task_bindings: tables
            .task_bindings
            .iter()
            .map(|(&instance, binding)| (instance, binding.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        db_changed_routes: tables.db_changed_routes.clone(),
    })
}

fn ensure_frozen_runtime_tables_compiled_only(
    payloads: &mut FrozenPayloadRegistry,
    backend: FrozenBackendHandle,
    tables: &aivi_runtime::BackendLinkedRuntimeTables,
) -> Result<(), String> {
    fn ensure_eval_lane(
        payloads: &FrozenPayloadRegistry,
        backend: FrozenBackendHandle,
        lane: &aivi_runtime::startup::LinkedEvalLane,
        context: &str,
        fallback_kernel: Option<aivi_backend::KernelId>,
    ) -> Result<(), String> {
        match lane {
            aivi_runtime::startup::LinkedEvalLane::Native(eval) => {
                payloads.ensure_kernel_entry_abi(backend, eval.kernel, context)
            }
            aivi_runtime::startup::LinkedEvalLane::Fallback => {
                if let Some(kernel) = fallback_kernel {
                    payloads.ensure_kernel_entry_abi(backend, kernel, context)?;
                    Err(format!(
                        "{context} kernel {} still linked through interpreted fallback; built bundles are strict compiled-only",
                        kernel.as_raw()
                    ))
                } else {
                    Err(format!(
                        "{context} still uses interpreted fallback; built bundles are strict compiled-only"
                    ))
                }
            }
        }
    }

    for signal in tables.derived_signals.values() {
        if signal.entry_kernel.is_none() {
            continue;
        }
        ensure_eval_lane(
            payloads,
            backend,
            &signal.eval_lane,
            "derived signal entry",
            signal.entry_kernel,
        )?;
    }
    for signal in tables.reactive_signals.values() {
        if !signal.has_seed_body && signal.entry_kernel.is_none() {
            continue;
        }
        let payload = payloads.payload(backend)?;
        let item_name = match payload.backend.runtime_view().item(signal.backend_item) {
            Some(item) => item.name.as_ref(),
            None => "<missing-item>",
        };
        match &signal.seed_eval_lane {
            aivi_runtime::startup::LinkedEvalLane::Native(eval) => {
                payloads.ensure_kernel_entry_abi(
                    backend,
                    eval.kernel,
                    "reactive signal seed entry",
                )?;
            }
            aivi_runtime::startup::LinkedEvalLane::Fallback => {
                if let Some(kernel) = signal.body_kernel {
                    payloads.ensure_kernel_entry_abi(backend, kernel, "reactive signal seed entry")?;
                    return Err(format!(
                        "reactive signal seed entry for item {} ({item_name}) in backend {:016x} still linked through interpreted fallback via kernel {}; built bundles are strict compiled-only",
                        signal.item.as_raw(),
                        payload.key,
                        kernel.as_raw()
                    ));
                }
                if let Some(kernel) = signal.entry_kernel {
                    payloads.ensure_kernel_entry_abi(backend, kernel, "reactive signal seed entry")?;
                    return Err(format!(
                        "reactive signal seed entry for item {} ({item_name}) in backend {:016x} still linked through interpreted fallback via entry kernel {}; built bundles are strict compiled-only",
                        signal.item.as_raw(),
                        payload.key,
                        kernel.as_raw()
                    ));
                }
                return Err(format!(
                    "reactive signal seed entry for item {} ({item_name}) in backend {:016x} still uses interpreted fallback with no compiled body kernel; built bundles are strict compiled-only",
                    signal.item.as_raw(),
                    payload.key
                ));
            }
        }
    }
    for clause in tables.reactive_clauses.values() {
        let guard_backend = payloads
            .register_payload(
                clause.compiled_guard.backend.clone(),
                clause.compiled_guard.native_kernels.clone(),
            )?
            .handle;
        match &clause.guard_eval_lane {
            aivi_runtime::startup::LinkedEvalLane::Native(eval) => {
                payloads.ensure_kernel_entry_abi(guard_backend, eval.kernel, "reactive guard entry")?;
            }
            aivi_runtime::startup::LinkedEvalLane::Fallback => {
                payloads.ensure_item_entry_abi(guard_backend, clause.compiled_guard.entry_item)?;
                return Err(format!(
                    "reactive guard entry item {} still uses interpreted fallback in backend handle {}; built bundles are strict compiled-only",
                    clause.compiled_guard.entry_item.as_raw(),
                    guard_backend.0
                ));
            }
        }
        let body_backend = payloads
            .register_payload(
                clause.compiled_body.backend.clone(),
                clause.compiled_body.native_kernels.clone(),
            )?
            .handle;
        match &clause.body_eval_lane {
            aivi_runtime::startup::LinkedEvalLane::Native(eval) => {
                payloads.ensure_kernel_entry_abi(body_backend, eval.kernel, "reactive body entry")?;
            }
            aivi_runtime::startup::LinkedEvalLane::Fallback => {
                payloads.ensure_item_entry_abi(body_backend, clause.compiled_body.entry_item)?;
                return Err(format!(
                    "reactive body entry item {} still uses interpreted fallback in backend handle {}; built bundles are strict compiled-only",
                    clause.compiled_body.entry_item.as_raw(),
                    body_backend.0
                ));
            }
        }
    }
    for signal in tables.linked_recurrence_signals.values() {
        payloads.ensure_kernel_entry_abi(backend, signal.seed_kernel, "recurrence seed entry")?;
        for &kernel in signal.step_kernels.iter() {
            payloads.ensure_kernel_entry_abi(backend, kernel, "recurrence step entry")?;
        }
    }
    for binding in tables.source_bindings.values() {
        for argument in binding.arguments.iter() {
            payloads.ensure_kernel_entry_abi(backend, argument.kernel, "source argument entry")?;
        }
        for option in binding.options.iter() {
            payloads.ensure_kernel_entry_abi(backend, option.kernel, "source option entry")?;
        }
    }
    for binding in tables.task_bindings.values() {
        if let aivi_runtime::startup::LinkedTaskExecutionBinding::Ready { kernel, .. } =
            &binding.execution
        {
            payloads.ensure_kernel_entry_abi(backend, *kernel, "task entry")?;
        }
    }
    Ok(())
}

fn frozen_linked_runtime_tables_from_wire(
    wire: FrozenLinkedRuntimeTablesWire,
    payloads: &FrozenPayloadLoader,
) -> Result<aivi_runtime::BackendLinkedRuntimeTables, String> {
    Ok(aivi_runtime::BackendLinkedRuntimeTables {
        signal_items_by_handle: wire.signal_items_by_handle.into_vec().into_iter().collect(),
        runtime_signal_by_item: wire.runtime_signal_by_item.into_vec().into_iter().collect(),
        derived_signals: wire.derived_signals.into_vec().into_iter().collect(),
        reactive_signals: wire.reactive_signals.into_vec().into_iter().collect(),
        reactive_clauses: wire
            .reactive_clauses
            .into_vec()
            .into_iter()
            .map(|entry| {
                frozen_linked_reactive_clause_from_wire(entry.clause, payloads)
                    .map(|clause| (entry.handle, clause))
            })
            .collect::<Result<_, _>>()?,
        linked_recurrence_signals: wire.linked_recurrence_signals.into_vec().into_iter().collect(),
        source_bindings: wire.source_bindings.into_vec().into_iter().collect(),
        task_bindings: wire.task_bindings.into_vec().into_iter().collect(),
        db_changed_routes: wire.db_changed_routes,
    })
}

fn frozen_linked_reactive_clause_to_wire(
    clause: aivi_runtime::startup::LinkedReactiveClause,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenLinkedReactiveClauseWire, String> {
    Ok(FrozenLinkedReactiveClauseWire {
        owner: clause.owner,
        target: clause.target,
        clause: clause.clause,
        pipeline_ids: clause.pipeline_ids,
        body_mode: clause.body_mode,
        guard_eval_lane: clause.guard_eval_lane,
        body_eval_lane: clause.body_eval_lane,
        compiled_guard: frozen_hir_compiled_runtime_expr_to_wire(clause.compiled_guard, payloads)?,
        compiled_body: frozen_hir_compiled_runtime_expr_to_wire(clause.compiled_body, payloads)?,
    })
}

fn frozen_linked_reactive_clause_from_wire(
    wire: FrozenLinkedReactiveClauseWire,
    payloads: &FrozenPayloadLoader,
) -> Result<aivi_runtime::startup::LinkedReactiveClause, String> {
    Ok(aivi_runtime::startup::LinkedReactiveClause {
        owner: wire.owner,
        target: wire.target,
        clause: wire.clause,
        pipeline_ids: wire.pipeline_ids,
        body_mode: wire.body_mode,
        guard_eval_lane: wire.guard_eval_lane,
        body_eval_lane: wire.body_eval_lane,
        compiled_guard: frozen_hir_compiled_runtime_expr_from_wire(wire.compiled_guard, payloads)?,
        compiled_body: frozen_hir_compiled_runtime_expr_from_wire(wire.compiled_body, payloads)?,
    })
}

fn backfill_fragment_opaque_layout_variants(artifact: &mut RunArtifact) {
    let templates = collect_artifact_opaque_variant_templates(artifact);
    let carrier_templates = collect_artifact_representational_carrier_templates(artifact);
    if templates.is_empty() {
        if carrier_templates.is_empty() {
            return;
        }
    }

    backfill_backend_payload_opaque_variants(
        &mut artifact.backend,
        &templates,
        &carrier_templates,
    );

    if let Some(surface) = artifact.gtk_mut() {
        for input in surface.hydration_inputs.values_mut() {
            backfill_compiled_run_input_opaque_variants(input, &templates, &carrier_templates);
        }
    }

    let mut parts = artifact.runtime_assembly.clone().into_parts();
    for signal in parts.signals.iter_mut() {
        if let aivi_runtime::hir_adapter::HirSignalBindingKind::Reactive { clauses, .. } = &mut signal.kind
        {
            for clause in clauses.iter_mut() {
                backfill_runtime_expr_opaque_variants(
                    &mut clause.compiled_guard,
                    &templates,
                    &carrier_templates,
                );
                backfill_runtime_expr_opaque_variants(
                    &mut clause.compiled_body,
                    &templates,
                    &carrier_templates,
                );
            }
        }
    }
    artifact.runtime_assembly = aivi_runtime::hir_adapter::HirRuntimeAssembly::from_parts(parts);
}

fn collect_artifact_opaque_variant_templates(
    artifact: &RunArtifact,
) -> std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>> {
    let mut templates = std::collections::BTreeMap::new();
    collect_backend_payload_opaque_variant_templates(&mut templates, &artifact.backend);
    if let Some(surface) = artifact.gtk() {
        for input in surface.hydration_inputs.values() {
            collect_compiled_run_input_opaque_variant_templates(&mut templates, input);
        }
    }
    let parts = artifact.runtime_assembly.clone().into_parts();
    for signal in &parts.signals {
        if let aivi_runtime::hir_adapter::HirSignalBindingKind::Reactive { clauses, .. } = &signal.kind
        {
            for clause in clauses {
                collect_runtime_expr_opaque_variant_templates(&mut templates, &clause.compiled_guard);
                collect_runtime_expr_opaque_variant_templates(&mut templates, &clause.compiled_body);
            }
        }
    }
    templates
}

fn collect_artifact_representational_carrier_templates(
    artifact: &RunArtifact,
) -> std::collections::BTreeMap<String, String> {
    let mut templates = std::collections::BTreeMap::new();
    collect_backend_payload_representational_carrier_templates(&mut templates, &artifact.backend);
    if let Some(surface) = artifact.gtk() {
        for input in surface.hydration_inputs.values() {
            collect_compiled_run_input_representational_carrier_templates(&mut templates, input);
        }
    }
    let parts = artifact.runtime_assembly.clone().into_parts();
    for signal in &parts.signals {
        if let aivi_runtime::hir_adapter::HirSignalBindingKind::Reactive { clauses, .. } = &signal.kind
        {
            for clause in clauses {
                collect_runtime_expr_representational_carrier_templates(
                    &mut templates,
                    &clause.compiled_guard,
                );
                collect_runtime_expr_representational_carrier_templates(
                    &mut templates,
                    &clause.compiled_body,
                );
            }
        }
    }
    templates
}

fn collect_compiled_run_input_opaque_variant_templates(
    templates: &mut std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    input: &CompiledRunInput,
) {
    match input {
        CompiledRunInput::Expr(fragment) => {
            collect_run_fragment_opaque_variant_templates(templates, fragment);
        }
        CompiledRunInput::Text(text) => {
            for segment in &text.segments {
                if let CompiledRunTextSegment::Interpolation(fragment) = segment {
                    collect_run_fragment_opaque_variant_templates(templates, fragment);
                }
            }
        }
    }
}

fn collect_compiled_run_input_representational_carrier_templates(
    templates: &mut std::collections::BTreeMap<String, String>,
    input: &CompiledRunInput,
) {
    match input {
        CompiledRunInput::Expr(fragment) => {
            collect_run_fragment_representational_carrier_templates(templates, fragment);
        }
        CompiledRunInput::Text(text) => {
            for segment in &text.segments {
                if let CompiledRunTextSegment::Interpolation(fragment) = segment {
                    collect_run_fragment_representational_carrier_templates(templates, fragment);
                }
            }
        }
    }
}

fn collect_run_fragment_opaque_variant_templates(
    templates: &mut std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    fragment: &CompiledRunFragment,
) {
    collect_backend_payload_opaque_variant_templates(templates, &fragment.execution.backend);
}

fn collect_runtime_expr_opaque_variant_templates(
    templates: &mut std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    expr: &aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
) {
    collect_backend_payload_opaque_variant_templates(templates, &expr.backend);
}

fn collect_run_fragment_representational_carrier_templates(
    templates: &mut std::collections::BTreeMap<String, String>,
    fragment: &CompiledRunFragment,
) {
    collect_backend_payload_representational_carrier_templates(templates, &fragment.execution.backend);
}

fn collect_runtime_expr_representational_carrier_templates(
    templates: &mut std::collections::BTreeMap<String, String>,
    expr: &aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
) {
    collect_backend_payload_representational_carrier_templates(templates, &expr.backend);
}

fn collect_backend_payload_opaque_variant_templates(
    templates: &mut std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
) {
    let layouts = match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => program.layouts(),
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => meta.layouts(),
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => catalog.layouts(),
    };
    let signatures = layout_signatures(layouts);
    for (signature, variants) in opaque_variant_templates(layouts, &signatures) {
        templates.entry(signature).or_insert(variants);
    }
}

fn collect_backend_payload_representational_carrier_templates(
    templates: &mut std::collections::BTreeMap<String, String>,
    backend: &aivi_runtime::hir_adapter::BackendRuntimePayload,
) {
    let (layouts, carrier_for) = match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => (
            program.layouts(),
            Box::new(|layout| program.named_domain_carrier(layout))
                as Box<dyn Fn(aivi_backend::LayoutId) -> Option<aivi_backend::LayoutId>>,
        ),
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => (
            meta.layouts(),
            Box::new(|layout| meta.named_domain_carrier(layout))
                as Box<dyn Fn(aivi_backend::LayoutId) -> Option<aivi_backend::LayoutId>>,
        ),
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => (
            catalog.layouts(),
            Box::new(|layout| catalog.named_domain_carrier(layout))
                as Box<dyn Fn(aivi_backend::LayoutId) -> Option<aivi_backend::LayoutId>>,
        ),
    };
    let signatures = layout_signatures(layouts);
    for (layout_id, _) in layouts.iter() {
        let Some(carrier) = carrier_for(layout_id) else {
            continue;
        };
        let Some(layout_signature) = signatures.get(&layout_id) else {
            continue;
        };
        let Some(carrier_signature) = signatures.get(&carrier) else {
            continue;
        };
        templates
            .entry(layout_signature.clone())
            .or_insert_with(|| carrier_signature.clone());
    }
}

fn backfill_compiled_run_input_opaque_variants(
    input: &mut CompiledRunInput,
    templates: &std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    carrier_templates: &std::collections::BTreeMap<String, String>,
) {
    match input {
        CompiledRunInput::Expr(fragment) => {
            backfill_run_fragment_opaque_variants(fragment, templates, carrier_templates);
        }
        CompiledRunInput::Text(text) => {
            for segment in text.segments.iter_mut() {
                if let CompiledRunTextSegment::Interpolation(fragment) = segment {
                    backfill_run_fragment_opaque_variants(fragment, templates, carrier_templates);
                }
            }
        }
    }
}

fn backfill_run_fragment_opaque_variants(
    fragment: &mut CompiledRunFragment,
    templates: &std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    carrier_templates: &std::collections::BTreeMap<String, String>,
) {
    let execution = Arc::make_mut(&mut fragment.execution);
    backfill_backend_payload_opaque_variants(&mut execution.backend, templates, carrier_templates);
}

fn backfill_runtime_expr_opaque_variants(
    expr: &mut aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
    templates: &std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    carrier_templates: &std::collections::BTreeMap<String, String>,
) {
    backfill_backend_payload_opaque_variants(&mut expr.backend, templates, carrier_templates);
}

fn backfill_backend_payload_opaque_variants(
    backend: &mut aivi_runtime::hir_adapter::BackendRuntimePayload,
    templates: &std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
    carrier_templates: &std::collections::BTreeMap<String, String>,
) {
    match backend {
        aivi_runtime::hir_adapter::BackendRuntimePayload::Program(program) => {
            let program = Arc::make_mut(program);
            backfill_layout_collection_opaque_variants(program.layouts_mut(), templates);
            backfill_representational_carriers_in_program(program, carrier_templates);
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::Meta(meta) => {
            let meta = Arc::make_mut(meta);
            backfill_layout_collection_opaque_variants(meta.layouts_mut(), templates);
            backfill_representational_carriers_in_runtime_meta(meta, carrier_templates);
        }
        aivi_runtime::hir_adapter::BackendRuntimePayload::FrozenCatalog(catalog) => {
            let catalog = Arc::make_mut(catalog);
            backfill_layout_collection_opaque_variants(catalog.layouts_mut(), templates);
            backfill_representational_carriers_in_frozen_catalog(catalog, carrier_templates);
        }
    }
}

#[derive(Clone)]
struct OpaqueVariantTemplate {
    name: Box<str>,
    field_count: usize,
    field_signatures: Box<[String]>,
    payload_signature: Option<String>,
}

fn backfill_layout_collection_opaque_variants(
    layouts: &mut aivi_core::Arena<aivi_backend::LayoutId, aivi_backend::Layout>,
    templates: &std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>>,
) {
    let mut signatures = layout_signatures(layouts);
    let mut by_signature = signatures
        .iter()
        .map(|(layout, signature)| (signature.clone(), *layout))
        .collect::<std::collections::BTreeMap<_, _>>();
    let layout_ids = layouts.iter().map(|(layout_id, _)| layout_id).collect::<Vec<_>>();
    for layout_id in layout_ids {
        let aivi_backend::LayoutKind::Opaque { variants, .. } = &layouts[layout_id].kind else {
            continue;
        };
        if !variants.is_empty() {
            continue;
        }
        let Some(signature) = signatures.get(&layout_id) else {
            continue;
        };
        let Some(source_variants) = templates.get(signature) else {
            continue;
        };
        let rebuilt = source_variants
            .iter()
            .map(|variant| {
                let mut payload = variant
                    .payload_signature
                    .as_ref()
                    .and_then(|signature| by_signature.get(signature).copied());
                if payload.is_none() && variant.field_count > 1 {
                    let field_layouts = variant
                        .field_signatures
                        .iter()
                        .map(|signature| by_signature.get(signature).copied())
                        .collect::<Option<Vec<_>>>();
                    if let Some(field_layouts) = field_layouts {
                        let tuple_layout = aivi_backend::Layout::new(aivi_backend::LayoutKind::Tuple(
                            field_layouts,
                        ));
                        let tuple_id = layouts
                            .alloc(tuple_layout)
                            .expect("backfilled tuple payload layout should fit");
                        let tuple_signature = layout_signature(layouts, tuple_id, &mut signatures);
                        signatures.insert(tuple_id, tuple_signature.clone());
                        by_signature.insert(tuple_signature, tuple_id);
                        payload = Some(tuple_id);
                    }
                }
                aivi_backend::VariantLayout {
                    name: variant.name.clone(),
                    field_count: variant.field_count,
                    payload,
                }
            })
            .collect::<Vec<_>>();
        if rebuilt
            .iter()
            .all(|variant| variant.field_count == 0 || variant.payload.is_some())
        {
            let layout = layouts
                .get_mut(layout_id)
                .expect("layout collected from arena iteration should still exist");
            let aivi_backend::LayoutKind::Opaque { variants, .. } = &mut layout.kind else {
                continue;
            };
            *variants = rebuilt;
        }
    }
}

fn backfill_representational_carriers_in_program(
    program: &mut aivi_backend::Program,
    templates: &std::collections::BTreeMap<String, String>,
) {
    let signatures = layout_signatures(program.layouts());
    let by_signature = signatures
        .iter()
        .map(|(layout, signature)| (signature.clone(), *layout))
        .collect::<std::collections::BTreeMap<_, _>>();
    let layout_ids = program
        .layouts()
        .iter()
        .map(|(layout_id, _)| layout_id)
        .collect::<Vec<_>>();
    for layout_id in layout_ids {
        if program.named_domain_carrier(layout_id).is_some() {
            continue;
        }
        let Some(layout_signature) = signatures.get(&layout_id) else {
            continue;
        };
        let Some(carrier_signature) = templates.get(layout_signature) else {
            continue;
        };
        let Some(&carrier_layout) = by_signature.get(carrier_signature) else {
            continue;
        };
        program.register_named_domain_carrier(layout_id, carrier_layout);
    }
}

fn backfill_representational_carriers_in_runtime_meta(
    meta: &mut aivi_backend::BackendRuntimeMeta,
    templates: &std::collections::BTreeMap<String, String>,
) {
    let signatures = layout_signatures(meta.layouts());
    let by_signature = signatures
        .iter()
        .map(|(layout, signature)| (signature.clone(), *layout))
        .collect::<std::collections::BTreeMap<_, _>>();
    let layout_ids = meta.layouts().iter().map(|(layout_id, _)| layout_id).collect::<Vec<_>>();
    for layout_id in layout_ids {
        if meta.named_domain_carrier(layout_id).is_some() {
            continue;
        }
        let Some(layout_signature) = signatures.get(&layout_id) else {
            continue;
        };
        let Some(carrier_signature) = templates.get(layout_signature) else {
            continue;
        };
        let Some(&carrier_layout) = by_signature.get(carrier_signature) else {
            continue;
        };
        meta.register_named_domain_carrier(layout_id, carrier_layout);
    }
}

fn backfill_representational_carriers_in_frozen_catalog(
    catalog: &mut aivi_backend::FrozenBackendCatalog,
    templates: &std::collections::BTreeMap<String, String>,
) {
    let signatures = layout_signatures(catalog.layouts());
    let by_signature = signatures
        .iter()
        .map(|(layout, signature)| (signature.clone(), *layout))
        .collect::<std::collections::BTreeMap<_, _>>();
    let layout_ids = catalog
        .layouts()
        .iter()
        .map(|(layout_id, _)| layout_id)
        .collect::<Vec<_>>();
    for layout_id in layout_ids {
        if catalog.named_domain_carrier(layout_id).is_some() {
            continue;
        }
        let Some(layout_signature) = signatures.get(&layout_id) else {
            continue;
        };
        let Some(carrier_signature) = templates.get(layout_signature) else {
            continue;
        };
        let Some(&carrier_layout) = by_signature.get(carrier_signature) else {
            continue;
        };
        catalog.register_named_domain_carrier(layout_id, carrier_layout);
    }
}

fn opaque_variant_templates(
    layouts: &aivi_core::Arena<aivi_backend::LayoutId, aivi_backend::Layout>,
    signatures: &std::collections::BTreeMap<aivi_backend::LayoutId, String>,
) -> std::collections::BTreeMap<String, Box<[OpaqueVariantTemplate]>> {
    layouts
        .iter()
        .filter_map(|(layout_id, layout)| {
            let aivi_backend::LayoutKind::Opaque { variants, .. } = &layout.kind else {
                return None;
            };
            if variants.is_empty() {
                return None;
            }
            let signature = signatures.get(&layout_id)?.clone();
            let variants = variants
                .iter()
                .map(|variant| OpaqueVariantTemplate {
                    name: variant.name.clone(),
                    field_count: variant.field_count,
                    field_signatures: variant
                        .payload
                        .map(|layout| match &layouts[layout].kind {
                            aivi_backend::LayoutKind::Tuple(elements) if variant.field_count > 1 => {
                                elements
                                    .iter()
                                    .filter_map(|layout| signatures.get(layout).cloned())
                                    .collect::<Vec<_>>()
                                    .into_boxed_slice()
                            }
                            _ => signatures
                                .get(&layout)
                                .cloned()
                                .into_iter()
                                .collect::<Vec<_>>()
                                .into_boxed_slice(),
                        })
                        .unwrap_or_default(),
                    payload_signature: variant
                        .payload
                        .and_then(|layout| signatures.get(&layout).cloned()),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            Some((signature, variants))
        })
        .collect()
}

fn layout_signatures(
    layouts: &aivi_core::Arena<aivi_backend::LayoutId, aivi_backend::Layout>,
) -> std::collections::BTreeMap<aivi_backend::LayoutId, String> {
    let mut cache = std::collections::BTreeMap::new();
    for (layout_id, _) in layouts.iter() {
        let signature = layout_signature(layouts, layout_id, &mut cache);
        cache.insert(layout_id, signature);
    }
    cache
}

fn layout_signature(
    layouts: &aivi_core::Arena<aivi_backend::LayoutId, aivi_backend::Layout>,
    layout_id: aivi_backend::LayoutId,
    cache: &mut std::collections::BTreeMap<aivi_backend::LayoutId, String>,
) -> String {
    if let Some(existing) = cache.get(&layout_id) {
        return existing.clone();
    }
    let signature = match &layouts[layout_id].kind {
        aivi_backend::LayoutKind::Primitive(primitive) => format!("primitive:{primitive:?}"),
        aivi_backend::LayoutKind::Tuple(elements) => format!(
            "tuple({})",
            elements
                .iter()
                .map(|layout| layout_signature(layouts, *layout, cache))
                .collect::<Vec<_>>()
                .join(",")
        ),
        aivi_backend::LayoutKind::Record(fields) => format!(
            "record({})",
            fields
                .iter()
                .map(|field| format!(
                    "{}:{}",
                    field.name,
                    layout_signature(layouts, field.layout, cache)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        aivi_backend::LayoutKind::Sum(variants) => format!(
            "sum({})",
            variants
                .iter()
                .map(|variant| {
                    let payload = variant.payload.map_or_else(
                        || "none".to_owned(),
                        |layout| layout_signature(layouts, layout, cache),
                    );
                    format!("{}:{payload}", variant.name)
                })
                .collect::<Vec<_>>()
                .join(",")
        ),
        aivi_backend::LayoutKind::Arrow { parameter, result } => format!(
            "arrow({}->{})",
            layout_signature(layouts, *parameter, cache),
            layout_signature(layouts, *result, cache)
        ),
        aivi_backend::LayoutKind::List { element } => {
            format!("list({})", layout_signature(layouts, *element, cache))
        }
        aivi_backend::LayoutKind::Map { key, value } => format!(
            "map({},{})",
            layout_signature(layouts, *key, cache),
            layout_signature(layouts, *value, cache)
        ),
        aivi_backend::LayoutKind::Set { element } => {
            format!("set({})", layout_signature(layouts, *element, cache))
        }
        aivi_backend::LayoutKind::Option { element } => {
            format!("option({})", layout_signature(layouts, *element, cache))
        }
        aivi_backend::LayoutKind::Result { error, value } => format!(
            "result({},{})",
            layout_signature(layouts, *error, cache),
            layout_signature(layouts, *value, cache)
        ),
        aivi_backend::LayoutKind::Validation { error, value } => format!(
            "validation({},{})",
            layout_signature(layouts, *error, cache),
            layout_signature(layouts, *value, cache)
        ),
        aivi_backend::LayoutKind::Signal { element } => {
            format!("signal({})", layout_signature(layouts, *element, cache))
        }
        aivi_backend::LayoutKind::Task { error, value } => format!(
            "task({},{})",
            layout_signature(layouts, *error, cache),
            layout_signature(layouts, *value, cache)
        ),
        aivi_backend::LayoutKind::AnonymousDomain {
            carrier,
            surface_member,
        } => format!(
            "anonymous-domain({}:{})",
            surface_member,
            layout_signature(layouts, *carrier, cache)
        ),
        aivi_backend::LayoutKind::Domain { name, arguments } => format!(
            "domain({}:{})",
            name,
            arguments
                .iter()
                .map(|layout| layout_signature(layouts, *layout, cache))
                .collect::<Vec<_>>()
                .join(",")
        ),
        aivi_backend::LayoutKind::Opaque {
            name,
            arguments,
            ..
        } => format!(
            "opaque({}:{})",
            name,
            arguments
                .iter()
                .map(|layout| layout_signature(layouts, *layout, cache))
                .collect::<Vec<_>>()
                .join(",")
        ),
    };
    cache.insert(layout_id, signature.clone());
    signature
}

#[allow(dead_code)]
fn compiled_run_input_to_wire(
    input: &CompiledRunInput,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<CompiledRunInputWire, String> {
    match input {
        CompiledRunInput::Expr(fragment) => {
            Ok(CompiledRunInputWire::Expr(compiled_run_fragment_to_wire(
                fragment, payloads,
            )?))
        }
        CompiledRunInput::Text(text) => Ok(CompiledRunInputWire::Text(CompiledRunTextWire {
            segments: text
                .segments
                .iter()
                .map(|segment| match segment {
                    CompiledRunTextSegment::Text(text) => Ok(CompiledRunTextSegmentWire::Text(text.clone())),
                    CompiledRunTextSegment::Interpolation(fragment) => Ok(
                        CompiledRunTextSegmentWire::Interpolation(compiled_run_fragment_to_wire(
                            fragment, payloads,
                        )?),
                    ),
                })
                .collect::<Result<Vec<_>, String>>()?
                .into_boxed_slice(),
        })),
    }
}

fn compiled_run_input_from_wire(
    wire: CompiledRunInputWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<CompiledRunInput, String> {
    match wire {
        CompiledRunInputWire::Expr(fragment) => {
            compiled_run_fragment_from_wire(fragment, payloads).map(CompiledRunInput::Expr)
        }
        CompiledRunInputWire::Text(text) => Ok(CompiledRunInput::Text(CompiledRunText {
            segments: text
                .segments
                .into_vec()
                .into_iter()
                .map(|segment| match segment {
                    CompiledRunTextSegmentWire::Text(text) => Ok(CompiledRunTextSegment::Text(text)),
                    CompiledRunTextSegmentWire::Interpolation(fragment) => {
                        compiled_run_fragment_from_wire(fragment, payloads)
                            .map(CompiledRunTextSegment::Interpolation)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        })),
    }
}

fn frozen_compiled_run_input_to_wire(
    input: &CompiledRunInput,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenCompiledRunInputWire, String> {
    match input {
        CompiledRunInput::Expr(fragment) => Ok(FrozenCompiledRunInputWire::Expr(
            frozen_compiled_run_fragment_to_wire(fragment, payloads)?,
        )),
        CompiledRunInput::Text(text) => Ok(FrozenCompiledRunInputWire::Text(
            FrozenCompiledRunTextWire {
                segments: text
                    .segments
                    .iter()
                    .map(|segment| match segment {
                        CompiledRunTextSegment::Text(text) => {
                            Ok(FrozenCompiledRunTextSegmentWire::Text(text.clone()))
                        }
                        CompiledRunTextSegment::Interpolation(fragment) => Ok(
                            FrozenCompiledRunTextSegmentWire::Interpolation(
                                frozen_compiled_run_fragment_to_wire(fragment, payloads)?,
                            ),
                        ),
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .into_boxed_slice(),
            },
        )),
    }
}

fn frozen_compiled_run_input_from_wire(
    wire: FrozenCompiledRunInputWire,
    payloads: &FrozenPayloadLoader,
) -> Result<CompiledRunInput, String> {
    match wire {
        FrozenCompiledRunInputWire::Expr(fragment) => {
            frozen_compiled_run_fragment_from_wire(fragment, payloads).map(CompiledRunInput::Expr)
        }
        FrozenCompiledRunInputWire::Text(text) => Ok(CompiledRunInput::Text(CompiledRunText {
            segments: text
                .segments
                .into_vec()
                .into_iter()
                .map(|segment| match segment {
                    FrozenCompiledRunTextSegmentWire::Text(text) => {
                        Ok(CompiledRunTextSegment::Text(text))
                    }
                    FrozenCompiledRunTextSegmentWire::Interpolation(fragment) => {
                        frozen_compiled_run_fragment_from_wire(fragment, payloads)
                            .map(CompiledRunTextSegment::Interpolation)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        })),
    }
}

#[allow(dead_code)]
fn compiled_run_fragment_to_wire(
    fragment: &CompiledRunFragment,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<CompiledRunFragmentWire, String> {
    let execution = payloads.register_payload(
        fragment.execution.backend.clone(),
        fragment.execution.native_kernels.clone(),
    )?;
    Ok(CompiledRunFragmentWire {
        expr: fragment.expr,
        parameters: fragment.parameters.clone(),
        execution,
        item: fragment.item,
        required_signal_globals: fragment.required_signal_globals.clone(),
    })
}

fn compiled_run_fragment_from_wire(
    wire: CompiledRunFragmentWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<CompiledRunFragment, String> {
    let payload = payloads.load(&wire.execution)?;
    Ok(CompiledRunFragment {
        expr: wire.expr,
        parameters: wire.parameters,
        execution: Arc::new(RunFragmentExecutionUnit::new(
            payload.backend,
            payload.native_kernels,
        )),
        item: wire.item,
        required_signal_globals: wire.required_signal_globals,
    })
}

fn frozen_compiled_run_fragment_to_wire(
    fragment: &CompiledRunFragment,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenCompiledRunFragmentWire, String> {
    let entry = payloads.register_entry(
        fragment.execution.backend.clone(),
        fragment.execution.native_kernels.clone(),
        fragment.item,
    )?;
    Ok(FrozenCompiledRunFragmentWire {
        expr: fragment.expr,
        parameters: fragment.parameters.clone(),
        entry,
        required_signal_globals: fragment.required_signal_globals.clone(),
    })
}

fn frozen_compiled_run_fragment_from_wire(
    wire: FrozenCompiledRunFragmentWire,
    payloads: &FrozenPayloadLoader,
) -> Result<CompiledRunFragment, String> {
    let (payload, item) = payloads.load_entry(wire.entry)?;
    Ok(CompiledRunFragment {
        expr: wire.expr,
        parameters: wire.parameters,
        execution: Arc::new(RunFragmentExecutionUnit::new(
            payload.backend,
            payload.native_kernels,
        )),
        item,
        required_signal_globals: wire.required_signal_globals,
    })
}

#[allow(dead_code)]
fn hir_runtime_assembly_to_wire(
    assembly: HirRuntimeAssembly,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<HirRuntimeAssemblyWire, String> {
    let parts = assembly.into_parts();
    Ok(HirRuntimeAssemblyWire {
        graph: parts.graph,
        reactive_program: parts.reactive_program,
        owners: parts.owners,
        signals: parts
            .signals
                .into_vec()
                .into_iter()
                .map(|signal| hir_signal_binding_to_wire(signal, payloads))
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        sources: parts.sources,
        tasks: parts.tasks,
        gates: parts.gates,
        recurrences: parts.recurrences,
        db_changed_bindings: parts.db_changed_bindings,
    })
}

fn hir_runtime_assembly_from_wire(
    wire: HirRuntimeAssemblyWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<HirRuntimeAssembly, String> {
    Ok(HirRuntimeAssembly::from_parts(aivi_runtime::HirRuntimeAssemblyParts {
        graph: wire.graph,
        reactive_program: wire.reactive_program,
        owners: wire.owners,
        signals: wire
            .signals
            .into_vec()
            .into_iter()
            .map(|signal| hir_signal_binding_from_wire(signal, payloads))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        sources: wire.sources,
        tasks: wire.tasks,
        gates: wire.gates,
        recurrences: wire.recurrences,
        db_changed_bindings: wire.db_changed_bindings,
    }))
}

fn frozen_hir_runtime_assembly_to_wire(
    assembly: HirRuntimeAssembly,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenHirRuntimeAssemblyWire, String> {
    let parts = assembly.into_parts();
    Ok(FrozenHirRuntimeAssemblyWire {
        graph: parts.graph,
        reactive_program: parts.reactive_program,
        owners: parts.owners,
        signals: parts
            .signals
            .into_vec()
            .into_iter()
            .map(|signal| frozen_hir_signal_binding_to_wire(signal, payloads))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        sources: parts.sources,
        tasks: parts.tasks,
        gates: parts.gates,
        recurrences: parts.recurrences,
        db_changed_bindings: parts.db_changed_bindings,
    })
}

fn frozen_hir_runtime_assembly_from_wire(
    wire: FrozenHirRuntimeAssemblyWire,
    payloads: &FrozenPayloadLoader,
) -> Result<HirRuntimeAssembly, String> {
    Ok(HirRuntimeAssembly::from_parts(aivi_runtime::HirRuntimeAssemblyParts {
        graph: wire.graph,
        reactive_program: wire.reactive_program,
        owners: wire.owners,
        signals: wire
            .signals
            .into_vec()
            .into_iter()
            .map(|signal| frozen_hir_signal_binding_from_wire(signal, payloads))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
        sources: wire.sources,
        tasks: wire.tasks,
        gates: wire.gates,
        recurrences: wire.recurrences,
        db_changed_bindings: wire.db_changed_bindings,
    }))
}

#[allow(dead_code)]
fn hir_signal_binding_to_wire(
    binding: aivi_runtime::HirSignalBinding,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<HirSignalBindingWire, String> {
    let kind = match binding.kind {
        aivi_runtime::HirSignalBindingKind::Input { signal } => {
            HirSignalBindingKindWire::Input { signal }
        }
        aivi_runtime::HirSignalBindingKind::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        } => HirSignalBindingKindWire::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        },
        aivi_runtime::HirSignalBindingKind::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses,
        } => HirSignalBindingKindWire::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses: clauses
                .into_vec()
                .into_iter()
                .map(|clause| hir_reactive_update_binding_to_wire(clause, payloads))
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        },
    };
    Ok(HirSignalBindingWire {
        item: binding.item,
        span: binding.span,
        name: binding.name,
        owner: binding.owner,
        kind,
        source_input: binding.source_input,
    })
}

fn hir_signal_binding_from_wire(
    wire: HirSignalBindingWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<aivi_runtime::HirSignalBinding, String> {
    let kind = match wire.kind {
        HirSignalBindingKindWire::Input { signal } => {
            aivi_runtime::HirSignalBindingKind::Input { signal }
        }
        HirSignalBindingKindWire::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        } => aivi_runtime::HirSignalBindingKind::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        },
        HirSignalBindingKindWire::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses,
        } => aivi_runtime::HirSignalBindingKind::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses: clauses
                .into_vec()
                .into_iter()
                .map(|clause| hir_reactive_update_binding_from_wire(clause, payloads))
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        },
    };
    Ok(aivi_runtime::HirSignalBinding {
        item: wire.item,
        span: wire.span,
        name: wire.name,
        owner: wire.owner,
        kind,
        source_input: wire.source_input,
    })
}

fn frozen_hir_signal_binding_to_wire(
    binding: aivi_runtime::HirSignalBinding,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenHirSignalBindingWire, String> {
    let kind = match binding.kind {
        aivi_runtime::HirSignalBindingKind::Input { signal } => {
            FrozenHirSignalBindingKindWire::Input { signal }
        }
        aivi_runtime::HirSignalBindingKind::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        } => FrozenHirSignalBindingKindWire::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        },
        aivi_runtime::HirSignalBindingKind::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses,
        } => FrozenHirSignalBindingKindWire::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses: clauses
                .into_vec()
                .into_iter()
                .map(|clause| frozen_hir_reactive_update_binding_to_wire(clause, payloads))
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        },
    };
    Ok(FrozenHirSignalBindingWire {
        item: binding.item,
        span: binding.span,
        name: binding.name,
        owner: binding.owner,
        kind,
        source_input: binding.source_input,
    })
}

fn frozen_hir_signal_binding_from_wire(
    wire: FrozenHirSignalBindingWire,
    payloads: &FrozenPayloadLoader,
) -> Result<aivi_runtime::HirSignalBinding, String> {
    let kind = match wire.kind {
        FrozenHirSignalBindingKindWire::Input { signal } => {
            aivi_runtime::HirSignalBindingKind::Input { signal }
        }
        FrozenHirSignalBindingKindWire::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        } => aivi_runtime::HirSignalBindingKind::Derived {
            signal,
            dependencies,
            temporal_trigger_dependencies,
            temporal_helpers,
        },
        FrozenHirSignalBindingKindWire::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses,
        } => aivi_runtime::HirSignalBindingKind::Reactive {
            signal,
            dependencies,
            seed_dependencies,
            clauses: clauses
                .into_vec()
                .into_iter()
                .map(|clause| frozen_hir_reactive_update_binding_from_wire(clause, payloads))
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        },
    };
    Ok(aivi_runtime::HirSignalBinding {
        item: wire.item,
        span: wire.span,
        name: wire.name,
        owner: wire.owner,
        kind,
        source_input: wire.source_input,
    })
}

#[allow(dead_code)]
fn hir_reactive_update_binding_to_wire(
    binding: aivi_runtime::HirReactiveUpdateBinding,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<HirReactiveUpdateBindingWire, String> {
    Ok(HirReactiveUpdateBindingWire {
        span: binding.span,
        keyword_span: binding.keyword_span,
        target_span: binding.target_span,
        guard: binding.guard,
        body: binding.body,
        body_mode: binding.body_mode,
        clause: binding.clause,
        trigger_signal: binding.trigger_signal,
        guard_dependencies: binding.guard_dependencies,
        body_dependencies: binding.body_dependencies,
        compiled_guard: hir_compiled_runtime_expr_to_wire(binding.compiled_guard, payloads)?,
        compiled_body: hir_compiled_runtime_expr_to_wire(binding.compiled_body, payloads)?,
    })
}

fn hir_reactive_update_binding_from_wire(
    wire: HirReactiveUpdateBindingWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<aivi_runtime::HirReactiveUpdateBinding, String> {
    Ok(aivi_runtime::HirReactiveUpdateBinding {
        span: wire.span,
        keyword_span: wire.keyword_span,
        target_span: wire.target_span,
        guard: wire.guard,
        body: wire.body,
        body_mode: wire.body_mode,
        clause: wire.clause,
        trigger_signal: wire.trigger_signal,
        guard_dependencies: wire.guard_dependencies,
        body_dependencies: wire.body_dependencies,
        compiled_guard: hir_compiled_runtime_expr_from_wire(wire.compiled_guard, payloads)?,
        compiled_body: hir_compiled_runtime_expr_from_wire(wire.compiled_body, payloads)?,
    })
}

fn frozen_hir_reactive_update_binding_to_wire(
    binding: aivi_runtime::HirReactiveUpdateBinding,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenHirReactiveUpdateBindingWire, String> {
    Ok(FrozenHirReactiveUpdateBindingWire {
        span: binding.span,
        keyword_span: binding.keyword_span,
        target_span: binding.target_span,
        guard: binding.guard,
        body: binding.body,
        body_mode: binding.body_mode,
        clause: binding.clause,
        trigger_signal: binding.trigger_signal,
        guard_dependencies: binding.guard_dependencies,
        body_dependencies: binding.body_dependencies,
        compiled_guard: frozen_hir_compiled_runtime_expr_to_wire(binding.compiled_guard, payloads)?,
        compiled_body: frozen_hir_compiled_runtime_expr_to_wire(binding.compiled_body, payloads)?,
    })
}

fn frozen_hir_reactive_update_binding_from_wire(
    wire: FrozenHirReactiveUpdateBindingWire,
    payloads: &FrozenPayloadLoader,
) -> Result<aivi_runtime::HirReactiveUpdateBinding, String> {
    Ok(aivi_runtime::HirReactiveUpdateBinding {
        span: wire.span,
        keyword_span: wire.keyword_span,
        target_span: wire.target_span,
        guard: wire.guard,
        body: wire.body,
        body_mode: wire.body_mode,
        clause: wire.clause,
        trigger_signal: wire.trigger_signal,
        guard_dependencies: wire.guard_dependencies,
        body_dependencies: wire.body_dependencies,
        compiled_guard: frozen_hir_compiled_runtime_expr_from_wire(
            wire.compiled_guard,
            payloads,
        )?,
        compiled_body: frozen_hir_compiled_runtime_expr_from_wire(wire.compiled_body, payloads)?,
    })
}

#[allow(dead_code)]
fn hir_compiled_runtime_expr_to_wire(
    expr: aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<HirCompiledRuntimeExprWire, String> {
    Ok(HirCompiledRuntimeExprWire {
        backend: payloads.register_payload(expr.backend, expr.native_kernels.clone())?,
        entry_item: expr.entry_item,
        parameter_signals: expr.parameter_signals,
        required_signals: expr.required_signals,
    })
}

fn hir_compiled_runtime_expr_from_wire(
    wire: HirCompiledRuntimeExprWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<aivi_runtime::hir_adapter::HirCompiledRuntimeExpr, String> {
    let payload = payloads.load(&wire.backend)?;
    Ok(aivi_runtime::hir_adapter::HirCompiledRuntimeExpr {
        backend: payload.backend,
        native_kernels: payload.native_kernels,
        entry_item: wire.entry_item,
        parameter_signals: wire.parameter_signals,
        required_signals: wire.required_signals,
    })
}

fn frozen_hir_compiled_runtime_expr_to_wire(
    expr: aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
    payloads: &mut FrozenPayloadRegistry,
) -> Result<FrozenHirCompiledRuntimeExprWire, String> {
    Ok(FrozenHirCompiledRuntimeExprWire {
        entry: payloads.register_entry(expr.backend, expr.native_kernels.clone(), expr.entry_item)?,
        parameter_signals: expr.parameter_signals,
        required_signals: expr.required_signals,
    })
}

fn frozen_hir_compiled_runtime_expr_from_wire(
    wire: FrozenHirCompiledRuntimeExprWire,
    payloads: &FrozenPayloadLoader,
) -> Result<aivi_runtime::hir_adapter::HirCompiledRuntimeExpr, String> {
    let (payload, entry_item) = payloads.load_entry(wire.entry)?;
    Ok(aivi_runtime::hir_adapter::HirCompiledRuntimeExpr {
        backend: payload.backend,
        native_kernels: payload.native_kernels,
        entry_item,
        parameter_signals: wire.parameter_signals,
        required_signals: wire.required_signals,
    })
}

fn gtk_bridge_graph_to_wire(bridge: GtkBridgeGraph) -> GtkBridgeGraphWire {
    let parts = bridge.into_parts();
    GtkBridgeGraphWire {
        assembly: parts.assembly,
        root: parts.root,
        nodes: parts
            .nodes
            .into_vec()
            .into_iter()
            .map(gtk_bridge_node_to_wire)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    }
}

fn gtk_bridge_graph_from_wire(wire: GtkBridgeGraphWire) -> Result<GtkBridgeGraph, String> {
    Ok(GtkBridgeGraph::from_parts(aivi_gtk::GtkBridgeGraphParts {
        assembly: wire.assembly,
        root: wire.root,
        nodes: wire
            .nodes
            .into_vec()
            .into_iter()
            .map(gtk_bridge_node_from_wire)
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice(),
    }))
}

fn gtk_bridge_node_to_wire(node: aivi_gtk::GtkBridgeNode) -> GtkBridgeNodeWire {
    let kind = match node.kind {
        aivi_gtk::GtkBridgeNodeKind::Widget(widget) => GtkBridgeNodeKindWire::Widget(
            GtkWidgetNodeWire {
                widget: widget.widget,
                properties: widget.properties,
                event_hooks: widget.event_hooks,
                default_group_descriptor: widget
                    .default_group_descriptor
                    .map(|descriptor| descriptor.name.into()),
                default_children: widget.default_children,
            },
        ),
        aivi_gtk::GtkBridgeNodeKind::Group(group) => GtkBridgeNodeKindWire::Group(GtkGroupNodeWire {
            widget: group.widget,
            descriptor: group.descriptor.name.into(),
            body: group.body,
        }),
        aivi_gtk::GtkBridgeNodeKind::Show(show) => GtkBridgeNodeKindWire::Show(show),
        aivi_gtk::GtkBridgeNodeKind::Each(each) => GtkBridgeNodeKindWire::Each(each),
        aivi_gtk::GtkBridgeNodeKind::Empty(empty) => GtkBridgeNodeKindWire::Empty(empty),
        aivi_gtk::GtkBridgeNodeKind::Match(matched) => GtkBridgeNodeKindWire::Match(matched),
        aivi_gtk::GtkBridgeNodeKind::Case(case) => GtkBridgeNodeKindWire::Case(case),
        aivi_gtk::GtkBridgeNodeKind::Fragment(fragment) => {
            GtkBridgeNodeKindWire::Fragment(fragment)
        }
        aivi_gtk::GtkBridgeNodeKind::With(with) => GtkBridgeNodeKindWire::With(with),
    };
    GtkBridgeNodeWire {
        plan: node.plan,
        stable_id: node.stable_id,
        span: node.span,
        owner: node.owner,
        parent: node.parent,
        kind,
    }
}

fn gtk_bridge_node_from_wire(wire: GtkBridgeNodeWire) -> Result<aivi_gtk::GtkBridgeNode, String> {
    let kind = match wire.kind {
        GtkBridgeNodeKindWire::Widget(widget) => {
            let schema = lookup_widget_schema(&widget.widget).ok_or_else(|| {
                format!(
                    "run artifact references unknown GTK widget `{}`",
                    artifact_widget_name(&widget.widget)
                )
            })?;
            let default_group_descriptor = match widget.default_group_descriptor {
                Some(name) => Some(resolve_widget_child_group_descriptor(schema, name.as_ref())?),
                None => None,
            };
            aivi_gtk::GtkBridgeNodeKind::Widget(aivi_gtk::GtkWidgetNode {
                widget: widget.widget,
                properties: widget.properties,
                event_hooks: widget.event_hooks,
                default_group_descriptor,
                default_children: widget.default_children,
            })
        }
        GtkBridgeNodeKindWire::Group(group) => {
            let schema = lookup_widget_schema(&group.widget).ok_or_else(|| {
                format!(
                    "run artifact references unknown GTK widget `{}`",
                    artifact_widget_name(&group.widget)
                )
            })?;
            let descriptor = schema.child_group(group.descriptor.as_ref()).ok_or_else(|| {
                format!(
                    "run artifact references missing child group `{}` on GTK widget `{}`",
                    group.descriptor,
                    artifact_widget_name(&group.widget)
                )
            })?;
            aivi_gtk::GtkBridgeNodeKind::Group(aivi_gtk::GtkGroupNode {
                widget: group.widget,
                descriptor,
                body: group.body,
            })
        }
        GtkBridgeNodeKindWire::Show(show) => aivi_gtk::GtkBridgeNodeKind::Show(show),
        GtkBridgeNodeKindWire::Each(each) => aivi_gtk::GtkBridgeNodeKind::Each(each),
        GtkBridgeNodeKindWire::Empty(empty) => aivi_gtk::GtkBridgeNodeKind::Empty(empty),
        GtkBridgeNodeKindWire::Match(matched) => aivi_gtk::GtkBridgeNodeKind::Match(matched),
        GtkBridgeNodeKindWire::Case(case) => aivi_gtk::GtkBridgeNodeKind::Case(case),
        GtkBridgeNodeKindWire::Fragment(fragment) => {
            aivi_gtk::GtkBridgeNodeKind::Fragment(fragment)
        }
        GtkBridgeNodeKindWire::With(with) => aivi_gtk::GtkBridgeNodeKind::With(with),
    };
    Ok(aivi_gtk::GtkBridgeNode {
        plan: wire.plan,
        stable_id: wire.stable_id,
        span: wire.span,
        owner: wire.owner,
        parent: wire.parent,
        kind,
    })
}

fn resolve_widget_child_group_descriptor(
    schema: &aivi_gtk::GtkWidgetSchema,
    name: &str,
) -> Result<&'static aivi_gtk::GtkChildGroupDescriptor, String> {
    if let aivi_gtk::GtkDefaultChildGroup::One(group) = schema.default_child_group()
        && group.name == name
    {
        return Ok(group);
    }
    schema.child_group(name).ok_or_else(|| {
        format!(
            "run artifact references missing child group `{name}` on GTK widget `{}`",
            schema.markup_name
        )
    })
}

fn artifact_widget_name(path: &aivi_hir::NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}
