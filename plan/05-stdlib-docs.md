# Plan: AIVI Standard Library Documentation System

## Status: design draft — not yet implemented

---

## 1. Overview

The AIVI standard library needs inline documentation that is:

1. **Authored** as structured doc comments in stdlib `.aivi` source files
2. **Extracted** by the compiler into a compact binary index
3. **Served** by the LSP server in hover responses and completion documentation
4. **Browsable** via a `aivi doc` CLI command and (eventually) a web site

This plan does not design the stdlib itself — it designs the documentation *infrastructure*: the comment syntax, the extraction pipeline, the compiled index format, and the editor integration.

---

## 2. Doc comment syntax

Doc comments in AIVI source use triple-dash `---` for doc comments, distinguished from ordinary `--` line comments. They attach to the declaration immediately below them.

```aivi
--- The integer type.
--- Represented as a signed 64-bit integer.
--- Arithmetic overflow wraps with two's complement semantics.
type Int

--- The UTF-8 text type.
--- AIVI `Text` values are immutable and do not expose raw byte indices.
--- All indexing is by Unicode scalar value.
type Text

--- Returns the length of a list.
---
--- ```aivi
--- List.length [1, 2, 3]  -- 3
--- ```
---
--- Complexity: O(n).
fun length: Int #xs: List A =>
    ...
```

### 2.1 Syntax rules

- `---` followed by a space (or nothing) starts a doc comment line.
- Doc comment lines must be consecutive; a blank line or `--` line ends the block.
- Doc comments attach to the *next* top-level declaration. Attachment fails if there is no declaration below (diagnostic: `W_ORPHAN_DOC`).
- Nested declarations (class methods, constructors) have their own doc comments written directly above them inside the block.
- Inline markup inside doc comment text uses Markdown: `**bold**`, `*italic*`, `\`code\``, `` ```aivi ... ``` `` fenced blocks, `[link text]` cross-references.

### 2.2 Cross-references in doc comments

`[Symbol]` inside a doc comment links to another symbol by name. The compiler resolves the name relative to the current module's scope.

Examples:

```aivi
--- Combines two `Option` values applicatively.
--- See also: [Applicative], [Result].
fun Option.apply: ...
```

```aivi
--- The canonical truthy/falsy type.
--- Used with [T|>] and [F|>] operators.
type Bool = True | False
```

Cross-references resolve at doc-extraction time. Unresolvable references emit `W_UNRESOLVED_DOC_REF` and are rendered as plain inline code.

### 2.3 Parameter and return documentation

Named parameters are documented with `@param` tags inside the doc comment block:

```aivi
--- Applies a function to each element of a list and returns the results.
---
--- @param f  The mapping function.
--- @param xs The input list.
--- @returns  A new list of the same length with each element transformed.
fun map: List B #f: (A -> B) #xs: List A => ...
```

`@param` lines are extracted separately and shown as a parameter table in hover.

`@returns` documents the return value.

`@since` records the stdlib version that introduced the symbol.

`@deprecated` marks the symbol as deprecated with a replacement note:

```aivi
--- @deprecated Use [Option.getOr] instead.
fun Option.fromMaybe: ...
```

The compiler attaches the `deprecated` semantic token modifier and LSP diagnostic tag to deprecated symbols.

---

## 3. Compiled doc index

The compiler, when building the stdlib, extracts all doc comments into a compact binary index. The LSP server loads this index on startup instead of parsing stdlib source files at query time.

### 3.1 Index format

The index is a single file: `aivi-docs.msgpack` (MessagePack for compact binary, with a JSON fallback `aivi-docs.json` for debugging). It lives in the stdlib directory alongside the source:

```
$(aivi stdlib-path)/aivi-docs.msgpack
```

### 3.2 Schema

```typescript
interface DocsIndex {
  version:  number;        // format version, must match compiler version
  modules:  ModuleDocs[];
}

interface ModuleDocs {
  path:     string;        // e.g. "aivi.core.option"
  file:     string;        // absolute path to source
  symbols:  SymbolDocs[];
}

interface SymbolDocs {
  name:     string;        // e.g. "Option.map"
  kind:     LspSymbolKind;
  range:    LspRange;      // selection range in source file
  // rendered markdown ready to embed in hover (compiler pre-renders it)
  markdown: string;
  // structured data for hover parameter table
  params:   ParamDoc[];
  returns:  string | null;
  since:    string | null;
  deprecated: string | null;  // null = not deprecated; string = replacement note
  // resolved cross-references for link rendering
  links:    DocLink[];
}

interface ParamDoc {
  name:       string;
  type:       string;   // type annotation as AIVI source text
  description: string;
}

interface DocLink {
  text:   string;       // text as written in [text]
  file:   string;       // resolved file path
  range:  LspRange;     // resolved selection range
}
```

### 3.3 Build command

```
aivi docs build-index --stdlib-path <path> --out <path/to/aivi-docs.msgpack>
```

Run as part of the stdlib build step. Not run by end users.

---

## 4. LSP server integration

### 4.1 Startup

On LSP `initialize`, the server:

1. Runs `aivi stdlib-path` to get the stdlib root.
2. Reads `${stdlibPath}/aivi-docs.msgpack` and deserializes it into a `DocsStore` in memory.
3. Falls back to `aivi-docs.json` if the msgpack file is absent.
4. If neither file exists, emits a warning notification and proceeds without stdlib docs.

### 4.2 `DocsStore` API

```typescript
class DocsStore {
  // look up docs by qualified symbol name
  lookup(qualifiedName: string): SymbolDocs | undefined

  // look up by file + selection range (for compiler-resolved locations)
  lookupByLocation(file: string, range: LspRange): SymbolDocs | undefined

  // all symbols in a module (for completion documentation)
  module(path: string): SymbolDocs[]

  // fuzzy search across all stdlib symbols (for workspace symbol provider)
  search(query: string, limit: number): SymbolDocs[]
}
```

### 4.3 Hover integration

When the compiler returns an `LspHoverResult`, the TypeScript server:

1. Checks each `LspTypeLink` against `DocsStore.lookupByLocation(link.file, link.range)`.
2. If docs are found, appends the `markdown` string as Layer 2 of the hover response.
3. Pre-rendered markdown from the index already has Markdown formatting; it is injected verbatim.
4. `@param` tables are already rendered in the index markdown; they appear below the description.

### 4.4 Completion documentation

When resolving a completion item (`completionItem/resolve`), the server:

1. Uses the completion item's `data.qualifiedName` to call `DocsStore.lookup(name)`.
2. Attaches the `markdown` as the `documentation` field (kind `markdown`).
3. Attaches `deprecated: true` to the item if `SymbolDocs.deprecated` is non-null.

### 4.5 User-authored doc comments

Doc comments in *user* source files (not stdlib) are also supported. The Rust compiler extracts them during `aivi lsp-check` and includes them in the `LspSymbol[]` response as an optional `docs` field:

```typescript
interface LspSymbol {
  // ... existing fields ...
  docs?: string;  // pre-rendered markdown for this symbol's doc comment
}
```

The LSP server stores user docs in the workspace `DefinitionIndex` alongside the symbol range and serves them the same way as stdlib docs.

---

## 5. `aivi doc` CLI browser

A terminal-based documentation browser for use without an editor.

```
aivi doc                          # browse all modules (interactive TUI)
aivi doc aivi.core.option         # show module index
aivi doc Option.map               # show one symbol's full docs
aivi doc --search "list map"      # fuzzy search
```

Output format: Markdown rendered to the terminal with ANSI color. Not a full TUI in v1; just `less`-piped formatted output.

### 5.1 Symbol page format

```
━━━ Option.map ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Module:   aivi.core.option
Kind:     fun
Since:    v0.1.0

SIGNATURE
  fun map: List B #f: (A -> B) #xs: List A

DESCRIPTION
  Applies a function to each element of a list and returns the results.

PARAMETERS
  f    (A -> B)   The mapping function.
  xs   List A     The input list.

RETURNS
  A new list of the same length with each element transformed.

EXAMPLES
  List.map (x => x + 1) [1, 2, 3]   -- [2, 3, 4]

SEE ALSO
  List.filter, List.foldl, *|> operator
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 6. Operator documentation

All 11 pipe operators are documented in the index as if they were named symbols in a virtual `aivi.operators` module. Their "source file" is a virtual file `operators.aivi` that contains their definitions as comments.

| Operator | Qualified name         |
|----------|------------------------|
| `\|>`    | `operators.pipe`        |
| `?|>`    | `operators.gate`        |
| `\|\|>` | `operators.caseSplit`   |
| `*\|>`   | `operators.fanOut`      |
| `&\|>`   | `operators.cluster`     |
| `@\|>`   | `operators.recur`       |
| `<\|@`   | `operators.recurStep`   |
| `\|`     | `operators.tap`         |
| `<\|*`   | `operators.fanOutJoin`  |
| `T\|>`   | `operators.truthy`      |
| `F\|>`   | `operators.falsy`       |

Hovering over any operator in the editor looks up `operators.<name>` in the `DocsStore` and shows the full reference card (Layer 4 of the hover response, see `plan/04-hover-and-navigation.md §2.5`).

---

## 7. Stdlib documentation authoring guidelines

These are the rules for writing doc comments in AIVI stdlib source files.

1. **Every exported declaration must have a doc comment.** The compiler emits `W_MISSING_DOC` for exported symbols without one (configurable, default: warning).
2. **First sentence is a one-liner.** It appears in completion popups and search results. Keep it under 80 characters.
3. **Code examples are required for non-trivial functions.** Use ` ```aivi ` fenced blocks.
4. **`@param` for every labeled parameter** when the function has more than one parameter.
5. **`@since` on every new symbol.** Format: `vMAJOR.MINOR.PATCH`.
6. **Cross-reference related operators and types** with `[Name]` syntax.
7. **No implementation details** in doc comments. Complexity notes (`O(n)`) are welcome; internal mechanism notes are not.
8. **Keep doc comments in English.** Translations are future work.

---

## 8. Required Rust changes

1. **Doc comment lexer**: the lexer in `aivi-syntax/src/lex.rs` must recognize `---` as a `DocComment` token distinct from `--`.
2. **Doc comment attachment**: a post-parse pass attaches `DocComment` token sequences to the next declaration node in the CST.
3. **Doc extraction pass**: a pass over the stdlib CST extracts `SymbolDocs` structs. This pass runs at stdlib build time.
4. **Index serializer**: serializes `DocsIndex` to MessagePack using `rmp-serde`.
5. **`aivi docs build-index` sub-command**: runs the extraction + serialization pipeline.
6. **`aivi lsp-check` doc attachment**: includes `docs` field in `LspSymbol[]` for user-authored doc comments.
7. **`W_MISSING_DOC` and `W_ORPHAN_DOC` diagnostics**: emit from the HIR validation pass.
8. **`W_UNRESOLVED_DOC_REF` diagnostic**: emit from the doc extraction pass when `[Name]` cannot be resolved.

---

## 9. TypeScript package additions

### `DocsStore` (`lsp-server/src/docs-store.ts`)

```typescript
import * as msgpack from "@msgpack/msgpack";

export class DocsStore {
  private byName = new Map<string, SymbolDocs>();
  private byLocation = new Map<string, SymbolDocs>(); // key: `${file}:${line}:${col}`

  static async load(stdlibPath: string): Promise<DocsStore> {
    const msgpackPath = path.join(stdlibPath, "aivi-docs.msgpack");
    const jsonPath    = path.join(stdlibPath, "aivi-docs.json");
    let raw: DocsIndex;
    try {
      raw = msgpack.decode(await fs.readFile(msgpackPath)) as DocsIndex;
    } catch {
      raw = JSON.parse(await fs.readFile(jsonPath, "utf8")) as DocsIndex;
    }
    return DocsStore.fromIndex(raw);
  }

  lookup(qualifiedName: string): SymbolDocs | undefined {
    return this.byName.get(qualifiedName);
  }

  lookupByLocation(file: string, range: LspRange): SymbolDocs | undefined {
    const key = `${file}:${range.start.line}:${range.start.character}`;
    return this.byLocation.get(key);
  }
}
```

### Dependency

| Package           | Purpose                         |
|-------------------|---------------------------------|
| `@msgpack/msgpack`| Decode the binary docs index    |

---

## 10. Milestones

| Milestone | Deliverable                                                                  |
|-----------|------------------------------------------------------------------------------|
| M1        | `---` doc comment lexer token + attachment pass in Rust                      |
| M2        | Doc extraction pass + `aivi docs build-index` command                        |
| M3        | `aivi-docs.json` (JSON fallback) generated for the stdlib skeleton           |
| M4        | `DocsStore` in LSP server; stdlib docs appear in hover Layer 2               |
| M5        | `@param` / `@returns` parameter tables rendered in hover                     |
| M6        | User doc comments extracted in `lsp-check`; appear in hover for user symbols |
| M7        | `@deprecated` tag → LSP `deprecated` tag + semantic token modifier           |
| M8        | Completion item documentation populated from `DocsStore`                     |
| M9        | `aivi doc` CLI browser (terminal output)                                     |
| M10       | `W_MISSING_DOC` lint for exported symbols without doc comments               |
| M11       | Operator reference cards in `DocsStore` (virtual `operators.aivi` module)    |
| M12       | `aivi-docs.msgpack` binary format replaces JSON for production               |
