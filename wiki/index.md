# AIVI Wiki — Index

A persistent, LLM-maintained knowledge base for the AIVI compiler project.  
**Raw sources**: `src/`, `crates/`, `stdlib/`, `manual/`, `syntax.md`, `AIVI_RFC.md`  
**LLM writes this layer. Humans read it.**

---

## Architecture & Design

| Page | Summary |
|------|---------|
| [architecture.md](architecture.md) | High-level system overview: layers, crates, data flow |
| [compiler-pipeline.md](compiler-pipeline.md) | CST → HIR → Core → Lambda → Backend → Cranelift codegen |
| [type-system.md](type-system.md) | Types, kinds, HKT, type classes, Eq derivation, constraints |
| [signal-model.md](signal-model.md) | Reactive signals, sources, merge syntax, signal graph |
| [runtime.md](runtime.md) | Scheduler, signal graph execution, task executor, GC |
| [gtk-bridge.md](gtk-bridge.md) | GTK4/libadwaita widget bridge, markup lowering, event routing |
| [query-layer.md](query-layer.md) | Incremental Salsa-style query layer, workspace, memoisation |
| [lsp-server.md](lsp-server.md) | Language Server: diagnostics, completion, hover, navigation |
| [cli.md](cli.md) | CLI commands: check, run, compile, fmt, lsp, mcp, openapi-gen |
| [stdlib.md](stdlib.md) | Standard library modules overview |
| [openapi-source.md](openapi-source.md) | OpenAPI capability handle: `@source api`, codegen, auth |

| [demo-audit.md](demo-audit.md) | Snake & Reversi audit — issues found and fixed |

## Log

See [log.md](log.md) for a chronological record of wiki activity.

---

*Last updated: 2026-04-06*
