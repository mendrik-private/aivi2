const RUN_ARTIFACT_FORMAT: &str = "aivi.run-artifact";
const RUN_ARTIFACT_VERSION: u32 = 2;
const RUN_ARTIFACT_FILE_NAME: &str = "run-artifact.bin";
const RUN_ARTIFACT_PAYLOAD_DIR: &str = "payloads";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SerializedRunArtifact {
    format: Box<str>,
    version: u32,
    view_name: Box<str>,
    patterns: Box<[RunPatternEntryWire]>,
    bridge: GtkBridgeGraphWire,
    hydration_inputs: Box<[RunInputEntryWire]>,
    required_signal_globals: Box<[RequiredSignalGlobalWire]>,
    runtime_assembly: HirRuntimeAssemblyWire,
    runtime_link: aivi_runtime::BackendRuntimeLinkSeed,
    backend: BackendPayloadWire,
    event_handlers: Box<[RunEventHandlerEntryWire]>,
    stub_signal_defaults: Box<[StubSignalDefaultWire]>,
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

#[derive(Default)]
struct ArtifactPayloadRegistry {
    entries: BTreeMap<Box<str>, RegisteredBackendPayload>,
}

struct RegisteredBackendPayload {
    program: Arc<BackendProgram>,
    native_kernels: Box<[RegisteredNativeKernelPayload]>,
}

struct RegisteredNativeKernelPayload {
    kernel: aivi_backend::KernelId,
    artifact_path: Box<str>,
    artifact: aivi_backend::NativeKernelArtifact,
}

#[derive(Clone)]
struct LoadedBackendPayload {
    program: Arc<BackendProgram>,
    native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
}

impl ArtifactPayloadRegistry {
    fn register_program(&mut self, program: Arc<BackendProgram>) -> Result<BackendPayloadWire, String> {
        let key = compute_program_fingerprint(program.as_ref());
        self.register_keyed_program(key, program)
    }

    fn register_keyed_program(
        &mut self,
        key: u64,
        program: Arc<BackendProgram>,
    ) -> Result<BackendPayloadWire, String> {
        let program_path =
            format!("{RUN_ARTIFACT_PAYLOAD_DIR}/backend-{key:016x}.bin").into_boxed_str();
        let entry = match self.entries.entry(program_path.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let native_kernels = compile_native_kernel_payloads(key, program.as_ref())?;
                entry.insert(RegisteredBackendPayload {
                    program,
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
            let bytes = aivi_backend::encode_program_binary(payload.program.as_ref()).map_err(|error| {
                format!(
                    "failed to encode backend payload {} as binary: {error}",
                    relative_path.as_ref()
                )
            })?;
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
        let program = Arc::new(decode_backend_payload_bytes(
            &bytes,
            payload.program_path.as_ref(),
        )?);
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
            native_kernels.insert(
                aivi_backend::compute_kernel_fingerprint(program.as_ref(), native.kernel),
                artifact,
            );
        }
        let loaded = LoadedBackendPayload {
            program: program.clone(),
            native_kernels: Arc::new(native_kernels),
        };
        self.entries
            .insert(payload.program_path.clone(), loaded.clone());
        Ok(loaded)
    }
}

fn compile_native_kernel_payloads(
    key: u64,
    program: &BackendProgram,
) -> Result<Box<[RegisteredNativeKernelPayload]>, String> {
    let mut native_kernels = Vec::new();
    for (kernel, _) in program.kernels().iter() {
        let Some(artifact) = aivi_backend::compile_native_kernel_artifact(program, kernel).map_err(
            |error| {
                format!(
                    "failed to compile native backend payload for kernel {} in backend {key:016x}: {error}",
                    kernel.as_raw()
                )
            },
        )? else {
            continue;
        };
        let fingerprint = aivi_backend::compute_kernel_fingerprint(program, kernel);
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
    load_serialized_run_artifact(path, requested_view).map(Some)
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

fn decode_backend_payload_bytes(bytes: &[u8], payload_path: &str) -> Result<BackendProgram, String> {
    aivi_backend::decode_program_binary(bytes)
        .or_else(|binary_error| {
            aivi_backend::decode_program_json(bytes).map_err(|json_error| {
                format!(
                    "failed to decode backend payload {payload_path} as binary ({binary_error}) or JSON ({json_error})"
                )
            })
        })
}

fn serialize_run_artifact(
    artifact: &RunArtifact,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<SerializedRunArtifact, String> {
    Ok(SerializedRunArtifact {
        format: RUN_ARTIFACT_FORMAT.into(),
        version: RUN_ARTIFACT_VERSION,
        view_name: artifact.view_name.clone(),
        patterns: artifact
            .patterns
            .patterns
            .iter()
            .map(|(&id, pattern)| RunPatternEntryWire {
                id,
                pattern: pattern.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        bridge: gtk_bridge_graph_to_wire(artifact.bridge.clone()),
        hydration_inputs: artifact
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
        backend: payloads.register_program(artifact.backend.clone())?,
        event_handlers: artifact
            .event_handlers
            .iter()
            .map(|(&handler, resolved)| RunEventHandlerEntryWire {
                handler,
                resolved: resolved.clone(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
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
    Ok(RunArtifact {
        view_name: serialized.view_name,
        patterns: RunPatternTable {
            patterns: serialized
                .patterns
                .into_vec()
                .into_iter()
                .map(|entry| (entry.id, entry.pattern))
                .collect(),
        },
        bridge: gtk_bridge_graph_from_wire(serialized.bridge)?,
        hydration_inputs: serialized
            .hydration_inputs
            .into_vec()
            .into_iter()
            .map(|entry| {
                compiled_run_input_from_wire(entry.compiled, &mut payloads)
                    .map(|compiled| (entry.input, compiled))
            })
            .collect::<Result<_, _>>()?,
        required_signal_globals: serialized
            .required_signal_globals
            .into_vec()
            .into_iter()
            .map(|entry| (entry.item, entry.name))
            .collect(),
        runtime_assembly: hir_runtime_assembly_from_wire(serialized.runtime_assembly, &mut payloads)?,
        runtime_link: serialized.runtime_link,
        backend: backend.program,
        backend_native_kernels: backend.native_kernels,
        event_handlers: serialized
            .event_handlers
            .into_vec()
            .into_iter()
            .map(|entry| (entry.handler, entry.resolved))
            .collect(),
        stub_signal_defaults: serialized
            .stub_signal_defaults
            .into_vec()
            .into_iter()
            .map(|entry| (entry.input, entry.value))
            .collect(),
    })
}

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

fn compiled_run_fragment_to_wire(
    fragment: &CompiledRunFragment,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<CompiledRunFragmentWire, String> {
    let execution = payloads.register_keyed_program(
        fragment.execution.cache_key(),
        fragment.execution.backend.clone(),
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
    let cache_key = compute_program_fingerprint(payload.program.as_ref());
    Ok(CompiledRunFragment {
        expr: wire.expr,
        parameters: wire.parameters,
        execution: Arc::new(RunFragmentExecutionUnit::new(
            payload.program,
            payload.native_kernels,
            cache_key,
        )),
        item: wire.item,
        required_signal_globals: wire.required_signal_globals,
    })
}

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

fn hir_compiled_runtime_expr_to_wire(
    expr: aivi_runtime::hir_adapter::HirCompiledRuntimeExpr,
    payloads: &mut ArtifactPayloadRegistry,
) -> Result<HirCompiledRuntimeExprWire, String> {
    Ok(HirCompiledRuntimeExprWire {
        backend: payloads.register_program(expr.backend)?,
        entry_item: expr.entry_item,
        required_signals: expr.required_signals,
    })
}

fn hir_compiled_runtime_expr_from_wire(
    wire: HirCompiledRuntimeExprWire,
    payloads: &mut ArtifactPayloadLoader,
) -> Result<aivi_runtime::hir_adapter::HirCompiledRuntimeExpr, String> {
    let payload = payloads.load(&wire.backend)?;
    Ok(aivi_runtime::hir_adapter::HirCompiledRuntimeExpr {
        backend: payload.program,
        native_kernels: payload.native_kernels,
        entry_item: wire.entry_item,
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
