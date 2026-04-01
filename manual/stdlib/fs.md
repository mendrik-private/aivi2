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

type Text -> Task Text Text
func loadConfig = path=>    readText path
```

### `readBytes : Text -> Task Text Bytes`

Read a file as raw bytes.

```aivi
use aivi.fs (readBytes)

type Text -> Task Text Bytes
func loadIcon = path=>    readBytes path
```

### `readDir : Text -> Task Text (List Text)`

List the names of entries in a directory (filenames only, not full paths).

```aivi
use aivi.fs (readDir)

type Text -> Task Text (List Text)
func listFiles = dir=>    readDir dir
```

### `exists : Text -> Task Text Bool`

Return `True` if the path exists (file or directory).

```aivi
use aivi.fs (exists)

type Text -> Task Text Bool
func checkCachePresent = cacheDir=>    exists cacheDir
```

---

## Write operations

### `writeText : Text -> Text -> Task Text Unit`

Write UTF-8 text to a file, creating it if necessary.

```aivi
use aivi.fs (writeText)

type Text -> Text -> Task Text Unit
func saveConfig = path content=>    writeText path content
```

### `writeBytes : Text -> Bytes -> Task Text Unit`

Write raw bytes to a file.

```aivi
use aivi.fs (writeBytes)

type Text -> Bytes -> Task Text Unit
func saveThumbnail = path data=>    writeBytes path data
```

### `createDirAll : Text -> Task Text Unit`

Create a directory and any missing parent directories (equivalent to `mkdir -p`).

```aivi
use aivi.fs (createDirAll)

type Text -> Task Text Unit
func ensureCacheDir = dir=>    createDirAll dir
```

---

## Mutate operations

### `rename : Text -> Text -> Task Text Unit`

Rename (or move) a file or directory.

```aivi
use aivi.fs (rename)

type Text -> Text -> Task Text Unit
func moveFile = from to=>    rename from to
```

### `copy : Text -> Text -> Task Text Unit`

Copy a file. The destination directory must already exist.

```aivi
use aivi.fs (copy)

type Text -> Text -> Task Text Unit
func backupFile = src dest=>    copy src dest
```

### `deleteFile : Text -> Task Text Unit`

Delete a single file.

```aivi
use aivi.fs (deleteFile)

type Text -> Task Text Unit
func removeTempFile = path=>    deleteFile path
```

### `deleteDir : Text -> Task Text Unit`

Recursively delete a directory and all its contents (equivalent to `rm -rf`).

```aivi
use aivi.fs (deleteDir)

type Text -> Task Text Unit
func cleanCache = cacheDir=>    deleteDir cacheDir
```

---

## Real-world example

```aivi
use aivi.fs (
    readText
    writeText
)
```
::: tip
All `Task Text A` operations integrate with the AIVI scheduler. Background filesystem work never blocks the GTK main thread.
:::
