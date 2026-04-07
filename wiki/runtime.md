# Runtime

The AIVI runtime executes compiled programs by linking Cranelift-compiled backend artifacts to a reactive signal engine, task executor, and GLib/GTK main-loop integration.

## Overview

**Sources**: `crates/aivi-runtime/src/`

```
BackendLinkedRuntime  ←  link_backend_runtime()  ←  BackendProgram
        │
        ├── SignalGraph (immutable topology)
        ├── Scheduler   (tick engine)
        ├── SourceProviderManager (manages source instances)
        ├── TaskExecutor (worker pool for tasks)
        └── GlibLinkedRuntimeDriver (GLib main-loop integration)
```

## Startup & Linking

**Source**: `startup.rs`

`link_backend_runtime()` wires a `BackendProgram` into a live runtime:

1. Resolve source configurations (`EvaluatedSourceConfig`, `EvaluatedSourceOption`)
2. Build `HirRuntimeAssembly` via `assemble_hir_runtime()`
3. Construct `SignalGraph` from the assembly
4. Register source providers with `SourceProviderManager`
5. Register task bindings
6. Return `BackendLinkedRuntime`

Key types:
- `LinkedSourceBinding` — a source instance wired to an `InputHandle`
- `LinkedDerivedSignal` — a derived signal with an evaluator
- `LinkedTaskBinding` — a task function wired to a task slot
- `BackendRuntimeLinkError` / `BackendRuntimeLinkErrors` — link-time errors

## HIR Runtime Assembly

**Source**: `hir_adapter.rs`

`assemble_hir_runtime()` translates HIR elaboration reports into the `HirRuntimeAssembly` that the signal graph builder consumes:

- `HirSignalBinding` / `HirSignalBindingKind` — input, derived, signal, fanout, recurrence
- `HirGateStageBinding` — a gate stage (pipe stage at runtime)
- `HirOwnerBinding` — module-level ownership node
- `HirReactiveUpdateBinding` — reactive clause binding
- `HirRecurrenceBinding` — self-referential signal binding
- `HirRuntimeGatePlan` — the runtime plan for a gate

## Signal Graph

**Source**: `graph.rs`

See [signal-model.md](signal-model.md) for full signal semantics.

The `SignalGraph` is built by `SignalGraphBuilder`:
- `add_input()` → `InputHandle`
- `add_derived()` → `DerivedHandle`
- `add_signal()` → `SignalHandle`
- `add_reactive_clause()` → `ReactiveClauseHandle`
- `build()` → topological sort → `TopologyBatch[]`

## Scheduler

**Source**: `scheduler.rs`

`Scheduler` is the tick engine. It runs on the GLib main thread (via `GlibSchedulerDriver`):

```
WorkerPublicationSender  →  Scheduler queue  →  tick()  →  TickOutcome
```

- `Publication` carries `(InputHandle, value, PublicationStamp, Generation)`
- `Generation` is a monotonic counter; stale publications are dropped
- `TickOutcome` reports changed signals and fired reactive clauses
- `DependencyValues` snapshots dependency values for derived node evaluation
- `DerivedNodeEvaluator` / `TryDerivedNodeEvaluator` are function-pointer traits called per derived node

Thread model: `WorkerPublicationSender` is `Send`; the scheduler itself is single-threaded (main thread).

## Source Providers

**Source**: `providers.rs`, `effects.rs`

`SourceProviderManager` manages the lifecycle of all running source instances:
- Starts providers when their configuration is ready
- Routes publications from providers to the scheduler
- Handles restart, stop, and replacement policies

`RuntimeSourceProvider` is the trait each source kind implements:
- `start()` — allocate resources, begin producing values
- `stop()` — release resources

`SourceProviderContext` is passed to providers at start time; it carries `SourcePublicationPort` for publishing and `CancellationObserver` for stop signals.

`SourceProviderExecutionError` wraps provider-level errors for runtime error reporting.

## Task Executor

**Source**: `task_executor.rs`

Tasks (custom source commands and effects) run on a worker thread pool:

- `execute_runtime_task_plan()` — runs a task plan to completion
- `execute_runtime_value()` — evaluates a pure runtime value expression
- `execute_runtime_db_task_plan()` — executes a database task plan
- `CustomCapabilityCommandExecutor` — executes custom source capability commands

Tasks communicate results back to the main thread via `TaskCompletionPort`.

## Source Decode

**Source**: `source_decode.rs`

External source values (JSON, D-Bus replies, HTTP responses) are decoded into typed AIVI values:

- `decode_external()` — runs a `SourceDecodeProgram` against an `ExternalSourceValue`
- `encode_runtime_json()` — encodes a runtime value back to JSON
- `parse_json_text()` — raw JSON text → serde_json Value
- `validate_supported_program()` — validates a decode program is executable

`SourceDecodeError` / `SourceDecodeErrorWithPath` report decode failures with field paths.

## GLib Integration

**Source**: `glib_adapter.rs`

`GlibLinkedRuntimeDriver` bridges the AIVI scheduler to the GLib main context:
- Drives `tick()` calls from GLib idle sources
- Routes worker publications safely across thread boundaries
- `GlibWorkerPublicationSender` is the `Send` channel from workers to the scheduler
- `GlibSchedulerDriver` drives tick scheduling within the GLib main loop

Fairness boundary:
- Async GLib wakeups are now **budgeted** in `glib_adapter.rs`: one wake drains at most 32 ticks before yielding back to the main loop.
- If queued work remains after the budget is exhausted, the driver re-arms another GLib callback and continues on the next wake instead of monopolising the GTK thread.
- Explicit synchronous drains (`queue_publication_now()`, `tick_now()`) still run until idle; the budget only applies to async `request_tick()` / worker-triggered wakes.
- `GlibLinkedRuntimeDriver::stop()` remains authoritative over queued follow-up callbacks: rescheduled async wakes check the stop flag and become inert under teardown.

`GlibLinkedRuntimeFailure` represents fatal runtime errors that cause application teardown.

## Runtime Errors

**Source**: `runtime_errors.rs`

`render_runtime_error()` produces a `Diagnostic` (with source spans from `RuntimeSourceMap`) for user-facing error display. Uses `DiagnosticRenderer` with the Ghostty colour palette (see `aivi-base/src/render.rs`).

`RuntimeSourceMap` maps runtime signal/source IDs back to source spans for error attribution.

*See also: [signal-model.md](signal-model.md), [gtk-bridge.md](gtk-bridge.md), [architecture.md](architecture.md)*
