# Plan: AIVI VSCode Extension

## Status: design draft — not yet implemented

---

## 1. Overview

The AIVI VSCode extension (`vscode-aivi`) is a TypeScript/Vite package that provides:

- Syntax highlighting via TextMate grammar
- Language configuration (brackets, comments, folding markers, word pattern)
- LSP client integration (starts the LSP server, routes all editor features through it)
- Snippets
- Custom editor commands
- Status bar indicator
- Extension settings (mapped to LSP server configuration)

The extension is distributed as a `.vsix` file built by `vsce package` and published to the Open VSX Registry (primary) and the VS Marketplace.

---

## 2. Project layout

```
tooling/packages/vscode-aivi/
├── syntaxes/
│   └── aivi.tmLanguage.json     # TextMate grammar
├── snippets/
│   └── aivi.json
├── icons/
│   └── aivi-file.svg            # file icon
├── src/
│   ├── extension.ts             # activate / deactivate entry point
│   ├── client.ts                # LanguageClient setup
│   ├── status.ts                # status bar item
│   ├── commands.ts              # custom commands
│   └── config.ts                # reads workspace/user settings
├── tests/
│   ├── grammar/                 # TextMate grammar unit tests (vscode-tmgrammar-test)
│   │   ├── basic.aivi
│   │   └── ...
│   └── extension.test.ts        # integration tests via @vscode/test-electron
├── package.json                 # extension manifest
├── tsconfig.json
└── vite.config.ts               # bundles extension.ts → dist/extension.js (CJS)
```

---

## 3. Extension manifest (`package.json`)

### 3.1 Identifiers

```json
{
  "name": "vscode-aivi",
  "displayName": "AIVI",
  "description": "AIVI language support — syntax, diagnostics, formatting, and intelligence",
  "version": "0.1.0",
  "publisher": "aivi-lang",
  "engines": { "vscode": "^1.90.0" },
  "categories": ["Programming Languages", "Formatters", "Linters"],
  "icon": "icons/aivi-logo.png",
  "keywords": ["aivi", "functional", "reactive", "gtk", "linux"],
  "license": "MIT"
}
```

### 3.2 Language contribution

```json
{
  "contributes": {
    "languages": [
      {
        "id": "aivi",
        "aliases": ["AIVI", "aivi"],
        "extensions": [".aivi"],
        "configuration": "./language-configuration.json",
        "icon": { "light": "./icons/aivi-file.svg", "dark": "./icons/aivi-file.svg" }
      }
    ],
    "grammars": [
      {
        "language": "aivi",
        "scopeName": "source.aivi",
        "path": "./syntaxes/aivi.tmLanguage.json"
      }
    ],
    "snippets": [
      { "language": "aivi", "path": "./snippets/aivi.json" }
    ]
  }
}
```

### 3.3 Commands

| Command ID                  | Title                        | When clause              |
|-----------------------------|------------------------------|--------------------------|
| `aivi.restartServer`        | AIVI: Restart Language Server | always                  |
| `aivi.showOutputChannel`    | AIVI: Show Output Channel    | always                   |
| `aivi.formatDocument`       | AIVI: Format Document        | `editorLangId == aivi`   |
| `aivi.checkFile`            | AIVI: Check Current File     | `editorLangId == aivi`   |
| `aivi.openCompilerLog`      | AIVI: Open Compiler Log      | always                   |

### 3.4 Settings

```json
{
  "aivi.compiler.path": {
    "type": "string",
    "default": "aivi",
    "description": "Path to the aivi compiler binary."
  },
  "aivi.compiler.args": {
    "type": "array",
    "items": { "type": "string" },
    "default": [],
    "description": "Extra arguments passed to the compiler."
  },
  "aivi.compiler.timeout": {
    "type": "number",
    "default": 5000,
    "description": "Compiler request timeout in milliseconds."
  },
  "aivi.diagnostics.debounceMs": {
    "type": "number",
    "default": 200,
    "description": "Delay before triggering diagnostics after a document change."
  },
  "aivi.inlayHints.enabled": {
    "type": "boolean",
    "default": true,
    "description": "Show inferred type inlay hints."
  },
  "aivi.inlayHints.maxLength": {
    "type": "number",
    "default": 30,
    "description": "Maximum character length of an inlay hint before truncation."
  },
  "aivi.codeLens.enabled": {
    "type": "boolean",
    "default": true,
    "description": "Show code lens annotations (dependencies, references)."
  },
  "aivi.completion.autoImport": {
    "type": "boolean",
    "default": true,
    "description": "Automatically insert `use` declarations for completion items from other modules."
  },
  "aivi.trace.server": {
    "type": "string",
    "enum": ["off", "messages", "verbose"],
    "default": "off",
    "description": "Trace communication between VSCode and the language server."
  },
  "aivi.format.onSave": {
    "type": "boolean",
    "default": false,
    "description": "Automatically format AIVI files on save."
  }
}
```

---

## 4. Language configuration (`language-configuration.json`)

```json
{
  "comments": {
    "lineComment": "--"
  },
  "brackets": [
    ["(", ")"],
    ["{", "}"],
    ["[", "]"],
    ["<", ">"]
  ],
  "autoClosingPairs": [
    { "open": "(", "close": ")" },
    { "open": "{", "close": "}" },
    { "open": "[", "close": "]" },
    { "open": "\"", "close": "\"", "notIn": ["string"] },
    { "open": "<", "close": ">", "notIn": ["string"] }
  ],
  "surroundingPairs": [
    ["(", ")"],
    ["{", "}"],
    ["[", "]"],
    ["\"", "\""],
    ["<", ">"]
  ],
  "wordPattern": "(-?\\d*\\.\\d\\w*)|([^\\`\\~\\!\\@\\#\\%\\^\\&\\*\\(\\)\\-\\=\\+\\[\\{\\]\\}\\\\\\|\\;\\:\\'\\\"\\,\\.\\<\\>\\/\\?\\s]+)",
  "indentationRules": {
    "increaseIndentPattern": "=\\s*$|=>\\s*$|\\{\\s*$|\\(\\s*$",
    "decreaseIndentPattern": "^\\s*[}\\)]"
  },
  "folding": {
    "markers": {
      "start": "^\\s*-- #region",
      "end": "^\\s*-- #endregion"
    }
  },
  "onEnterRules": [
    {
      "beforeText": "=\\s*$",
      "action": { "indent": "indent" }
    }
  ]
}
```

---

## 5. TextMate grammar

The grammar file `aivi.tmLanguage.json` is written in JSON (converted from Plist). It defines token scopes for syntax highlighting in editors that don't use the LSP semantic token layer.

### 5.1 Scope inventory

| TextMate scope                          | Matches                                              |
|-----------------------------------------|------------------------------------------------------|
| `keyword.declaration.aivi`              | `type val fun sig use class instance export provider`|
| `keyword.operator.pipe.aivi`            | `\|>` `?|>` `||>` `*|>` `&|>` `@|>` `<|@` `\|` `<\|*` `T|>` `F|>` |
| `keyword.operator.arrow.aivi`           | `=>`                                                 |
| `keyword.operator.assign.aivi`          | `=`                                                  |
| `keyword.operator.type.aivi`            | `->` `:`                                             |
| `keyword.operator.bar.aivi`             | `\|` (in sum type position)                          |
| `storage.type.aivi`                     | built-in type names: `Int Float Text Bool List ...`  |
| `entity.name.type.aivi`                 | user-defined type names (capitalized identifiers)    |
| `entity.name.function.aivi`             | `fun` binding names                                  |
| `variable.other.aivi`                   | `val` and `sig` binding names                        |
| `entity.name.tag.aivi`                  | markup tag names in `<Tag>` position                 |
| `entity.other.attribute-name.aivi`      | attribute names inside markup tags                   |
| `support.class.aivi`                    | constructor names (capitalized in term position)     |
| `meta.decorator.aivi`                   | `@source` `@recur.timer` `@recur.backoff`            |
| `entity.name.label.aivi`                | source provider paths `http.get`, `timer.every`      |
| `string.quoted.double.aivi`             | text literals `"..."`                                |
| `constant.character.escape.aivi`        | escape sequences inside strings                      |
| `meta.embedded.expression.aivi`         | `{expr}` holes inside interpolated strings           |
| `constant.numeric.integer.aivi`         | integer literals                                     |
| `constant.numeric.float.aivi`           | float literals                                       |
| `constant.numeric.suffix.aivi`          | suffix literals `250ms`, `5s`                        |
| `comment.line.double-dash.aivi`         | `-- ...`                                             |
| `punctuation.definition.record.aivi`    | `{` `}` in record context                            |
| `punctuation.definition.list.aivi`      | `[` `]`                                              |
| `punctuation.definition.tuple.aivi`     | `(` `)`                                              |
| `variable.parameter.aivi`               | labeled parameter names `#x` `#y`                    |
| `meta.record.field.aivi`                | `field: value` pairs                                 |
| `meta.use.aivi`                         | entire `use` declaration                             |

### 5.2 Grammar structure

The grammar uses a single `source.aivi` root scope with the following top-level patterns:

1. `comment` — matches `--` line comments
2. `decorator` — matches `@source`, `@recur.timer`, `@recur.backoff` and captures the decorator arguments
3. `use-declaration` — matches `use module.path (...)` with nested name list
4. `type-declaration` — matches `type Name = ...` with separate patterns for sums vs records
5. `class-declaration` — matches `class Name TypeVar (=> SuperClass)?` and member signatures
6. `instance-declaration` — matches `instance Class Type` and method bindings
7. `fun-declaration` — matches `fun name: RetType #param: ParamType => body`
8. `val-declaration` — matches `val name (: Type)? = body`
9. `sig-declaration` — matches `sig name (: Type)? = body`
10. `pipe-operators` — matches the 11 pipe operators as a standalone pattern included everywhere
11. `markup-tag` — matches `<Tag ...>`, `</Tag>`, `<Tag ... />`
12. `string-interpolated` — matches `"..."` with embedded `{...}` holes
13. `type-expression` — matches type syntax in annotation positions
14. `number-literal` — matches integers, floats, and suffix literals
15. `constructor-ref` — matches uppercase identifiers in term position
16. `labeled-parameter` — matches `#name` in both definition and application positions

---

## 6. Snippets

```json
{
  "Type declaration (sum)": {
    "prefix": "type",
    "body": [
      "type ${1:Name} =",
      "    | ${2:Constructor}"
    ],
    "description": "Declare a sum type"
  },
  "Type declaration (record)": {
    "prefix": "typerec",
    "body": [
      "type ${1:Name} = {",
      "    ${2:field}: ${3:Type},",
      "}"
    ],
    "description": "Declare a record type"
  },
  "Function declaration": {
    "prefix": "fun",
    "body": [
      "fun ${1:name}: ${2:ReturnType} #${3:param}: ${4:ParamType} =>",
      "    ${5:body}"
    ],
    "description": "Declare a function"
  },
  "Value binding": {
    "prefix": "val",
    "body": ["val ${1:name} = ${2:expr}"],
    "description": "Declare a value binding"
  },
  "Signal binding": {
    "prefix": "sig",
    "body": ["sig ${1:name} =", "    ${2:expr}"],
    "description": "Declare a signal"
  },
  "HTTP source signal": {
    "prefix": "source-http",
    "body": [
      "@source http.${1|get,post,put,delete|} \"${2:/path}\" with {",
      "    decode: Strict,",
      "    timeout: ${3:5s},",
      "}",
      "sig ${4:name} : Signal (Result HttpError ${5:Type})"
    ],
    "description": "Declare an HTTP source signal"
  },
  "Timer source signal": {
    "prefix": "source-timer",
    "body": [
      "@source timer.every ${1:1000}",
      "sig ${2:tick} : Signal Unit"
    ],
    "description": "Declare a timer source signal"
  },
  "File watch source": {
    "prefix": "source-watch",
    "body": [
      "@source fs.watch \"${1:path}\" with {",
      "    events: [${2:Changed}],",
      "}",
      "sig ${3:fileEvents} : Signal FsEvent"
    ],
    "description": "Declare a file-watch source signal"
  },
  "Applicative cluster": {
    "prefix": "cluster",
    "body": [
      " &|> ${1:first}",
      " &|> ${2:second}",
      "  |> ${3:Constructor}"
    ],
    "description": "Applicative cluster pipe spine"
  },
  "Case split pipe": {
    "prefix": "cases",
    "body": [
      " ||> ${1:PatternA} => ${2:expr}",
      " ||> ${3:PatternB} => ${4:expr}"
    ],
    "description": "Case split (||>) pipe arms"
  },
  "Class declaration": {
    "prefix": "class",
    "body": [
      "class ${1:Name} ${2:A}",
      "    ${3:method} : ${4:Type}"
    ],
    "description": "Declare a type class"
  },
  "Instance declaration": {
    "prefix": "instance",
    "body": [
      "instance ${1:Class} ${2:Type}",
      "    ${3:method} ${4:args} =",
      "        ${5:body}"
    ],
    "description": "Declare a class instance"
  },
  "Use import": {
    "prefix": "use",
    "body": ["use ${1:module} (${2:name})"],
    "description": "Import from a module"
  },
  "Match markup": {
    "prefix": "match",
    "body": [
      "<match on={${1:expr}}>",
      "    <case pattern={${2:Pattern}}>",
      "        ${3:child}",
      "    </case>",
      "</match>"
    ],
    "description": "Match control node in markup"
  },
  "Each markup": {
    "prefix": "each",
    "body": [
      "<each of={${1:items}} as={${2:item}} key={${2:item}.${3:id}}>",
      "    ${4:child}",
      "</each>"
    ],
    "description": "Each (list rendering) control node"
  },
  "Show markup": {
    "prefix": "show",
    "body": [
      "<show when={${1:condition}}>",
      "    ${2:child}",
      "</show>"
    ],
    "description": "Conditional show control node"
  }
}
```

---

## 7. LSP client (`client.ts`)

Uses `vscode-languageclient` to start and manage the `aivi lsp` process. The server is the `aivi` binary itself — no bundled Node.js server module.

```typescript
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

function createClient(context: vscode.ExtensionContext): LanguageClient {
  const config = vscode.workspace.getConfiguration("aivi");
  const aiviPath = config.get<string>("compiler.path") ?? "aivi";
  const extraArgs = config.get<string[]>("compiler.args") ?? [];

  const serverOptions: ServerOptions = {
    // Run `aivi lsp` as a subprocess, communicate over stdio
    run: {
      command:   aiviPath,
      args:      ["lsp", ...extraArgs],
      transport: TransportKind.stdio,
    },
    // Debug: same command with verbose logging to a temp file
    debug: {
      command:   aiviPath,
      args:      ["lsp", "--log", "/tmp/aivi-lsp-debug.log", "--log-level", "debug", ...extraArgs],
      transport: TransportKind.stdio,
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "aivi" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.aivi"),
    },
    // initializationOptions are read by the Rust server from the LSP initialize request
    initializationOptions: {
      diagnosticsDebounceMs: config.get<number>("diagnostics.debounceMs") ?? 200,
      inlayHintsEnabled:     config.get<boolean>("inlayHints.enabled") ?? true,
      codeLensEnabled:       config.get<boolean>("codeLens.enabled") ?? true,
      completionAutoImport:  config.get<boolean>("completion.autoImport") ?? true,
    },
    outputChannel:      vscode.window.createOutputChannel("AIVI"),
    traceOutputChannel: vscode.window.createOutputChannel("AIVI Trace"),
    markdown: { isTrusted: true, supportHtml: false },
  };

  return new LanguageClient("aivi", "AIVI Language Server", serverOptions, clientOptions);
}
```

The client restarts the server when `aivi.compiler.path` or `aivi.compiler.args` configuration changes. If the binary is not found, a one-time notification prompts the user to install it.

---

## 8. Status bar (`status.ts`)

A status bar item at `StatusBarAlignment.Left` with priority 10 shows:

| State                   | Display                        |
|-------------------------|--------------------------------|
| Server starting         | `$(loading~spin) AIVI`         |
| Server running, no errors | `$(check) AIVI`              |
| Server running, errors  | `$(error) AIVI (N errors)`     |
| Server crashed          | `$(alert) AIVI — click to restart` |
| Formatting in progress  | `$(loading~spin) AIVI: formatting` |

Clicking the status bar item opens the AIVI output channel.

---

## 9. Commands (`commands.ts`)

### `aivi.restartServer`

1. Stops the current `LanguageClient` (calls `client.stop()`).
2. Starts a new one.
3. Shows the output channel.

### `aivi.formatDocument`

Calls `vscode.commands.executeCommand("editor.action.formatDocument")`. Provided so it can be bound to a keybinding independently.

### `aivi.checkFile`

Saves the active document, then calls `aivi check <path>` in a terminal. The output is shown in the AIVI output channel.

### `aivi.showOutputChannel`

Reveals the AIVI output channel panel.

### `aivi.openCompilerLog`

Opens the most recent compiler log file (written to the extension's storage path) in a new editor tab.

---

## 10. Format on save

When `aivi.format.onSave` is `true`, the extension registers a `vscode.workspace.onWillSaveTextDocument` listener that calls `textDocument/formatting` and applies the edits before the file is written.

This is implemented via the standard LSP formatting integration: the extension does not implement formatting itself; it delegates entirely to `aivi lsp`, which calls the formatter directly in-process.

---

## 11. File icons

A custom file icon theme contribution (optional) that associates `.aivi` files with a distinctive icon in the VSCode file explorer. Implemented as a minimal `iconTheme` contribution pointing to a single SVG.

---

## 12. Dependencies

### Runtime dependencies

| Package                       | Purpose                              |
|-------------------------------|--------------------------------------|
| `vscode-languageclient`       | LSP client protocol                  |

### Dev dependencies

| Package                        | Purpose                                      |
|--------------------------------|----------------------------------------------|
| `@types/vscode`                | VSCode API types                             |
| `@vscode/test-cli`             | Integration test runner                      |
| `@vscode/test-electron`        | Electron-based test host for VSCode          |
| `vscode-tmgrammar-test`        | TextMate grammar unit testing                |
| `vsce`                         | Extension packaging and publishing           |
| `typescript`                   | Compilation                                  |
| `vite`                         | Bundling                                     |
| `vitest`                       | Unit tests                                   |

---

## 13. Build and packaging

```
pnpm build                  # bundles vscode-aivi extension only
pnpm package                # runs vsce package → aivi-X.Y.Z.vsix
pnpm publish                # publishes to Open VSX + VS Marketplace
```

The Vite config bundles only `extension.ts` to `dist/extension.js` (CJS, `external: ["vscode"]`). There is no bundled server — the extension starts `aivi lsp` as an external process.

The `aivi` binary is **not** bundled in the VSIX. Users install it separately (via distro package, or `cargo install aivi`). The extension shows a one-time notification if `aivi` is not found on `PATH`, with a link to installation instructions.

---

## 14. TextMate grammar tests

Each `.aivi` test file in `tests/grammar/` uses the `vscode-tmgrammar-test` format:

```aivi
-- SYNTAX TEST "source.aivi"

type Bool = True | False
--   ^^^^ entity.name.type.aivi
--          ^^^^ support.class.aivi
--                  ^^^^^ support.class.aivi

fun add: Int #x: Int #y: Int => x + y
-- ^^^ keyword.declaration.aivi
--     ^^^ entity.name.function.aivi
--             ^^ variable.parameter.aivi
```

Grammar tests run as part of `pnpm test`.

---

## 15. Milestones

| Milestone | Deliverable                                                                 |
|-----------|-----------------------------------------------------------------------------|
| M1        | TextMate grammar covering all token types + grammar tests                   |
| M2        | Language configuration (brackets, comments, word pattern)                   |
| M3        | Snippets for all top-level forms and common patterns                        |
| M4        | LSP client integration (diagnostics visible in editor)                      |
| M5        | Status bar indicator                                                        |
| M6        | Format on save + `aivi.formatDocument` command                              |
| M7        | Extension commands (restart, check, log)                                    |
| M8        | Settings contribution with full schema                                      |
| M9        | VSIX packaging + CI publish workflow                                        |
| M10       | File icon theme                                                             |
