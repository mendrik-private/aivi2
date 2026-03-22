# AIVI Tooling: Overview

## Components

Three packages live under `tooling/` as a pnpm workspace:

| Package               | Plan file              | Purpose                                   |
|-----------------------|------------------------|-------------------------------------------|
| `lsp-server`          | `01-lsp-server.md`     | LSP server (TypeScript, drives Rust binary)|
| `formatter`           | `02-formatter.md`      | Formatter bridge (TypeScript, thin wrapper)|
| `vscode-aivi`         | `03-vscode-extension.md`| VSCode extension (TextMate + LSP client)  |

Cross-cutting feature plans:

| Topic                          | Plan file                      |
|--------------------------------|--------------------------------|
| Rich hover + cross-module nav  | `04-hover-and-navigation.md`   |
| Stdlib documentation system    | `05-stdlib-docs.md`            |
| Incremental computation (salsa)| `06-incremental-computation.md`|

`plan/06` is a hard dependency of `plan/01`. The LSP server is built on the salsa query
database from day one. The naive "re-run everything" approach is not in the plan.

## Architecture in one picture

```
VSCode / any LSP editor
        │
        │  JSON-RPC (stdio / IPC)
        ▼
  lsp-server (TypeScript)
        │
        │  spawn subprocess per request
        ▼
  aivi binary (Rust)
   ├── aivi lsp-check     → diagnostics + symbol index
   └── aivi lsp          → LSP server (long-running, JSON-RPC over stdio)
                            (formatter is also part of the binary: aivi fmt)
```

**Important:** Both the LSP server and the formatter are implemented entirely in Rust
inside the existing `aivi` binary. There is no separate Node.js/TypeScript process for
language intelligence. TypeScript exists only in the VSCode extension package (the LSP
client side). See `01-lsp-server.md` and `02-formatter.md` for the revised architecture.

```
VSCode / any LSP editor
        │
        │  JSON-RPC over stdio
        ▼
  aivi lsp  (Rust, long-running process)
   ├── full LSP server implementation (tower-lsp / lsp-types)
   ├── drives the same parser/HIR/typing pipeline as aivi-cli
   └── exposes all LSP methods: hover, completion, definition, ...

  aivi fmt [file...]        format in-place
  aivi fmt --check          exit 1 if any file would change
  aivi fmt --stdin          read from stdin, write to stdout
```

## Prerequisites in the Rust workspace

1. Span → `(line, col)` conversion in `aivi-base::Source`.
2. `ImportResolver` trait in `aivi-hir` (needed by salsa cross-file queries).
3. `aivi_hir::exports(module) -> ExportedNames` + `ExportedNames: PartialEq`.
4. `aivi_typing::TypedModule::type_of_node(offset)` for hover and inlay hints.
5. `aivi fmt --stdin` and `aivi fmt --check` (formatter completeness per `02-formatter.md §4`).

## Implementation order

```
Phase 1: aivi-query crate (salsa backbone)
  RootDatabase + SourceFile / WorkspaceFiles inputs
  parsed_file, hir_module, typed_module, symbol_index queries
  all_diagnostics aggregation
  stdlib pre-warming at initialize
  (plan/06 M1–M5)

Phase 2: Rust LSP skeleton
  aivi lsp command: tower-lsp, stdio transport
  ServerState backed by RootDatabase (not a manual AnalysisResult cache)
  textDocument/didOpen + didChange → salsa input mutation + debounced diagnostics
  aivi fmt --stdin

Phase 3: Core navigation
  textDocument/documentSymbol + workspace/symbol
  textDocument/definition (in-file)
  textDocument/references (in-file)
  textDocument/hover (signature only)

Phase 3: Rich intelligence
  semanticTokens/full + delta
  textDocument/completion with snippets
  signatureHelp
  textDocument/inlayHint
  hover: type expansion tree + doc comments (plan/04, plan/05)

Phase 4: Editing features
  textDocument/codeAction (quick-fix, organize imports)
  textDocument/rename + prepareRename
  textDocument/formatting + rangeFormatting
  codeLens + callHierarchy + foldingRange

Phase 5: Cross-module + stdlib
  cross-file definition / references
  stdlib source indexing (aivi stdlib-path)
  aivi-docs.msgpack index (plan/05)
```

## TypeScript scope (VSCode extension only)

The `vscode-aivi` package (`03-vscode-extension.md`) remains TypeScript/Vite. It:
- starts `aivi lsp` as a child process and connects via `vscode-languageclient`
- contributes the TextMate grammar, language config, snippets, commands, status bar
- does **no** language intelligence of its own

## Line-width and style canon

All TypeScript is written in strict mode, ESM internally, bundled to CJS for Node.js/VSCode compatibility. No framework beyond `vscode-languageserver` for the LSP server and `vscode-languageclient` for the extension. Formatting: @biomejs/biome default config. Tests: Vitest. pnpm instead of npm.

## Open questions

1. **Rust LSP protocol crate**: should this be a standalone `aivi-lsp-protocol` crate or inline types in `aivi-cli`? Recommend standalone for testability.
2. **Binary distribution**: the VSIX does not bundle the `aivi` binary. A future companion installer script or distro package is needed.
3. **Multi-file type checking**: the current Rust workspace does not yet have cross-file import resolution. Until that exists, `lsp-refs` and workspace symbols are limited to single-file scope.
4. **Debug adapter**: a DAP implementation for step-through debugging is out of scope for v1 but should be planned once the runtime exists.
