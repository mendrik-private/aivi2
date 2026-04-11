#![forbid(unsafe_code)]

//! Runtime and scheduler foundations for the AIVI execution engine.

pub mod effects;
pub mod glib_adapter;
pub mod graph;
pub mod hir_adapter;
pub mod providers;
pub mod reactive_program;
pub mod runtime_errors;
pub mod scheduler;
mod source_decode;
pub mod source_map;
pub mod startup;
pub mod task_executor;

pub use effects::{
    CancellationObserver, PublicationPortError, RuntimeSourceProvider, SourceActiveWhenEvaluator,
    SourceInstanceId, SourceLifecycleAction, SourceLifecycleActionKind,
    SourceProviderRuntimeContractViolation, SourcePublicationPort, SourceReplacementPolicy,
    SourceRuntimeSpec, SourceStaleWorkPolicy, TaskCompletionPort, TaskInstanceId, TaskRuntimeSpec,
    TaskSourceRuntime, TaskSourceRuntimeError, TaskSourceTickOutcome,
};
pub use glib_adapter::{
    GlibLinkedRuntimeAccessError, GlibLinkedRuntimeDriver, GlibLinkedRuntimeFailure,
    GlibLinkedSourceMode, GlibSchedulerDriver, GlibSchedulerError, GlibWorkerPublicationSender,
};
pub use graph::{
    DerivedHandle, DerivedSpec, GraphBuildError, InputHandle, InputValidationError, OwnerHandle,
    OwnerSpec, ReactiveClauseBuilderSpec, ReactiveClauseHandle, ReactiveClauseSpec,
    ReactiveSignalSpec, SignalGraph, SignalGraphBuilder, SignalHandle, SignalKind, SignalSpec,
    TopologyBatch,
};
pub use hir_adapter::{
    HirGateStageBinding, HirGateStageId, HirOwnerBinding, HirReactiveUpdateBinding,
    HirRecurrenceBinding, HirRecurrenceNodeId, HirRuntimeAdapterError, HirRuntimeAdapterErrors,
    HirRuntimeAssembly, HirRuntimeAssemblyBuilder, HirRuntimeAssemblyStats, HirRuntimeGatePlan,
    HirRuntimeInstantiationError, HirSignalBinding, HirSignalBindingKind, HirSourceBinding,
    HirTaskBinding, ProfiledHirRuntimeAssembly, assemble_hir_runtime,
    assemble_hir_runtime_with_items, assemble_hir_runtime_with_items_profiled,
};
pub use providers::{
    MailboxPublishError, SourceProviderContext, SourceProviderExecutionError, SourceProviderManager,
};
pub use reactive_program::{
    ReactiveClauseNode, ReactiveDerivedNode, ReactiveInputNode, ReactivePartition,
    ReactivePartitionId, ReactiveProgram, ReactiveReactiveNode, ReactiveSignalNode,
    ReactiveSignalNodeKind,
};
pub use runtime_errors::render_runtime_error;
pub use scheduler::{
    DependencyValue, DependencyValues, DerivedNodeEvaluator, DerivedSignalUpdate,
    DroppedPublication, Generation, Publication, PublicationDropReason, PublicationStamp,
    Scheduler, SchedulerAccessError, SchedulerMessage, TickOutcome, TryDerivedNodeEvaluator,
    WorkerPublicationSender, WorkerSendError,
};
pub use source_decode::{
    ExternalSourceValue, SourceDecodeError, SourceDecodeErrorWithPath,
    SourceDecodeProgramSupportError, decode_external, encode_runtime_json, parse_json_text,
    validate_supported_program,
};
pub use source_map::{RuntimeSignalInfo, RuntimeSignalKind, RuntimeSourceInfo, RuntimeSourceMap};
pub use startup::{
    BackendLinkedRuntime, BackendRuntimeError, BackendRuntimeLinkError, BackendRuntimeLinkErrors,
    EvaluatedSourceConfig, EvaluatedSourceOption, LinkedDerivedSignal, LinkedSourceArgument,
    LinkedSourceBinding, LinkedSourceLifecycleAction, LinkedSourceOption, LinkedSourceTickOutcome,
    LinkedTaskBinding, LinkedTaskExecutionBinding, LinkedTaskExecutionBlocker,
    LinkedTaskWorkerError, LinkedTaskWorkerOutcome, link_backend_runtime,
    set_native_kernel_plans_enabled,
};
pub use task_executor::{
    CustomCapabilityCommandExecutor, RuntimeTaskExecutionError, execute_runtime_db_task_plan,
    execute_runtime_task_plan, execute_runtime_task_plan_with_context, execute_runtime_value,
    execute_runtime_value_with_context,
};
