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

`link_backend_runtime_with_seed()` is the source-free launch seam used by serialized run artifacts.
It consumes a pre-derived `BackendRuntimeLinkSeed`, so bundle startup no longer needs typed core
or full source/HIR reconstruction just to rebuild backend↔runtime origins.

`link_backend_runtime_with_seed_and_native_kernels()` is the native-payload companion used by
artifact launch. It threads precompiled native kernel sidecars into linked-runtime evaluators, so
bundle startup can reuse build-time machine code for supported kernels while still keeping
`BackendProgram` metadata for runtime linking and fallback execution.

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

### Slot storage model

Phase 4 of the signal-engine rewrite replaced the scheduler's old split between committed
`SignalRuntimeState` and tick-local `PendingValue` vectors with an explicit:

```rust
SlotStore {
    committed: Vec<CommittedSlot>,
    pending: Vec<PendingSlot>,
}
```

Current slot classes:

- `CommittedSlot::Empty` — no committed value
- `CommittedSlot::Raw` — raw bytes plus the decoded value shadow needed by the current
  borrow-based runtime API
- `CommittedSlot::Stored` — store-managed committed state; for the linked runtime's
  `MovingRuntimeValueStore` this is the heap-backed GC-root path

Pending tick state is now explicit too:

- `PendingSlot::Unchanged`
- `PendingSlot::Clear`
- `PendingSlot::NextRaw`
- `PendingSlot::NextStored`

This change keeps the scheduler semantics the same — topological, transactional, glitch-free
reads within a tick — but makes the storage class visible in the runtime model so later phases
can route native kernels directly into raw slots without rewriting commit logic again.

### Partition-driven linked runtime ticks

Phase 5 moved the linked runtime off the coarse `SignalGraph::batches()` traversal and onto the
`ReactiveProgram` sidecar:

- `ReactiveProgram` partitions are now root-cone slices, not just batch aliases
- disjoint same-batch cones split into separate partitions with contiguous topo slices
- `BackendLinkedRuntime::tick()` uses a `ReactiveProgram`-driven scheduler path when the linked
  assembly graph matches the runtime graph
- task-only helper runtimes whose scheduler graph is synthesized separately still fall back to the
  generic batch tick

This keeps ordinary scheduler semantics unchanged while letting linked runtime execution skip
unaffected partitions in deterministic partition order.

Thread model: `WorkerPublicationSender` is `Send`; the scheduler itself is single-threaded (main thread).

### 2026-04-08 live-click latency note

- `GlibLinkedRuntimeDriver` now distinguishes between “drain until fully idle” and “drain only the current scheduler queue”.
- The narrower path is used for direct UI publications so a click can settle its own synchronous work without also draining newly armed timer wakeups before GTK gets a chance to paint.

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
