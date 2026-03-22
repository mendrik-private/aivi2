# Plan: AIVI LSP Server (Rust)

## Status: design draft — not yet implemented

---

## 1. Overview

The AIVI LSP server is a **long-running Rust process** started with `aivi lsp`. It speaks JSON-RPC over stdio and implements Language Server Protocol 3.17. It is built directly into the `aivi` binary alongside the compiler, reusing the existing parser, HIR, and typing pipeline without spawning sub-processes.

The VSCode extension (and any other LSP client) starts it with:

```
aivi lsp
```

No flags are needed for normal operation. Optional:

```
aivi lsp --log <path>      # write structured log to file
aivi lsp --log-level debug # verbosity (error|warn|info|debug|trace)
```

---

## 2. Crate layout

A new crate `aivi-lsp` is added to the workspace:

```
crates/
├── aivi-base/          (existing)
├── aivi-syntax/        (existing — parser, CST, formatter)
├── aivi-hir/           (existing — HIR, elaboration)
├── aivi-typing/        (existing — kind/type checking)
├── aivi-lsp/           (new)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── server.rs       # tower-lsp LanguageServer impl
│   │   ├── state.rs        # shared mutable server state
│   │   ├── documents.rs    # incremental text document store
│   │   ├── workspace.rs    # file discovery, workspace symbol index
│   │   ├── analysis.rs     # per-file analysis pipeline (parse → HIR → types)
│   │   ├── hover.rs
│   │   ├── completion.rs
│   │   ├── definition.rs
│   │   ├── references.rs
│   │   ├── symbols.rs
│   │   ├── semantic_tokens.rs
│   │   ├── signature_help.rs
│   │   ├── code_actions.rs
│   │   ├── rename.rs
│   │   ├── inlay_hints.rs
│   │   ├── folding.rs
│   │   ├── formatting.rs
│   │   ├── code_lens.rs
│   │   ├── call_hierarchy.rs
│   │   ├── diagnostics.rs
│   │   └── docs.rs         # DocsStore (stdlib doc index)
│   ├── tests/
│   │   ├── fixtures/       # .aivi fixture files
│   │   ├── hover.rs
│   │   ├── completion.rs
│   │   ├── diagnostics.rs
│   │   └── ...
│   └── Cargo.toml
└── aivi-cli/           (existing — entry point, adds `lsp` sub-command)
```

`aivi-cli/src/main.rs` gains:

```rust
SubCommand::Lsp(args) => aivi_lsp::run(args).await,
```

---

## 3. Dependencies

| Crate                  | Purpose                                               |
|------------------------|-------------------------------------------------------|
| `tower-lsp`            | LSP server framework (JSON-RPC, trait-based dispatch) |
| `lsp-types`            | LSP protocol type definitions                         |
| `tokio`                | Async runtime (tower-lsp requires it)                 |
| `serde` / `serde_json` | JSON serialization                                    |
| `aivi-query`           | Salsa-backed incremental query database (see plan/06) |
| `parking_lot`          | RwLock for the salsa database (faster than std)       |
| `dashmap`              | Concurrent map for URI → SourceFile handle            |
| `rmp-serde`            | MessagePack deserialization for docs index            |
| `ropey`                | Rope for O(log n) incremental text edits              |
| `tracing`              | Structured logging                                    |
| `tracing-subscriber`   | Log output to file (when `--log` is set)              |
| `tokio-util`           | Codec for stdio framing                               |

`tower-lsp` handles JSON-RPC framing, request/response correlation, and cancellation.
`aivi-query` provides the salsa database that memoises and incrementally re-evaluates
the parse → HIR → typing pipeline. See `plan/06-incremental-computation.md` for the
full design — that plan is a required dependency of this one.

---

## 4. Server state

All language intelligence flows through the salsa `RootDatabase`. The LSP server owns
one database instance for the lifetime of the editor session.

```rust
// aivi-lsp/src/state.rs

pub struct ServerState {
    // The salsa database: owns all memoised query results.
    // Read lock for queries; write lock only when mutating inputs (text changes).
    pub db:          Arc<parking_lot::RwLock<aivi_query::RootDatabase>>,
    // URI → salsa SourceFile input handle
    pub files:       DashMap<Url, aivi_query::SourceFile>,
    // Stdlib docs loaded at startup (plan/05)
    pub docs:        Arc<aivi_query::DocsStore>,
    // Client capabilities, set during initialize
    pub client_caps: OnceLock<ClientCapabilities>,
    // Per-file debounce abort handles
    pub debounce:    DashMap<Url, tokio::task::AbortHandle>,
}
```

There is no separate `AnalysisResult` cache. Salsa IS the cache. Every query result
is memoised automatically and invalidated only when its inputs change.

### 4.1 Document change → salsa input mutation

```rust
async fn did_change(&self, params: DidChangeTextDocumentParams) {
    let uri  = params.text_document.uri.clone();
    let text = apply_rope_changes(&self.state, &uri, &params.content_changes);

    // Write lock is held only for input mutation — microseconds.
    {
        let mut db = self.state.db.write();
        let file   = *self.state.files.get(&uri).unwrap();
        file.set_text(&mut *db).to(text);
    }
    // Salsa marks all dependent queries dirty. Lock released.

    // Debounce: cancel previous pending task, schedule new one.
    if let Some(handle) = self.state.debounce.remove(&uri).map(|(_, h)| h) {
        handle.abort();
    }
    let state  = Arc::clone(&self.inner);
    let client = self.client.clone();
    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(
            state.db.read().lsp_config().debounce_ms()
        )).await;
        // Re-evaluate diagnostics under read lock (salsa recomputes incrementally).
        let diags = {
            let db   = state.db.read();
            let file = *state.files.get(&uri).unwrap();
            aivi_query::all_diagnostics(&*db, file)
                .iter()
                .map(into_lsp_diagnostic)
                .collect::<Vec<_>>()
        };
        client.publish_diagnostics(uri, diags, None).await;
    });
    self.state.debounce.insert(uri, handle.abort_handle());
}
```

### 4.2 Request handler pattern

All LSP request handlers take only a **read lock** and call salsa queries directly.
Salsa returns the memoised result immediately if no relevant input changed, or
re-evaluates the minimum affected subgraph otherwise.

```rust
async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri  = &params.text_document_position_params.text_document.uri;
    let pos  = params.text_document_position_params.position;
    let file = *self.state.files.get(uri).ok_or(not_open())?;

    let db  = self.state.db.read();   // read lock only
    let res = aivi_lsp::hover::build(&*db, file, pos, &self.state.docs);
    Ok(res)
}
```

Multiple concurrent LSP requests (hover + completion + a diagnostics refresh) all hold
read locks simultaneously and execute query subgraphs in parallel through salsa's
internal Rayon pool. Write contention only occurs during input mutation.

---

## 5. LSP capabilities

The server declares the following capabilities on `initialize`:

```rust
ServerCapabilities {
    text_document_sync: Some(TextDocumentSyncCapability::Options(
        TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::INCREMENTAL),
            save: Some(TextDocumentSyncSaveOptions::SaveOptions(
                SaveOptions { include_text: Some(false) }
            )),
            ..Default::default()
        }
    )),
    hover_provider: Some(HoverProviderCapability::Simple(true)),
    completion_provider: Some(CompletionOptions {
        trigger_characters: Some(vec![
            ".".into(), "|".into(), "&".into(), "?".into(),
            "*".into(), "@".into(), "<".into(), "{".into(),
            "\"".into(), " ".into(), "#".into(),
        ]),
        resolve_provider: Some(true),
        ..Default::default()
    }),
    signature_help_provider: Some(SignatureHelpOptions {
        trigger_characters: Some(vec!["(".into(), " ".into()]),
        retrigger_characters: Some(vec![",".into()]),
        ..Default::default()
    }),
    definition_provider: Some(OneOf::Left(true)),
    type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
    references_provider: Some(OneOf::Left(true)),
    document_symbol_provider: Some(OneOf::Left(true)),
    workspace_symbol_provider: Some(OneOf::Left(true)),
    code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
        code_action_kinds: Some(vec![
            CodeActionKind::QUICKFIX,
            CodeActionKind::REFACTOR,
            CodeActionKind::SOURCE_ORGANIZE_IMPORTS,
        ]),
        resolve_provider: Some(true),
        ..Default::default()
    })),
    document_formatting_provider: Some(OneOf::Left(true)),
    document_range_formatting_provider: Some(OneOf::Left(true)),
    rename_provider: Some(OneOf::Right(RenameOptions {
        prepare_provider: Some(true),
        ..Default::default()
    })),
    folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
    semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
        SemanticTokensOptions {
            legend: semantic_token_legend(),
            range: Some(true),
            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
            ..Default::default()
        }
    )),
    inlay_hint_provider: Some(OneOf::Left(true)),
    code_lens_provider: Some(CodeLensOptions {
        resolve_provider: Some(true),
    }),
    call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
    ..Default::default()
}
```

---

## 6. Feature implementations

### 6.1 Diagnostics

`textDocument/publishDiagnostics` is sent after each analysis cycle. `aivi-base::Diagnostic` maps to LSP `Diagnostic` via:

```rust
fn into_lsp_diagnostic(d: &aivi_base::Diagnostic, source: &Source) -> lsp_types::Diagnostic {
    Diagnostic {
        range:    span_to_range(d.span, source),
        severity: Some(into_severity(d.level)),
        code:     Some(NumberOrString::String(d.code.to_string())),
        message:  d.message.clone(),
        related_information: d.related.iter().map(|r| DiagnosticRelatedInformation {
            location: Location {
                uri:   Url::from_file_path(&r.file).unwrap(),
                range: span_to_range(r.span, source),
            },
            message: r.message.clone(),
        }).collect::<Vec<_>>().into(),
        tags: d.tags.as_ref().map(|t| t.iter().map(into_tag).collect()),
        data: d.fix_data.as_ref().map(|f| serde_json::to_value(f).unwrap()),
        ..Default::default()
    }
}
```

### 6.2 Hover

See `plan/04-hover-and-navigation.md` for the full hover design (layered content, type expansion tree, doc comments from `DocsStore`).

Implementation in `hover.rs`. The handler calls `aivi_query::typed_module` — salsa
returns the memoised result if the file has not changed since the last check:

```rust
// in the tower-lsp handler:
async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri  = &params.text_document_position_params.text_document.uri;
    let pos  = params.text_document_position_params.position;
    let file = *self.state.files.get(uri).ok_or(not_open())?;
    let db   = self.state.db.read();

    // salsa returns cached typed_module if nothing changed
    let typed  = aivi_query::typed_module(&*db, file);
    let parsed = aivi_query::parsed_file(&*db, file);
    let offset = lc_to_offset(pos, parsed.source(&*db));
    let depth  = db.lsp_config().expand_depth();

    let content = aivi_lsp::hover::build(
        offset, &typed, parsed.source(&*db), &self.state.docs, depth
    );
    Ok(content.map(|(span, md)| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown, value: md,
        }),
        range: Some(span_to_range(span, parsed.source(&*db))),
    }))
}
```

`aivi_lsp::hover::build` calls `typed.module().type_of_node(offset)` for type-at-offset,
runs the recursive type expansion algorithm, and consults `DocsStore` for doc comments.

### 6.3 Completion

Trigger context is determined from the CST at the cursor position. The CST comes from
the salsa `parsed_file` query — always the current memoised parse, no re-parse:

```rust
enum CompletionContext {
    TopLevel,                            // fresh line at top level
    PipeBody { op: PipeOp },             // after a pipe operator
    FieldAccess,                         // after "."
    TypeAnnotation,                      // after ":"
    RecordLiteral { ty: Option<TypeId> },// inside "{ ... }"
    SourceProvider,                      // after "@source "
    SourceOption { provider: ProviderId },// inside "with { ... }"
    PatternPosition,                     // after "||> "
    MarkupTag,                           // after "<"
    MarkupAttribute,                     // inside markup tag
    ImportList,                          // inside "use module.path ("
}
```

Each context maps to a different completion strategy. In-scope names come from the salsa
`symbol_index` query for this file plus `exported_names` queries for imported files —
all already memoised from the background analysis cycle.

### 6.4 Semantic tokens

The `aivi_query::semantic_tokens` salsa query (see plan/06 §5.5) memoises the token
data. The LSP server stores the last-emitted version per URI for delta computation:

```rust
struct SemanticTokenData {

```rust
struct SemanticTokenData {
    // Encoded per LSP spec: [deltaLine, deltaStart, length, tokenType, tokenModifiers]*
    encoded: Vec<u32>,
    // Also store by version for delta computation
    version: i32,
}
```

Delta encoding diffs the previous `encoded` vector against the new one and emits `SemanticTokensDelta` with `edits`.

Semantic token types mirror those in `plan/01-lsp-server.md §3.5` of the original plan — all 14 token types are produced by the Rust pass.

### 6.5 Go-to-definition / type definition

In-file: walk the HIR's name resolution table (`aivi_hir::Resolver`) to find the declaration site of the name under the cursor.

Cross-file: look up the name in `WorkspaceIndex` which maintains a `HashMap<QualifiedName, Location>` populated from all workspace files. See `plan/04-hover-and-navigation.md §4`.

Type definition (`textDocument/typeDefinition`): query `aivi_typing::TypedModule::type_of_node(offset)`, then resolve the type's declaration location.

### 6.6 Code actions

Quick fixes are generated in two ways:

1. **From diagnostic data**: diagnostics with a non-null `data` field carry a `FixData` payload. `textDocument/codeAction` deserializes it and turns it into a `TextEdit`. No second compiler pass needed.

2. **On-demand analysis**: for refactor actions (extract val, inline val, expand shorthand), the server runs a targeted sub-pass of the HIR to compute the edit.

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
enum FixData {
    AddMissingPatterns { patterns: Vec<String> },
    RemoveUnusedImport { range: LspRange },
    AddMissingFields   { fields: Vec<(String, String)> },  // (name, type_text)
    InsertAnnotation   { range: LspRange, text: String },
}
```

### 6.7 Rename

```
prepareRename → validates the name is local and renameable, returns its range
rename        → calls aivi_hir::find_all_references(name, hir_module(file)) → Vec<Span>
               for cross-file: iterates workspace files, calls hir_module via salsa
               (already memoised — no re-parse for unchanged files)
               → groups spans by file → WorkspaceEdit
```

### 6.8 Formatting

Calls `aivi_query::parsed_file` to get the memoised CST, then passes it to
`aivi_syntax::format::format_cst(cst, &FormatOptions::default())` — the same code path
as `aivi fmt`. No subprocess. The result is diffed against the original text with a
Myers line-diff to produce minimal `Vec<TextEdit>`.

### 6.9 Inlay hints

```rust
async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
    let file   = *self.state.files.get(&uri).ok_or(not_open())?;
    let db     = self.state.db.read();
    let typed  = aivi_query::typed_module(&*db, file);    // memoised
    let parsed = aivi_query::parsed_file(&*db, file);     // memoised
    let hints  = collect_inlay_hints(
        typed.module(&*db), parsed.source(&*db), &params.range
    );
    Ok(Some(hints))
}
```

Hint locations: `val`/`sig` bindings without explicit annotations, labeled parameters,
record shorthands. Each hint is an `InlayHint` with `kind: Type` or `kind: Parameter`.

---

## 7. Incremental document text (ropey)

The document text is kept as a `ropey::Rope` inside the salsa `SourceFile` input.
`TextDocumentContentChangeEvent` in incremental mode provides `range` + `text`; the
rope applies each change in O(log n) before the string is committed to salsa:

```rust
fn apply_rope_changes(
    state:   &ServerState,
    uri:     &Url,
    changes: &[TextDocumentContentChangeEvent],
) -> String {
    // Retrieve current text from salsa (read lock, very cheap)
    let file = *state.files.get(uri).unwrap();
    let mut rope = {
        let db = state.db.read();
        ropey::Rope::from_str(file.text(&*db))
    };
    for change in changes {
        if let Some(range) = &change.range {
            let start = lsp_pos_to_char_idx(&rope, range.start);
            let end   = lsp_pos_to_char_idx(&rope, range.end);
            rope.remove(start..end);
            rope.insert(start, &change.text);
        } else {
            rope = ropey::Rope::from_str(&change.text);
        }
    }
    rope.to_string()
}
```

`lsp_pos_to_char_idx` converts LSP line/character (UTF-16 code units) to ropey char
indices (Unicode scalars). This conversion must be explicit and correct — the off-by-one
errors here are a common source of subtle LSP bugs.

---

## 8. Workspace index

The workspace index lives inside the salsa database as `WorkspaceFiles` (a salsa input)
and the per-file `symbol_index` and `exported_names` queries (see plan/06 §5). There is
no separate `WorkspaceIndex` struct — salsa provides the index automatically.

For workspace/symbol queries the server iterates `WorkspaceFiles` and calls
`symbol_index(db, file)` for each, all memoised. For cross-file definition, it calls
`exported_names(db, file)` for the relevant module.

File system events update the `WorkspaceFiles` input:
- `workspace/didCreateFiles` → add new `SourceFile` inputs + update `WorkspaceFiles`
- `workspace/didDeleteFiles` → remove `SourceFile` handles + update `WorkspaceFiles`
- `workspace/didRenameFiles` → treat as delete + create

Salsa propagates each change to only the files that transitively depend on the
changed input.

---

## 9. Stdlib path discovery

```rust
pub fn root_path() -> Option<PathBuf> {
    // 1. AIVI_STDLIB env var (for dev/test overrides)
    if let Ok(p) = std::env::var("AIVI_STDLIB") {
        return Some(PathBuf::from(p));
    }
    // 2. Relative to current executable: ../lib/aivi/std
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.parent()?.join("lib").join("aivi").join("std");
    candidate.is_dir().then_some(candidate)
}
```

---

## 10. Docs store

See `plan/05-stdlib-docs.md` for the full design. The `DocsStore` is a struct in `docs.rs`:

```rust
pub struct DocsStore {
    by_name:     HashMap<String, SymbolDocs>,
    by_location: HashMap<(PathBuf, u32, u32), SymbolDocs>,  // (file, line, col)
}

impl DocsStore {
    pub fn load(stdlib_path: &Path) -> anyhow::Result<Self> { ... }
    pub fn lookup(&self, name: &str) -> Option<&SymbolDocs> { ... }
    pub fn lookup_by_location(&self, file: &Path, line: u32, col: u32) -> Option<&SymbolDocs> { ... }
}
```

---

## 11. Logging and tracing

All LSP traffic can be logged to a file when `--log` is set:

```
aivi lsp --log /tmp/aivi-lsp.log --log-level debug
```

The `tower-lsp` framework logs request/response pairs. The server additionally logs analysis timings (parse, HIR, typing) at `debug` level.

The VSCode extension sets `aivi.trace.server: "verbose"` to forward LSP traffic to the "AIVI Trace" output channel. This is independent of `--log` and uses tower-lsp's built-in tracing support.

---

## 12. Testing

### 12.1 Unit tests (in `aivi-lsp/tests/`)

Each feature module has a corresponding test file. Tests use a `TestServer` helper that creates a real `ServerState`, feeds it source text, and calls handler methods directly (no JSON-RPC serialization):

```rust
struct TestServer {
    state: Arc<ServerState>,
}

impl TestServer {
    async fn open(&self, uri: &str, text: &str) { ... }
    async fn change(&self, uri: &str, version: i32, changes: Vec<TextDocumentContentChangeEvent>) { ... }
    async fn hover(&self, uri: &str, line: u32, col: u32) -> Option<String> { ... }
    async fn diagnostics(&self, uri: &str) -> Vec<Diagnostic> { ... }
    async fn complete(&self, uri: &str, line: u32, col: u32) -> Vec<String> { ... }
}
```

### 12.2 Snapshot tests

Hover markdown and completion item lists are snapshot-tested using `insta`. Snapshots are committed and reviewed on change.

### 12.3 Integration tests (in `aivi-cli/tests/`)

`tests/lsp.rs` tests the actual `aivi lsp` process via stdin/stdout JSON-RPC using a minimal test client. Exercises the full stack including framing and serialization.

### 12.4 Regression fixtures

Every bug fix adds a minimal `.aivi` fixture in `tests/fixtures/regressions/` and a test that would have failed before the fix.

---

## 13. Milestones

| Milestone | Deliverable                                                                     |
|-----------|---------------------------------------------------------------------------------|
| M1        | `aivi lsp` starts, negotiates capabilities, publishes diagnostics               |
| M2        | Incremental document sync + debounced analysis pipeline                         |
| M3        | `textDocument/documentSymbol` + `workspace/symbol`                              |
| M4        | `textDocument/definition` (in-file) + `textDocument/references` (in-file)       |
| M5        | `textDocument/hover` (signature only)                                           |
| M6        | `semanticTokens/full` + `delta`                                                 |
| M7        | `textDocument/completion` with context classification + snippets                |
| M8        | `signatureHelp`                                                                 |
| M9        | `textDocument/inlayHint`                                                        |
| M10       | Hover type expansion tree + `LspTypeLink` (plan/04 M1–M4)                      |
| M11       | `textDocument/formatting` + `rangeFormatting` (delegates to aivi-syntax format) |
| M12       | `textDocument/codeAction` (quick-fix + refactor)                                |
| M13       | `textDocument/rename` + `prepareRename`                                         |
| M14       | `codeLens` + `callHierarchy`                                                    |
| M15       | `foldingRange`                                                                  |
| M16       | `textDocument/typeDefinition` (plan/04 M5)                                      |
| M17       | Stdlib source indexing + cross-file definition (plan/04 M6–M7)                  |
| M18       | `DocsStore` + stdlib hover docs (plan/05 M4)                                    |
