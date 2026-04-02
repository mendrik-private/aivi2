# aivi.fs

Filesystem error/event vocabulary plus the `FsSource` capability-handle type.

`aivi.fs` no longer exports public read/write/delete functions. Use `@source fs ...` so reads,
watches, and commands stay on one provider boundary.

## Import

```aivi
use aivi.fs (
    FsSource
    FsError
    NotFound
    PermissionDenied
    ReadFailed
    WriteFailed
    FsProtocolError
    FsEvent
    Created
    Changed
    Deleted
    FsReadTask
    FsWriteTask
    FsBytesTask
    FsDirTask
    FsExistsTask
    FsUnitTask
)
```

## Capability handle

```aivi
@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
signal changes : Signal FsEvent = files.watch "config.json"

value configExists : FsExistsTask = files.exists "config.json"
value removeCache : FsUnitTask = files.deleteFile "cache.tmp"
```

## Exported types

- `FsError` — structured source/read failure vocabulary.
- `FsEvent` — filesystem watch events.
- `FsSource` — nominal handle annotation for `@source fs ...`.
- `Fs*Task` aliases — current one-shot command/read task shapes.

`FsError` is the typed signal/result vocabulary. One-shot `value` members still lower through the
current task intrinsic path, so the exported `Fs*Task` aliases continue to use `Task Text ...`.
