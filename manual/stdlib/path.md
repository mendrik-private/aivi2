# aivi.path

Lexical path manipulation on `Text` strings. All functions are **synchronous and pure** — they perform no I/O and never touch the filesystem.

```aivi
use aivi.path (
    parent
    filename
    stem
    extension
    join
    isAbsolute
    normalize
)
```

---

## Types

### `Path`

```aivi
type Path = Text
```

A type alias for `Text` that signals intent. Use it in your own types to make path arguments self-documenting.

```aivi
use aivi.path (Path)

type FileRef = {
    path: Path,
    label: Text
}
```

### `PathError`

```aivi
type PathError =
  | InvalidPath Text
  | PathNotFound Text
```

---

## Intrinsics

### `parent : Text -> Option Text`

Return the directory containing this path. Returns `None` for a root or empty path.

```aivi
use aivi.path (parent)
```

### `filename : Text -> Option Text`

Return the final path component, including its extension. Returns `None` for a root path.

```aivi
use aivi.path (filename)
```

### `stem : Text -> Option Text`

Return the final path component without its extension.

```aivi
use aivi.path (stem)
```

### `extension : Text -> Option Text`

Return the extension (characters after the last dot in the filename).

```aivi
use aivi.path (extension)
```

### `join : Text -> Text -> Text`

Append a segment to a base path. If the segment is absolute it replaces the base (POSIX semantics).

```aivi
use aivi.path (join)
```

### `isAbsolute : Text -> Bool`

Return `True` when the path begins with `/`.

```aivi
use aivi.path (isAbsolute)
```

### `normalize : Text -> Text`

Resolve `.` (current directory) and `..` (parent directory) segments lexically, without touching the filesystem.

```aivi
use aivi.path (normalize)
```

---

## Real-world example

```aivi
use aivi.path (
    join
    parent
    stem
    extension
    isAbsolute
)

use aivi.fs (
    readText
    writeText
)

type Text -> Text
func backupPath = originalPath=>    let base

type Text -> Task Text Text
func safeReadConfig = configDir=>    let path
```

::: tip
Combine `aivi.path` with `aivi.fs` for complete file management: use the path functions to build and manipulate path strings, then pass them to filesystem tasks.
:::
