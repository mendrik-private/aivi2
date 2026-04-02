# aivi.env

Environment capability vocabulary plus the `EnvSource` handle type.

Environment lookups now belong to `@source env`, not to public `get`/`list` imports.

## Import

```aivi
use aivi.env (
    EnvSource
    EnvEntry
    EnvLookupTask
    EnvListTask
)
```

## Capability handle

```aivi
@source env
signal environment : EnvSource

signal shell : Signal (Option Text) = environment.get "SHELL"
value xdgVars : EnvListTask = environment.list "XDG_"
```

## Exported vocabulary

- `EnvSource` — nominal handle annotation for `@source env`.
- `EnvEntry` — one `(name, value)` environment pair.
- `EnvLookupTask` — current one-shot lookup task shape.
- `EnvListTask` — current one-shot prefix-list task shape.
