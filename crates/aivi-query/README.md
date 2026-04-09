# aivi-query

## Purpose

Incremental query database for AIVI tooling — workspace discovery, file parsing and HIR queries,
LSP backing store, and first typed backend-unit queries for runtime lowering. `aivi-query`
memoises parse/HIR results per file revision so that editor-facing features (diagnostics, symbols,
completions, formatting) avoid re-parsing unchanged files, and it now provides stable
whole-program/runtime-fragment backend fingerprints that later JIT/cache layers can key on. It is
the single source of truth for file-to-module mapping and import resolution in the tooling layer.

## Entry points

```rust
// Central incremental database
RootDatabase::new() -> RootDatabase
RootDatabase::set_file_text(file: SourceFile, text: Arc<str>)
RootDatabase::invalidate(file: SourceFile)

// File / module queries
parsed_file(db: &RootDatabase, file: SourceFile) -> ParsedFileResult
hir_module(db: &RootDatabase, file: SourceFile) -> HirModuleResult
resolve_module_file(db: &RootDatabase, path: &Path) -> Option<SourceFile>
exported_names(db: &RootDatabase, file: SourceFile) -> ExportedNames
reachable_workspace_hir_modules(db: &RootDatabase, file: SourceFile) -> Arc<[WorkspaceHirModule]>

// Diagnostics and symbols
all_diagnostics(db: &RootDatabase, file: SourceFile) -> Vec<Diagnostic>
symbol_index(db: &RootDatabase, file: SourceFile) -> Vec<LspSymbol>
format_file(db: &RootDatabase, file: SourceFile) -> Option<String>

// Typed backend-unit queries
whole_program_backend_unit(db: &RootDatabase, file: SourceFile) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError>
whole_program_backend_unit_with_items(db: &RootDatabase, file: SourceFile, included_items: &IncludedItems) -> Result<Arc<WholeProgramBackendUnit>, BackendUnitError>
runtime_fragment_backend_unit(db: &RootDatabase, file: SourceFile, fragment: &RuntimeFragmentSpec) -> Result<Arc<RuntimeFragmentBackendUnit>, BackendUnitError>
whole_program_backend_fingerprint(db: &RootDatabase, file: SourceFile) -> Result<WholeProgramFingerprint, BackendUnitError>
runtime_fragment_backend_fingerprint(db: &RootDatabase, file: SourceFile, fragment: &RuntimeFragmentSpec) -> Result<RuntimeFragmentFingerprint, BackendUnitError>

// Workspace discovery
discover_workspace_root(path: &Path) -> Option<PathBuf>
discover_workspace_root_from_directory(dir: &Path) -> Option<PathBuf>

// Entrypoint resolution
resolve_v1_entrypoint(db: &RootDatabase, path: &Path) -> Result<ResolvedEntrypoint, EntrypointResolutionError>
```

## Invariants

- `RootDatabase` is `Send + Sync`; it uses `parking_lot` read-write locks for internal caches.
- Queries are memoised by file content hash; `set_file_text` invalidates all cached results for that file and any file that transitively imports it.
- `parsed_file` and `hir_module` never panic; errors are carried inside `ParsedFileResult` / `HirModuleResult`.
- File-to-module mapping is deterministic: the same path always resolves to the same `SourceFile` within a database lifetime.
- `all_diagnostics` aggregates parse and HIR diagnostics; it does not itself run backend or runtime passes.
- Whole-program and runtime-fragment backend queries cache successful and failed lowering results
  against the current HIR snapshot identity, so transitive import invalidation naturally evicts
  stale backend units without guessing a global revision counter.

## Diagnostic codes

This crate emits no `DiagnosticCode` values of its own. It surfaces diagnostics produced by
`aivi-syntax` and `aivi-hir`.

## RFC reference

See [`../../AIVI_RFC.md`](../../AIVI_RFC.md) §26 (incremental query database and tooling).
