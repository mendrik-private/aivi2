#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHostValue(DetachedRuntimeValue);

impl GtkHostValue for RunHostValue {
    fn unit() -> Self {
        Self(DetachedRuntimeValue::unit())
    }

    fn from_bool(v: bool) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Bool(v),
        ))
    }

    fn from_text(v: &str) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(
            RuntimeValue::Text(v.to_owned().into()),
        ))
    }

    fn from_f64(v: f64) -> Self {
        match RuntimeFloat::new(v) {
            Some(rf) => Self(DetachedRuntimeValue::from_runtime_owned(
                RuntimeValue::Float(rf),
            )),
            None => Self::unit(),
        }
    }

    fn from_i64(v: i64) -> Self {
        Self(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            v,
        )))
    }

    fn as_bool(&self) -> Option<bool> {
        strip_signal_runtime_value(self.0.to_runtime()).as_bool()
    }

    fn as_i64(&self) -> Option<i64> {
        strip_signal_runtime_value(self.0.to_runtime()).as_i64()
    }

    fn as_f64(&self) -> Option<f64> {
        strip_signal_runtime_value(self.0.to_runtime()).as_float()
    }

    fn as_text(&self) -> Option<&str> {
        match strip_signal_runtime_ref(self.0.as_runtime()) {
            RuntimeValue::Text(value) => Some(value.as_ref()),
            RuntimeValue::Sum(sum) if sum.fields.is_empty() => Some(sum.variant_name.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct RunArtifact {
    view_name: Box<str>,
    kind: RunArtifactKind,
    required_signal_globals: BTreeMap<BackendItemId, Box<str>>,
    runtime_assembly: HirRuntimeAssembly,
    runtime_link: aivi_runtime::BackendRuntimeLinkSeed,
    runtime_tables: Option<aivi_runtime::BackendLinkedRuntimeTables>,
    backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
    backend_native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    /// Default values to publish into stub Input signal handles for cross-module
    /// workspace imports before the first hydration cycle. Keyed by the input handle
    /// that was synthesised in the runtime assembly for each import signal.
    stub_signal_defaults: Vec<(RuntimeInputHandle, DetachedRuntimeValue)>,
}

#[derive(Clone, Debug)]
enum RunArtifactKind {
    Gtk(RunGtkArtifact),
    HeadlessTask { task_owner: HirItemId },
}

#[derive(Clone, Debug)]
struct RunGtkArtifact {
    patterns: RunPatternTable,
    bridge: GtkBridgeGraph,
    hydration_inputs: BTreeMap<RuntimeInputHandle, CompiledRunInput>,
    event_handlers: BTreeMap<HirExprId, ResolvedRunEventHandler>,
}

impl RunArtifact {
    fn gtk(&self) -> Option<&RunGtkArtifact> {
        match &self.kind {
            RunArtifactKind::Gtk(surface) => Some(surface),
            RunArtifactKind::HeadlessTask { .. } => None,
        }
    }

    fn gtk_mut(&mut self) -> Option<&mut RunGtkArtifact> {
        match &mut self.kind {
            RunArtifactKind::Gtk(surface) => Some(surface),
            RunArtifactKind::HeadlessTask { .. } => None,
        }
    }

    fn expect_gtk(&self) -> &RunGtkArtifact {
        self.gtk()
            .expect("run artifact should carry a GTK surface in this context")
    }
}

impl std::ops::Deref for RunArtifact {
    type Target = RunGtkArtifact;

    fn deref(&self) -> &Self::Target {
        self.expect_gtk()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RunArtifactPreparationMetrics {
    workspace_collection: Duration,
    markup_lowering: Duration,
    widget_bridge_lowering: Duration,
    run_plan_validation: Duration,
    runtime_backend_lowering: Duration,
    runtime_assembly: Duration,
    reactive_fragment_compilation: Duration,
    markup_site_collection: Duration,
    hydration_fragment_compilation: Duration,
    event_handler_resolution: Duration,
    stub_signal_defaults: Duration,
    total: Duration,
    workspace_module_count: usize,
    runtime_backend_item_count: usize,
    runtime_backend_kernel_count: usize,
    hydration_fragment_count: usize,
    reactive_guard_fragment_count: usize,
    reactive_body_fragment_count: usize,
}

impl RunArtifactPreparationMetrics {
    fn reactive_fragment_count(self) -> usize {
        self.reactive_guard_fragment_count + self.reactive_body_fragment_count
    }
}

#[derive(Clone, Debug)]
struct PreparedRunArtifact {
    artifact: RunArtifact,
    metrics: RunArtifactPreparationMetrics,
}

#[derive(Clone, Debug)]
struct RunValidationBlocker {
    span: SourceSpan,
    message: String,
}

#[derive(Clone, Debug)]
struct RunFragmentExecutionUnit {
    backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
    native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
}

impl RunFragmentExecutionUnit {
    fn new(
        backend: aivi_runtime::hir_adapter::BackendRuntimePayload,
        native_kernels: Arc<aivi_backend::NativeKernelArtifactSet>,
    ) -> Self {
        Self {
            backend,
            native_kernels,
        }
    }

    fn backend_view(&self) -> aivi_backend::BackendRuntimeView<'_> {
        self.backend.runtime_view()
    }

    fn create_engine(&self, profiled: bool) -> BackendExecutionEngineHandle<'_> {
        let executable = self
            .backend
            .executable_program(self.native_kernels.as_ref())
            .with_execution_options(aivi_backend::BackendExecutionOptions {
                prefer_interpreter: cfg!(test),
                ..Default::default()
            });
        if profiled {
            executable.create_profiled_engine()
        } else {
            executable.create_engine()
        }
    }
}

#[derive(Clone, Debug)]
struct CompiledRunFragment {
    expr: HirExprId,
    parameters: Vec<RunFragmentParameter>,
    execution: Arc<RunFragmentExecutionUnit>,
    item: BackendItemId,
    required_signal_globals: Vec<CompiledRunSignalGlobal>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RunFragmentParameter {
    binding: aivi_hir::BindingId,
    name: Box<str>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CompiledRunSignalGlobal {
    fragment_item: BackendItemId,
    runtime_item: BackendItemId,
    name: Box<str>,
}

#[derive(Clone, Debug)]
enum CompiledRunInput {
    Expr(CompiledRunFragment),
    Text(CompiledRunText),
}

#[derive(Clone, Debug)]
struct CompiledRunText {
    segments: Box<[CompiledRunTextSegment]>,
}

#[derive(Clone, Debug)]
enum CompiledRunTextSegment {
    Text(Box<str>),
    Interpolation(CompiledRunFragment),
}

#[derive(Clone, Debug)]
enum RunInputSpec {
    Expr(HirExprId),
    Text(aivi_hir::TextLiteral),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RunPatternTable {
    patterns: BTreeMap<HirPatternId, RunPattern>,
}

impl RunPatternTable {
    fn insert(&mut self, id: HirPatternId, pattern: RunPattern) {
        self.patterns.insert(id, pattern);
    }

    fn get(&self, id: HirPatternId) -> Option<&RunPattern> {
        self.patterns.get(&id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RunPattern {
    kind: RunPatternKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum RunPatternKind {
    Wildcard,
    Binding {
        binding: aivi_hir::BindingId,
        name: Box<str>,
    },
    Integer {
        raw: Box<str>,
    },
    Text {
        value: Box<str>,
    },
    Tuple(Box<[HirPatternId]>),
    List {
        elements: Box<[HirPatternId]>,
        rest: Option<HirPatternId>,
    },
    Record(Box<[RunRecordPatternField]>),
    Constructor {
        callee: RunPatternConstructor,
        arguments: Box<[HirPatternId]>,
    },
    UnresolvedName,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RunRecordPatternField {
    label: Box<str>,
    pattern: HirPatternId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum RunPatternConstructor {
    Builtin(BuiltinTerm),
    Item {
        item: HirItemId,
        variant_name: Box<str>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RunInputCompilationMetrics {
    compiled_fragment_count: usize,
}

#[derive(Clone, Debug)]
struct RunHydrationStaticState {
    view_name: Box<str>,
    patterns: RunPatternTable,
    bridge: GtkBridgeGraph,
    inputs: BTreeMap<RuntimeInputHandle, CompiledRunInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHydrationPlan {
    root: HydratedRunNode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunHydrationFragmentProfile {
    program_key: usize,
    entry_item: BackendItemId,
    evaluations: u64,
    total_time: Duration,
    max_time: Duration,
}

impl RunHydrationFragmentProfile {
    fn record(&mut self, elapsed: Duration) {
        self.evaluations += 1;
        self.total_time += elapsed;
        self.max_time = self.max_time.max(elapsed);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RunHydrationProfile {
    total_time: Duration,
    planned_nodes: u64,
    evaluated_inputs: u64,
    evaluated_texts: u64,
    fragment_profiles: BTreeMap<HirExprId, RunHydrationFragmentProfile>,
    program_profiles: BTreeMap<usize, KernelEvaluationProfile>,
}

#[cfg_attr(not(test), allow(dead_code))]
enum RunHydrationProfiler {
    Disabled,
    Enabled(RunHydrationProfile),
}

#[cfg_attr(not(test), allow(dead_code))]
impl RunHydrationProfiler {
    fn disabled() -> Self {
        Self::Disabled
    }

    fn enabled() -> Self {
        Self::Enabled(RunHydrationProfile::default())
    }

    fn kernel_profiling_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    fn record_node(&mut self) {
        if let Self::Enabled(profile) = self {
            profile.planned_nodes += 1;
        }
    }

    fn record_input(&mut self) {
        if let Self::Enabled(profile) = self {
            profile.evaluated_inputs += 1;
        }
    }

    fn record_text(&mut self) {
        if let Self::Enabled(profile) = self {
            profile.evaluated_texts += 1;
        }
    }

    fn record_fragment(&mut self, fragment: &CompiledRunFragment, elapsed: Duration) {
        let Self::Enabled(profile) = self else {
            return;
        };
        let program_key = Arc::as_ptr(&fragment.execution) as usize;
        let entry = profile
            .fragment_profiles
            .entry(fragment.expr)
            .or_insert_with(|| RunHydrationFragmentProfile {
                program_key,
                entry_item: fragment.item,
                evaluations: 0,
                total_time: Duration::ZERO,
                max_time: Duration::ZERO,
            });
        entry.record(elapsed);
    }

    fn finish<'a>(&mut self, total_time: Duration, evaluators: &EvaluatorCache<'a>) {
        let Self::Enabled(profile) = self else {
            return;
        };
        profile.total_time = total_time;
        for (program_key, evaluator) in evaluators {
            let Some(kernel_profile) = evaluator.profile_snapshot() else {
                continue;
            };
            profile
                .program_profiles
                .entry(*program_key)
                .or_default()
                .merge_from(&kernel_profile);
        }
    }

    fn into_profile(self) -> Option<RunHydrationProfile> {
        match self {
            Self::Disabled => None,
            Self::Enabled(profile) => Some(profile),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HydratedRunNode {
    Widget {
        instance: GtkNodeInstance,
        properties: Box<[HydratedRunProperty]>,
        event_inputs: Box<[HydratedRunProperty]>,
        children: Box<[HydratedRunNode]>,
    },
    Show {
        instance: GtkNodeInstance,
        when_input: RuntimeInputHandle,
        when: bool,
        keep_mounted_input: Option<RuntimeInputHandle>,
        keep_mounted: bool,
        children: Box<[HydratedRunNode]>,
    },
    Each {
        instance: GtkNodeInstance,
        collection_input: RuntimeInputHandle,
        kind: HydratedRunEachKind,
        empty_branch: Option<Box<HydratedRunNode>>,
    },
    Match {
        instance: GtkNodeInstance,
        scrutinee_input: RuntimeInputHandle,
        active_case: usize,
        branch: Box<HydratedRunNode>,
    },
    Case {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
    Fragment {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
    With {
        instance: GtkNodeInstance,
        value_input: RuntimeInputHandle,
        children: Box<[HydratedRunNode]>,
    },
    Empty {
        instance: GtkNodeInstance,
        children: Box<[HydratedRunNode]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HydratedRunProperty {
    input: RuntimeInputHandle,
    value: DetachedRuntimeValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HydratedRunEachKind {
    Positional {
        item_count: usize,
        items: Box<[HydratedRunEachItem]>,
    },
    Keyed {
        key_input: RuntimeInputHandle,
        keys: Box<[GtkCollectionKey]>,
        items: Box<[HydratedRunEachItem]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HydratedRunEachItem {
    children: Box<[HydratedRunNode]>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum ResolvedRunEventPayload {
    GtkPayload,
    ScopedInput,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ResolvedRunEventHandler {
    signal_item: aivi_hir::ItemId,
    signal_name: Box<str>,
    signal_input: RuntimeInputHandle,
    payload: ResolvedRunEventPayload,
}
/// RAII wrapper that deletes a temporary file on drop.
///
/// This ensures temporary files are cleaned up even when the program exits
/// early due to an error or a panic.
#[allow(dead_code)]
struct TempFile(PathBuf);

#[allow(dead_code)]
impl TempFile {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// RAII wrapper around [`tempfile::TempDir`] that places a staging directory
/// in the given parent so it lives on the same filesystem as the final output
/// path, enabling an atomic `fs::rename` on success.
///
/// On drop the directory and all its contents are removed automatically,
/// even when the process exits early due to an error or a panic.
struct StagingDir(tempfile::TempDir);

impl StagingDir {
    /// Create a new temporary staging directory inside `parent`.
    fn new_in(parent: &Path) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix(".aivi-bundle-staging-")
            .tempdir_in(parent)
            .map_err(|error| {
                format!(
                    "failed to create staging directory in {}: {error}",
                    parent.display()
                )
            })?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}
