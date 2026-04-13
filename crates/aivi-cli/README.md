# aivi-cli

## Purpose

The `aivi` command-line binary — the primary developer-facing entry point for the AIVI compiler
and toolchain. `aivi-cli` wires together all compiler layers (`aivi-syntax`, `aivi-hir`,
`aivi-core`, `aivi-lambda`, `aivi-backend`, `aivi-runtime`, `aivi-gtk`, `aivi-query`, `aivi-lsp`)
into end-to-end subcommands and manages the application lifecycle: compilation, execution, the
GTK event loop, and tooling servers.

## Entry points

```rust
// Binary entry point
fn main() -> ExitCode
```

Subcommands dispatched from `main`:

| Subcommand | Action |
|---|---|
| `check` | Parse + HIR-check one or more source files; emit diagnostics |
| `compile` | Compile a source file through the full pipeline to object code |
| `build` | Full bundle: compile + link + package a runnable application |
| `run` | Build and immediately execute the compiled application |
| `execute` | Interpret a source file through the HIR/runtime path (no codegen) |
| `test` | Discover and run `@test`-decorated declarations |
| `fmt` | Format source files in-place using the canonical formatter |
| `mcp` | Start the Model Context Protocol server (for AI tooling integration) |

Internal modules:

| Module | Purpose |
|---|---|
| `manual_snippets` | Built-in manual/help text snippets |
| `mcp` | MCP server implementation |
| `run_session` | Shared session logic for `run` / `execute` subcommands |

## Invariants

- The GTK main loop runs on the process main thread; all widget operations are dispatched there.
- Worker threads communicate with the scheduler exclusively via message passing; no shared mutable state crosses the thread boundary.
- `execute` and `run` share a common session abstraction (`run_session`) to avoid duplicating GTK + scheduler setup.
- Subcommand dispatch is argument-position based (first argument), not flag-based, to keep the CLI surface minimal.
- `compile` stops at object emission; `build` performs the runnable source-free bundle path by writing the runtime binary, serialized run artifact, serialized backend metadata payloads, precompiled native-kernel sidecars, and launcher.
- Exit codes follow Unix conventions: 0 for success, non-zero for any error.

## Diagnostic codes

This crate emits no `DiagnosticCode` values of its own. Diagnostics from all upstream crates are
collected and rendered to stderr by the CLI's reporting layer.

## RFC reference

See [`../../AIVI_RFC.md`](../../AIVI_RFC.md) §26 (CLI interface and tooling).
