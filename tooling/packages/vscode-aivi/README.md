# AIVI for VS Code

Official VS Code support for the AIVI language.

## Features

- AIVI syntax highlighting and snippets
- Semantic highlighting
- Diagnostics, hover, go-to-definition, symbols, and completions through the AIVI language server
- Document formatting for `.aivi` files
- An included AIVI editor theme tuned to the extension's syntax colors

## Formatter and language server

Opening an `.aivi` file activates the extension and starts the AIVI language server.

The extension also sets itself as the default formatter for AIVI files:

- `Format Document` uses the AIVI formatter
- `AIVI: Format Document` invokes formatting directly for the active AIVI editor
- `aivi.format.onSave` enables format-on-save for AIVI files

If formatting or diagnostics stop responding, run `AIVI: Restart Language Server`.

## License

This VS Code extension is distributed under `GPL-3.0-only`.

The full license text is included in the packaged `licence.txt` file.
