# AIVI Wiki — Index

A persistent, LLM-maintained knowledge base for the AIVI compiler project.  
**Raw sources**: `src/`, `crates/`, `stdlib/`, `manual/`, `syntax.md`, `AIVI_RFC.md`  
**LLM writes this layer. Humans read it.**

---

## Architecture & Design

| Page | Summary |
|------|---------|
| [architecture.md](architecture.md) | High-level system overview: layers, crates, data flow |
| [anonymous-lambdas.md](anonymous-lambdas.md) | Expression lambda surface, shorthand boundary, and hoisting model |
| [compiler-pipeline.md](compiler-pipeline.md) | CST → HIR → Core → Lambda → Backend → Cranelift codegen |
| [type-system.md](type-system.md) | Types, kinds, HKT, type classes, Eq derivation, constraints |
| [equality-semantics.md](equality-semantics.md) | Concrete structural Eq, generic constraints, and why demos still use comparator helpers |
| [signal-model.md](signal-model.md) | Reactive signals, sources, merge syntax, signal graph |
| [pipe-algebra.md](pipe-algebra.md) | Pipe operators, `#name` memos, grouped branch behavior, cluster boundary |
| [surface-syntax.md](surface-syntax.md) | Audit summary of recent main-branch surface syntax additions and where they are documented |
| [temporal-design.md](temporal-design.md) | Tradeoff between source-shaped and recurrence-shaped temporal scheduling |
| [runtime.md](runtime.md) | Scheduler, signal graph execution, task executor, GC |
| [gtk-bridge.md](gtk-bridge.md) | GTK4/libadwaita widget bridge, markup lowering, event routing |
| [query-layer.md](query-layer.md) | Incremental Salsa-style query layer, workspace, memoisation |
| [lsp-server.md](lsp-server.md) | Language Server: diagnostics, completion, hover, navigation |
| [cli.md](cli.md) | CLI commands: check, run, compile, fmt, lsp, mcp, openapi-gen |
| [stdlib.md](stdlib.md) | Standard library modules overview |
| [indexed-collections.md](indexed-collections.md) | Indexed list/matrix ergonomics, implemented ADT companion bodies, and deferred indexed-HKT work |
| [openapi-source.md](openapi-source.md) | OpenAPI capability handle: `@source api`, codegen, auth |

| [demo-audit.md](demo-audit.md) | Snake & Reversi audit — issues found and fixed |
| [manual-hallucination-report.md](manual-hallucination-report.md) | Hallucination audit of 78 manual files: 7 critical, 5 high, 3 medium, 2 low findings |

## Log

See [log.md](log.md) for a chronological record of wiki activity.

---

*Last updated: 2026-04-10*
