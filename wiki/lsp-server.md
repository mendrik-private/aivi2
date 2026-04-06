# LSP Server

The AIVI Language Server implements the Language Server Protocol over stdio, providing IDE features for `.aivi` files.

## Overview

**Source**: `crates/aivi-lsp/src/`

```
Editor (VS Code, Neovim, etc.)
    │ LSP over stdio (JSON-RPC)
    ▼
aivi-lsp: tower_lsp Server
    │
    ├── state.rs      — server state, RootDatabase, open documents
    ├── documents.rs  — document lifecycle (open, change, close, sync)
    ├── diagnostics.rs — pull/push diagnostics
    ├── completion.rs  — completions
    ├── hover.rs       — hover documentation
    ├── definition.rs  — go-to-definition
    ├── references.rs  — find references
    ├── rename.rs      — symbol rename
    ├── formatting.rs  — document formatting
    ├── symbols.rs     — workspace/document symbols
    ├── semantic_tokens.rs — semantic token highlighting
    ├── inlay_hints.rs — inlay type hints
    ├── code_actions.rs — code actions
    ├── code_lens.rs   — code lens
    ├── implementation.rs — go-to-implementation
    ├── navigation.rs  — shared navigation helpers
    ├── analysis.rs    — cross-cutting analysis
    └── unused.rs      — unused symbol detection
```

## Server State

**Source**: `state.rs`

`Backend` (the tower_lsp service) holds:
- `RootDatabase` — the query layer database
- Open document map (path → current text + revision)
- Workspace configuration

## Navigation

**Source**: `navigation.rs`

`NavigationAnalysis` is the core cross-cutting analysis used by definition, references, hover, and rename:

- `all_reference_locations_for_targets()` — walks `collect_all_sites()` to find all reference locations for a set of targets
- `NavigationTarget::find_symbol_at_target()` — resolves hover/definition from a reference site

Used by: definition, references, hover, rename.

## Diagnostics

**Source**: `diagnostics.rs`

Pulls diagnostics from `aivi-query::all_diagnostics()` and maps them to LSP `Diagnostic` objects.

Unused-symbol warnings are generated separately by `collect_unused_native_diagnostics()` (from `unused.rs`) — only when the module has no HIR errors.

## Completion

**Source**: `completion.rs`

Provides completions for:
- Local bindings in scope
- Module-level names
- Class members after `.`
- Import path segments
- New completable language forms (updated per feature delivery)

## Hover

**Source**: `hover.rs`

Returns type signatures and doc comments for:
- Value and function declarations
- Type names
- Class members
- Import paths

## Formatting

**Source**: `formatting.rs`

Calls `format_file()` from the query layer. Returns `None` (no edits) if the file has parse errors — **prevents code deletion on save**.

## Semantic Tokens

**Source**: `semantic_tokens.rs`

Provides semantic token classifications for syntax highlighting:
- Keywords, operators, types, functions, values, parameters, string literals, comments
- Updated when new keywords or operators are added to the language

## Inlay Hints

**Source**: `inlay_hints.rs`

Type annotation hints for unannotated bindings.

## Code Actions

**Source**: `code_actions.rs`

Quick fixes and refactors triggered by diagnostics or explicit request.

## Unused Symbols

**Source**: `unused.rs`

`collect_unused_native_diagnostics()` analyses the HIR for unreferenced declarations and emits `Diagnostic::warning` items. Only runs on modules with no HIR errors to avoid false positives.

## VS Code Extension

**Source**: `tooling/packages/vscode-aivi/`

The VS Code extension is the LSP client:
- `src/extension.ts` — activates the language client
- `package.json` — contributes language, commands, configuration
- `syntaxes/aivi.tmLanguage.json` — TextMate grammar for syntax highlighting
- `snippets/aivi.json` — code snippets

Build: `cd tooling && pnpm install && pnpm -F vscode-aivi build`

*See also: [query-layer.md](query-layer.md), [compiler-pipeline.md](compiler-pipeline.md)*
