# aivi.fs

Filesystem operations for reading, writing, and managing files and directories. All functions return `Task Text A` — they are asynchronous and run on worker threads so the UI thread stays responsive.

Import what you need:

```aivi
use aivi.fs (
    readText
    writeText
    readDir
    exists
    rename
    copy
    deleteFile
    deleteDir
)
```

---

## Types

### `FsError`

```aivi
type FsError =
  | NotFound Text
  | PermissionDenied Text
  | ReadFailed Text
  | WriteFailed Text
  | FsProtocolError Text
```

Structured error variants you can pattern-match on.

### `FsEvent`

```aivi
type FsEvent =
  | Created
  | Changed
  | Deleted
```

Event kind for filesystem watching (used with source-backed signals).

---

## Read operations

### `readText : Text -> Task Text Text`

Read a file as UTF-8 text.

```aivi
use aivi.fs (readText)

fun loadConfig: Task Text Text path:Text =>
    readText path
```

### `readBytes : Text -> Task Text Bytes`

Read a file as raw bytes.

```aivi
use aivi.fs (readBytes)

fun loadIcon: Task Text Bytes path:Text =>
    readBytes path
```

### `readDir : Text -> Task Text (List Text)`

List the names of entries in a directory (filenames only, not full paths).

```aivi
use aivi.fs (readDir)

fun listFiles: Task Text (List Text) dir:Text =>
    readDir dir
```

### `exists : Text -> Task Text Bool`

Return `True` if the path exists (file or directory).

```aivi
use aivi.fs (exists)

fun checkCachePresent: Task Text Bool cacheDir:Text =>
    exists cacheDir
```

---

## Write operations

### `writeText : Text -> Text -> Task Text Unit`

Write UTF-8 text to a file, creating it if necessary.

```aivi
use aivi.fs (writeText)

fun saveConfig: Task Text Unit path:Text content:Text =>
    writeText path content
```

### `writeBytes : Text -> Bytes -> Task Text Unit`

Write raw bytes to a file.

```aivi
use aivi.fs (writeBytes)

fun saveThumbnail: Task Text Unit path:Text data:Bytes =>
    writeBytes path data
```

### `createDirAll : Text -> Task Text Unit`

Create a directory and any missing parent directories (equivalent to `mkdir -p`).

```aivi
use aivi.fs (createDirAll)

fun ensureCacheDir: Task Text Unit dir:Text =>
    createDirAll dir
```

---

## Mutate operations

### `rename : Text -> Text -> Task Text Unit`

Rename (or move) a file or directory.

```aivi
use aivi.fs (rename)

fun moveFile: Task Text Unit from:Text to:Text =>
    rename from to
```

### `copy : Text -> Text -> Task Text Unit`

Copy a file. The destination directory must already exist.

```aivi
use aivi.fs (copy)

fun backupFile: Task Text Unit src:Text dest:Text =>
    copy src dest
```

### `deleteFile : Text -> Task Text Unit`

Delete a single file.

```aivi
use aivi.fs (deleteFile)

fun removeTempFile: Task Text Unit path:Text =>
    deleteFile path
```

### `deleteDir : Text -> Task Text Unit`

Recursively delete a directory and all its contents (equivalent to `rm -rf`).

```aivi
use aivi.fs (deleteDir)

fun cleanCache: Task Text Unit cacheDir:Text =>
    deleteDir cacheDir
```

---

## Real-world example

```
use aivi.fs (
    readText
    writeText
)
```
::: tip
All `Task Text A` operations integrate with the AIVI scheduler. Background filesystem work never blocks the GTK main thread.
:::
