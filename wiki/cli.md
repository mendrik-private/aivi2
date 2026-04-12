# CLI

The `aivi` binary provides the developer-facing command-line interface.

## Source

`crates/aivi-cli/src/main.rs`, `mcp.rs`, `manual_snippets.rs`, `run_session.rs`

## Commands

| Command | Description |
|---------|-------------|
| `aivi check <file>` | Type-check a file and report diagnostics |
| `aivi run <file>` | Run an AIVI application (GTK app with live runtime) |
| `aivi execute <expr>` | Execute an expression and print the result |
| `aivi compile <file>` | Compile to native object code; not yet a linked runnable app |
| `aivi build` | Package the current runtime binary, stdlib, and reachable workspace files into a runnable bundle |
| `aivi test` | Run AIVI test files |
| `aivi fmt <file>` | Format a source file (idempotent) |
| `aivi lsp` | Start the LSP server on stdio |
| `aivi mcp` | Start the MCP server for live app introspection |
| `aivi manual-snippets --root <dir>` | Verify all AIVI code blocks in manual pages parse and check cleanly |

## MCP Server

**Source**: `mcp.rs`

The MCP (Model Context Protocol) server exposes live app introspection tools for LLM agents:

| Tool | Description |
|------|-------------|
| `list_signals` | List live runtime signals with IDs, values, generations, dependencies |
| `get_signal` | Fetch one signal by ID or name |
| `assert_signal` | Assert a signal equals an expected value |
| `list_sources` | List live source instances and their modes |
| `set_source_mode` | Switch a source between live and manual modes |
| `publish_source_value` | Inject a value into a source (enters manual mode) |
| `snapshot_gtk_tree` | Capture the live GTK widget tree semantically |
| `find_widgets` | Search the GTK snapshot for widgets by role, text, focus, or actionability |
| `emit_gtk_event` | Emulate a GTK interaction (click, set_text, key press, etc.) |
| `check_workspace` | Run a full HIR check and return structured diagnostics |
| `list_diagnostics` | List diagnostics for a single file |
| `read_source_file` | Read source file content |
| `get_type_at` | Get type info for the symbol at a position |
| `launch_app` | Launch the configured app |
| `restart_app` | Restart the configured app |
| `stop_app` | Stop the current app session |
| `session_status` | Inspect app/session lifecycle and hydration state |

The MCP server uses `prepare_run_artifact` → `compile_run_expr_fragment` → `lower_runtime_fragment` for markup expression compilation.

Install: `cargo install --path crates/aivi-cli`

## Manual Snippets

**Source**: `manual_snippets.rs`

`aivi manual-snippets --root manual` walks all `manual/guide/*.md` and `manual/stdlib/*.md` files, extracts fenced AIVI code blocks, and verifies they parse and type-check cleanly. Must be run after any language change that touches the manual.

Script alias: `./tooling/check-manual-aivi-snippets.sh`

## Build & Test

```sh
# Build the compiler
cargo build --bin aivi

# Test affected crates
cargo test -p aivi-syntax -p aivi-hir -p aivi-query -p aivi-lsp

# Verify all manual code blocks
./tooling/check-manual-aivi-snippets.sh
```

Pre-existing known failures:
- `aivi-core`: `snapshot_core_func_module`, `snapshot_core_value_module`
- `aivi-backend`: `workspace_imported_builtin_class_members_lower_through_backend_runtime`
- `aivi-runtime`: `dbus_method_source_replies_with_configured_body` (flaky GLib threading), `linked_runtime_executes_signal_fanout_map_and_join_pipelines` (layout mismatch)

## Execution boundary

- `aivi compile` lowers through Cranelift and can emit an object file, but it stops before runtime
  startup / final app linking.
- `aivi build` is the current runnable packaging path. It validates the same runnable surface as
  `aivi run`, then assembles a bundle from the runtime binary, bundled stdlib, and reachable
  workspace sources.
- Backend execution can still attach compiled object artifacts while constructing a lazy-JIT engine,
  so object emission and runtime execution currently coexist rather than replacing each other.

*See also: [lsp-server.md](lsp-server.md), [architecture.md](architecture.md)*
