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
    MissingTemporalHelper {
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline: BackendPipelineId,
        stage_index: usize,
    },
    SpawnTemporalWorker {
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline: BackendPipelineId,
        stage_index: usize,
        message: Box<str>,
    },
    InvalidTemporalDelayDuration {
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline: BackendPipelineId,
        stage_index: usize,
        value: RuntimeValue,
    },
    InvalidTemporalBurstInterval {
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline: BackendPipelineId,
        stage_index: usize,
        value: RuntimeValue,
    },
    InvalidTemporalBurstCount {
        signal: DerivedHandle,
        item: hir::ItemId,
        pipeline: BackendPipelineId,
        stage_index: usize,
        value: RuntimeValue,
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
            Self::MissingTemporalHelper {
                signal,
                item,
                pipeline,
                stage_index,
            } => write!(
                f,
                "derived signal {:?} / item {item} is missing a temporal helper for pipeline {:?} stage {}",
                signal, pipeline, stage_index
            ),
            Self::SpawnTemporalWorker {
                signal,
                item,
                pipeline,
                stage_index,
                message,
            } => write!(
                f,
                "derived signal {:?} / item {item} failed to start temporal worker for pipeline {:?} stage {}: {message}",
                signal, pipeline, stage_index
            ),
            Self::InvalidTemporalDelayDuration {
                signal,
                item,
                pipeline,
                stage_index,
                value,
            } => write!(
                f,
                "derived signal {:?} / item {item} produced invalid delay duration for pipeline {:?} stage {}: {value:?}",
                signal, pipeline, stage_index
            ),
            Self::InvalidTemporalBurstInterval {
                signal,
                item,
                pipeline,
                stage_index,
                value,
            } => write!(
                f,
                "derived signal {:?} / item {item} produced invalid burst interval for pipeline {:?} stage {}: {value:?}",
                signal, pipeline, stage_index
            ),
            Self::InvalidTemporalBurstCount {
                signal,
                item,
                pipeline,
                stage_index,
                value,
            } => write!(
                f,
                "derived signal {:?} / item {item} produced invalid burst count for pipeline {:?} stage {}: {value:?}",
                signal, pipeline, stage_index
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
