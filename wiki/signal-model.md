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

## `from` Signal Fan-Out Sugar

`from` is surface sugar for defining several reactive bindings from one shared upstream signal:

```aivi
from state = {
    boardText: renderBoard
    type Int -> Bool
    atLeast threshold: .score >= threshold
    dirLine: .dir |> dirLabel
    gameOver: .status
        ||> Running -> False
        ||> GameOver -> True
}
```

Semantics:

- Zero-parameter entries lower to ordinary top-level `signal`s.
- Parameterized entries lower to ordinary top-level `func`s.
- The shared source is piped into each entry body.
- Headless pipe entries such as `.dir |> dirLabel` become `state |> .dir |> dirLabel`.
- Plain expressions such as `renderBoard` become `state |> renderBoard`.
- Parameterized entries still produce reactive results, so a surface annotation like
  `type Int -> Bool` is wrapped internally as `Int -> Signal Bool`.
- Inside parameterized entry bodies, direct references to earlier same-block signals and
  selector calls are treated as one outer signal payload read in body contexts. That lift
  also applies inside `T|>` / `F|>` branches when the branch subject is a `Signal Bool`.
- A standalone `type` line inside the block attaches to the immediately following
  entry only.
- Entry boundaries are indentation-sensitive: a peer `name:` line starts a new derived signal; deeper-indented lines stay attached to the current entry.

**Source**: `crates/aivi-syntax/src/parse.rs` (`parse_from_item`, `parse_from_entries`), `crates/aivi-hir/src/lower.rs` (`lower_from_item`, `prepend_from_source`), `crates/aivi-hir/src/typecheck_context.rs` (`infer_transform_stage_info`, `infer_truthy_falsy_branch`, `infer_pipe_expr`), `crates/aivi-hir/src/general_expr_elaboration.rs` (`lower_truthy_falsy_stage`)

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

## AsyncTracker pattern

**Source**: `stdlib/aivi/async.aivi`, `manual/stdlib/async.md`

When a signal is backed by an async source (`Result E A`), the lifecycle has three observable states: loading, done, and error. `AsyncTracker E A` is a plain record type that captures all three:

```
type AsyncTracker E A = { pending: Bool, done: Option A, error: Option E }
```

Used with `+|>` accumulation: the seed is `{ pending: True, done: None, error: None }` and `async.step` drives state transitions on each new `Result`. Because the payload is a record, the three fields become first-class signal projections:

- `sig.pending : Signal Bool` — True until first result arrives
- `sig.done : Signal (Option A)` — last successful value (stale-while-revalidate: preserved on subsequent errors)
- `sig.error : Signal (Option E)` — current error, or None

**Fire-once idiom**: accumulate a `Bool` that flips to `True` on first `done` and never resets. Use it as `activeWhen` on a follow-up source. This is the current surface pattern; there is no dedicated `@effect` / `doOnce` syntax documented in the compiler or manual.
