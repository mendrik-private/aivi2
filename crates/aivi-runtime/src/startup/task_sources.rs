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

fn spawn_temporal_worker(
    _signal: DerivedHandle,
    _item: hir::ItemId,
    _key: TemporalStageKey,
    sender: WorkerPublicationSender<RuntimeValue>,
) -> Result<TemporalWorkerHandle, String> {
    let (command_tx, command_rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name("aivi-temporal-stage".into())
        .spawn(move || run_temporal_worker(command_rx, sender))
        .map_err(|error| error.to_string())?;
    Ok(TemporalWorkerHandle {
        commands: command_tx,
        join: Some(join),
    })
}

fn run_temporal_worker(
    command_rx: mpsc::Receiver<TemporalWorkerCommand>,
    sender: WorkerPublicationSender<RuntimeValue>,
) {
    let mut active: Option<TemporalWorkerSchedule> = None;
    loop {
        let Some(mut schedule) = active.take() else {
            match command_rx.recv() {
                Ok(TemporalWorkerCommand::Schedule(schedule)) => {
                    active = Some(schedule);
                }
                Ok(TemporalWorkerCommand::Stop) | Err(_) => break,
            }
            continue;
        };

        let wait = match schedule.kind {
            TemporalWorkerScheduleKind::Delay { wait }
            | TemporalWorkerScheduleKind::Burst { wait, .. } => wait,
        };
        match command_rx.recv_timeout(wait) {
            Ok(TemporalWorkerCommand::Schedule(next)) => {
                active = Some(next);
            }
            Ok(TemporalWorkerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if sender
                    .publish(Publication::new(schedule.stamp, schedule.value.clone()))
                    .is_err()
                {
                    break;
                }
                active = match schedule.kind {
                    TemporalWorkerScheduleKind::Delay { .. } => None,
                    TemporalWorkerScheduleKind::Burst { wait, remaining } if remaining > 1 => {
                        schedule.kind = TemporalWorkerScheduleKind::Burst {
                            wait,
                            remaining: remaining - 1,
                        };
                        Some(schedule)
                    }
                    TemporalWorkerScheduleKind::Burst { .. } => None,
                };
            }
        }
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
    execution_context: SourceProviderContext,
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
        execution_context,
    } = task;
    if completion.is_cancelled() {
        return Ok(LinkedTaskWorkerOutcome::Cancelled);
    }
    let mut engine = BackendExecutableProgram::interpreted(backend.as_ref()).create_engine();
    let runtime_globals = materialize_detached_globals(&globals);
    let value = engine
        .evaluate_item(backend_item, &runtime_globals)
        .map_err(|error| LinkedTaskWorkerError::Evaluation {
            instance,
            owner,
            backend_item,
            error,
        })?;
    // Use the applier-aware executor so that deferred Task composition plans
    // (Map, Apply, Chain, Join) can call back into the execution engine.
    let mut applier = EvaluatorApplier {
        evaluator: &mut *engine,
        globals: &runtime_globals,
    };
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    let outcome = execute_runtime_value_with_context_effects_and_applier(
        value,
        &execution_context,
        &mut stdout,
        &mut stderr,
        &mut applier,
        &runtime_globals,
    )
    .map_err(|error| LinkedTaskWorkerError::TaskExecution {
        instance,
        owner,
        backend_item,
        error,
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

/// Bridges [`BackendExecutionEngine`] to the [`TaskFunctionApplier`] interface so the task
/// executor can apply user closures while executing deferred `Map`/`Chain`/`Join` task plans.
struct EvaluatorApplier<'a, 'b> {
    evaluator: &'a mut (dyn BackendExecutionEngine + 'b),
    globals: &'a BTreeMap<BackendItemId, RuntimeValue>,
}

impl TaskFunctionApplier for EvaluatorApplier<'_, '_> {
    fn apply_task_function(
        &mut self,
        function: RuntimeValue,
        args: Vec<RuntimeValue>,
        _globals: &BTreeMap<aivi_backend::ItemId, RuntimeValue>,
    ) -> Result<RuntimeValue, EvaluationError> {
        // Delegate through the execution engine's TaskFunctionApplier impl, which uses
        // the sentinel composition context IDs for diagnostic purposes.
        self.evaluator
            .apply_task_function(function, args, self.globals)
    }
}
