---
apply: always
---

# AGENTS.md

You are implementing **AIVI** in **Rust**: a purely functional, reactive, GTK/libadwaita-first language. Optimize for correctness, explicit invariants, strong abstractions, deterministic behavior, and production-quality implementation.

## Source of truth

- Follow the language spec over preference.
- If the spec is ambiguous: identify the ambiguity, list plausible interpretations, choose the narrowest coherent one, implement so later refinement is cheap, and document the decision.
- Do not silently invent material semantics.

## Assume

- purely functional surface model,
- strict closed types,
- no null / undefined,
- no if/else or loops in surface syntax,
- expression-first design,
- pipe algebra as primary control flow,
- first-class signals and source-backed reactivity,
- higher-kinded abstractions in the core,
- typed external decoding,
- native compilation,
- lowering through **HIR -> typed core -> closed typed lambda IR -> backend IR**,
- Cranelift for AOT and JIT,
- runtime scheduler, signal engine, GC, source watchers, and GTK bridge,
- GTK main thread must never block on background work.

## AIVI comment syntax

AIVI uses `//` for line comments, `/* */` for block comments, and `/** **/` for doc comments.
**Never use `--` or `#` as comment syntax** — both are parse errors in AIVI.

## Before coding

Identify:
- semantic invariants,
- ownership/lifetime invariants,
- threading and scheduler invariants,
- stack-safety invariants,
- IR invariants,
- diagnostic invariants.

Prefer:
- principled models over special cases,
- typed structure over stringly protocols,
- explicit costs over hidden costs,
- deterministic scheduling over opportunistic behavior,
- message passing over shared mutable state,
- reusable abstractions over copy-paste,
- root-cause fixes over patches.

Make illegal states unrepresentable where practical.

## Layering

Use the correct layer:

1. parser / CST
2. name resolution / HIR
3. type + kind checking
4. typed core desugaring
5. closure/lambda lowering
6. monomorphization and/or dictionary passing
7. runtime-aware/backend IR
8. Cranelift codegen
9. runtime / scheduler / GTK bridge
10. tooling / diagnostics / formatting

Do not solve type problems in the parser, runtime semantics in ad-hoc AST rewrites, or GTK concerns in the pure core unless the spec requires it.

## Rust rules

- Encode invariants in types.
- Use explicit enums, typed IDs, and clear ownership boundaries.
- Minimize global mutable state.
- Keep `unsafe` tiny, audited, and justified by an explicit invariant.
- Make `Send`/`Sync` boundaries explicit.
- Avoid `Rc<RefCell<_>>` as architecture unless it is the best constrained local tradeoff.
- Justify new crates by invariant, runtime cost, compile time, binary size, and maintenance risk.

Prefer arenas, interners, slot maps, immutable sharing, bounded queues, and explicit worklists where they improve clarity and predictability.

## IR and semantics

Each IR must define:
- ownership model,
- identity strategy,
- span/source mapping,
- validation rules,
- debug/pretty-print form,
- test fixtures.

Be rigorous about:
- closed ADTs and records,
- constructor arity,
- exhaustiveness,
- kind checking,
- HKTs,
- partial application of type constructors,
- monomorphization vs dictionary-passing boundaries,
- lawful core abstractions,
- signal vs non-signal separation,
- purity boundaries.

Keep inference local and predictable. Prefer explicit, actionable diagnostics over cleverness.

## Runtime, concurrency, stack safety

Never assume recursion is safe. Prevent stack overflow by design using tail-position analysis, trampolines, loops, or explicit worklists where depth may be unbounded. Avoid recursive evaluators or walkers that fail on adversarial input.

Runtime rules:
- GTK widget creation, mutation, and event dispatch stay on the GTK main thread.
- I/O, decoding, file watching, networking, D-Bus round-trips, and heavy computation run on workers.
- Workers publish immutable messages into scheduler-owned queues.
- Workers never mutate UI-owned state directly.
- Signal propagation must be batched, topologically ordered, glitch-free, and transactional per scheduler tick.
- Design out deadlocks, starvation, leaks, races, spin loops, and teardown bugs early.

## Memory and FFI

Assume ordinary language values may move. Do not rely on stable addresses for them.

At FFI and UI boundaries:
- use stable handles, pinning, or copied representations only where required,
- keep ownership transfer explicit,
- keep pinning narrow,
- preserve abstractions so the allocator/collector can evolve without semantic churn.

## GTK / GNOME boundary

Target the real Linux desktop: GTK4/libadwaita, GLib main-context integration, GObject ownership semantics, non-blocking UI behavior, D-Bus, filesystem watching, process/OS integration, and correct startup/disposal/shutdown.

Keep pure language logic pure. Cross the UI boundary through controlled, testable effect layers.

## Testing

Use the right mix of unit, snapshot, parser round-trip, type-check expectation, property, fuzz, scheduler stress, diagnostic regression, GTK integration, leak/drop/ownership, stack-depth, and malformed-input tests.

Every bug fix should state:
- which invariant failed,
- which test now locks it down,
- whether a missing abstraction caused it.

## Feature delivery workflow

A feature is not done until every affected artifact is updated. Use the conditional dependency graph below to determine which steps apply. Skip steps whose trigger condition does not match your change.

### Artifact inventory

| Artifact | Location | Trigger |
| --- | --- | --- |
| Parser / CST | `crates/aivi-syntax/` | New syntax forms, keywords, operators |
| HIR / name resolution | `crates/aivi-hir/` | New declarations, type forms, elaboration |
| Type system | `crates/aivi-typing/` | New type rules, kinds, constraints |
| Core desugaring | `crates/aivi-core/` | New core IR forms, lowering rules |
| Lambda IR | `crates/aivi-lambda/` | Closure/lambda changes |
| Backend / codegen | `crates/aivi-backend/` | Cranelift emission changes |
| Runtime | `crates/aivi-runtime/` | Signal engine, scheduler, sources, GC |
| GTK bridge | `crates/aivi-gtk/` | Widget catalog, event routing, markup |
| Query layer | `crates/aivi-query/` | Salsa database, incremental queries |
| LSP server | `crates/aivi-lsp/` | Diagnostics, completion, hover, semantic tokens, formatting, go-to-def, symbols, code lens |
| CLI | `crates/aivi-cli/` | Commands: `check`, `run`, `execute`, `compile`, `build`, `test`, `fmt`, `lsp`, `mcp`, `manual-snippets` |
| MCP server | `crates/aivi-cli/src/mcp.rs` | Live app introspection tools (signals, sources, GTK tree, events) |
| VSCode extension | `tooling/packages/vscode-aivi/` | LSP client, commands, configuration |
| TextMate grammar | `tooling/packages/vscode-aivi/syntaxes/aivi.tmLanguage.json` | New keywords, operators, syntax patterns |
| Snippets | `tooling/packages/vscode-aivi/snippets/aivi.json` | New common declaration or expression patterns |
| Manual (guide) | `manual/guide/*.md` | Feature documentation, examples |
| Manual (stdlib) | `manual/stdlib/*.md` | Standard library API reference |
| Stdlib | `stdlib/` | Standard library `.aivi` source files |
| Fixtures | `fixtures/frontend/` | Frontend pipeline test fixtures |
| Surface feature matrix | `manual/guide/surface-feature-matrix.md` | Implementation status truth table |

### Dependency chains by change category

#### Core language change (syntax, types, semantics)

1. Implement in the affected compiler crate(s) — only touch layers the change requires: parser → HIR → typing → core → lambda → backend.
2. Add or update tests in the affected crate(s) (unit, snapshot, expectation).
3. If new syntax form or keyword: update TextMate grammar (`syntaxes/aivi.tmLanguage.json`).
4. If new syntax form or keyword: update LSP semantic tokens (`crates/aivi-lsp/src/semantic_tokens.rs`).
5. If new completable form: update LSP completion (`crates/aivi-lsp/src/completion.rs`).
6. If new hoverable form: update LSP hover (`crates/aivi-lsp/src/hover.rs`).
7. If commonly used pattern: add VSCode snippet (`snippets/aivi.json`).
8. Run `aivi manual-snippets --root manual` to verify and fix all manual code blocks.
9. Update the relevant `manual/guide/` page(s).
10. Update `manual/guide/surface-feature-matrix.md` status columns.

#### Runtime or source provider change

1. Implement in `crates/aivi-runtime/`.
2. Add runtime tests (scheduler stress, signal propagation, source lifecycle).
3. If new source provider: update `manual/guide/source-catalog.md`.
4. If new source provider: check whether MCP `list_sources` / `set_source_mode` / `publish_source_value` schemas need updates in `crates/aivi-cli/src/mcp.rs`.
5. If source changes affect live introspection: verify MCP tool behavior end to end.
6. Run `aivi manual-snippets --root manual`.
7. Update `manual/guide/surface-feature-matrix.md`.

#### GTK or widget catalog change

1. Implement in `crates/aivi-gtk/`.
2. Add GTK integration tests.
3. If new widget or event signal: verify MCP `snapshot_gtk_tree` / `find_widgets` / `emit_gtk_event` output reflects the change.
4. If new event signal type: update `manual/guide/markup.md`.
5. Run `aivi manual-snippets --root manual`.
6. Update `manual/guide/surface-feature-matrix.md`.

#### Stdlib change

1. Implement in `stdlib/*.aivi`.
2. Verify via `crates/aivi-cli/tests/check.rs` or backend foundation tests.
3. Update `manual/stdlib/<module>.md`.
4. Run `aivi manual-snippets --root manual` to verify stdlib doc code blocks.
5. If new common pattern: add VSCode snippet.

#### LSP-only change (diagnostics, code actions, new capabilities)

1. Implement in `crates/aivi-lsp/`.
2. Add LSP tests (`crates/aivi-lsp/tests/lsp_*.rs`).
3. If new LSP capability: update VSCode extension client options or commands (`tooling/packages/vscode-aivi/src/`).
4. If new command: register in `commands.ts` and add to `package.json` `contributes.commands`.

#### Documentation-only change

1. Edit the relevant `manual/guide/*.md` or `manual/stdlib/*.md`.
2. Run `aivi manual-snippets --root manual` to verify all AIVI code blocks parse and check cleanly.
3. Fix any broken snippets reported by the tool.

### Verification checkpoints

After every feature delivery, confirm:

```sh
# Build the compiler
cargo build --bin aivi

# Test affected crates (substitute the crates you touched)
cargo test -p <affected-crates>

# Verify all manual code blocks parse/check
./tooling/check-manual-aivi-snippets.sh
```

If the LSP was changed:

```sh
cargo test -p aivi-lsp
```

If the VSCode extension was changed:

```sh
cd tooling && pnpm install && pnpm -F vscode-aivi build
```

## How to work

For non-trivial work:
1. identify subsystem and invariants,
2. state the architecture decision before patching,
3. implement the full change across affected layers,
4. add or update tests,
5. report what changed, what was validated, and what remains.

Fix nearby in-scope issues. Remove obsolete code, comments, branches, and unused helpers.

## Done means

Do not stop at a superficially working patch.

Done requires:
- requested behavior works end to end,
- affected paths and call sites are updated consistently,
- implied edge cases are handled,
- validation is proportionate,
- no stubs, TODOs, fake behavior, brittle special cases, or incomplete refactors remain unless explicitly requested,
- result is production-worthy and architecture-aligned.

If blocked, state the exact blocker and mark the work incomplete. Never present partial work as finished.

## Wiki

`wiki/` is a persistent, LLM-maintained knowledge base for this project.

**Raw sources** (read-only, never modify):
- `src/`, `crates/` — Rust source code
- `stdlib/` — AIVI standard library source
- `manual/` — human-facing documentation
- `syntax.md`, `AIVI_RFC.md`, `AGENTS.md` — spec and schema

**The wiki** (`wiki/`): LLM-generated markdown. You write it; humans read it.

### Conventions

- `wiki/index.md` — catalog: every page listed with a link and one-line summary, organised by category.
- `wiki/log.md` — append-only activity log. Each entry: `## [YYYY-MM-DD] <verb> | <title>`. Never edit past entries.
- All other pages are topic files. Name them `kebab-case.md`.
- Use relative links between pages.
- Pages may include AIVI code blocks but these are illustrative, not checked by `manual-snippets`.

### Operations

**Ingest** (when you read a source file or a new raw document):
1. Extract key facts from the source.
2. Write or update the relevant wiki page(s).
3. Update `wiki/index.md` if a new page was created.
4. Append an entry to `wiki/log.md`: `## [date] ingest | <source name>`.

**Query** (when answering a question):
1. Read `wiki/index.md` to identify relevant pages.
2. Read those pages.
3. Synthesise an answer with citations to wiki pages and raw sources.
4. If the answer is valuable and reusable, file it as a new wiki page and update the index.
5. Append an entry to `wiki/log.md`: `## [date] query | <question summary>`.

**Lint** (periodically or on request):
1. Scan for orphan pages (not linked from `index.md`).
2. Flag stale claims superseded by newer source changes.
3. Flag important concepts mentioned but lacking their own page.
4. Append: `## [date] lint | wiki health check`.

### Current pages

| Page | Topic |
|------|-------|
| [architecture.md](wiki/architecture.md) | Crate map, pipeline overview, key invariants |
| [compiler-pipeline.md](wiki/compiler-pipeline.md) | CST → HIR → Core → Lambda → Backend stages |
| [type-system.md](wiki/type-system.md) | Kinds, type classes, Eq, HKT, constraints |
| [signal-model.md](wiki/signal-model.md) | Signals, sources, merge syntax, fanout, recurrence |
| [runtime.md](wiki/runtime.md) | Scheduler, signal graph, task executor, GLib integration |
| [gtk-bridge.md](wiki/gtk-bridge.md) | GTK4/libadwaita widget bridge, markup lowering |
| [query-layer.md](wiki/query-layer.md) | Incremental query layer, workspace, formatter safety |
| [lsp-server.md](wiki/lsp-server.md) | LSP server, navigation, diagnostics, VS Code extension |
| [cli.md](wiki/cli.md) | CLI commands, MCP server, build & test commands |
| [stdlib.md](wiki/stdlib.md) | Standard library module overview |
