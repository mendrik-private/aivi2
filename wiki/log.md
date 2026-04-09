# Log

Append-only chronological record of wiki activity.  
Parse with: `grep "^## \[" log.md | tail -10`

---

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
- `crates/aivi-gtk/src/schema.rs` + `crates/aivi-gtk/src/host.rs`: buttons now support `focusable={Bool}` so board cells can suppress focus flashes

## [2026-04-08] ingest | ADT companion function syntax

Updated companion syntax notes in [type-system.md](type-system.md) and [indexed-collections.md](indexed-collections.md) to match the new explicit receiver style.  
Sources updated alongside the wiki: `syntax.md`, `AIVI_RFC.md`, `manual/guide/types.md`, `crates/aivi-syntax/src/parse.rs`, and `crates/aivi-hir/src/lower.rs`.
## [2026-04-08] query | triggered temporal scheduling design

Compared two designs for delayed/finite repeated signal triggering: a new source-shaped helper
versus a recurrence/pipe-shaped temporal transform. Concluded that recurrence/temporal pipe is the
more AIVI-like long-term home, while a narrow source helper is a pragmatic short-term fallback.  

## [2026-04-09] ingest | truthy/falsy branch carriers

Aligned the manual and wiki with the implemented `T|>` / `F|>` semantics.
Documented the canonical carrier set (`Bool`, `Option`, `Result`, `Validation`)
plus the one-outer-`Signal (...)` lift, clarified ambient `.` rebinding for
single-payload constructors, and corrected the surface feature matrix note.

## [2026-04-09] ingest | pipe stage memos

Documented `#name` as the way to remember stage inputs/results inside ordinary pipe flows, added
manual examples framing it as the local-`let` replacement for pipes, and recorded the grouped
branch memo behavior plus the `&|>` cluster boundary in the wiki.
