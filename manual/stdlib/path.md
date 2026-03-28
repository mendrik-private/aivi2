# aivi.path

Lexical path manipulation on `Text` strings. All functions are **synchronous and pure** — they perform no I/O and never touch the filesystem.

```aivi
use aivi.path (parent, filename, stem, extension, join, isAbsolute, normalize)
```

---

## Types

### `Path`

```aivi
type Path = Text
```

A type alias for `Text` that signals intent. Use it in your own types to make path arguments self-documenting.

```aivi
type FileRef = { path: Path, label: Text }
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

parent "/home/user/docs/notes.txt"   -- Some "/home/user/docs"
parent "/home"                        -- Some "/"
parent "/"                            -- None
```

### `filename : Text -> Option Text`

Return the final path component, including its extension. Returns `None` for a root path.

```aivi
use aivi.path (filename)

filename "/home/user/notes.txt"  -- Some "notes.txt"
filename "/home/user/"           -- None
```

### `stem : Text -> Option Text`

Return the final path component without its extension.

```aivi
use aivi.path (stem)

stem "/home/user/notes.txt"  -- Some "notes"
stem "/home/user/archive"    -- Some "archive"
```

### `extension : Text -> Option Text`

Return the extension (characters after the last dot in the filename).

```aivi
use aivi.path (extension)

extension "/home/user/photo.jpg"   -- Some "jpg"
extension "/home/user/Makefile"    -- None
extension "/home/user/archive.tar.gz" -- Some "gz"
```

### `join : Text -> Text -> Text`

Append a segment to a base path. If the segment is absolute it replaces the base (POSIX semantics).

```aivi
use aivi.path (join)

join "/home/user" "docs"          -- "/home/user/docs"
join "/home/user/docs" "notes.txt" -- "/home/user/docs/notes.txt"
```

### `isAbsolute : Text -> Bool`

Return `True` when the path begins with `/`.

```aivi
use aivi.path (isAbsolute)

isAbsolute "/home/user"   -- True
isAbsolute "relative/path" -- False
```

### `normalize : Text -> Text`

Resolve `.` (current directory) and `..` (parent directory) segments lexically, without touching the filesystem.

```aivi
use aivi.path (normalize)

normalize "/home/user/docs/../photos"   -- "/home/user/photos"
normalize "/home/user/./notes.txt"      -- "/home/user/notes.txt"
normalize "/a/b/../../c"                -- "/c"
```

---

## Real-world example

```aivi
use aivi.path (join, parent, stem, extension, isAbsolute)
use aivi.fs (readText, writeText)

fun backupPath:Text originalPath:Text =>
    let base = parent originalPath in
    let name = stem originalPath in
    let ext  = extension originalPath in
    join (base |> withDefault "/tmp") ((name |> withDefault "file") <> ".backup")

fun safeReadConfig:Task Text Text configDir:Text =>
    let path = join configDir "app.conf" in
    isAbsolute path
     T|> readText path
     F|> readText (join "/etc" path)
```

::: tip
Combine `aivi.path` with `aivi.fs` for complete file management: use the path functions to build and manipulate path strings, then pass them to filesystem tasks.
:::
