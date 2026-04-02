# aivi.stdio

Standard I/O vocabulary plus the `StdioSource` capability-handle type.

Public `stdoutWrite` / `stderrWrite` imports have been folded into `@source stdio`.

## Import

```aivi
use aivi.stdio (
    StdioSource
    WriteError
    StdioUnavailable
    Stream
    Stdout
    Stderr
    StdioTask
    StdoutTask
    StderrTask
)
```

## Capability handle

```aivi
@source stdio
signal console : StdioSource

signal stdinText : Signal Text = console.read
value prompt : StdoutTask = console.stdoutWrite "Name: "
value failure : StderrTask = console.stderrWrite "Missing config\n"
```

## Exported vocabulary

- `StdioSource` — nominal handle annotation for `@source stdio`.
- `WriteError` / `StdioUnavailable` — stdio failure vocabulary.
- `Stream`, `Stdout`, `Stderr` — stream-selection vocabulary.
- `StdioTask`, `StdoutTask`, `StderrTask` — current one-shot task aliases.
