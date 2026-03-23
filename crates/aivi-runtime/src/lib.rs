#![forbid(unsafe_code)]

//! Runtime and scheduler foundations for the AIVI execution engine.

pub mod effects;
pub mod graph;
pub mod hir_adapter;
pub mod scheduler;

pub use effects::{
    CancellationObserver, PublicationPortError, RuntimeSourceProvider, SourceActiveWhenEvaluator,
    SourceInstanceId, SourceLifecycleAction, SourceLifecycleActionKind,
    SourceProviderRuntimeContractViolation, SourcePublicationPort, SourceReplacementPolicy,
    SourceRuntimeSpec, SourceStaleWorkPolicy, TaskCompletionPort, TaskInstanceId, TaskRuntimeSpec,
    TaskSourceRuntime, TaskSourceRuntimeError, TaskSourceTickOutcome,
};
pub use graph::{
    DerivedHandle, DerivedSpec, GraphBuildError, InputHandle, OwnerHandle, OwnerSpec, SignalGraph,
    SignalGraphBuilder, SignalHandle, SignalKind, SignalSpec, TopologyBatch,
};
pub use hir_adapter::{
    HirGateStageBinding, HirGateStageId, HirOwnerBinding, HirRecurrenceBinding,
    HirRecurrenceNodeId, HirRuntimeAdapterError, HirRuntimeAdapterErrors, HirRuntimeAssembly,
    HirRuntimeAssemblyBuilder, HirRuntimeGatePlan, HirRuntimeInstantiationError, HirSignalBinding,
    HirSignalBindingKind, HirSourceBinding, assemble_hir_runtime,
};
pub use scheduler::{
    DependencyValue, DependencyValues, DerivedNodeEvaluator, DroppedPublication, Generation,
    Publication, PublicationDropReason, PublicationStamp, Scheduler, SchedulerAccessError,
    SchedulerMessage, TickOutcome, WorkerPublicationSender, WorkerSendError,
};
