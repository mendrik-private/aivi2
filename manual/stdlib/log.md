# aivi.log

Logging vocabulary plus the `LogSource` capability-handle type.

`aivi.log` exports typed levels, log-entry data, and pure helpers. Emitting logs happens through
`@source log` handles.

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

value started : Task LogError Unit = logger.emit levelInfo "Started"
value slowQuery : Task LogError Unit =
    logger.emitContext levelWarn "Slow query" [
        kv "mailbox" "primary"
    ]
```

## Exported vocabulary

- `LogSource` - nominal handle annotation for `@source log`.
- `LogLevel` - typed severity values.
- `LogContext` - `Map Text Text`.
- `LogEntry` - `{ level, message, context }`.
- `LogError` - current log command failure surface.
- `LogSink` - `LogEntry -> Task LogError Unit`.
- `level*`, `levelToText`, and `kv` - pure helper values.
