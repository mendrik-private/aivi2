# aivi.log

Runtime logging helpers.

`aivi.log` gives you small helpers for writing diagnostic messages. The built-in runtime log
sink currently writes to standard error, so even `debug` and `info` messages show up on the
terminal error stream today.

## Import

```aivi
use aivi.log (
    LogLevel
    Debug
    Warn
    levelToText
    emit
    emitContext
    info
    warnCtx
    kv
)
```

## Overview

| Value | Type | Description |
| --- | --- | --- |
| `emit` | `Text -> Text -> Task Text Unit` | Write a log line at a named level |
| `emitContext` | `Text -> Text -> List (Text, Text) -> Task Text Unit` | Write a log line with extra key/value fields |
| `debug` | `Text -> Task Text Unit` | Shorthand for `emit "DEBUG" ...` |
| `info` | `Text -> Task Text Unit` | Shorthand for `emit "INFO" ...` |
| `warn` | `Text -> Task Text Unit` | Shorthand for `emit "WARN" ...` |
| `error` | `Text -> Task Text Unit` | Shorthand for `emit "ERROR" ...` |
| `fatal` | `Text -> Task Text Unit` | Shorthand for `emit "FATAL" ...` |
| `debugCtx` | `Text -> List (Text, Text) -> Task Text Unit` | Debug message with context |
| `infoCtx` | `Text -> List (Text, Text) -> Task Text Unit` | Info message with context |
| `warnCtx` | `Text -> List (Text, Text) -> Task Text Unit` | Warning message with context |
| `errorCtx` | `Text -> List (Text, Text) -> Task Text Unit` | Error message with context |
| `levelToText` | `LogLevel -> Text` | Convert a typed level to uppercase text |
| `kv` | `Text -> Text -> (Text, Text)` | Build one context pair |

## Log levels

```aivi
type LogLevel =
  | Debug
  | Info
  | Warn
  | Error
  | Fatal
```

Use `LogLevel` when you want a typed level in your own data, then turn it into text with
`levelToText` when needed.

```aivi
use aivi.log (
    LogLevel
    Warn
    levelToText
)

value currentLevel : LogLevel = Warn
value currentLevelText : Text = levelToText currentLevel
```

## Core functions

### `emit`

```aivi
emit : Text -> Text -> Task Text Unit
```

Write one log message.

Today the runtime prints it in this shape:

```text
[LEVEL] message
```

```aivi
use aivi.log (emit)

value started : Task Text Unit = emit "INFO" "Sync started"
```

### `emitContext`

```aivi
emitContext : Text -> Text -> List (Text, Text) -> Task Text Unit
```

Write a message plus extra key/value fields.

Today the runtime prints it in this shape:

```text
[LEVEL] message {key=value, key=value}
```

```aivi
use aivi.log (
    emitContext
    kv
)

value requestLog : Task Text Unit =
    emitContext "INFO" "Loaded profile" [
        kv "userId" "42",
        kv "source" "cache"
    ]
```

## Convenience helpers

For the common levels you can skip the explicit level string:

```aivi
use aivi.log (
    info
    warnCtx
    kv
)

value bootMessage : Task Text Unit = info "App started"

value slowQuery : Task Text Unit =
    warnCtx "Query is taking longer than expected" [
        kv "mailbox" "primary",
        kv "retry" "2"
    ]
```

There is currently no `fatalCtx` helper. For a fatal message with context, call
`emitContext "FATAL" ...` directly.

## Supporting types

```aivi
type LogContext = (Dict Text Text)

type LogEntry = {
    level: LogLevel
}

type LogError = Text
type LogTask = (Task LogError Unit)
type LogSink = (LogEntry -> LogTask)
```

These are useful when you want logging-related data in your own types and function
signatures.

Note that `emitContext` currently takes a `List (Text, Text)`, not a `LogContext` dictionary.
The `kv` helper exists to make that current runtime shape easier to build.

## Example — a small logging policy

```aivi
use aivi.log (
    errorCtx
    kv
)

value saveFailed : Task Text Unit =
    errorCtx "Could not save draft" [
        kv "mailbox" "primary",
        kv "action" "autosave"
    ]
```
