# Plan: Incremental Computation with Salsa

## Status: design draft — must be implemented before cross-file type checking lands

---

## 1. Why this must be designed upfront

A naive LSP pipeline re-runs parse → HIR → type check on every keystroke (debounced).
For a single self-contained file this is fast. The moment cross-file imports exist it
becomes untenable: changing one type in `core/types.aivi` would force re-checking every
file that transitively imports it.

Retrofitting incremental computation onto a working compiler is painful. The query
boundaries and ownership model must be established before the cross-file layer is built,
not after. This plan establishes that backbone.

The approach: adopt **`salsa`** as the incremental computation framework.
Salsa is the same system that powers rust-analyzer. It provides:

- demand-driven, memoised query evaluation
- automatic fine-grained dependency tracking (no manual cache keys)
- minimal re-computation on change (only queries whose inputs actually changed)
- cycle detection with user-defined recovery
- parallel query execution on a Rayon thread pool

The `aivi lsp` server IS the daemon — it is a long-running process that owns the salsa
database for the lifetime of the editor session. There is no separate daemon process.
A separate process would add IPC complexity and solve nothing that salsa does not already
solve more precisely.

---

## 2. New crate: `aivi-query`

```
crates/
├── aivi-base/          (existing)
├── aivi-syntax/        (existing)
├── aivi-hir/           (existing)
├── aivi-typing/        (existing)
├── aivi-query/         (new)   ← incremental query database
│   ├── src/
│   │   ├── lib.rs
│   │   ├── db.rs           # Database trait + storage
│   │   ├── inputs.rs       # #[salsa::input] structs
│   │   ├── queries/
│   │   │   ├── mod.rs
│   │   │   ├── source.rs   # source text → CST
│   │   │   ├── hir.rs      # CST → HIR
│   │   │   ├── typing.rs   # HIR → typed module
│   │   │   ├── symbols.rs  # HIR → symbol index
│   │   │   ├── tokens.rs   # CST + HIR → semantic token data
│   │   │   ├── docs.rs     # stdlib doc index queries
│   │   │   └── resolve.rs  # cross-file name resolution
│   │   └── diagnostics.rs  # accumulate diagnostics from all layers
│   └── Cargo.toml
└── aivi-lsp/           (existing, now depends on aivi-query)
```

`aivi-query` depends on all pipeline crates. The pipeline crates themselves
(`aivi-syntax`, `aivi-hir`, `aivi-typing`) are **not** modified to know about salsa —
they remain pure functions that take data in and return data out. Salsa wraps them.

---

## 3. Database definition

```rust
// aivi-query/src/db.rs

#[salsa::db]
pub trait AiviDb: salsa::Database {
    // Gives the database access to stdlib source (set once at startup)
    fn stdlib_root(&self) -> &Path;
}

/// The concrete runtime database used by the LSP server.
/// One instance per server session, shared via Arc<RwLock<_>> across tokio tasks.
#[salsa::db]
#[derive(Default)]
pub struct RootDatabase {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for RootDatabase {
    fn salsa_event(&self, event: &dyn Fn() -> salsa::Event) {
        // hook for tracing/metrics if needed
        let _ = event;
    }
}

#[salsa::db]
impl AiviDb for RootDatabase {
    fn stdlib_root(&self) -> &Path {
        // stored as a salsa input set once at initialize
        stdlib_root_path(self).as_path()
    }
}
```

---

## 4. Inputs

Inputs are the roots of the dependency graph. Salsa tracks everything that reads them.

```rust
// aivi-query/src/inputs.rs

/// The source text of one file. Set by the LSP server on open/change.
#[salsa::input]
pub struct SourceFile {
    pub path: PathBuf,
    #[return_ref]
    pub text: String,
}

/// The set of all known workspace files.
/// Updated when files are created/deleted/renamed.
#[salsa::input]
pub struct WorkspaceFiles {
    #[return_ref]
    pub paths: Vec<PathBuf>,
}

/// Path to the stdlib root. Set once at LSP initialize.
#[salsa::input]
pub fn stdlib_root_path(db: &dyn AiviDb) -> PathBuf;

/// Configuration from the LSP client (debounce ms, inlay hints enabled, etc.).
#[salsa::input]
pub struct LspConfig {
    pub debounce_ms:         u64,
    pub inlay_hints_enabled: bool,
    pub code_lens_enabled:   bool,
    pub expand_depth:        u32,   // hover type expansion depth (default 3)
}
```

Changing a `SourceFile`'s text automatically invalidates every query that read it,
transitively, across all files. No manual cache key management.

---

## 5. Query definitions

Each query is a pure function from `&dyn AiviDb` + inputs to an output.
Salsa memoises the output and re-runs the function only when its inputs change.

### 5.1 Layer 1 — Parsed CST

```rust
// aivi-query/src/queries/source.rs

/// Parse a source file into a lossless CST.
/// Depends only on: SourceFile.text
/// Cost: O(file size). Typically < 2 ms.
#[salsa::tracked]
pub fn parsed_file(db: &dyn AiviDb, file: SourceFile) -> ParsedFile {
    let text   = file.text(db);
    let source = aivi_base::Source::new(file.path(db), text);
    let cst    = aivi_syntax::parse(&source);
    ParsedFile::new(db, source, cst)
}

#[salsa::tracked]
pub struct ParsedFile {
    pub source: aivi_base::Source,
    pub cst:    aivi_syntax::Cst,
}
```

### 5.2 Layer 2 — HIR

```rust
// aivi-query/src/queries/hir.rs

/// Lower a parsed CST to HIR (name resolution, elaboration).
/// Depends on: parsed_file(file), plus parsed_file of every imported file.
/// Cost: O(file size + imported symbol count). Typically < 5 ms.
#[salsa::tracked]
pub fn hir_module(db: &dyn AiviDb, file: SourceFile) -> HirModule {
    let parsed = parsed_file(db, file);
    // resolve imports — each import triggers parsed_file + hir_module of the dep
    let imports = resolve_imports(db, file, &parsed.cst(db));
    let result  = aivi_hir::lower(&parsed.cst(db), &parsed.source(db), &imports);
    HirModule::new(db, result.module, result.diagnostics)
}

#[salsa::tracked]
pub struct HirModule {
    #[return_ref]
    pub module:      aivi_hir::Module,
    #[return_ref]
    pub diagnostics: Vec<aivi_base::Diagnostic>,
}
```

### 5.3 Layer 3 — Type checking

```rust
// aivi-query/src/queries/typing.rs

/// Type-check an HIR module.
/// Depends on: hir_module(file), plus hir_module of every imported file.
/// Cost: O(declarations * type complexity). Typically < 10 ms per file.
#[salsa::tracked]
pub fn typed_module(db: &dyn AiviDb, file: SourceFile) -> TypedModule {
    let hir    = hir_module(db, file);
    let result = aivi_typing::check(hir.module(db), db);
    TypedModule::new(db, result.module, result.diagnostics)
}

#[salsa::tracked]
pub struct TypedModule {
    #[return_ref]
    pub module:      aivi_typing::TypedModule,
    #[return_ref]
    pub diagnostics: Vec<aivi_base::Diagnostic>,
}
```

### 5.4 Layer 4 — Symbol index

```rust
// aivi-query/src/queries/symbols.rs

/// Extract the symbol index from HIR (does not require type checking).
/// Used by documentSymbol and workspace/symbol.
/// Cheap: just a structural walk of the HIR.
#[salsa::tracked]
pub fn symbol_index(db: &dyn AiviDb, file: SourceFile) -> SymbolIndex {
    let hir = hir_module(db, file);
    let idx = aivi_hir::extract_symbols(hir.module(db));
    SymbolIndex::new(db, idx)
}
```

### 5.5 Layer 5 — Semantic tokens

```rust
// aivi-query/src/queries/tokens.rs

/// Produce semantic token data for a file.
/// Depends on: parsed_file + hir_module + typed_module.
/// Re-runs only when any of those change.
#[salsa::tracked]
pub fn semantic_tokens(db: &dyn AiviDb, file: SourceFile) -> SemanticTokenData {
    let parsed = parsed_file(db, file);
    let hir    = hir_module(db, file);
    let typed  = typed_module(db, file);
    aivi_lsp::semantic_tokens::build(&parsed.cst(db), hir.module(db), typed.module(db))
}
```

### 5.6 Aggregated diagnostics

All diagnostic sources are merged in one query so the LSP server calls one query, not three:

```rust
// aivi-query/src/diagnostics.rs

/// All diagnostics for a file, from all pipeline layers.
#[salsa::tracked]
pub fn all_diagnostics(db: &dyn AiviDb, file: SourceFile) -> Vec<aivi_base::Diagnostic> {
    let mut diags = vec![];
    // parse errors come from the CST
    diags.extend(parsed_file(db, file).cst(db).errors().map(into_diag));
    // HIR diagnostics (name resolution, elaboration, source contracts, etc.)
    diags.extend(hir_module(db, file).diagnostics(db).iter().cloned());
    // type-check diagnostics
    diags.extend(typed_module(db, file).diagnostics(db).iter().cloned());
    diags.sort_by_key(|d| d.span.start);
    diags
}
```

### 5.7 Cross-file name resolution

```rust
// aivi-query/src/queries/resolve.rs

/// The set of exported names from a file.
/// Other files' hir_module queries call this when resolving imports.
#[salsa::tracked]
pub fn exported_names(db: &dyn AiviDb, file: SourceFile) -> ExportedNames {
    let hir = hir_module(db, file);
    aivi_hir::exports(hir.module(db))
}

/// Resolve a module path to a SourceFile.
/// Depends on: WorkspaceFiles + stdlib_root_path.
#[salsa::tracked]
pub fn resolve_module(db: &dyn AiviDb, path: ModulePath) -> Option<SourceFile> {
    // check workspace files first, then stdlib
    ...
}
```

This is the key cross-file query. When file A imports file B:
- `hir_module(A)` calls `exported_names(B)`
- salsa records that `hir_module(A)` depends on `exported_names(B)`
- when B's text changes, `exported_names(B)` is invalidated, which transitively
  invalidates `hir_module(A)`, `typed_module(A)`, `all_diagnostics(A)`, etc.
- salsa re-runs only those queries, not the entire workspace

---

## 6. Dependency invalidation model

```
                    ┌─────────────────────────────────────────────┐
                    │  user edits types.aivi                      │
                    └──────────────────┬──────────────────────────┘
                                       │ db.set_text(types.aivi, new_text)
                                       ▼
                          ┌────────────────────────┐
                          │ parsed_file(types.aivi) │  ← invalidated, re-run
                          └────────────┬───────────┘
                                       │
                          ┌────────────▼───────────┐
                          │  hir_module(types.aivi) │  ← re-run
                          └────────────┬───────────┘
                                       │
                    ┌──────────────────▼──────────────────────────┐
                    │       exported_names(types.aivi)             │  ← re-run
                    └──┬───────────────────────────────────────┬──┘
                       │ (if exports changed)                  │
              ┌────────▼────────┐                   ┌─────────▼────────┐
              │ hir_module(a)   │                   │ hir_module(b)    │  ← re-run
              └────────┬────────┘                   └─────────┬────────┘
                       │                                      │
              ┌────────▼────────┐                   ┌─────────▼────────┐
              │ typed_module(a) │                   │ typed_module(b)  │  ← re-run
              └─────────────────┘                   └──────────────────┘

         files that don't import types.aivi: zero queries re-run
```

**If only a function body changes** (not its type signature), `exported_names` may be
unchanged. Salsa compares the new result to the old result using `Eq`. If they are equal,
it marks dependent queries as "verified" without re-running them. This early-exit is
automatic and applies at every query boundary.

---

## 7. LSP server integration

The `aivi-lsp` crate owns one `Arc<RwLock<RootDatabase>>`. All request handlers
take a read lock, call the relevant salsa query, and release the lock.

```rust
// aivi-lsp/src/state.rs

pub struct ServerState {
    pub db:     Arc<parking_lot::RwLock<RootDatabase>>,
    pub files:  DashMap<Url, SourceFile>,   // URI → salsa SourceFile handle
    pub config: LspConfig,                  // salsa input, updated on config change
}
```

### 7.1 Document change handler

```rust
async fn did_change(&self, params: DidChangeTextDocumentParams) {
    let uri  = params.text_document.uri.clone();
    let text = apply_changes(&self.state, &uri, params.content_changes);

    // Write lock only to mutate the salsa input — held briefly
    {
        let mut db = self.state.db.write();
        let file   = *self.state.files.get(&uri).unwrap();
        file.set_text(&mut *db).to(text);
    }
    // Lock released. Salsa has marked affected queries as dirty.

    // Spawn background task to re-run diagnostics (debounced)
    self.schedule_diagnostics(uri);
}
```

### 7.2 Request handler pattern

Every LSP handler takes a **read lock** only:

```rust
async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri  = &params.text_document_position_params.text_document.uri;
    let pos  = params.text_document_position_params.position;
    let file = *self.state.files.get(uri).ok_or(not_open())?;

    // Read lock: salsa evaluates (or returns cached) typed_module(file)
    let db   = self.state.db.read();
    let result = build_hover(&*db, file, pos);
    Ok(result)
}
```

Salsa queries are re-entrant under read locks. Multiple concurrent read-lock holders
can evaluate independent queries in parallel on salsa's internal Rayon thread pool.
Write locks are held only during input mutation (microseconds).

### 7.3 Parallel query evaluation

Salsa uses Rayon internally. When the diagnostics task fires for file A, and the user
simultaneously requests hover for file B, salsa runs both query graphs concurrently
without any explicit coordination — the dependency tracking handles isolation.

The `parking_lot::RwLock` allows multiple concurrent readers. Write contention only
occurs when a document change arrives, which is always brief.

---

## 8. Stdlib pre-warming

At LSP `initialize`, before responding to the client:

```rust
async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
    let stdlib = aivi_lsp::stdlib::root_path()
        .ok_or_else(|| Error::new(ErrorCode::InternalError, "stdlib not found"))?;

    {
        let mut db = self.state.db.write();

        // Set the stdlib root input (never changes after this)
        stdlib_root_path::set(&mut *db, stdlib.clone());

        // Discover all stdlib .aivi files and register them as SourceFiles
        for path in walk_aivi_files(&stdlib) {
            let text = std::fs::read_to_string(&path)?;
            let file = SourceFile::new(&mut *db, path.clone(), text);
            self.state.files.insert(Url::from_file_path(&path).unwrap(), file);
        }
    }

    // Pre-warm: evaluate hir_module and typed_module for all stdlib files.
    // Done in a blocking task so initialize can respond immediately if we want,
    // or we can block until done for best first-request latency.
    let db_ref = Arc::clone(&self.state.db);
    tokio::task::spawn_blocking(move || {
        let db = db_ref.read();
        for file in stdlib_source_files(&*db) {
            // These calls populate the salsa memo tables for stdlib.
            // Subsequent user-file queries that import stdlib hit the cache.
            let _ = typed_module(&*db, file);
            let _ = symbol_index(&*db, file);
        }
    }).await?;

    Ok(build_capabilities())
}
```

After pre-warming, `typed_module` for any stdlib file returns instantly from the memo
table. User files that import stdlib pay zero re-parsing cost.

---

## 9. Workspace indexing

On startup (after stdlib pre-warming), index all workspace files:

```rust
async fn index_workspace(state: &ServerState, root: &Path) {
    let paths: Vec<PathBuf> = walk_aivi_files(root).collect();

    {
        let mut db = state.db.write();
        for path in &paths {
            if !state.files.contains_key(&Url::from_file_path(path).unwrap()) {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                let file = SourceFile::new(&mut *db, path.clone(), text);
                state.files.insert(Url::from_file_path(path).unwrap(), file);
            }
        }
        let ws = WorkspaceFiles::new(&mut *db, paths);
        // store as input so resolve_module can see all files
        workspace_files::set(&mut *db, ws);
    }

    // Index symbols for workspace/symbol — in parallel via Rayon through salsa
    let db = state.db.read();
    for (_, &file) in state.files.iter() {
        let _ = symbol_index(&*db, file);
    }
}
```

File create/delete/rename events update `WorkspaceFiles` and the `files` map, which
salsa propagates to `resolve_module` queries automatically.

---

## 10. Cycle detection

Module import cycles (`A imports B imports A`) must not deadlock the query engine.

Salsa detects cycles at the query level and invokes a user-provided recovery function:

```rust
#[salsa::tracked(recovery_fn = recover_hir_cycle)]
pub fn hir_module(db: &dyn AiviDb, file: SourceFile) -> HirModule { ... }

fn recover_hir_cycle(
    db:    &dyn AiviDb,
    cycle: &salsa::Cycle,
    file:  SourceFile,
) -> HirModule {
    // Emit a diagnostic naming the cycle participants.
    let diag = aivi_base::Diagnostic {
        level:   DiagnosticLevel::Error,
        code:    DiagCode::CyclicImport,
        message: format!(
            "import cycle detected: {}",
            cycle.participant_keys().map(|k| k.debug_name()).join(" → ")
        ),
        span:    file.import_span(db),  // span of the import declaration
        ..Default::default()
    };
    HirModule::new(db, aivi_hir::Module::empty(), vec![diag])
}
```

The same recovery pattern applies to `typed_module` and `exported_names`.

---

## 11. Salsa result equality and early exit

For salsa's early-exit optimization to fire on `exported_names`, that type must implement
`Eq`. The derived `Eq` compares exported name sets structurally. If only a function body
changes (not its type), exported names are unchanged, salsa marks dependents as "verified"
without re-running them.

```rust
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct ExportedNames {
    // Sorted vec for deterministic Eq
    pub names: Vec<ExportedName>,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct ExportedName {
    pub name: String,
    pub kind: SymbolKind,
    pub ty:   TypeRepr,   // surface type representation, not internal ID
}
```

All salsa `#[tracked]` struct fields that participate in early-exit must derive
`PartialEq + Eq`. Internal node IDs (arena indices) must not leak into these types
because they change on every re-parse even if the content is identical.

This means `TypeRepr` used in `ExportedName` must be a **stable surface representation**
(e.g., a rendered type string or a content-addressed type key), not an internal arena
index. The typing pass already normalises types; this constraint pins down which
representation to use as the stable equality surface.

---

## 12. Required changes to existing crates

### `aivi-hir`

- `aivi_hir::lower` must accept an `ImportResolver` trait object so `hir_module` can
  inject salsa-backed import resolution without `aivi-hir` depending on salsa.
- `aivi_hir::exports(module) -> ExportedNames` — new function, cheap structural walk.
- `aivi_hir::Module::empty()` — returns a valid but empty module for cycle recovery.

### `aivi-typing`

- `aivi_typing::check` must accept the same `ImportResolver` for cross-file instance
  resolution.
- `aivi_typing::TypedModule` must expose `type_of_node(offset) -> Option<TypeRepr>` for
  hover and inlay hints.

### `aivi-base`

- `aivi_base::Source::span_to_lc(span) -> (line, col)` — needed by all query outputs
  that produce LSP ranges. Must be cheap (O(line count) with a pre-built line table).

### `aivi-syntax`

- No changes needed. `aivi_syntax::parse` is already a pure function.

---

## 13. `aivi-query` dependencies

```toml
# crates/aivi-query/Cargo.toml
[dependencies]
salsa          = "0.21"          # or latest stable
aivi-base      = { path = "../aivi-base" }
aivi-syntax    = { path = "../aivi-syntax" }
aivi-hir       = { path = "../aivi-hir" }
aivi-typing    = { path = "../aivi-typing" }
```

`aivi-lsp` replaces its direct pipeline dependencies with `aivi-query`:

```toml
# crates/aivi-lsp/Cargo.toml
[dependencies]
aivi-query     = { path = "../aivi-query" }
aivi-base      = { path = "../aivi-base" }   # for Diagnostic, Source types
tower-lsp      = "0.20"
# aivi-syntax, aivi-hir, aivi-typing are now accessed through aivi-query
```

---

## 14. Performance budget

| Query                    | Expected cold time | Expected warm time |
|--------------------------|--------------------|--------------------|
| `parsed_file`            | 1–3 ms             | 0 µs (memoised)    |
| `hir_module`             | 3–8 ms             | 0 µs               |
| `typed_module`           | 5–15 ms            | 0 µs               |
| `symbol_index`           | < 1 ms             | 0 µs               |
| `semantic_tokens`        | 2–5 ms             | 0 µs               |
| `all_diagnostics`        | 0 µs (aggregates)  | 0 µs               |
| `exported_names`         | < 1 ms             | 0 µs               |
| stdlib pre-warm (all)    | 50–200 ms once     | 0 µs forever       |

With a 200 ms debounce, the steady-state user experience is: type → wait 200 ms →
diagnostics appear. The 200 ms is the debounce, not the computation. In practice,
re-parsing and re-checking one file after a local change will complete in < 20 ms,
leaving 180 ms of headroom before the debounce fires.

---

## 15. Future: on-disk cache

Salsa's memo tables live in process memory and are lost on server restart. A future
optimisation can persist them:

- Serialise the salsa database to `~/.cache/aivi/db.bin` on LSP shutdown
- Restore on next startup, validate against file mtimes
- Fall back to cold evaluation for any stale entry

This is strictly a startup latency optimisation (avoids re-parsing stdlib and unchanged
workspace files after editor restart). It is not required for correctness and should
not be implemented until the query boundaries are stable.

---

## 16. Milestones

| Milestone | Deliverable                                                                         |
|-----------|-------------------------------------------------------------------------------------|
| M1        | `aivi-query` crate scaffolded; `parsed_file` query wrapping `aivi_syntax::parse`   |
| M2        | `hir_module` query; `ImportResolver` trait in `aivi-hir`                           |
| M3        | `typed_module` query; `aivi-lsp` analysis pipeline replaced with salsa queries     |
| M4        | `symbol_index` + `semantic_tokens` queries                                          |
| M5        | `exported_names` query with `PartialEq` early-exit; stdlib pre-warm at initialize  |
| M6        | `resolve_module` + `WorkspaceFiles`; cross-file import resolution through salsa    |
| M7        | Cycle detection and recovery with diagnostics                                       |
| M8        | `all_diagnostics` aggregation query; parallel workspace indexing at startup         |
| M9        | `LspConfig` as salsa input; server reacts to config changes without restart         |
| M10       | Performance benchmarks: cold + warm times for all queries on a 100-file project    |
| M11       | *(future)* On-disk cache for warm restart                                           |
