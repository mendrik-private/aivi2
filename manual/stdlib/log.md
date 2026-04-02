# aivi.log

Logging vocabulary plus the `LogSource` capability-handle type.

`aivi.log` now exports typed levels, log-entry data, and pure helpers. Emit operations happen
through `@source log`.

## Import

```aivi
use aivi.log (
    LogSource
    LogLevel
    Debug
    Info
    Warn
    Error
    Fatal
    LogContext
    LogEntry
    LogError
    LogTask
    LogSink
    levelDebug
    levelInfo
    levelWarn
    levelError
    levelFatal
    levelToText
    kv
)
```

## Capability handle

```aivi
@source log
signal logger : LogSource

value started : LogTask = logger.emit levelInfo "Started"
value slowQuery : LogTask =
    logger.emitContext levelWarn "Slow query" [
        kv "mailbox" "primary"
    ]
```

## Exported vocabulary

- `LogLevel` — typed severity values.
- `LogContext` — `Map Text Text`.
- `LogEntry` — `{ level, message, context }`.
- `LogTask` / `LogSink` — current command/task aliases.
- `level*`, `levelToText`, and `kv` — pure helper values.
