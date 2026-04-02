# aivi.env

Environment vocabulary plus the `EnvSource` capability-handle type.

Environment access now lives on `@source env` handles. The module itself is shared data vocabulary
for that handle family.

## Import

```aivi
use aivi.env (
    EnvSource
    EnvEntry
)
```

## Capability handle

```aivi
@source env
signal environment : EnvSource

signal shell : Signal (Option Text) = environment.get "SHELL"
value xdgVars : Task Text (List EnvEntry) = environment.list "XDG_"
```

## Exported vocabulary

- `EnvSource` - nominal handle annotation for `@source env`.
- `EnvEntry` - one `(name, value)` environment pair.

## Handle members

| Member | Type | Description |
| --- | --- | --- |
| `environment.get key` | `Signal (Option Text)` | Snapshot one environment variable as a signal |
| `environment.list prefix` | `Task Text (List EnvEntry)` | Read matching entries on demand |

For option-level details on `env.get`, see the [Built-in Source Catalog](/guide/source-catalog).
