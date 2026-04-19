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

fn stage_subject_value(
    backend: BackendRuntimeView<'_>,
    layout: aivi_backend::LayoutId,
    value: &RuntimeValue,
) -> RuntimeValue {
    match (&backend.layout(layout).expect("linked runtime layout should exist").kind, value) {
        (LayoutKind::Signal { .. }, RuntimeValue::Signal(_)) => value.clone(),
        (LayoutKind::Signal { .. }, other) => RuntimeValue::Signal(Box::new(other.clone())),
        (_, RuntimeValue::Signal(inner)) => inner.as_ref().clone(),
        _ => value.clone(),
    }
}

fn unwrap_signal_layout_result(
    backend: BackendRuntimeView<'_>,
    layout: aivi_backend::LayoutId,
    value: RuntimeValue,
) -> RuntimeValue {
    match (&backend.layout(layout).expect("linked runtime layout should exist").kind, value) {
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

fn parse_temporal_duration(value: &RuntimeValue) -> Option<Duration> {
    match strip_runtime_signal(value) {
        RuntimeValue::Int(value) if *value > 0 => Some(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount = raw.parse::<u64>().ok()?;
            match suffix.as_ref() {
                "ns" => Some(Duration::from_nanos(amount)),
                "us" => Some(Duration::from_micros(amount)),
                "ms" => Some(Duration::from_millis(amount)),
                "s" | "sec" => Some(Duration::from_secs(amount)),
                "m" | "min" => amount.checked_mul(60).map(Duration::from_secs),
                "h" | "hr" => amount.checked_mul(60 * 60).map(Duration::from_secs),
                "dy" => amount.checked_mul(60 * 60 * 24).map(Duration::from_secs),
                _ => None,
            }
        }
        other => match other {
            RuntimeValue::Int(_) => None,
            _ => None,
        },
    }
}

fn parse_temporal_count(value: &RuntimeValue) -> Option<u64> {
    match strip_runtime_signal(value) {
        RuntimeValue::Int(value) if *value > 0 => Some(*value as u64),
        RuntimeValue::SuffixedInteger { raw, suffix } if suffix.as_ref() == "times" => {
            raw.parse::<u64>().ok().filter(|value| *value > 0)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct TemporalStageKey {
    pipeline: BackendPipelineId,
    stage_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedTemporalHelper {
    pub input: InputHandle,
    pub dependency_index: usize,
    pub pipeline: BackendPipelineId,
    pub pipeline_position: usize,
    pub stage_index: usize,
    pub stage_offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedEvalLane {
    Native(LinkedNativeKernelEval),
    Fallback,
}

impl LinkedEvalLane {
    pub fn as_native(&self) -> Option<&LinkedNativeKernelEval> {
        match self {
            Self::Native(eval) => Some(eval),
            Self::Fallback => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedNativeKernelEval {
    pub kernel: KernelId,
    pub dependency_layouts: Box<[aivi_backend::LayoutId]>,
    pub result_layout: aivi_backend::LayoutId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedDerivedSignal {
    pub item: hir::ItemId,
    pub signal: DerivedHandle,
    pub backend_item: BackendItemId,
    pub body_kernel: Option<KernelId>,
    pub eval_lane: LinkedEvalLane,
    pub runtime_dependency_count: usize,
    pub dependency_items: Box<[BackendItemId]>,
    pub dependency_layouts: Box<[aivi_backend::LayoutId]>,
    pub source_input: Option<InputHandle>,
    /// Backend pipeline IDs that must be applied to the body result in order.
    pub pipeline_ids: Box<[BackendPipelineId]>,
    pub temporal_trigger_dependencies: Box<[SignalHandle]>,
    pub temporal_helpers: Box<[LinkedTemporalHelper]>,
}

impl LinkedDerivedSignal {
    fn temporal_helper(&self, key: TemporalStageKey) -> Option<&LinkedTemporalHelper> {
        self.temporal_helpers
            .iter()
            .find(|helper| helper.pipeline == key.pipeline && helper.stage_index == key.stage_index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TemporalResumePoint {
    pipeline_position: usize,
    stage_offset: usize,
}

#[derive(Debug)]
struct TemporalWorkerHandle {
    commands: mpsc::Sender<TemporalWorkerCommand>,
    join: Option<JoinHandle<()>>,
}

impl TemporalWorkerHandle {
    fn schedule(
        &self,
        schedule: TemporalWorkerSchedule,
    ) -> Result<(), mpsc::SendError<TemporalWorkerCommand>> {
        self.commands
            .send(TemporalWorkerCommand::Schedule(schedule))
    }

    fn stop(&mut self) {
        let _ = self.commands.send(TemporalWorkerCommand::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Clone, Debug)]
enum TemporalWorkerCommand {
    Schedule(TemporalWorkerSchedule),
    Stop,
}

#[derive(Clone, Debug)]
struct TemporalWorkerSchedule {
    stamp: crate::PublicationStamp,
    value: RuntimeValue,
    kind: TemporalWorkerScheduleKind,
}

#[derive(Clone, Debug)]
enum TemporalWorkerScheduleKind {
    Delay { wait: Duration },
    Burst { wait: Duration, remaining: u64 },
}

#[derive(Clone, Debug)]
struct PendingTemporalSchedule {
    signal: DerivedHandle,
    item: hir::ItemId,
    key: TemporalStageKey,
    value: RuntimeValue,
    kind: TemporalWorkerScheduleKind,
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
    pub body_kernel: Option<KernelId>,
    pub seed_eval_lane: LinkedEvalLane,
    pub dependency_items: Box<[BackendItemId]>,
    pub dependency_layouts: Box<[aivi_backend::LayoutId]>,
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
    pub guard_eval_lane: LinkedEvalLane,
    pub body_eval_lane: LinkedEvalLane,
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
