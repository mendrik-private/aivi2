# AIVI Code Review

**Date:** 2026-03-23
**Reviewer:** Static analysis pass over all 12 workspace crates
**Scope:** Every source file read directly; findings are file:line-anchored.

---

## Executive Summary

The AIVI compiler/runtime is a well-architected Rust codebase. Layering is largely respected,
IRs have identity strategies and validation, stack safety is achieved iteratively in the
scheduler (worklist-based topological sort, 4096-deep chain test), and `#![forbid(unsafe_code)]`
is set at workspace level. The quality bar is high.

Three production-blocking bugs exist. Several performance patterns violate the "explicit costs"
principle from AGENTS.md. A handful of invariants that should be unrepresentable in types are
currently stringly- or single-variant-typed. Testing coverage has specific gaps AGENTS.md
explicitly requires (stack-depth torture, property/fuzz).

**Severity distribution:**

| Severity | Count |
|----------|-------|
| Critical | 3     |
| High     | 6     |
| Medium   | 8     |
| Low      | 6     |

---

## Critical Issues

### C1 — Recursive `strip_signal` overflows the stack on deep `RuntimeValue::Signal` nesting

**File:** `crates/aivi-runtime/src/providers.rs:269–273`

```rust
fn strip_signal(value: &RuntimeValue) -> &RuntimeValue {
    match value {
        RuntimeValue::Signal(inner) => strip_signal(inner),   // unbounded recursion
        other => other,
    }
}
```

`RuntimeValue::Signal(Box<RuntimeValue>)` is a boxed recursive type. The runtime legitimately
constructs nested `Signal` wrappers via `committed_signal_snapshots` in `startup.rs:247`:

```rust
snapshots.insert(item, RuntimeValue::Signal(Box::new(value.clone())));
```

A provider whose source argument or option kernel references another signal-backed signal can
receive multiply-wrapped values. AGENTS.md §"Runtime, concurrency, stack safety": *"Never assume
recursion is safe."*

The fix is mechanical and zero-cost:

```rust
fn strip_signal(mut value: &RuntimeValue) -> &RuntimeValue {
    while let RuntimeValue::Signal(inner) = value {
        value = inner;
    }
    value
}
```

There is no test exercising depth > 1. A `strip_signal` test with depth 1000 should be added.

---

### C2 — `RuntimeValue::display_text` recurses without depth bound over boxed payloads

**File:** `crates/aivi-backend/src/runtime.rs:97–171`

`display_text` calls itself recursively on `OptionSome`, `ResultOk`, `ResultErr`,
`ValidationValid`, `ValidationInvalid`, and `Signal` variants (lines 151–156). Each boxes its
payload (`Box<RuntimeValue>`), so nesting depth is user-data-driven. `display_text` is called
from the `Display for RuntimeValue` impl at line 176, and every `EvaluationError` variant that
embeds a `RuntimeValue` (e.g. `KernelResultLayoutMismatch`, `InlinePipeCaseNoMatch`,
`UnsupportedBinary`, etc.) will trigger this path when formatted in error messages — which
happens on real evaluation failures in production.

Additionally, `Tuple`, `List`, `Map`, `Set`, and `Record` variants each allocate an intermediate
`Vec<String>` and then `.join(", ")` it, allocating a second `String`. For a nested structure
with K elements at depth D this is O(K × D) allocations.

Fix: implement `Display` using a write-based formatter with explicit stack (or a depth cap of,
say, 64), and use `write!` directly to the formatter instead of collecting `Vec<String>`.

---

### C3 — `GlibSchedulerShared::drive_pending_ticks` holds the `Mutex` for the entire tick including evaluation

**File:** `crates/aivi-runtime/src/glib_adapter.rs:210–234`

```rust
fn drive_pending_ticks(&self) {
    loop {
        self.tick_enqueued.store(false, Ordering::Release);
        let should_continue = {
            let mut state = self.state.lock()   // lock acquired
                .expect("...");
            let outcome = scheduler.tick(evaluator);  // entire tick under lock
            // ...
        };  // lock dropped
        // ...
    }
}
```

`scheduler.tick(evaluator)` calls `evaluator.evaluate()` on every derived signal. The evaluator
is a `KernelEvaluator` over a `BackendProgram`. While currently synchronous, the design comment
in `choices_made.md` item 104 states *"actual scheduler mutation still happens on the main-context
side"* — but the lock is held for evaluation, not just mutation. Any call to `queue_publication`,
`drain_outcomes`, `current_value`, or `advance_generation` on the *same driver* from any other
async task on the same `MainContext` will block for the full tick duration. Because GLib tasks can
be interleaved on the main context, this is a latency hazard and a potential deadlock if any
evaluator code re-enters the driver.

Fix: split tick into a "drain + commit" phase (lock held) and an "evaluate" phase (lock released,
using a snapshot). Alternatively, document as an explicit invariant that the evaluator must never
re-enter the driver, and add an assertion.

---

## High Issues

### H1 — `Rc<RefCell<_>>` as architectural state in `GtkConcreteHost`

**File:** `crates/aivi-gtk/src/host.rs:53`

```rust
queued_events: Rc<RefCell<VecDeque<GtkQueuedEvent<V>>>>,
```

AGENTS.md: *"Avoid `Rc<RefCell<_>>` as architecture unless it is the best constrained local
tradeoff."* The clone at line 389 (`let queue = self.queued_events.clone()`) is used to share
the queue with GTK signal callbacks. The only reason this exists is to get `'static` captures
into GLib signal closures. The correct pattern is `Rc<RefCell<_>>` *only* if `GtkConcreteHost`
is guaranteed single-threaded (GTK main thread only, which it is), but the design should make
this explicit and document why `Arc<Mutex<_>>` is not needed. Currently neither the type nor any
comment states the threading contract.

Additionally, `borrow_mut()` in `drain_events` at line 88 can panic at runtime if a GTK signal
fires during a drain (re-entrancy). This is a real risk during `button.emit_clicked()` in tests.
AGENTS.md: *"Make illegal states unrepresentable where practical."*

Recommendation: either document the single-thread invariant and add an assertion, or replace with
a simpler `VecDeque<_>` field behind the existing `&mut self` methods (since GLib signal
callbacks need separate infrastructure for safe re-entrancy anyway).

---

### H2 — `KernelEvaluator` is reconstructed per source argument and per option, discarding the item-body evaluation cache

**File:** `crates/aivi-runtime/src/startup.rs:194, 212`

```rust
let mut evaluator = KernelEvaluator::new(self.backend);
let value = evaluator.evaluate_kernel(argument.kernel, None, &[], &globals)?;
// ...
let mut evaluator = KernelEvaluator::new(self.backend);
let value = evaluator.evaluate_kernel(option.kernel, None, &[], &globals)?;
```

`KernelEvaluator` caches item-body evaluations (guarded by `evaluating` to detect recursive
evaluation). That cache is thrown away after each argument/option evaluation. If multiple source
arguments or options depend on the same top-level `val` (which is common in real programs), each
`val` body kernel is re-evaluated from scratch. For N sources × M arguments each sharing K
shared `val` bodies, this is O(N × M × K) evaluations instead of O(K + N × M).

Fix: construct one `KernelEvaluator` for the full `evaluate_source_config` call and pass it
through, or pre-evaluate all required item-body globals once before the argument/option loops.

---

### H3 — `text_literal` in GTK host silently drops interpolation segments

**File:** `crates/aivi-gtk/src/host.rs:775–783`

```rust
fn text_literal(text: &TextLiteral) -> String {
    text.segments
        .iter()
        .filter_map(|segment| match segment {
            TextSegment::Text(fragment) => Some(fragment.raw.as_ref()),
            TextSegment::Interpolation(_) => None,   // silently dropped
        })
        .collect()
}
```

This is called from `apply_static_property` (line 347) for `StaticPropertyValue::Text`. A
static property can contain interpolated text (the HIR `TextLiteral` type models interpolation
explicitly per `choices_made.md` item 25). Silently dropping the expression holes produces wrong
output at runtime with no error. This is a correctness bug that will surface when any interpolated
text is used in a static widget property value.

Fix: either assert that no `Interpolation` segments are present (they should not be in a truly
static path), or return an error when interpolation is found.

---

### H4 — `CompiledProgram::kernel` uses O(n) linear scan

**File:** `crates/aivi-backend/src/codegen.rs:35–38`

```rust
pub fn kernel(&self, id: KernelId) -> Option<&CompiledKernel> {
    self.kernels.iter().find(|compiled| compiled.kernel == id)
}
```

`CompiledProgram` stores compiled kernels in a `Vec`. Every lookup (which happens during
runtime startup linking and test verification) scans the whole vec. This should be a
`BTreeMap<KernelId, CompiledKernel>` or an indexed `HashMap`.

---

### H5 — `required_signal_globals` panics instead of returning an error on missing signal-item mapping

**File:** `crates/aivi-runtime/src/startup.rs:266`

```rust
let signal = self
    .runtime_signal_by_item
    .get(item)
    .copied()
    .expect("linked runtime should preserve signal-item mappings");
```

This is called on every source config evaluation tick. The invariant *should* hold, but it is
produced by `link_backend_runtime` which can silently omit mappings for items whose HIR binding
is ambiguous. An `.expect` in a tick-critical path kills the process instead of returning a
`BackendRuntimeError`. Replace with `ok_or(BackendRuntimeError::MissingSignalItemMapping {...})`.

---

### H6 — `RootDatabase` uses `std::collections::HashMap` with the default (SipHash) hasher

**File:** `crates/aivi-query/src/db.rs:42–45`

```rust
files: HashMap<u32, SourceInput>,
paths: HashMap<PathBuf, SourceFile>,
parsed: HashMap<u32, Cached<ParsedFileResult>>,
hir: HashMap<u32, Cached<HirModuleResult>>,
```

These maps are in the hot LSP path (every keystroke in the editor hits `open_file`,
`cached_parsed`, `cached_hir`). `u32` keys are perfect candidates for `FxHashMap`
(rustc-hash), which is 2–4× faster for small integer keys. AGENTS.md: *"explicit costs over
hidden costs."* Use `rustc-hash::FxHashMap` or `ahash::AHashMap` throughout this crate.

---

## Medium Issues

### M1 — Four heap allocations per scheduler tick in `tick_with`

**File:** `crates/aivi-runtime/src/scheduler.rs:340–350`

```rust
let mut pending = repeat_with(|| PendingValue::Unchanged)
    .take(self.signals.len())
    .collect::<Vec<_>>();          // alloc 1
let messages = self.queue.drain(..).collect::<Vec<_>>();  // alloc 2
// ...
let mut dirty = vec![false; self.signals.len()];          // alloc 3
let mut publications = repeat_with(|| None::<Publication<V>>)
    .take(self.signals.len())
    .collect::<Vec<_>>();          // alloc 4
```

All four are per-tick allocations proportional to the signal graph size. In a 60Hz UI these fire
~60 times/second. The scheduler could reuse pre-allocated scratch buffers (stored in `Scheduler`
itself) that are grown-on-demand and cleared at the start of each tick. The `messages` drain can
process the `VecDeque` in-place with `self.queue.drain(..)` without collecting into a `Vec`.

---

### M2 — `committed_signal_snapshots` boxes every signal value per tick

**File:** `crates/aivi-runtime/src/startup.rs:241–250`

```rust
for (&signal, &item) in &self.signal_items_by_handle {
    if let Some(value) = self.runtime.current_value(signal)? {
        snapshots.insert(item, RuntimeValue::Signal(Box::new(value.clone())));
    }
}
```

Every non-empty signal value is cloned AND boxed into a `RuntimeValue::Signal(Box<_>)` on every
tick, then the snapshots are passed to `KernelEvaluator`. The boxing is needed only because
`RuntimeValue::Signal` contains `Box<RuntimeValue>`. This could be avoided by separating the
"this is a signal snapshot" marker from the `RuntimeValue` type — either with a newtype wrapper
or by having `KernelEvaluator` accept `(ItemId, &RuntimeValue)` pairs with an explicit "is signal"
flag, instead of encoding the signal-ness inside the value itself.

---

### M3 — `move_children` iterates all children on every reorder in O(n) GTK calls

**File:** `crates/aivi-gtk/src/host.rs:591–598`

```rust
for index in 0..next_children.len() {
    let child_widget = self.widget_object(&next_children[index])?;
    let sibling = ...;
    box_widget.reorder_child_after(&child_widget, sibling.as_ref());
}
```

This calls `reorder_child_after` on **every child** in the container, not just the moved ones.
GTK4 `reorder_child_after` is an O(n) operation internally (linked list relink). For a container
with N children this is O(n²) GTK mutations. Only the moved children and their new neighbors need
to be relinked.

---

### M4 — Single-variant enums make exhaustive match arms future-invisible

**Files:** `crates/aivi-runtime/src/effects.rs:67–75`

```rust
pub enum SourceReplacementPolicy { DisposeSupersededBeforePublish }
pub enum SourceStaleWorkPolicy   { DropStalePublications }
```

Both currently have exactly one variant. Every `match` on them implicitly covers all cases with
one arm. When a second variant is added, the compiler will force updates to all match sites —
which is good — but callers that currently use `_ =>` patterns will silently accept the new
variant without implementing behavior. These enums are part of the runtime contract surface and
will grow. Document them explicitly and prefer `match` without wildcards at all call sites.

---

### M5 — `coalesce: True` is accepted as a no-op without any warning

**File:** `crates/aivi-runtime/src/providers.rs:207–213`

```rust
"coalesce" => {
    let coalesced = parse_bool(...)?;
    if !coalesced {
        return Err(SourceProviderExecutionError::UnsupportedTimerOption { ... });
    }
    // coalesced == true: silently accepted as no-op
}
```

`coalesce: False` is rejected (correctly, since non-coalesced timers are unsupported). But
`coalesce: True` is silently accepted as a no-op, giving the user no feedback that the option
is being ignored. Per `choices_made.md` item 109, options not implemented honestly should fail
explicitly. Either document `coalesce: True` as the only valid value (and do nothing), or emit
an `UnsupportedTimerOption` for both values until coalescing is actually implemented.

---

### M6 — `parse_orientation` performs unnecessary heap allocation

**File:** `crates/aivi-gtk/src/host.rs:785–790`

```rust
fn parse_orientation(value: &str) -> Option<Orientation> {
    match value.trim().to_ascii_lowercase().as_str() {
```

`.to_ascii_lowercase()` allocates a `String` to lowercase the input. This runs on every
`Box::orientation` property set. Use `unicase` or simply match both cases explicitly:

```rust
match value.trim() {
    "Vertical" | "vertical" => Some(Orientation::Vertical),
    "Horizontal" | "horizontal" => Some(Orientation::Horizontal),
    _ => None,
}
```

---

### M7 — `BackendLinkedRuntime` lifetime couples the runtime to the program's borrow

**File:** `crates/aivi-runtime/src/startup.rs:47–55`

```rust
pub struct BackendLinkedRuntime<'a> {
    backend: &'a BackendProgram,
    ...
}
```

The lifetime `'a` ties `BackendLinkedRuntime` to the borrow of `BackendProgram`. In practice the
runtime needs to outlive individual ticks and be passed to the GTK bridge loop. If `BackendProgram`
is owned elsewhere (e.g. in a `CompilationResult` struct), this lifetime leaks into every
downstream type that touches the runtime. Consider `Arc<BackendProgram>` if the program is
immutable post-compilation.

---

### M8 — `RootDatabase` cross-file invalidation is absent

**File:** `crates/aivi-query/src/db.rs`

When `open_file` or `set_text` changes a file, it invalidates only that file's `parsed` and `hir`
caches (lines 87–89, 139–143). It does not invalidate other files that import the changed file.
In a multi-file workspace (the LSP manages multiple files), a change to a type definition in
`module-a.aivi` will not invalidate the HIR cache for `module-b.aivi` which imports from it.
The LSP will serve stale diagnostics and completions. AGENTS.md memory note on this exact
architectural concern exists.

Fix: maintain a reverse-dependency graph (file → files that import it) and cascade invalidation.
This is the fundamental incremental computation problem for language servers.

---

## Missing Invariant Enforcement

### I1 — `Generation` overflow message loses the handle context

**File:** `crates/aivi-runtime/src/scheduler.rs:58–61`

```rust
fn advance(self) -> Self {
    Self(self.0.checked_add(1).expect("input generation overflow"))
}
```

The panic message does not include which input handle overflowed. With `u64` this is
unreachable in practice, but the `.expect` should be either removed (if `u64` makes overflow
truly impossible) or include `self.0` in the message for diagnosability.

---

### I2 — Duplicate `SourceInstanceId` type across crates

`SourceInstanceId` is defined via the `define_runtime_id!` macro in
`crates/aivi-runtime/src/effects.rs:38`, AND separately in `crates/aivi-backend/src/program.rs`
(exported from `crates/aivi-backend/src/lib.rs:53` as `SourceInstanceId`). These are two
distinct types with the same name and same `u32` representation. `startup.rs` imports the runtime
one; `validate.rs` uses the backend one. Any code path that mixes them will compile silently.
One of them should be defined in `aivi-base` or `aivi-typing` and re-exported from both crates.

---

### I3 — `KernelOriginKind::ItemBody` is a zero-information variant

**File:** `crates/aivi-backend/src/kernel.rs:175–176`

```rust
pub enum KernelOriginKind {
    ItemBody,
    GateTrue { pipeline: PipelineId, stage_index: usize },
    ...
}
```

`ItemBody` carries no identity. It is unclear whether an item-body kernel belongs to a value,
function, or signal item. When validation errors reference `origin.kind == ItemBody` they cannot
direct the user to the specific item. The `KernelOrigin` struct does carry `item: ItemId`, so
this is recoverable, but the kind variant itself is informationally incomplete vs all other
variants which carry their structural context.

---

### I4 — `widget_name` asserts a NamePath invariant silently

**File:** `crates/aivi-gtk/src/host.rs:767–772`

```rust
fn widget_name(path: &NamePath) -> &str {
    path.segments()
        .iter()
        .last()
        .expect("NamePath is non-empty")
        .text()
}
```

The `NamePath` type's non-emptiness invariant is not encoded in its type. If any code path
produces an empty `NamePath` (possible if the HIR layer produces one from a parse error), this
panics at GTK widget creation time. The invariant should either be encoded in `NamePath` itself
(a `NonEmpty<Vec<_>>` wrapper), or `widget_name` should return `Option<&str>` and be handled
gracefully.

---

## Testing Gaps

### T1 — No stack-depth torture test for `strip_signal`

AGENTS.md explicitly requires stack-depth tests. `strip_signal` is the identified recursive
function. A test with `RuntimeValue::Signal(Box::new(Signal(Box::new(...))))` nested 10,000 deep
must be added. Without this, C1 above is not regression-locked.

---

### T2 — No property or fuzz test for the scheduler glitch-freedom invariant

The scheduler has good unit tests for basic correctness (transactional snapshot, diamond graph,
disposal). But glitch-freedom — that a derived signal never observes a mix of old and new values
across one tick — is a *property* that holds for all graph shapes. This should be a proptest or
quickcheck test over randomly generated graphs and publication sequences.

---

### T3 — `Rc<RefCell<_>>` re-entrancy in `GtkConcreteHost` is untested

`drain_events` calls `borrow_mut()` on the `queued_events` `RefCell`. The GTK signal handler for
`clicked` also calls `borrow_mut()` (line 397). If `drain_events` is ever called from within a
GTK signal callback on the same thread (which is possible in GLib async tasks), this panics at
runtime. The test at line 991 (`button.emit_clicked()` then `drain_events`) does not cover the
interleaved case.

---

### T4 — No round-trip test for `display_text` / `Display for RuntimeValue`

All error variants that embed `RuntimeValue` format it via `Display`, which calls `display_text`.
There is no test that exercises these format paths for nested values. Adding tests for
`format!("{}", RuntimeValue::OptionSome(Box::new(RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(1))))))` etc. would lock down the recursive formatting paths.

---

### T5 — No test for multi-file invalidation in `RootDatabase`

`RootDatabase` tests in `crates/aivi-query/tests/database.rs` cover single-file change
invalidation. There is no test that verifies that changing file A invalidates the cached HIR
of file B that depends on A. This is the exact gap from M8 above and should be a failing test
until the cross-file invalidation is implemented.

---

### T6 — `timer_every` worker cancellation not tested

`spawn_timer_every` polls `port.is_cancelled()` between sleeps (lines 245–252). There is no
test that verifies the timer thread terminates promptly after the source is suspended/disposed
— only that it publishes. A test cancelling the source after the first tick and verifying no
further publications arrive would lock down the teardown path.

---

## Per-Crate Review Summary

### `aivi-runtime`
- C1, C3, H2, H5, M1, M2, M4, M5 live here
- `glib_adapter.rs`: The `AtomicBool::swap(true, AcqRel)` at line 200 could use `Acquire` on
  the swap and `Release` on the store for minimal ordering (current AcqRel is correct but
  slightly stronger than needed on the swap side)
- `effects.rs`: Single-variant enums (M4). `SourceRuntimeSpec::new` has good defaults;
  the `ProviderManaged` cancellation fallback for custom providers is the right explicit choice

### `aivi-backend`
- C2, H4 live here
- `validate.rs`: Thorough validation of kernel ABI, pipeline consistency, decode steps. No
  obvious gaps. The `KernelExprKind` expression-type consistency (does the declared `layout`
  match the expression kind?) is not validated — e.g. nothing checks that a `Bool`-layout
  expression doesn't contain a `KernelExprKind::Tuple`. This would catch lowering bugs earlier
- `codegen.rs`: `HashSet<KernelId>` at line 1 uses the default hasher — same issue as M6 for
  `RootDatabase`; use `FxHashSet`
- `runtime.rs`: `KernelEvaluator` evaluation cache (`item_values: BTreeMap`) is per-instance
  and lost between calls (H2). The `evaluating: BTreeSet` correctly detects recursive evaluation

### `aivi-gtk`
- H1, H3, M3, I4 live here
- `executor.rs`: `GtkRuntimeExecutor` correctly drives child group transitions and event routing
- `bridge.rs`: Widget identity uses stable HIR-backed `GtkNodeInstance` — correct per
  `choices_made.md` item 73
- `lower.rs`: `on*` attribute convention for events (item 74) is explicit and easy to replace

### `aivi-query`
- H6, M8 live here
- `db.rs`: `store_parsed`/`store_hir` correctly handle the race between compute and store
  (revision check before insert). The pattern is correct but would benefit from a comment
  explaining the optimistic-store semantics
- `queries/hir.rs`: Calls `lower_hir_module` without going through validation — if HIR
  validation is expensive, calling it unconditionally per-file-change may be a bottleneck
  in large workspaces

### `aivi-hir`
- No critical issues found. The elaboration pipeline (gate, fanout, recurrence, source lifecycle,
  truthy/falsy, domain operator) is organized correctly by concern
- `general_expr_elaboration.rs`: Large file. The exhaustiveness check (item 64) correctly scopes
  to provably-known scrutinee types

### `aivi-core`, `aivi-lambda`, `aivi-syntax`, `aivi-typing`, `aivi-base`, `aivi-lsp`, `aivi-cli`
- No critical or high issues found in these crates from direct reading
- `aivi-syntax/src/lex.rs` and `parse.rs` are not fuzz-tested (T2-equivalent gap)
- `aivi-lsp`: LSP features (`completion.rs`, `hover.rs`, `definition.rs`) call through the query
  database correctly; no direct HIR bypass detected

---

## Prioritized Action Items

### Priority 1 — Fix before any integration testing

1. **C1** Replace recursive `strip_signal` with iterative loop (`providers.rs:269`)
2. **C2** Replace recursive `display_text` with write-based formatter or depth-capped recursion (`runtime.rs:97`)
3. **C3** Document or enforce that `KernelEvaluator` must not re-enter the GLib driver, or split tick lock scope (`glib_adapter.rs:210`)
4. **H3** Fix `text_literal` to error on interpolation segments rather than silently dropping them (`host.rs:775`)
5. **H5** Replace `.expect` with explicit `BackendRuntimeError` in `required_signal_globals` (`startup.rs:266`)

### Priority 2 — Fix before sustained development on dependent layers

6. **H1** Document or eliminate `Rc<RefCell<_>>` in `GtkConcreteHost` with a threading invariant comment and re-entrancy guard (`host.rs:53`)
7. **H2** Reuse a single `KernelEvaluator` across all argument/option evaluations in `evaluate_source_config` (`startup.rs:194`)
8. **I2** Resolve duplicate `SourceInstanceId` — define once in `aivi-base` or `aivi-typing`, re-export from both crates
9. **M8** Implement cross-file cache invalidation in `RootDatabase` before multi-file LSP work begins
10. **H4** Index `CompiledProgram::kernels` by `KernelId` instead of linear scan (`codegen.rs:35`)

### Priority 3 — Performance / quality

11. **M1** Pre-allocate scheduler scratch buffers (`pending`, `dirty`, `publications`) on `Scheduler` and reuse per tick
12. **M2** Avoid per-tick boxing in `committed_signal_snapshots`
13. **M3** Reorder only moved children in `move_children`, not all children
14. **H6** Replace `HashMap` with `FxHashMap` in `RootDatabase` and `HashSet` in `codegen.rs`
15. **M5** Make `coalesce: True` explicit (no-op with a note, or also unsupported)
16. **M6** Remove heap allocation in `parse_orientation`

### Priority 4 — Tests AGENTS.md requires

17. **T1** Stack-depth test for `strip_signal` at depth 10000
18. **T2** Property/fuzz test for scheduler glitch-freedom invariant
19. **T3** Re-entrancy test for `GtkConcreteHost` event queue
20. **T4** `Display for RuntimeValue` round-trip tests for nested values
21. **T5** Multi-file invalidation test for `RootDatabase` (write as a failing test first)
22. **T6** Timer source cancellation / teardown test
