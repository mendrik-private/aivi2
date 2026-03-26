# Playground

The browser playground is planned for a future milestone.

## What the playground will do

When complete, the playground page will embed a live AIVI coding environment directly in the
browser — no installation required:

- **Live editor** with full AIVI syntax highlighting (using the same TextMate grammar as the
  VSCode extension and this documentation site).
- **Real compiler errors** as you type, sourced from the AIVI compiler compiled to WASM.
  Error squiggles, error messages, source locations — all the same diagnostics you get locally.
- **Format button** that runs `aivi fmt` on your code in the browser.
- **Example selector** pre-populated from the `demos/` directory: Snake, Counter, Todo, and more.
- **Share links** — encode the current editor content into the URL so you can share examples.

## How it will work

The AIVI compiler will be compiled to WebAssembly using `wasm-bindgen`.
A thin `aivi-playground` crate will expose two functions:

- `check(source)` — returns a list of diagnostics as JSON.
- `format(source)` — returns the formatted source.

The playground page loads the WASM module lazily (it does not affect initial page load) and
uses CodeMirror 6 as the editor with the AIVI grammar for highlighting.

## Try AIVI today

While the playground is being built, you can try AIVI locally:

1. Clone the repository: `git clone https://github.com/mendrik/aivi2`
2. Build the compiler: `cargo build --release`
3. Install the binary: `cp target/release/aivi ~/.local/bin/`
4. Create a file `hello.aivi`:

```aivi
use aivi.stdio (
    stdoutWrite
)

val main : Task Text Unit =
    stdoutWrite "hello from AIVI"
```

5. Run it: `aivi execute hello.aivi`

The [VSCode extension](https://github.com/mendrik/aivi2/tree/main/tooling/packages/vscode-aivi)
provides syntax highlighting and error reporting while you edit.

Use `aivi run` when your entrypoint is GTK markup. Use `aivi execute` when your entrypoint is a
headless `Task`.
