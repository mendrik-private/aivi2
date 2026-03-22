# Plan: AIVI Formatter

## Status: design draft — not yet implemented

---

## 1. Overview

The AIVI formatter is a **canonical, opinionated, idempotent pretty-printer** for AIVI source files. It is the single source of truth for what valid AIVI code looks like; there are no style options beyond what is listed under §6.

The formatter is implemented entirely in Rust inside `crates/aivi-syntax/src/format.rs`. There is no TypeScript wrapper. The formatter is exposed as sub-commands of the `aivi` binary:

```
aivi fmt [file...]        # format files in-place
aivi fmt --check          # exit 1 if any file would change, print changed paths
aivi fmt --stdin          # read from stdin, write formatted output to stdout
```

The LSP server (`aivi lsp`) calls `aivi_syntax::format::format_cst(...)` directly — no subprocess, no TypeScript indirection. The VSCode extension triggers formatting through the standard LSP `textDocument/formatting` request.

---

## 2. Design principles

- **CST-based**: formatting operates on the lossless CST, not the AST. Comments, blank lines between top-level items, and source trivia are preserved where the rules permit.
- **Idempotent**: `fmt(fmt(x)) == fmt(x)` is a hard requirement. The test suite enforces this on every fixture.
- **Canonical**: given the same semantics, there is exactly one canonical formatting. No options, no `.aivirc` for style.
- **Error-tolerant**: the formatter formats the portions of a file it understands even when the file contains parse errors, emitting the unformatted fragment verbatim in the error region.
- **Span-preserving for diagnostics**: reformatted output preserves semantic content byte-for-byte; only whitespace changes.

---

## 3. Formatting rules

### 3.1 Top-level declarations

- One blank line between consecutive top-level declarations.
- Two blank lines before a `class` or `instance` block.
- No trailing blank line at end of file (single trailing newline).
- `use` declarations are sorted (lexicographically by module path) and grouped: a blank line separates groups of `use` from other declarations.
- Consecutive `use` declarations for the same module are merged into one.

```aivi
-- before
use aivi.network (http)
use aivi.fs (read)
use aivi.network (socket)

-- after
use aivi.fs (
    read
)
use aivi.network (
    http
    socket
)
```

### 3.2 `use` imports

Multi-name imports are always written on separate indented lines (2-space indent), sorted lexicographically:

```aivi
use aivi.network (
    http
    socket
    websocket
)
```

Single-name imports use the one-line form:

```aivi
use aivi.fs (read)
```

### 3.3 `type` declarations

Product types on one line if they fit within the line width (100 chars):

```aivi
type Vec2 = Vec2 Int Int
```

Sum types: each constructor on its own line, aligned under the `=`:

```aivi
type Status =
    | Pending
    | Paid Amount
    | Failed Text
```

Record types: each field on its own line, indented:

```aivi
type User = {
    name:     Text,
    age:      Int,
    nickname: Option Text,
}
```

Field type annotations are column-aligned when the type names form a readable column (within 4 chars of the longest label). A trailing comma after the last field is always emitted.

### 3.4 `class` and `instance` blocks

```aivi
class Functor F
    map : (A -> B) -> F A -> F B

instance Functor Option
    map f opt =
        opt
         ||> None   => None
         ||> Some a => Some (f a)
```

- 4-space indent inside `class` and `instance` bodies.
- One blank line between method signatures in a `class`.

### 3.5 `val` and `fun`

`val` with a short body: one line.

```aivi
val answer = 42
```

`val` with a long body: the `=` stays on the first line; the body starts on the next line with 4-space indent:

```aivi
val longValue =
    someVeryLongExpression
     |> transform
     |> finish
```

`fun` always writes the return type and parameters on the first line. If parameters fit, one line:

```aivi
fun add: Int #x: Int #y: Int => x + y
```

If parameters overflow 100 chars, each parameter goes on its own line, indented 4:

```aivi
fun buildRequest: Request
    #method: Method
    #url:    Text
    #headers: Map Text Text =>
    { method, url, headers }
```

### 3.6 `sig` and `@source`

```aivi
@source http.get "/users" with {
    decode: Strict,
    timeout: 5s,
}
sig users : Signal (Result HttpError (List User))
```

- `@source` and `sig` always appear as a pair on consecutive lines.
- The `with { ... }` options block is always multi-line with a trailing comma and 4-space indent.
- One-line `with` blocks are expanded when they contain more than one option.

### 3.7 Pipe spines

This is the most important formatting rule. A pipe spine is a sequence of pipe operator stages (`|>`, `?|>`, `||>`, `*|>`, `&|>`, `@|>`, `<|@`, `|`, `<|*`, `T|>`, `F|>`).

**Rule:** every pipe operator is placed at the start of its own line, indented 1 space past the base of the subject expression. The operator is right-aligned in a column shared with the other operators in the same spine.

```aivi
order
 |> .status
 ||> Paid    => "paid"
 ||> Pending => "pending"
```

```aivi
users
 ?|> .active
 *|> .email
 <|* Text.join ", "
```

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

Alignment column is `max(len(op) for op in spine)` so that all operators in a spine are right-aligned:

```
 &|>  validateName nameText
 &|>  validateEmail emailText
  |>  UserDraft
```

`||>` arms: each arm is formatted as `||> Pattern => body`. If the body is short, one line. If the body is long, the body goes on the next line indented 8 spaces from the base:

```aivi
result
 ||> Ok a =>
        a
         |> process
         |> finish
 ||> Err e => showError e
```

`T|>` / `F|>` pairs: always adjacent:

```aivi
ready
 T|> start
 F|> wait
```

### 3.8 Record literals

Short records on one line (≤ 60 chars total):

```aivi
{ name: "Ada", age: 36 }
```

Long records multi-line with trailing comma:

```aivi
{
    name: "Ada",
    age: 36,
    nickname: None,
}
```

Record shorthand is preserved as-is: `{ name, age }` is not expanded to `{ name: name, age: age }`.

### 3.9 List, tuple, map, set literals

Lists: inline if short, multi-line with trailing comma if long:

```aivi
[1, 2, 3]

[
    "very long element one",
    "very long element two",
]
```

Tuples: always inline.

Map / Set: multi-line with trailing comma:

```aivi
Map {
    "x": 1,
    "y": 2,
}
```

### 3.10 Markup

Each child element goes on its own line, indented 4 spaces:

```aivi
<Box orientation={Vertical}>
    <Label text={title} />
    <show when={isVisible}>
        <Label text="Ready" />
    </show>
    <each of={items} as={item} key={item.id}>
        <Row item={item} />
    </each>
</Box>
```

Self-closing tags with no attributes: `<Label />`.
Self-closing tags with one short attribute: `<Label text="Hi" />`.
Tags with multiple attributes: attributes on the same line if total ≤ 100 chars, else each attribute on its own line with 4-space indent:

```aivi
<Button
    label="Click me"
    sensitive={isEnabled}
    onClicked={handleClick}
/>
```

### 3.11 Comments

Line comments (`--`) are preserved verbatim. Their placement (before a declaration, inline after an expression) is preserved. Blank lines between a comment and its target are collapsed to zero.

Block comments are not currently in the syntax spec; if added, they would be formatted with alignment.

### 3.12 String literals and interpolation

Interpolated strings are kept on one line unless they exceed 100 chars, in which case the formatter does not reflow them (interpolated content cannot be safely split across lines without semantic change).

### 3.13 Operators and spacing

All binary operators have one space on each side: `x + y`, `x == y`, `A -> B`.
Function application has no space between function and argument: `f x y`.
Constructor application: `Some x`, `Ok value`, `Err e`.

### 3.14 Number and suffix literals

`1_000_000` underscores for long integers: the formatter normalizes integer literals ≥ 10000 to use `_` separators in groups of 3 (can be configured off). Suffix literals `250ms` are left as-is.

---

## 4. Rust formatter requirements

The existing `crates/aivi-syntax/src/format.rs` needs the following to be complete:

1. **All grammar productions** covered: the formatter must handle every node type that the parser produces, including:
   - pipe operator spines with all 11 operators
   - `@source` / `with { ... }` forms
   - markup nodes (`<tag>`, `<show>`, `<each>`, `<match>`, `<empty>`, `<case>`)
   - `@recur.timer` / `@recur.backoff` decorators
   - interpolated text `"... {expr} ..."`
   - suffix literals `250ms`
   - `domain` declarations
   - `provider` declarations

2. **Error-tolerant output**: when a CST node is of kind `Error`, emit the original source text for that span verbatim. The surrounding well-formed nodes are still formatted normally.

3. **`--stdin` mode**: `aivi fmt --stdin` reads from stdin, formats, writes to stdout. Exit 0 on success, exit 1 on internal error (not on parse errors, which are tolerated).

4. **`--check` mode**: `aivi fmt --check [file...]` exits 0 if nothing would change, 1 if any file would change (with the file paths printed to stdout).

5. **Line-width**: 100 columns (not configurable; canonical).

6. **Output**: always UTF-8 with Unix line endings (`\n`).

---

## 6. Non-configurable choices (intentional)

These are **not** options. They are the canonical style:

- 4-space indentation
- 100-column line width
- trailing commas in multi-line records, lists, options
- pipe operators right-aligned in their spine column
- `use` imports sorted and merged
- no tabs
- no semicolons
- single trailing newline at EOF

There is no `.aivirc`, no `--indent` flag, no `--trailing-comma=avoid`. The formatter is canonical by design.

---

## 7. Testing strategy

### 7.1 Round-trip tests

For each fixture in `tests/fixtures/input/`:
1. Format the file.
2. Assert output matches `tests/fixtures/expected/<name>.aivi`.
3. Format the output again.
4. Assert output is unchanged (idempotency).

### 7.2 Rust-level unit tests

In `crates/aivi-syntax/src/format.rs`:
- One test per grammar production exercising the formatting of that node type.
- Property test: `forall src: String. is_valid(src) => fmt(fmt(src)) == fmt(src)` using `proptest` with a grammar-aware string generator.

### 7.3 Error-tolerant tests

Fixtures with deliberate parse errors: assert that the formatter outputs the valid parts correctly and leaves the error regions verbatim.

### 7.4 Regression fixtures

One fixture per reported formatting bug, named after the issue.

---

## 8. Integration with LSP server

The LSP server (`aivi lsp`) calls `aivi_syntax::format::format_cst(cst, &FormatOptions::default())` directly from its `formatting.rs` handler. No subprocess. No TypeScript. The formatted text is diffed against the original using a Myers line-diff to produce minimal `TextEdit` objects for the `textDocument/formatting` response.

For range formatting, the formatter formats the entire document and then trims the edits to the requested range. This is the only correct approach for a pipe-spine language where indentation is sensitive to context.

---

## 9. Milestones

| Milestone | Deliverable                                                              |
|-----------|--------------------------------------------------------------------------|
| M1        | Rust formatter covers all CST node types; idempotency test suite passes  |
| M2        | `aivi fmt --stdin` and `aivi fmt --check` modes                          |
| M3        | Integration with LSP server `textDocument/formatting`                    |
| M4        | Range formatting                                                          |
| M5        | Golden fixture test suite (≥ 30 fixtures covering all production rules)  |
