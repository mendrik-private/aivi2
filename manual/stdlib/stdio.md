# aivi.stdio

Standard I/O vocabulary plus the `StdioSource` capability-handle type.

Public stdio work now goes through `@source stdio` handles.

## Import

```aivi
use aivi.stdio (
    StdioSource
    WriteError
    StdioUnavailable
    Stream
    Stdout
    Stderr
)
```

## Capability handle

```aivi
@source stdio
signal console : StdioSource

signal stdinText : Signal Text = console.read
value prompt : Task Text Unit = console.stdoutWrite "Name: "
value failure : Task Text Unit = console.stderrWrite "Missing config\n"
```

## Exported vocabulary

- `StdioSource` - nominal handle annotation for `@source stdio`.
- `WriteError` / `StdioUnavailable` - stdio failure vocabulary.
- `Stream`, `Stdout`, `Stderr` - stream-selection vocabulary.

`console.read` is the source-backed snapshot side. `console.stdoutWrite` and
`console.stderrWrite` are the command side and return ordinary `Task Text Unit` values.
