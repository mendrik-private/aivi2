# Query Layer

The query layer wraps all compiler stages in an incremental, memoised system. The LSP server, CLI, and MCP server all go through this layer rather than calling compiler stages directly.

## Overview

**Source**: `crates/aivi-query/src/`

```
Source file text (SourceFile)
    │ inputs.rs: register source text as durable input
    ▼
RootDatabase (db.rs)
    │ queries/: memoised query functions
    ├── parsed_file()    → ParsedFileResult
    ├── hir_module()     → HirModuleResult
    ├── format_file()    → Option<String>
    ├── symbol_index()   → SymbolIndex
    ├── exported_names() → ExportedNames
    └── resolve_module_file() → Option<PathBuf>
    ▼
Results + diagnostics (memoised per revision)
```

## Key Types

### `RootDatabase`

**Source**: `db.rs`

The central database. Holds:
- All registered `SourceFile` inputs
- Memoised query results keyed by `(file, revision)`
- Reverse dependency tracking for invalidation

### `SourceFile`

**Source**: `inputs.rs`

Represents a source file as a durable input:
- `path: PathBuf` — canonical file path
- `text: Arc<String>` — current source text
- `revision: u64` — incremented on each text change

### Queries

**Source**: `queries/`

| Query | Returns | Notes |
|-------|---------|-------|
| `parsed_file(db, file)` | `ParsedFileResult` | CST + parse diagnostics, memoised |
| `hir_module(db, file)` | `HirModuleResult` | HIR + elaboration + type check diagnostics |
| `format_file(db, file)` | `Option<String>` | Formatted source; `None` if parse errors exist |
| `all_diagnostics(db, file)` | `Vec<Diagnostic>` | All diagnostics for a file |
| `symbol_index(db, file)` | `SymbolIndex` | All symbols for go-to-symbol |
| `exported_names(db, file)` | `ExportedNames` | Public names for cross-module resolution |
| `resolve_module_file(db, name)` | `Option<PathBuf>` | Module name → file path |

### Formatter Safety

`format_file()` returns `None` when the file has parse errors. The LSP `format_document` handler propagates this as "no formatting available" — **this prevents code deletion on save when there are syntax errors**.

**Source**: `queries/hir.rs:243-253`, `crates/aivi-lsp/src/formatting.rs`

## Workspace

**Source**: `workspace.rs`

`discover_workspace_root()` and `discover_workspace_root_from_directory()` locate the workspace root by searching for `aivi.toml` upward from a given path.

## Manifest

**Source**: `manifest.rs`

`AiviManifest` is the parsed `aivi.toml` manifest:
- `WorkspaceConfig` — workspace-level settings
- `AppConfig` — application entry point and metadata
- `RunConfig` — runtime configuration

`parse_manifest()` deserialises an `aivi.toml` file.

## Entrypoint Resolution

**Source**: `entry.rs`

`resolve_v1_entrypoint()` resolves the application entrypoint from a manifest or explicit path:
- Returns `ResolvedEntrypoint` with the main module file path
- `EntrypointResolutionError` covers missing files, ambiguous entries, etc.

## Invalidation Model

The query layer tracks reverse dependencies: when a source file changes, all queries that transitively depend on it are invalidated. This is intentionally simple (per-file granularity) rather than fine-grained Salsa-style — typed queries are marked as future work in the crate doc comment.

*See also: [compiler-pipeline.md](compiler-pipeline.md), [lsp-server.md](lsp-server.md)*
