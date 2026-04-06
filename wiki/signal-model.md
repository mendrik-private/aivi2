# Signal Model

AIVI is a purely reactive language. Every value that changes over time is modelled as a **signal**. Signals are first-class; the runtime maintains a topologically ordered `SignalGraph` that propagates updates glitch-free.

## Signal Kinds

**Source**: `crates/aivi-runtime/src/graph.rs`

| Kind | Description |
|------|-------------|
| `InputHandle` | Mutable input published by a source provider |
| `DerivedHandle` | Computed from other signals (pure function of dependencies) |
| `SignalHandle` | Explicit `signal` declaration — owns a merge body and a current value |
| `ReactiveClauseHandle` | Reactive update clause — fires side effects (e.g. GTK mutations) |

```
InputHandle ──▶ DerivedHandle ──▶ SignalHandle ──▶ ReactiveClauseHandle
                      ▲                 │
                      └─────────────────┘ (recurrence: signal reads its own previous value)
```

## Signal Declarations

```aivi
signal count: Int = 0
```

A `signal` declaration:
- declares a name and type
- has a **seed value** (the initial state)
- optionally has a **merge body** (reactive arms that update the value when sources fire)

## Signal Merge Syntax

The `when` keyword was removed. Signal merge uses a pipe-case style:

```aivi
signal x: Int = sources | src1 | src2
  ||> src1 value => value + 1
  ||> src2 _     => 0
  ||> _          => x      // default arm = seed
```

- `||>` introduces a reactive arm
- `pattern => body` (fat arrow `=>`, not `->`)
- The default arm (`||> _ => ...`) becomes the seed body
- Sources listed after `=` are the merge inputs

**Source**: `crates/aivi-syntax/src/parse.rs` — `try_parse_signal_merge_body()`, `find_signal_reactive_arm_start()`

The parser disambiguates reactive arms (`||> pattern => body`) from pipe-case arms (`||> pattern -> body`) by checking for `=>` vs `->` after `||>`.

## Sources

A **source** is an external observable that produces a stream of values over time. Sources are declared with `@source` and backed by a provider.

**Source**: `crates/aivi-runtime/src/effects.rs`, `providers.rs`

Key types:
- `RuntimeSourceProvider` — trait implemented by each source kind (HTTP, timer, file watch, D-Bus, etc.)
- `SourceInstanceId` — stable handle for a running source instance
- `SourcePublicationPort` — channel a provider uses to publish values
- `SourceReplacementPolicy` — what happens when a new value arrives before the previous is consumed
- `SourceStaleWorkPolicy` — how stale in-flight work is handled on source restart

### Source Lifecycle

Sources have an explicit lifecycle:
1. **Configure** — options are evaluated from the program
2. **Start** — provider allocates resources, begins watching/polling
3. **Publish** — sends values via `SourcePublicationPort`
4. **Stop** — provider releases resources

`SourceLifecycleAction` and `SourceLifecycleActionKind` model start/stop/restart transitions.

## Signal Graph

**Source**: `crates/aivi-runtime/src/graph.rs`

`SignalGraph` is built once by `SignalGraphBuilder` and is immutable after construction. It stores:
- `OwnerHandle` / `OwnerSpec` — module-level ownership
- `DerivedSpec` — dependency list + evaluator function pointer
- `ReactiveClauseSpec` — trigger signals + update callback
- `TopologyBatch` — pre-computed topological batches for glitch-free propagation

`SignalKind` distinguishes the four node types.

## Scheduler

**Source**: `crates/aivi-runtime/src/scheduler.rs`

`Scheduler` owns the `SignalGraph` and processes ticks:

1. A source publishes a `Publication` (a value + `PublicationStamp` + `Generation`)
2. The scheduler queues the publication as a `SchedulerMessage`
3. On tick, it processes all pending messages in topological batch order
4. Derived nodes are re-evaluated; reactive clauses fire in dependency order
5. `TickOutcome` reports whether any signals changed

Key types:
- `DependencyValues` — snapshot of dependency values for a derived node evaluation
- `DerivedNodeEvaluator` — function pointer called per derived node
- `DroppedPublication` / `PublicationDropReason` — dropped-publication accounting

## Recurrence (Self-Reference)

A signal can reference its own previous value in a reactive arm — this is **recurrence**. The recurrence planner (`aivi-typing/src/recurrence.rs`) validates that recurrence targets are well-founded and assigns `RecurrencePlan` + `RecurrenceWakeupPlan` to each recurrent signal.

In the runtime, `HirRecurrenceBinding` carries the recurrence metadata and the scheduler ensures the previous-value slot is stable across ticks.

## Fanout

**Source**: `crates/aivi-typing/src/fanout.rs`, `crates/aivi-hir/src/fanout_elaboration.rs`

Fanout is the mechanism for distributing a container (e.g. `List`) of signals into parallel reactive branches and joining them back. `FanoutPlan` / `FanoutPlanner` derive the carrier kind and segment structure; `FanoutJoinPlan` / `FanoutFilterPlan` cover join and filter operations.

In the runtime, `HirRuntimeAssembly` carries `HirSignalBindingKind::Fanout` bindings.

*See also: [runtime.md](runtime.md), [architecture.md](architecture.md)*
