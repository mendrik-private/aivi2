# Plan: Rich Hover, Type Expansion, and Cross-Module Navigation

## Status: design draft — not yet implemented

---

## 1. Motivation

Plain hover showing `val user : User` is not enough. The user wants to know what `User` *is* without leaving the file. At the same time, Ctrl+click must jump to the definition of any named type — including types defined in the standard library or in other modules.

This plan covers:
1. Recursive type expansion in hover
2. Clickable type names inside hover content
3. `textDocument/typeDefinition` (type → its declaration)
4. Cross-module and stdlib definition navigation
5. The Rust compiler queries that back all of this

---

## 2. Hover: layered type presentation

### 2.1 Structure

A hover response has four layers, rendered top to bottom in the hover popup:

```
┌─────────────────────────────────────────────────┐
│ LAYER 1  signature line                         │
│ LAYER 2  doc comment (if any)                   │
│ LAYER 3  type expansion tree (if composite)     │
│ LAYER 4  operator / keyword semantics (if kw)   │
└─────────────────────────────────────────────────┘
```

Layers 2–4 are optional and omitted when empty.

### 2.2 Layer 1 — Signature line

A single fenced code block:

````markdown
```aivi
val user : User
```
````

For a function:

````markdown
```aivi
fun formatDate: Text #d: DateTime
```
````

For a type constructor:

````markdown
```aivi
type DateTime = { year: Year, month: Month, day: Day }
```
````

For a signal:

````markdown
```aivi
sig currentUser : Signal (Option User)
```
````

For a pipe operator (hovering over `||>`):

````markdown
```aivi
||> : (A -> B) -> A -> B  -- case split
```
````

### 2.3 Layer 2 — Doc comment

If the symbol has an attached doc comment (see `plan/05-stdlib-docs.md`), it is rendered as plain markdown below the signature:

```markdown
Represents a calendar date in the proleptic Gregorian calendar.
Constructed from a `Year`, `Month`, and `Day` domain value.
Does not encode a time zone.
```

Inline doc links `[Symbol]` in doc comments are rendered as clickable command links (see §3.2).

### 2.4 Layer 3 — Type expansion tree

This layer is the new capability. It renders only when the type at the hover site contains at least one named non-primitive type.

#### 2.4.1 Primitive types (never expanded)

The following types are terminal — they are shown as-is and never expanded further:

`Int`, `Float`, `Decimal`, `BigInt`, `Bool`, `Text`, `Unit`, `Bytes`

Standard container types are also terminal at their outer level but their type argument is expanded if it is named:

`List A`, `Option A`, `Result E A`, `Validation E A`, `Signal A`, `Task E A`, `Map K V`, `Set A`

For example `Option DateTime` → the `DateTime` argument is expanded; `Option Text` → terminal.

#### 2.4.2 Expansion algorithm

```
expand(T, depth, visited):
  if T is primitive → terminal, render as inline code
  if T is in visited → render as inline code with "↻ recursive" annotation
  if depth == 0 → render as link only (collapsed)
  if T is a sum type → render constructors table, recurse into each payload
  if T is a record type → render fields table, recurse into each field type
  if T is a domain type → render "domain of <underlying>" and expand underlying
  if T is a container(A) → render "container of" and recurse into A
```

Default `depth` = 3. The compiler controls the depth; the TypeScript server does not re-expand.

#### 2.4.3 Record type expansion

For `val user : User` where `User = { name: Text, birthday: DateTime, email: Option Text }`:

```markdown
---
**`User`** &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)

| Field      | Type            |
|------------|-----------------|
| `name`     | `Text`          |
| `birthday` | [`DateTime`](command:aivi.goToDefinition?...)  |
| `email`    | `Option Text`   |

---
**`DateTime`** &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)

| Field   | Type                                    |
|---------|-----------------------------------------|
| `year`  | [`Year`](command:aivi.goToDefinition?...) |
| `month` | [`Month`](command:aivi.goToDefinition?...) |
| `day`   | [`Day`](command:aivi.goToDefinition?...)   |

---
**`Year`** &nbsp;·&nbsp; `domain Int` &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)
**`Month`** &nbsp;·&nbsp; `domain Int` &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)
**`Day`** &nbsp;·&nbsp; `domain Int` &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)
```

The `---` divider separates each type level. The hover popup is scrollable in VSCode so depth-3 trees are comfortable to read.

#### 2.4.4 Sum type expansion

For `Option DateTime`:

```markdown
---
**`Option DateTime`**

| Constructor | Payload |
|-------------|---------|
| `None`      | —       |
| `Some`      | [`DateTime`](command:aivi.goToDefinition?...) |
```

#### 2.4.5 Domain type expansion

For `Year` where `domain Year : Int`:

```markdown
---
**`Year`** &nbsp;·&nbsp; domain of `Int` &nbsp;·&nbsp; [→ definition](command:aivi.goToDefinition?...)
```

Domain types do not recurse further since their underlying type is always a primitive.

#### 2.4.6 Collapse for depth overflow

If the tree is cut at `depth = 0`, the type name is shown as a link only with a "…" hint:

```markdown
| `tags` | [`Tag`](command:aivi.goToDefinition?...) *(expand with Ctrl+hover)* |
```

The "Ctrl+hover" note is aspirational; in practice the user just Ctrl+clicks the link.

### 2.5 Layer 4 — Operator semantics

When hovering over a pipe operator (`|>`, `?|>`, `||>`, `*|>`, `&|>`, `@|>`, `<|@`, `|`, `<|*`, `T|>`, `F|>`), layers 3 and the signature are replaced by a compact reference card:

```markdown
**`||>` — case split**

Pattern-matches the current ambient subject against one or more constructor arms.
Each arm binds the constructor payload. All arms must return the same type.
Exhaustiveness is checked.

```aivi
status
 ||> Paid    => "paid"
 ||> Pending => "pending"
```
```

One card per operator is pre-authored as structured data in the Rust compiler (no runtime analysis needed).

---

## 3. Clickable type links in hover

### 3.1 VSCode command URI scheme

VSCode hover markdown supports `command:` URI links:

```markdown
[DateTime](command:aivi.goToDefinition?%7B%22file%22%3A%22%2Fpath%2Fto%2Ftypes.aivi%22%2C%22line%22%3A12%2C%22col%22%3A5%7D)
```

The argument is a URL-encoded JSON object:

```json
{ "file": "/path/to/types.aivi", "line": 12, "col": 5 }
```

The `aivi.goToDefinition` command (registered in the extension) calls `vscode.commands.executeCommand("editor.action.goToLocations", uri, position, locations, "peek", "")`.

### 3.2 Compiler output: resolved type links

The Rust compiler must embed navigation targets directly into the hover result so the TypeScript server does not have to do a separate resolution round-trip.

Extended `LspHoverResult`:

```typescript
interface LspHoverResult {
  range: LspRange;
  // Markdown with command:aivi.goToDefinition? URIs already embedded
  contents: string;
  // All navigation targets referenced in contents, keyed by type name
  // The TypeScript server uses these to register one-shot command handlers
  typeLinks: LspTypeLink[];
}

interface LspTypeLink {
  name:   string;   // type name as it appears in the markdown
  file:   string;   // absolute path (may be a stdlib virtual path, see §4.3)
  range:  LspRange; // selection range of the type's declaration name token
}
```

The TypeScript server pre-registers a temporary command `aivi.goToDefinition` before returning the hover response. This command is already registered as a permanent extension command; the hover data provides the target coordinates.

### 3.3 Hover activation on type name tokens

The extension registers a `vscode.languages.registerHoverProvider` that fires whenever the cursor is on any token of type `entity.name.type.aivi` or `support.class.aivi` (constructor names). This ensures that hovering over `DateTime` inside a type annotation also gives the full expansion, not just when hovering over the variable that has that type.

---

## 4. Cross-module and stdlib navigation

### 4.1 `textDocument/definition` (Ctrl+click)

The server handles `textDocument/definition` for:

| Cursor on                   | Navigates to                                     |
|-----------------------------|--------------------------------------------------|
| Any name binding in scope   | Its `val`/`fun`/`sig`/`type` declaration         |
| Type name in annotation     | Its `type` declaration                           |
| Constructor name            | The parent `type` declaration                    |
| Record field name           | The field entry in the `type` declaration        |
| Module path in `use`        | The module's source file (first line)            |
| Source provider `http.get`  | The provider's definition in stdlib source       |
| `@source` decorator keyword | The source provider definition                   |

### 4.2 `textDocument/typeDefinition` (dedicated command)

Separate from "go to definition of the name", this navigates from *a value* to *the declaration of its type*.

Example: cursor is on `user` in `val user = { ... }`. `typeDefinition` navigates to `type User = { ... }`.

This is the LSP `textDocument/typeDefinition` request. The compiler must return the type's declaration site, not the binding site of the variable.

### 4.3 Stdlib virtual file system

The AIVI standard library is distributed as source files alongside the compiler binary. For navigation to work, the compiler must know the absolute path of every stdlib module source file.

Stdlib source lives at a path discoverable from the binary:

```
$(dirname $(which aivi))/../lib/aivi/std/
├── core/
│   ├── types.aivi     -- Int, Float, Text, Bool, Unit, Bytes
│   ├── option.aivi    -- Option A
│   ├── result.aivi    -- Result E A
│   ├── list.aivi      -- List A
│   ├── signal.aivi    -- Signal A, class Functor, Applicative
│   ├── task.aivi      -- Task E A
│   └── ...
├── network/
│   ├── http.aivi      -- @source http.get, http.post ...
│   └── ...
└── fs/
    └── fs.aivi        -- @source fs.watch, fs.read
```

The compiler exposes the stdlib root path as:

```
aivi stdlib-path   →  /usr/local/lib/aivi/std
```

The LSP server stores this path on startup and uses it to resolve stdlib navigation targets. Stdlib files open as normal files in VSCode (read-only by convention but not enforced at the LSP level).

### 4.4 Module resolution in the compiler

For cross-module `definition` to work, the Rust workspace needs:

1. **Module map**: the compiler builds a map from module path (`aivi.network.http`) to source file path at startup (via `use` resolution or explicit discovery).
2. **Name resolution across modules**: when a `use` brings names into scope, those names carry their source file + span.
3. **`lsp-refs` cross-file**: the `aivi lsp-refs` command accepts a workspace root argument and walks all `.aivi` files to find references across files.

This is a prerequisite for full cross-module navigation. Until it exists, definition works for in-file names and stdlib (pre-indexed), but not for user modules in other files.

### 4.5 Workspace-level definition index

The LSP server maintains a `DefinitionIndex` (in-memory):

```typescript
class DefinitionIndex {
  // file path → list of declared symbols with their ranges
  private fileSymbols: Map<string, LspSymbol[]> = new Map();

  // qualified name → { file, range }
  private nameToLocation: Map<string, { file: string; range: LspRange }> = new Map();

  update(file: string, symbols: LspSymbol[]): void
  lookup(name: string): { file: string; range: LspRange } | undefined
  lookupInFile(file: string, name: string): LspRange | undefined
}
```

This index is populated:
- on startup by running `aivi lsp-symbols` on all `.aivi` files in the workspace
- on each `textDocument/didSave` for the changed file
- on workspace file create/delete/rename events

The stdlib is pre-indexed once at startup (stdlib source files are discovered via `aivi stdlib-path`).

---

## 5. Updated Rust compiler protocol

### 5.1 Extended `LspHoverResult`

```typescript
interface LspHoverResult {
  range:     LspRange;
  contents:  string;         // markdown with command URIs embedded
  typeLinks: LspTypeLink[];  // navigation targets for all type names in contents
}

interface LspTypeLink {
  name:  string;
  file:  string;             // absolute path (stdlib or user file)
  range: LspRange;           // selection range of the declaration name token
}
```

### 5.2 New `aivi lsp-type-def` sub-command

```
aivi lsp-type-def --file <path> --line <l> --col <c> [--text <content>]
```

Returns the declaration site of the *type* of the expression under the cursor.

Output:

```typescript
interface LspTypeDefResult {
  // null if cursor is not on a typed expression or type is primitive/anonymous
  location: { file: string; range: LspRange } | null;
}
```

### 5.3 New `aivi stdlib-path` sub-command

```
aivi stdlib-path
```

Prints the absolute path to the stdlib source directory on stdout (one line). Exits 1 if the stdlib cannot be located.

### 5.4 `aivi lsp-hover` expansion depth parameter

```
aivi lsp-hover --file <path> --line <l> --col <c> --expand-depth <n> [--text <content>]
```

`--expand-depth` defaults to 3. The editor can request shallower expansions for performance. 0 means no expansion (returns signature only).

---

## 6. Extension: `aivi.goToDefinition` command

Registered in `commands.ts`:

```typescript
vscode.commands.registerCommand(
  "aivi.goToDefinition",
  async (args: { file: string; line: number; col: number }) => {
    const uri = vscode.Uri.file(args.file);
    const pos = new vscode.Position(args.line, args.col);
    await vscode.commands.executeCommand(
      "editor.action.goToLocations",
      uri,
      pos,
      [new vscode.Location(uri, pos)],
      "goto",
      "No definition found",
    );
  }
);
```

The command is also exposed as a keybinding context menu entry for any AIVI editor: right-click → "Go to Type Definition".

---

## 7. UX details

### 7.1 Hover popup width

VSCode hover popups are 600px wide by default. Tables with two columns (`Field`, `Type`) render comfortably within this width. Three-level expansions produce a scrollable popup — this is acceptable.

### 7.2 Duplicate type suppression

If the same type appears at multiple levels of the expansion (e.g., `User` has a field of type `User`), the second occurrence renders as:

```markdown
**`User`** &nbsp;·&nbsp; *see above* &nbsp;·&nbsp; [→ definition](command:...)
```

The `visited` set in the expansion algorithm (§2.4.2) prevents infinite recursion.

### 7.3 Hover on already-expanded types

If the user hovers over `DateTime` in a record field position (not on a value that has type `DateTime`, but on the type *name itself*), the hover shows:

- Layer 1: the type's own declaration
- Layer 2: the type's doc comment
- Layer 3: the type's fields/constructors expanded one level

This works because the hover provider fires on `entity.name.type.aivi` tokens regardless of position context.

### 7.4 Keyboard navigation shortcut

The extension adds a keybinding suggestion in `package.json`:

```json
{
  "key": "ctrl+shift+t",
  "command": "aivi.goToTypeDefinition",
  "when": "editorLangId == aivi && editorTextFocus"
}
```

`aivi.goToTypeDefinition` calls `textDocument/typeDefinition` for the symbol under the cursor, which is separate from `textDocument/definition`.

---

## 8. Milestones

| Milestone | Deliverable                                                                        |
|-----------|------------------------------------------------------------------------------------|
| M1        | `aivi lsp-hover` returns flat signature + doc comment (no expansion)               |
| M2        | `LspTypeLink[]` embedded in hover result; clickable type names in popup            |
| M3        | Recursive type expansion (depth 3) for record types                                |
| M4        | Sum type expansion; domain type expansion                                          |
| M5        | `aivi lsp-type-def` + `textDocument/typeDefinition` + Ctrl+Shift+T shortcut        |
| M6        | `aivi stdlib-path` + stdlib source indexed in `DefinitionIndex`                    |
| M7        | Cross-module definition navigation for user `.aivi` files                          |
| M8        | Hover on type-name tokens (not just value sites)                                   |
| M9        | Collapse with "…" at depth overflow; dedup of repeated types                       |
| M10       | Operator reference cards in hover                                                  |
