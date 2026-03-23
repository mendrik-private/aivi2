# AIVI Codebase — Expert Code Review

**Reviewer**: Claude (Rust/compiler expert)
**Date**: 2026-03-23
**Scope**: All 12 workspace crates, cross-referenced against AGENTS.md, AIVI_RFC.md, choices_made.md
**Standards applied**: AGENTS.md §3 (stack-safety, Send/Sync, invariant encoding), §4 (performance), §5 (correctness)

---

## Severity Index

| ID | Severity | Location | Title |
|----|----------|----------|-------|
| C1 | **Critical** | `aivi-runtime/src/providers.rs:269` | `strip_signal` unbounded recursion |
| C2 | **Critical** | `aivi-backend/src/runtime.rs:97` | `display_text` unbounded recursion |
| C3 | **Critical** | `aivi-runtime/src/glib_adapter.rs:210` | Mutex held across full scheduler tick |
| PA-C1 | **Critical** | `aivi-hir/src/gate_elaboration.rs:698` | `lower_gate_runtime_expr` unbounded recursion |
| H1 | **High** | `aivi-gtk/src/host.rs:53` | `Rc<RefCell>` as architectural load-bearing type |
| H2 | **High** | `aivi-runtime/src/startup.rs:194,212` | `KernelEvaluator` reconstructed per argument/option |
| H3 | **High** | `aivi-gtk/src/host.rs:775` | `text_literal` silently discards interpolation segments |
| H4 | **High** | `aivi-backend/src/codegen.rs:35` | `CompiledProgram::kernel` O(n) linear scan |
| H5 | **High** | `aivi-runtime/src/startup.rs:266` | `.expect` inside fallible pipeline instead of error return |
| H6 | **High** | `aivi-query/src/db.rs` | `HashMap` with SipHash in hot query path |
| PA-H1 | **High** | `aivi-hir/src/validate.rs:3708–4710` | 5 separate O(n) item traversals with fresh `GateTypeContext` each |
| PA-H2 | **High** | `aivi-hir/src/validate.rs:5347` | `elaborate_fanout_segment` called inside gate validation — double elaboration |
| M1 | **Medium** | `aivi-runtime/src/scheduler.rs:340` | 4 `Vec` allocations per scheduler tick |
| M2 | **Medium** | `aivi-runtime/src/startup.rs:247` | Per-tick `Box` per signal value snapshot |
| M3 | **Medium** | `aivi-gtk/src/host.rs` | `move_children` O(n²) GTK reorder |
| M4 | **Medium** | `aivi-runtime/src/effects.rs` | Single-variant enums waste expressivity |
| M5 | **Medium** | `aivi-runtime/src/startup.rs` | `BackendLinkedRuntime<'a>` lifetime couples runtime to transient stack |
| M6 | **Medium** | `aivi-query/src/db.rs` | No cross-file cache invalidation |
| PA-M1 | **Medium** | `aivi-hir/src/{gate,fanout,truthy_falsy}_elaboration.rs` | 6+ near-identical subject-tracking pipe walkers |
| PA-M2 | **Medium** | `aivi-hir/src/truthy_falsy_elaboration.rs:310` | Lossy multi-error reporting in `elaborate_truthy_falsy_pair` |
| I1 | **Info** | `aivi-runtime/src/scheduler.rs` | `Generation::advance` panics on overflow instead of saturating/wrapping |
| I2 | **Info** | `aivi-runtime/src/effects.rs` | `SourceInstanceId` redefined via macro, duplicating backend definition |
| I3 | **Info** | `aivi-backend/src/kernel.rs` | `KernelOriginKind::ItemBody` carries no item identity |
| I4 | **Info** | `aivi-gtk/src/host.rs` | `widget_name` invariant undocumented |
| PA-I1 | **Info** | `aivi-hir/src/recurrence_elaboration.rs:753` | `unreachable!` arms in `recurrence_runtime_stage_blocker` are fragile |
| PA-I2 | **Info** | `aivi-hir/src/{truthy_falsy,recurrence}_elaboration.rs` | `*_env_for_function` triplicated |
| PA-I3 | **Info** | `aivi-hir/src/gate_elaboration.rs` | `lower_single_parameter_function_pipe_body_runtime_expr` inlines without purity gate |

---

## Critical Findings

### C1 — `strip_signal` unbounded recursion
**File**: `crates/aivi-runtime/src/providers.rs:269`
**AGENTS.md**: §3.4 — all tree traversals must be iterative

```rust
// CURRENT (stack overflow on deeply nested Signal wrappers)
fn strip_signal(value: &RuntimeValue) -> &RuntimeValue {
    match value {
        RuntimeValue::Signal(inner) => strip_signal(inner),
        other => other,
    }
}
```

**Fix**:
```rust
fn strip_signal(mut value: &RuntimeValue) -> &RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = inner;
    }
    value
}
```

Signals can nest arbitrarily (choices_made.md §57 allows `Signal<Signal<T>>`). A malformed source payload or a bug in lowering can produce deep nesting. Stack overflow here crashes the runtime process, not just the current evaluation.

---

### C2 — `display_text` unbounded recursion
**File**: `crates/aivi-backend/src/runtime.rs:97–171`
**AGENTS.md**: §3.4

`display_text` and its callers recurse on `RuntimeValue` without depth bounding. `EvaluationError::Display` fires from kernel evaluation, so an adversarially-shaped value tree (Tuple of Tuple of … deeply nested) will overflow the stack at the error reporting site — precisely when recovery is most important.

**Fix**: Thread a depth counter through all `RuntimeValue` display formatters. Return `"[depth limit exceeded]"` past a fixed cap (e.g. 64).

---

### C3 — Mutex held across full scheduler tick
**File**: `crates/aivi-runtime/src/glib_adapter.rs:210–234`

`drive_pending_ticks` acquires the `Arc<Mutex<Scheduler>>` and holds it for the entire `scheduler.tick(evaluator)` call. A tick includes kernel execution, GTK widget updates, and any I/O callbacks. This blocks the GLib main thread from responding to any other event source for the tick duration, defeating the non-blocking GLib integration contract and risking priority inversion against time-sensitive GTK redraws.

**Fix**: Take the lock only to drain the pending queue and swap in results. Do the tick with the lock released:
```rust
let tick_input = {
    let mut sched = scheduler.lock().unwrap();
    sched.drain_pending()
};
let tick_output = run_tick_without_lock(tick_input, evaluator);
{
    let mut sched = scheduler.lock().unwrap();
    sched.apply_tick_output(tick_output);
}
```

---

### PA-C1 — `lower_gate_runtime_expr` unbounded recursion
**File**: `crates/aivi-hir/src/gate_elaboration.rs:698`
**AGENTS.md**: §3.4

This is the same class of bug as C1 and C2. Every compound `ExprKind` variant recurses directly:

```rust
ExprKind::Tuple(elements) => GateRuntimeExprKind::Tuple(
    elements.iter()
        .map(|element| lower_gate_runtime_expr(module, *element, env, ambient, typing))
        .collect::<Result<_, _>>()?,
),
// … same for List, Map, Set, Record, Apply (callee + N args), Unary, Binary, Pipe
ExprKind::Apply { callee, arguments } => GateRuntimeExprKind::Apply {
    callee: Box::new(lower_gate_runtime_expr(module, callee, env, ambient, typing)?),
    arguments: arguments.iter()
        .map(|arg| lower_gate_runtime_expr(module, *arg, env, ambient, typing))
        .collect::<Result<_, _>>()?,
},
```

A user can write a deeply nested gate predicate expression (`((((a && b) && c) && d) …)`) that causes a stack overflow during HIR elaboration. This is a compile-time crash, not a runtime crash, but it is still a process abort that the IDE process cannot recover from.

The same recursive function is called from:
- `lower_gate_pipe_body_runtime_expr` (gate elaboration entry)
- `lower_recurrence_guard_predicate` in `recurrence_elaboration.rs:558`
- `lower_runtime_text_literal` (for text interpolation segments)
- `lower_runtime_record_field`
- `lower_runtime_pipe_stage` (for pipe-inside-predicate)

**Fix**: Convert to an explicit worklist. The output tree (`GateRuntimeExpr`) must be built bottom-up, so use a two-phase approach (push phase and pop phase), or build an iterative post-order traversal using an explicit stack of `(ExprId, continuation)` frames. This mirrors the correct pattern already used by `walk_expr_tree` in `validate.rs:11351`.

Minimum: add a `#[cfg(test)]` recursion depth torture test at depth 4096 (matching the scheduler's existing test) to catch regressions.

---

## High Findings

### H1 — `Rc<RefCell<VecDeque>>` as architectural load-bearing type
**File**: `crates/aivi-gtk/src/host.rs:53`
**AGENTS.md**: §2.1 — avoid Rc<RefCell> as architecture

```rust
event_queue: Rc<RefCell<VecDeque<GtkQueuedEvent<V>>>>
```

AGENTS.md explicitly prohibits `Rc<RefCell>` as an architectural primitive, allowing it only for leaf-level GTK adapter glue. This queue is the main event bus between the GTK callback layer and the host evaluation loop. It is not a leaf — it is the architectural seam. If a GTK callback fires during host evaluation, or if host evaluation triggers a GTK event, `borrow_mut()` will panic. The GLib main loop is cooperative but not single-callsite; reentrant borrows are realistic.

**Fix**: Use `std::sync::Mutex<VecDeque<…>>` or a lock-free `crossbeam::SegQueue`. The host's `Rc<GtkConcreteHost<V>>` already precludes `Send`, so `Mutex` has no overhead versus `RefCell` in a single-threaded context but eliminates the panic surface.

---

### H2 — `KernelEvaluator` reconstructed per source argument/option
**File**: `crates/aivi-runtime/src/startup.rs:194, 212`

```rust
// line 194: reconstructed for every argument
let value = KernelEvaluator::new(&program).evaluate(argument)?;
// line 212: reconstructed for every option
let value = KernelEvaluator::new(&program).evaluate(option)?;
```

`KernelEvaluator::new` allocates its internal lookup tables on each call. If a source has M arguments and N options, this creates M+N evaluator instances where one would suffice. During startup with many signals this is O(sources × (args + options)) allocations.

**Fix**: Create one `KernelEvaluator` before the argument/option loops and reuse it across all evaluations in the source.

---

### H3 — `text_literal` silently discards interpolation segments
**File**: `crates/aivi-gtk/src/host.rs:775`

When a `TextLiteral` contains interpolation segments (e.g. `"Hello, \(name)!"`), the GTK host's `text_literal` method silently returns only the static prefix. This is a correctness bug: users will see incomplete text with no error, making it extremely difficult to diagnose. It also violates the RFC §11 semantics for `|` tap stages and text interpolation in attribute values.

**Fix**: Either evaluate each interpolation segment through the active `RuntimeValue` environment and concatenate, or emit a `EvaluationError::UnsupportedFeature` with the segment span so the toolchain surfaces it.

---

### H4 — `CompiledProgram::kernel` O(n) linear scan
**File**: `crates/aivi-backend/src/codegen.rs:35`

```rust
pub fn kernel(&self, id: KernelId) -> Option<&CompiledKernel> {
    self.kernels.iter().find(|k| k.id == id)
}
```

Every kernel lookup scans the full compiled kernel list. Kernel lookups happen during signal propagation at runtime — once per pipeline stage per tick. For a program with K kernels and T ticks per second, this is O(K × stages × T) comparisons. At 60 fps with 100 kernels and 10 stages per signal, that is 60,000 linear scans per second.

**Fix**: Replace with `HashMap<KernelId, usize>` index built at compile time, or switch the storage to a sorted `Vec` and use binary search.

---

### H5 — `.expect` inside fallible pipeline
**File**: `crates/aivi-runtime/src/startup.rs:266`

```rust
let globals = required_signal_globals(&program)
    .expect("signal globals should be present");
```

This `expect` is inside `build_runtime`, a function that returns `Result`. A missing signal global should propagate as an error, not abort the process. Any future code path that reaches this without the precondition met (e.g., a partially-linked program, incremental recompilation) will silently crash instead of returning a diagnostic.

**Fix**: Replace with `ok_or(StartupError::MissingSignalGlobals)?`.

---

### H6 — SipHash in hot query path
**File**: `crates/aivi-query/src/db.rs`

`RootDatabase` uses `HashMap`/`HashSet` with the default SipHash13 hasher. SipHash is DoS-resistant but slow for integer keys. All query keys in `RootDatabase` are arena IDs (u32/u64 newtypes) — there is no untrusted string data to protect against. This is 3–5× slower than FxHashMap or AHashMap for integer key lookups.

**Fix**: Replace with `rustc-hash::FxHashMap` or `ahash::AHashMap`. The type aliases already in use (`HashMap`) make this a one-line change per import.

---

### PA-H1 — Five separate O(n) item traversals with fresh `GateTypeContext`
**File**: `crates/aivi-hir/src/validate.rs:3708–4710`

`Validator::run` dispatches to:
- `validate_gate_semantics` (line 3754)
- `validate_fanout_semantics` (line 3708)
- `validate_truthy_falsy_semantics` (line 3800)
- `validate_case_exhaustiveness` (line 3852)
- `validate_recurrence_targets` (line 4569)

Each of these:
1. Clones all items into a fresh `Vec<Item>` (O(n) heap allocation)
2. Constructs a new `GateTypeContext::new(self.module)` from scratch
3. Walks every item body via `walk_expr_tree`

This means every item body is traversed **5 times**, and `GateTypeContext` — which interns type structure — is rebuilt from scratch **5 times** for the same module. `GateTypeContext` contains interning tables that accumulate entries across the traversal; rebuilding it discards that work.

For a module with 500 items, this is 2500 tree walks and 5 full `GateTypeContext` constructions per validation run. In the LSP context (choices_made.md §88, real-time re-validation), this runs on every keystroke.

**Fix**: Merge all five passes into a single `validate_pipe_semantics` pass that carries one shared `GateTypeContext` and dispatches to per-operator validation at each pipe expression. The item iteration and context construction happen once.

---

### PA-H2 — `elaborate_fanout_segment` called inside gate validation
**File**: `crates/aivi-hir/src/validate.rs:5347`

```rust
PipeStageKind::Map { expr } => {
    let segment = pipe.fanout_segment(stage_index)
        .expect("map stages should expose a fan-out segment");
    if segment.join_stage().is_some() {
        let outcome = crate::fanout_elaboration::elaborate_fanout_segment(
            self.module, &segment, Some(&subject), env, typing,
        );
        // …
    }
```

`validate_gate_pipe` calls the full `fanout_elaboration::elaborate_fanout_segment` entrypoint to infer the post-fanout subject type when a `<|*` join is present. This is the same work that `validate_fanout_semantics` already performs — every joined fanout segment is fully elaborated twice. The duplicate work scales with the number of joined fanout usages in the codebase.

**Fix**: The gate validation pass only needs the result type; extract a lighter `infer_fanout_segment_result_type` query from `GateTypeContext` rather than re-running the full elaboration.

---

## Medium Findings

### M1 — 4 heap allocations per scheduler tick
**File**: `crates/aivi-runtime/src/scheduler.rs:340–343`

Four `Vec::new()` calls inside the hot tick loop. For a 60fps application this is 240 allocations per second minimum, scaling with signal count. Use bump allocators or pre-allocated ring buffers that are cleared between ticks.

---

### M2 — `Box` per signal snapshot per tick
**File**: `crates/aivi-runtime/src/startup.rs:247`

`committed_signal_snapshots` boxes every signal value on each tick. For signals with small value types (bool, i64, enum discriminant) this is a pointer indirection and allocation where a `RuntimeValue` could be stored inline or in a `SmallBox`.

---

### M3 — `move_children` O(n²) GTK reorder
**File**: `crates/aivi-gtk/src/host.rs`

The child widget reorder algorithm removes and re-inserts each widget individually using GTK positional APIs, yielding O(n²) GTK operations for a list of n children. GTK reorders trigger layout recomputation per operation. Use GTK4's `gtk_widget_insert_after`/`before` in a single-pass optimal reorder (matching `move_children` to the desired order in one linear scan).

---

### M4 — Single-variant enums in effects.rs
**File**: `crates/aivi-runtime/src/effects.rs`

`SourceReplacementPolicy` and `SourceStaleWorkPolicy` are single-variant enums. They encode no information (there is only one variant to match) and add match noise at every callsite. Either extend them with real variants now (choices_made.md §46 anticipates `DropStale` vs `QueueStale`) or use type aliases until that extension lands.

---

### M5 — `BackendLinkedRuntime<'a>` lifetime coupling
**File**: `crates/aivi-runtime/src/startup.rs`

`BackendLinkedRuntime<'a>` borrows from the `CompiledProgram` on the stack of `build_runtime`. This makes the runtime non-`'static` and prevents it from being stored in `Arc`, sent to threads, or used across async await points. The lifetime leaks into every downstream API. The `CompiledProgram` should be `Arc<CompiledProgram>`-shared and owned by the runtime, not borrowed.

---

### M6 — No cross-file cache invalidation
**File**: `crates/aivi-query/src/db.rs`

`RootDatabase` caches query results keyed by revision. When file A changes and file B imports A, queries derived from A are invalidated, but queries about B that transitively depend on A are not. In an IDE with 50 source files, a change to a shared type definition will serve stale results for all importers. choices_made.md §88 identifies incremental computation as a first-class requirement; the current implementation satisfies the letter (revision key) but not the spirit (transitive invalidation).

---

### PA-M1 — 6+ near-identical subject-tracking pipe walkers
**Files**:
- `crates/aivi-hir/src/gate_elaboration.rs` — `collect_gate_pipe`
- `crates/aivi-hir/src/fanout_elaboration.rs` — `collect_fanout_pipe`
- `crates/aivi-hir/src/truthy_falsy_elaboration.rs` — `collect_truthy_falsy_pipe`
- `crates/aivi-hir/src/recurrence_elaboration.rs` — `infer_recurrence_input_subject`
- `crates/aivi-hir/src/validate.rs` — `validate_gate_pipe`, `validate_fanout_pipe`, `validate_truthy_falsy_pipe`

All of these implement the same pattern: iterate pipe stages left to right, maintaining a `current: Option<GateType>` subject, advancing it through Transform/Gate/Map/FanIn/Truthy/Falsy stages. They already diverge subtly:

- `validate_gate_pipe` has special logic for joined fanout segments that the elaboration-phase walkers do not
- `infer_recurrence_input_subject` handles `Tap` but skips `Case`/`Apply`/`RecurStart`/`RecurStep` differently from the others
- `collect_fanout_pipe` stops at `*|>` by design; the others continue

These will drift further as new pipe operators are added. When `&|>` applicative clusters are elaborated (RFC §12), a seventh walker will likely be added.

**Fix**: Extract a `PipeSubjectWalker<CB>` struct that takes a per-stage callback and handles the subject threading. All elaboration passes provide their specific callback; the iteration logic is owned once.

---

### PA-M2 — Lossy multi-error reporting in `elaborate_truthy_falsy_pair`
**File**: `crates/aivi-hir/src/truthy_falsy_elaboration.rs:310–354`

When the truthy branch type is unknown, the function returns early and discards all blockers accumulated from the falsy branch:
```rust
let Some(truthy_result_type) = truthy_ty else {
    blockers.push(TruthyFalsyElaborationBlocker::UnknownBranchType { branch: Truthy });
    return TruthyFalsyStageOutcome::Blocked(BlockedTruthyFalsyStage {
        subject: Some(subject.clone()),
        blockers,  // falsy branch issues silently dropped here
    });
};
```

A user with both branches broken sees only the truthy error. The falsy blockers were collected (lines 315–320) but abandoned. The fix is trivial: do not early-return between the two branch type extractions; push `UnknownBranchType` for each unknown type and continue to accumulate all blockers before the final return.

---

## Informational Findings

### I1 — `Generation::advance` panics on overflow
**File**: `crates/aivi-runtime/src/scheduler.rs`

`checked_add` + `expect` means a long-running AIVI process that exceeds `u64::MAX` ticks will crash. At 60fps this takes ~9.7 billion years, so this is not a practical concern, but `wrapping_add` with a documented invariant (no two live values share a generation) is cleaner than a panic.

---

### I2 — `SourceInstanceId` redefined via macro
**File**: `crates/aivi-runtime/src/effects.rs`

The macro-generated `SourceInstanceId` in the runtime crate duplicates the definition in the backend crate. If the two IDs ever drift (e.g., one gains a debug impl the other does not), subtle mismatches will appear at the backend/runtime boundary. Unify under a single definition in `aivi-base` or expose the backend type via re-export.

---

### I3 — `KernelOriginKind::ItemBody` carries no item identity
**File**: `crates/aivi-backend/src/kernel.rs`

`KernelOriginKind::ItemBody` does not store the `ItemId`. When a kernel error is reported with this origin, the error message cannot name the item whose body produced the kernel. All other `KernelOriginKind` variants carry their parent ID. Add `item: ItemId` to `ItemBody`.

---

### I4 — `widget_name` invariant undocumented
**File**: `crates/aivi-gtk/src/host.rs`

`widget_name` is called on freshly-constructed widgets with the assumption that the widget has no children and its name slot is empty. This precondition is never asserted and not documented. Add a `debug_assert!(widget.css_name().is_empty())` or a comment explaining why the invariant holds.

---

### PA-I1 — `unreachable!` arms in `recurrence_runtime_stage_blocker` are fragile
**File**: `crates/aivi-hir/src/recurrence_elaboration.rs:773–776`

```rust
GateElaborationBlocker::UnknownSubjectType
| GateElaborationBlocker::UnknownPredicateType => {
    unreachable!("runtime expression lowering should not emit subject-only gate blockers")
}
```

This `unreachable!` is sound today because `lower_gate_pipe_body_runtime_expr` is only called after the subject type is already known. But this is an implicit coupling: if `lower_gate_pipe_body_runtime_expr` ever changes its call sites, or if new `GateElaborationBlocker` variants are added, this will panic at runtime rather than fail to compile. Convert `GateElaborationBlocker` into a split type (`GateRuntimeBlocker` vs `GateSubjectBlocker`) so the conversion is proven total by the type system.

---

### PA-I2 — `*_env_for_function` triplicated
**Files**:
- `aivi-hir/src/gate_elaboration.rs` — `gate_env_for_function` (via `validate.rs`)
- `aivi-hir/src/truthy_falsy_elaboration.rs:410` — `truthy_falsy_env_for_function`
- `aivi-hir/src/recurrence_elaboration.rs:737` — `recurrence_env_for_function`

All three functions are identical: iterate function parameters, look up annotations, insert into `GateExprEnv::locals`. Move to a single `fn gate_env_for_function(item: &FunctionItem, typing: &mut GateTypeContext<'_>) -> GateExprEnv` in `validate.rs` (or a shared module) and call it from all three elaboration passes.

---

### PA-I3 — `lower_single_parameter_function_pipe_body_runtime_expr` inlines without purity gate
**File**: `crates/aivi-hir/src/gate_elaboration.rs`

When a gate predicate is a single-parameter function application (`a |> ?|> myPredicate`), this function inlines `myPredicate`'s body into the `GateRuntimeExpr`, substituting the parameter with the ambient subject. This is semantically correct only if `myPredicate` is pure (captures no mutable state, has no effects). There is no purity check before inlining. If the type system later permits impure functions in predicate position, this will produce incorrect runtime behaviour silently. Add an assertion or a comment explaining the purity requirement and where it is enforced in the type system.

---

## Pipe Algebra RFC Compliance Summary

The HIR elaboration passes correctly implement the core pipe algebra semantics as specified in RFC §11:

| Operator | RFC Rule | Implementation | Status |
|----------|----------|----------------|--------|
| `\|>` transform | subject flows through | `infer_transform_stage` | Correct |
| `?\|>` gate | Option-returning predicate, wraps subject | `elaborate_gate_stage` (Ordinary path) | Correct |
| `?\|>` gate on Signal | signal filter, predicate must be pure | `elaborate_gate_stage` (SignalFilter path) | Correct |
| `\|\|>` case | exhaustive variant match | `validate_case_exhaustiveness` | Correct |
| `T\|>`/`F\|>` truthy/falsy | symmetric pair, same result type | `elaborate_truthy_falsy_pair` + `same_shape` check | Correct |
| `*\|>`/`<\|*` fanout/join | Signal/Ordinary carrier, join collects mapped | `FanoutSegmentPlan` + `gate_payload()` Signal unwrap | Correct |
| `\|` tap | side-effect, subject passes through | `Tap` handling in all walkers | Correct |
| `@\|>`/`<\|@` recurrence | start/guard/step closure check, wakeup proof | `elaborate_recurrence_pipe` | Correct |
| `&\|>` applicative cluster | RFC §12 desugaring | Not yet implemented (pending) | N/A |

Guard stages in recurrence correctly close over `start_subject` rather than `input_subject` (verified in `recurrence_elaboration.rs:400–430`). Non-source wakeup is lowered separately from source wakeup per choices_made.md §45. The `FanoutJoinPlan.collection_subject` correctly uses `gate_payload()` to strip the Signal wrapper before the join collection type is inferred.

---

## Fix Priority

1. **Fix immediately** (process-abort bugs): C1, C2, C3, PA-C1
2. **Fix before LSP ships** (correctness + performance at IDE scale): H3, PA-H1, PA-H2
3. **Fix before production** (architectural violations): H1, H2, M5
4. **Fix in next cleanup pass**: H4, H5, H6, M1, M2, M3, PA-M1, PA-M2
5. **Fix opportunistically**: all I-level findings
