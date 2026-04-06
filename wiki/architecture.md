# AIVI Architecture

AIVI is a purely functional, reactive, GTK/libadwaita-first programming language compiled to native code via Cranelift. It has no `if/else` or loops in surface syntax — control flow is expressed through pipes, pattern matching, and signal merge.

## Crate Map

```
src/main.rs                  ← binary entry point (delegates to aivi-cli)
crates/
  aivi-syntax/               ← lexer, CST, parser, formatter
  aivi-hir/                  ← name resolution, HIR, elaboration, type checking
  aivi-typing/               ← kind checking, Eq/Decode/Fanout/Gate derivation plans
  aivi-core/                 ← post-HIR typed-core IR, lowering from HIR reports
  aivi-lambda/               ← closure/lambda IR between core and backend
  aivi-backend/              ← Cranelift codegen, layout, program, GC integration
  aivi-runtime/              ← scheduler, signal graph, source providers, task executor
  aivi-gtk/                  ← GTK4/libadwaita widget bridge, markup lowering
  aivi-query/                ← incremental query layer (Salsa-style), workspace
  aivi-lsp/                  ← Language Server Protocol server
  aivi-base/                 ← shared: diagnostics, arenas, source spans, rendering
  aivi-cli/                  ← CLI commands: check, run, fmt, lsp, mcp
```

## Compilation Pipeline

```
Source text
    │ aivi-syntax: lex → parse → CST
    ▼
Concrete Syntax Tree (CST)
    │ aivi-hir: name resolution, elaboration, type checking
    ▼
HIR + elaboration reports
    │ aivi-core: lower_module()
    ▼
Typed Core IR
    │ aivi-lambda: lower_module()
    ▼
Lambda IR (closures explicit)
    │ aivi-backend: codegen via Cranelift
    ▼
Native binary / JIT
```

## Reactive Runtime

At runtime, the scheduler owns a `SignalGraph` of input/derived/signal/reactive-clause nodes. Source providers run on worker threads and publish immutable values into scheduler queues. The scheduler processes ticks topologically, batching signal propagation glitch-free.

```
Source Provider (worker thread)
    │ publishes via WorkerPublicationSender
    ▼
Scheduler queue
    │ batch tick: topological order, no glitches
    ▼
SignalGraph: InputHandle → DerivedHandle → SignalHandle → ReactiveClauseHandle
    │ GTK bridge reads reactive-clause outputs
    ▼
GTK widget tree (main thread only)
```

## Key Invariants

- **No GTK work on worker threads.** Widget creation, mutation, and event dispatch stay on the GLib main context.
- **No shared mutable state between workers.** Workers publish immutable messages into scheduler-owned queues.
- **Signal propagation is transactional.** One scheduler tick is a glitch-free atomic batch.
- **`unsafe` is minimised.** All crates use `#![forbid(unsafe_code)]` except the backend and runtime where FFI requires it.
- **All IR layers define:** ownership model, identity strategy, span mapping, validation rules, debug/pretty-print form, test fixtures.

## Source of Truth Hierarchy

1. `AGENTS.md` — agent behavioural schema
2. `AIVI_RFC.md` — language spec (authoritative)
3. `syntax.md` — surface grammar
4. `manual/guide/` — human-facing documentation
5. `crates/` — implementation

*See also: [compiler-pipeline.md](compiler-pipeline.md), [runtime.md](runtime.md), [signal-model.md](signal-model.md)*
