# aivi.fs

Filesystem capability vocabulary plus the `FsSource` handle type.

Public filesystem access now goes through `@source fs` handles. This module carries the shared error
and event types for that capability family.

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
)
```

## Capability handle

```aivi
value projectRoot : Text = "/tmp/demo"

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
signal changes : Signal FsEvent = files.watch "config.json"
value configExists : Task Text Bool = files.exists "config.json"
value backupBytes : Task Text Bytes = files.readBytes "backup.bin"
value saveBackup : Task Text Unit = files.writeText "backup.txt" "ok"
value removeCache : Task Text Unit = files.delete "cache.txt"
```

## Exported vocabulary

- `FsSource` - nominal handle annotation for `@source fs`.
- `FsError` - structured filesystem failures.
- `FsEvent` - filesystem watch event vocabulary: `Created`, `Changed`, `Deleted`.

## Canonical handle members

| Member | Type | Description |
| --- | --- | --- |
| `files.watch path` | `Signal FsEvent` | Watch a path for created/changed/deleted events |
| `files.read path` | `Signal (Result FsError A)` | Read and decode a file through the source pipeline |
| `files.exists path` | `Task Text Bool` | Check whether a path exists |
| `files.readBytes path` | `Task Text Bytes` | Read raw bytes on demand |
| `files.writeText path text` | `Task Text Unit` | Write text to a file |
| `files.writeBytes path bytes` | `Task Text Unit` | Write bytes to a file |
| `files.createDirAll path` | `Task Text Unit` | Create a directory tree |
| `files.delete path` | `Task Text Unit` | Delete one file |

For option-level support on `fs.watch` and `fs.read`, see the
[Built-in Source Catalog](/guide/source-catalog).

::: tip
The compiler still accepts compatibility spellings such as `readText`, `write`, and `deleteFile`
on handle members. The manual uses `read`, `writeText`, and `delete` as the canonical surface.
:::
