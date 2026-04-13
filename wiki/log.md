# Log

Append-only chronological record of wiki activity.  
Parse with: `grep "^## \[" log.md | tail -10`

---

## [2026-04-10] ingest | anonymous lambdas

Read syntax/parser/HIR lowering and docs around expression lambdas. Added [anonymous-lambdas.md](anonymous-lambdas.md).  
Captured explicit `x => ...`, narrow `. == value` shorthand, and hoisting-to-hidden-functions implementation model.

## [2026-04-12] ingest | audit followups

Documented the audit follow-up slice in the wiki.  
Added [codebase-audit-2026-04-12.md](codebase-audit-2026-04-12.md), corrected the `compile` vs `build`
execution boundary in [cli.md](cli.md), and updated [stdlib.md](stdlib.md) for low-level modules and
the preferred `first` / `second` pair naming surface.

## [2026-04-12] add | uniform elegance refactor contract

Codified the non-negotiable invariants, forbidden end states, and done criteria for the Wadler-driven cleanup backlog.  
Added [uniform-elegance-refactor.md](uniform-elegance-refactor.md) so the refactor cannot stop at dual paths, doc drift, or partial stdlib cleanup.

## [2026-04-12] add | prelude surface policy

Chose the canonical public direction for `aivi.prelude`: class-polymorphic first, carrier-specific helpers second.  
Added [prelude-surface-policy.md](prelude-surface-policy.md) so later stdlib cleanup tasks have a fixed target instead of drifting design.

## [2026-04-12] ingest | class laws documentation

Aligned the manual, RFC, and wiki on executable class support, law coverage, and the non-monadic
`Signal` / `Validation` boundaries.
Added [class-laws.md](class-laws.md), linked the manual guide to the new laws page, updated the
canonical `Task` support story, and changed the docs slice to teach `!=` as surface inequality over
the same `Eq` evidence instead of a second canonical instance body.

## [2026-04-13] ingest | list contains membership

Aligned list membership with ambient `Eq` instead of predicate/comparator plumbing.  
Updated `stdlib/aivi/list.aivi`, demos, manual pages, and wiki notes so `contains` now means
`Eq`-driven membership while predicate search stays on `any`.

## [2026-04-06] ingest | Initial wiki seeded from codebase

Seeded wiki from source files in `src/`, `crates/`, `manual/`, `stdlib/`, `syntax.md`, `AIVI_RFC.md`.  
Pages created: architecture, compiler-pipeline, type-system, signal-model, runtime, gtk-bridge, query-layer, lsp-server, cli, stdlib.  
Sources read: all `crates/*/src/lib.rs` files, `AGENTS.md`, `manual/guide/*.md` listing.

## [2026-04-06] query | Snake & Reversi demo audit

Read `demos/snake.aivi` and `demos/reversi.aivi`. Found and fixed 8 issues across both files. Created [demo-audit.md](demo-audit.md).  
Key fixes: domain abstraction leak in snake; three manual full-record constructions in reversi (should use `<|`); reversi timer missing unit; dead `Candidate.flips` field.

## [2026-04-06] query | Manual structure improvements

Restructured `manual/guide/README.md` into five story arcs (Functional programming → Pipe algebra → Domains → Signals & reactivity → GTK & markup) plus an External integrations arc.  
Created `manual/guide/integrations.md` — the missing integration patterns page (HTTP, timers, filesystem, database, D-Bus, custom providers, tips).  
Added sidecar note to `manual/stdlib/index.md`.  
Key insight: the old guide was a feature inventory; the new structure tells a learning journey.

## [2026-04-06] add | AsyncTracker signal lifecycle tracker

**Trigger**: user noted `sig.done`, `sig.error`, `sig.pending`, `sig.do once` were planned but missing.

**Finding**: No implementation existed anywhere — not in RFC, stdlib, HIR, runtime, or manual.

**Implemented**:
- `stdlib/aivi/async.aivi` — `AsyncTracker E A` record type + `step` accumulation function + `isPending`, `isDone`, `isFailed` helpers
- `manual/stdlib/async.md` — full reference page including fire-once idiom
- `manual/guide/signals.md` — "Tracking async state" section: tracker pattern + `sig.pending/done/error` projections + fire-once accumulation idiom
- `manual/stdlib/index.md` — added `aivi.async` to at-a-glance table
- `manual/.vitepress/navigation.ts` — added Async Tracker to stdlib "Core Values & Collections" section
- `wiki/signal-model.md` — AsyncTracker pattern documented

**Design decisions**:
- Tracker fields are `pending/done/error` (not `loading/value/error`) to match user's stated names
- `done` preserves last successful value on subsequent errors (stale-while-revalidate)
- `do once` documented as accumulation idiom; dedicated `@effect`/`doOnce` noted as planned
- No compiler changes needed — pure stdlib + documentation

## [2026-04-06] ingest | OpenAPI source feature implementation

Added full `@source api` capability handle feature:

- `crates/aivi-openapi/`: new crate with model, parser, resolver, operations, auth, typegen, diagnostics modules
- `crates/aivi-typing/src/source_contracts.rs`: 5 new BuiltinSourceProvider variants (ApiGet, ApiPost, ApiPut, ApiPatch, ApiDelete) + api_options()
- `crates/aivi-hir/src/capability_handle_elaboration.rs`: Api capability family, lower_api_signal_member, lower_api_value_member, supports_* functions updated
- `crates/aivi-hir/src/validate.rs`: exhaustive match updated for new providers
- `crates/aivi-runtime/src/providers.rs`: ApiPlan, spawn_api_worker, extract_auth_header, base64_encode; run_http_request updated for Api variants
- `stdlib/aivi/api.aivi`: ApiError, ApiAuth, ApiSource, ApiResponse stdlib types
- `crates/aivi-cli/src/main.rs`: `aivi openapi-gen` command
- `manual/guide/source-catalog.md`: OpenAPI section + unified capability families table updated
- `manual/guide/openapi-source.md`: new guide page
- `manual/guide/surface-feature-matrix.md`: `@source api` row added
- `fixtures/frontend/milestone-1/valid/sources/petstore.yaml`: Petstore OpenAPI spec fixture
- `fixtures/frontend/milestone-1/valid/sources/openapi_source.aivi`: API handle fixture

## [2026-04-07] ingest | Indexed collection ergonomics

Added indexed collection ergonomics and aligned the docs with the current higher-kinded execution slice.

- `stdlib/aivi/list.aivi`: `indexed`, `mapWithIndex`, `reduceWithIndex`, `filterMap`
- `stdlib/aivi/option.aivi`: `fold`, `mapOr`, `isSomeAnd`
- `stdlib/aivi/matrix.aivi`: `MatrixIndex`, `coord`, indexed traversal/update helpers, user-authored `Functor` / `Foldable` instances
- `stdlib/aivi/prelude.aivi`: ambient aliases for the new option/list surfaces
- manual + RFC updated to document imported unary higher-kinded instance execution and to propose indexed HKTs plus ADT bodies as deferred work

## [2026-04-07] ingest | ADT companion bodies

Implemented brace-bodied closed-sum companions and updated the spec/docs to stop treating them as deferred work.

- `crates/aivi-syntax/`: new CST nodes and parser/formatter support for brace-bodied sum companions
- `crates/aivi-hir/`: companion members now lower to ordinary synthetic function items that retain owner type parameters
- `crates/aivi-cli/tests/check.rs`: CLI checks for same-module and imported companion usage
- `manual/guide/types.md`, `syntax.md`, `manual/guide/surface-feature-matrix.md`, `AIVI_RFC.md`: user-facing docs and spec updated to describe the implemented surface

## [2026-04-07] ingest | GLib driver fairness budget

Hardened the GLib runtime adapter so async wake-driven draining can no longer run an unbounded number of ticks on one GTK main-loop wake.

- `crates/aivi-runtime/src/glib_adapter.rs`: both `GlibSchedulerShared` and `GlibLinkedRuntimeShared` now apply a fixed 32-tick async wake budget, yield between ticks, and reschedule another GLib callback when work remains
- Explicit synchronous drains (`queue_publication_now`, `tick_now`) still run until idle
- Added stress coverage for long worker-publication chains and linked-runtime stop safety after a queued follow-up callback

## [2026-04-12] ingest | source-free run artifact boundary

Documented the new source-free runnable bundle boundary after landing serialized run-artifact
support.

- updated [cli.md](cli.md) for `run-artifact.json`, source-free `build`, and the remaining compiled-payload gap
- updated [runtime.md](runtime.md) for `BackendRuntimeLinkSeed` / `link_backend_runtime_with_seed()`
- refreshed [codebase-audit-2026-04-12.md](codebase-audit-2026-04-12.md) so the closed source-carrying gap and the remaining compiled-payload gap are separated cleanly

## [2026-04-12] ingest | removed obsolete surface matrix refs

Kept `manual/guide/surface-feature-matrix.md` deleted to match `main` and cleaned active references to
it from current wiki/task notes.

- updated [surface-syntax.md](surface-syntax.md), `task_plan.md`, and `findings.md`

## [2026-04-12] ingest | native artifact bundle launch

Closed compiled bundle launch gap by teaching `aivi build` / `aivi run` artifacts to emit and
consume precompiled native kernel sidecars alongside serialized backend metadata payloads.

- updated [cli.md](cli.md) for native-sidecar bundle launch semantics
- updated [runtime.md](runtime.md) for `link_backend_runtime_with_seed_and_native_kernels()`
- refreshed [codebase-audit-2026-04-12.md](codebase-audit-2026-04-12.md) to mark the compiled
  bundle launch gap closed
- historical mentions inside this append-only log remain as history, not live documentation

## [2026-04-07] ingest | From signal fan-out sugar

Implemented top-level `from source = { ... }` syntax for grouped derived signals.

- `crates/aivi-syntax/`: lexer/CST/parser/formatter support for `from` items and indentation-aware entry parsing
- `crates/aivi-hir/src/lower.rs`: `from` lowers into ordinary synthetic `Signal` items by piping the shared source into each entry body
- `crates/aivi-lsp/src/semantic_tokens.rs`, `tooling/packages/vscode-aivi/syntaxes/aivi.tmLanguage.json`, `tooling/packages/vscode-aivi/snippets/aivi.json`: editor keyword highlighting and snippet support
- `manual/guide/building-snake.md`, `demos/snake.aivi`, `syntax.md`, `manual/guide/surface-feature-matrix.md`: user-facing docs and demo updated to use/document the sugar

## [2026-04-08] query | Reversi click latency

Adjusted `demos/reversi.aivi` so human clicks update the board immediately, delay the full snapshot recompute by 1ms, and derive the AI preview from the current board.  
Added a focused GTK regression test in `crates/aivi-cli/src/run_session.rs` that checks both the first and second human moves paint red stones promptly.

## [2026-04-08] ingest | UI click-path responsiveness

Documented the follow-up runtime and GTK bridge changes behind the Reversi latency fix.

- `crates/aivi-runtime/src/glib_adapter.rs`: direct UI publications can now drain only the current scheduler queue instead of also draining timer wakeups

## [2026-04-12] query | Wadler-style audit

Audited AIVI's language shape, stdlib, and executable typeclass support through a Philip Wadler lens.  
Added [wadler-audit.md](wadler-audit.md); key findings were strong algebraic intent, a less uniform builtin-vs-authored execution story, and RFC drift behind the current executable class support.
- `crates/aivi-gtk/src/schema.rs` + `crates/aivi-gtk/src/host.rs`: buttons now support `focusable={Bool}` so board cells can suppress focus flashes

## [2026-04-08] ingest | ADT companion function syntax

Updated companion syntax notes in [type-system.md](type-system.md) and [indexed-collections.md](indexed-collections.md) to match the new explicit receiver style.  
Sources updated alongside the wiki: `syntax.md`, `AIVI_RFC.md`, `manual/guide/types.md`, `crates/aivi-syntax/src/parse.rs`, and `crates/aivi-hir/src/lower.rs`.
## [2026-04-08] query | triggered temporal scheduling design

## [2026-04-12] query | codebase audit

Audited all Rust crates, the CLI execution boundary, stdlib/manual parity, and surface naming.  
Added [codebase-audit-2026-04-12.md](codebase-audit-2026-04-12.md) and recorded the current JIT/object-code/build split plus the highest-confidence cleanup targets.

Compared two designs for delayed/finite repeated signal triggering: a new source-shaped helper
versus a recurrence/pipe-shaped temporal transform. Concluded that recurrence/temporal pipe is the
more AIVI-like long-term home, while a narrow source helper is a pragmatic short-term fallback.  

## [2026-04-10] ingest | Phase 4 slot store runtime model

Documented the scheduler storage rewrite that introduced explicit committed/pending slot state for
signal values.

- `crates/aivi-runtime/src/scheduler.rs`: new `SlotStore`, `CommittedSlot`, `PendingSlot`,
  `RawBytes`, `RawSlotPlanId`, and `PendingRawValue` types
- Raw slots now carry bytes plus a decoded value shadow, while store-managed slots preserve the
  existing `CommittedValueStore` path
- Added runtime tests covering raw commit/clear, raw-to-store-managed transitions, and mixed
  pending dependency reads

## [2026-04-10] ingest | Phase 5 partition-driven linked ticks

Documented the final signal-refactor phase that switched linked-runtime ticking onto `ReactiveProgram`
partitions.

- `crates/aivi-runtime/src/reactive_program.rs`: partitions now group disjoint same-batch signals by
  identical root-signal cones and expose contiguous topo slices plus partition root metadata
- `crates/aivi-runtime/src/scheduler.rs`: added a `ReactiveProgram`-driven tick order alongside the
  existing graph-batch traversal
- `crates/aivi-runtime/src/startup/linked_runtime.rs`: linked ticks use the program-driven path when
  the assembly graph matches the runtime graph, with a fallback to the generic path for task-only
  helper runtimes

## [2026-04-09] ingest | From selector body signal lifting

Documented the HIR/typecheck fix that lets parameterized `from` selector bodies read earlier
same-block signals as payloads inside ambient body contexts, including `T|>` / `F|>` branches.
Updated [signal-model.md](signal-model.md) to describe the lift and cite the relevant lowering and
typecheck paths.

## [2026-04-09] ingest | truthy/falsy branch carriers

Aligned the manual and wiki with the implemented `T|>` / `F|>` semantics.
Documented the canonical carrier set (`Bool`, `Option`, `Result`, `Validation`)
plus the one-outer-`Signal (...)` lift, clarified ambient `.` rebinding for
single-payload constructors, and corrected the surface feature matrix note.

## [2026-04-09] ingest | pipe stage memos

Documented `#name` as the way to remember stage inputs/results inside ordinary pipe flows, added
manual examples framing it as the local-`let` replacement for pipes, and recorded the grouped
branch memo behavior plus the `&|>` cluster boundary in the wiki.

## [2026-04-09] ingest | reversi pipe memo cleanup

Updated the Reversi demo audit after `demos/reversi.aivi` switched several helper-heavy flows to
`#name` memos, using the demo as a concrete example of naming intermediate ray, board, snapshot,
and animation-step values without introducing throwaway helpers.

## [2026-04-09] ingest | reversi helper cleanup

Expanded the Reversi cleanup beyond the first memo pass: boolean routing now consistently uses
`T|>` / `F|>`, additional helpers memo derived move and score values where that improves readability,
and small `RayState` / animation updates now use `<|` patches instead of manual record rebuilds.

## [2026-04-09] ingest | selected-subject function headers

## [2026-04-13] ingest | reversi closeout validation

Closed the last Wadler backlog item by updating `demos/reversi.aivi` to use checked matrix initialization and
an explicit setup-error state, then refreshed stale snapshots, CLI fixtures, and formatter-guarded AIVI files.

## [2026-04-13] ingest | post-merge drift cleanup

Ran a post-merge codebase pass and corrected the remaining stale historical/wiki notes.  
Updated `wiki/wadler-audit.md`, `wiki/stdlib.md`, `wiki/indexed-collections.md`, and `wiki/index.md` so the wiki now matches the landed prelude, list, matrix, RFC, and class-law state.

Implemented `param!` and `param { path! }` header sugar so `func` and companion bodies can begin
with subject-rooted `|>` or `<|` continuations without an explicit `=>`.

- `crates/aivi-syntax/`: standalone `!` token plus parser/formatter support for selected-subject
  headers and projected subject selectors
- `crates/aivi-hir/tests/selected_subject_sugar.rs` and `crates/aivi-cli/tests/check.rs`: focused
  lowering/typechecking and `aivi check` coverage for direct, projected, and patch-rooted cases
- `manual/guide/values-and-functions.md`, `manual/guide/pipes.md`,
  `manual/guide/record-patterns.md`, `manual/guide/types.md`, `manual/guide/surface-feature-matrix.md`,
  and `syntax.md`: user-facing docs updated
- `demos/reversi.aivi`: `recordOpponent` and `flipsFromDirection` now use the new sugar

## [2026-04-09] ingest | reversi syntax showcase refactor

Refactored `demos/reversi.aivi` to showcase a wider cross-section of the implemented surface sugar.

- Expanded the demo's use of selected-subject headers, selector-based subject picks, record shorthand,
  pipe memos, and `T|>` / `F|>` routing across the pure game-logic helpers
- Kept the live GTK boolean helper chain on its older call shape after validation showed that path is
  the stable one currently exercised by the Reversi run-session tests

## [2026-04-09] query | recent surface syntax audit

Audited `main` commits from the last two days for user-facing syntax changes and reconciled the
documentation coverage across the RFC, manual, and wiki.

- `AIVI_RFC.md`: added missing coverage for top-level `from`, selected-subject function headers,
  companion-member selected-subject continuations, pipe memos, and temporal replay heads
- `wiki/surface-syntax.md`: added a stable audit summary page linking recent surface syntax work to
  the canonical manual/wiki/RFC locations
- `wiki/type-system.md`: extended closed-sum companion notes to mention selected-subject companion
  bodies explicitly

## [2026-04-09] ingest | parameterized from selectors

Implemented parameterized entries inside top-level `from source = { ... }` fan-out blocks.

- `crates/aivi-syntax/`: `from` entries now carry attached standalone `type` lines plus optional parameters, with parser/formatter coverage and orphan-annotation diagnostics
- `crates/aivi-hir/src/lower.rs`: zero-parameter entries still lower to synthetic `Signal` items; parameterized entries lower to synthetic `Function` items whose final result type is wrapped in builtin `Signal`
- `crates/aivi-cli/tests/check.rs`, `crates/aivi-hir/src/lower.rs`, and `crates/aivi-syntax/tests/`: end-to-end, lowering, formatter, parser, and snapshot coverage
- `syntax.md`, `manual/guide/signals.md`, `manual/guide/surface-feature-matrix.md`, and `wiki/signal-model.md`: docs updated to describe the new selector surface

## [2026-04-10] query | Structural equality vs comparator helpers

Read `wiki/type-system.md`, `crates/aivi-hir/src/typecheck/checker.rs`, `crates/aivi-typing/src/eq.rs`, `stdlib/aivi/list.aivi`, `stdlib/aivi/prelude.aivi`, `demos/snake.aivi`, and `demos/reversi.aivi`.

Created [equality-semantics.md](equality-semantics.md) to capture current behavior:
- concrete closed sums/records/domains do get compiler-derived structural `Eq`
- open type parameters still need explicit `Eq` constraints
- `coordEq` / `cellEq` in demos mainly exist to pass equality into comparator-taking list helpers, not because direct `Coord == Coord` is unsupported

## [2026-04-10] ingest | reversi run-path cleanup

Read `demos/reversi.aivi`, `crates/aivi-hir/src/general_expr_elaboration.rs`, `crates/aivi-hir/src/truthy_falsy_elaboration.rs`, `crates/aivi-cli/src/run_session.rs`, and `crates/aivi-cli/src/mcp.rs`.

Updated [demo-audit.md](demo-audit.md) with the Reversi run-path compatibility fix:
- replaced implicit `. == cell` comparator lambdas with explicit `coordEq`
- routed `clickState` through `stateLegalAt` so the run path sees an explicit Bool helper instead of an inline branch subject
- documented that this fixes the typed-core/runtime launch blocker without widening compiler semantics

## [2026-04-10] audit | manual hallucination scan
- Scanned 78 manual markdown files (guide + stdlib) against stdlib source, syntax.md, AIVI_RFC.md, schema.rs
- Found 7 critical, 5 high, 3 medium, 2 low findings — overall risk: HIGH
- Three main patterns: domain-member-as-standalone-export (url.md, duration.md, color.md), stale source option names (integrations.md), and nonexistent/placeholder stdlib names (record-patterns.md, modules.md, openapi-source.md)
- See wiki/manual-hallucination-report.md for full findings

## [2026-04-10] feat | aivi.bits stdlib + domain body dispatch + color.aivi bodies

Implemented three phases on branch `copilot/bits-stdlib`, merged to main:

- **Phase 1**: 7 bitwise intrinsics (BitAnd, BitOr, BitXor, BitNot, ShiftLeft, ShiftRight,
  ShiftRightUnsigned) wired through HIR → evaluator → Cranelift codegen; `stdlib/aivi/bits.aivi`
  exports thin wrappers with `@test` values.
- **Phase 2**: Domain body dispatch — `program.rs` now holds a `domain_member_items` map;
  evaluator and codegen prefer compiled item bodies over the Rust fallback when present.
- **Phase 3**: `stdlib/aivi/color.aivi` now has authored AIVI bodies for all extractors
  and constructors using `aivi.bits` intrinsics; bodies written in carrier view (return Int,
  not Color). `blend` remains body-less pending float arithmetic.
- Key invariant: domain member bodies must return the **carrier type**, not the nominal domain
  type — carrier-view checking is applied by `rewrite_domain_carrier_view` in the type checker.

## [2026-04-12] ingest | async fire-once docs

Removed stale roadmap claims about a future `@effect` decorator / `doOnce` combinator from `manual/stdlib/async.md` and `manual/guide/signals.md`.  
Updated [signal-model.md](signal-model.md) to describe only the current `activeWhen`-gated fire-once idiom.

## [2026-04-12] query | Domains versus product types

Clarified that `aivi.date` still models `Date`, `TimeOfDay`, `DateTime`, and `ZonedDateTime` as
constructor-backed product types, while `DateDelta` is the domain. Added
[data-shapes.md](data-shapes.md) to capture the current split between records, constructor-backed
types, and domains.

## [2026-04-12] query | Date equality versus ordering

Confirmed that imported `Date` values already support structural `==`, while infix `<` still fails
because ordering lowers through `Ord.compare` and the current checker only treats builtin or
same-module `Ord` instances as dependable. Updated [equality-semantics.md](equality-semantics.md)
with the current `Date` / `Duration` comparison split.

## [2026-04-12] ingest | executable evidence unification

Updated the wiki to match the executable-evidence refactor for class members.

- `wiki/type-system.md` now describes imported unary higher-kinded execution through authored executable evidence
- `wiki/indexed-collections.md` now frames `Matrix` participation in `map` / `reduce` through authored executable evidence instead of hidden callable lowering

## [2026-04-12] ingest | executable class doc canon

Recorded the new canonical ownership for executable class support docs.

- `manual/guide/typeclasses.md` now owns the registry-backed support table and dependent docs link there instead of describing their own matrices
- `wiki/type-system.md` and new `wiki/wadler-audit.md` capture the preserved `Signal` / `Validation` / `Task` invariants plus the traverse-result and `!=` notes

## [2026-04-13] ingest | prelude shadow cleanup

Updated the docs/wiki to match the narrowed ambient prelude surface.

- `stdlib/aivi/text.aivi` now hides text `join` from project-wide hoist, so bare `join` can stay the canonical `Monad` name while text callers import `aivi.text.join` explicitly where needed
- `stdlib/aivi/pair.aivi` now hides compatibility aliases `fst` / `snd` / `mapFst` / `mapSnd` from hoist, leaving only the preferred pair names ambient
- `manual/guide/building-snake.md`, `manual/stdlib/prelude.md`, and `wiki/stdlib.md` now teach explicit `aivi.text.join` imports for text joining and keep bare `join` for `Monad`

## [2026-04-13] ingest | executable boundary docs

Documented the current builtin-vs-authored executable class boundary in the manual/wiki.

- `manual/guide/typeclasses.md` now has an explicit execution-boundary section covering builtin evidence intrinsics, authored executable evidence, and hidden lowered member bodies
- `manual/guide/classes.md` now links directly to that boundary section instead of only naming higher-kinded support in general
- `wiki/type-system.md` now records that imported unary authored higher-kinded execution works by reusing hidden lowered member bodies rather than expanding the builtin carrier table

## [2026-04-13] ingest | advanced ambient class scope

Documented the ambient class graph outside the primary executable slice.

- `manual/guide/typeclasses.md` now lists the secondary ambient classes and explicitly de-scopes them from the builtin support table unless a narrower feature doc says otherwise
- `wiki/type-system.md` now records that those ambient declarations are real but not a blanket runtime-support promise
