use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

use aivi_backend::{
    BackendExecutableProgram, BackendExecutionEngine, DetachedRuntimeValue, EvaluationError,
    GateStage as BackendGateStage, ItemId as BackendItemId, ItemKind as BackendItemKind,
    KernelEvaluator, KernelId, LayoutKind, MovingRuntimeValueStore,
    NativeKernelExecutionError, NativeKernelPlan,
    PipelineId as BackendPipelineId,
    Program as BackendProgram, RuntimeCallable, RuntimeDbConnection, RuntimeRecordField,
    RuntimeSumValue, RuntimeValue, SourceId as BackendSourceId, StageKind as BackendStageKind,
    TaskFunctionApplier, TemporalStage as BackendTemporalStage,
};
use aivi_core as core;
use aivi_hir as hir;

use crate::{
    DerivedSignalUpdate, InputHandle, Publication, PublicationPortError, RuntimeSourceProvider,
    SourceInstanceId, SourceLifecycleActionKind, SourcePublicationPort, TaskCompletionPort,
    TaskInstanceId, TaskSourceRuntime, TaskSourceRuntimeError, TickOutcome,
    TryDerivedNodeEvaluator, WorkerPublicationSender,
    graph::{DerivedHandle, OwnerHandle, ReactiveClauseHandle, SignalHandle},
    hir_adapter::{HirCompiledRuntimeExpr, HirRuntimeAssembly, HirRuntimeInstantiationError},
    providers::SourceProviderContext,
    scheduler::DependencyValues,
    task_executor::{
        RuntimeDbCommitInvalidation, execute_runtime_value_with_context_effects_and_applier,
    },
};

include!("api.rs");

include!("linked_runtime.rs");

include!("linked_types.rs");

include!("task_sources.rs");

include!("errors.rs");

include!("derived_eval.rs");

include!("link_builder.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
