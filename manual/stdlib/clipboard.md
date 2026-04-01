# aivi.clipboard

Types for working with the desktop clipboard.

If you are new to AIVI's desktop integrations, this module is mostly a set of data shapes.
It tells you what clipboard content looks like and how clipboard errors are reported. The
stdlib comments also describe a reactive clipboard watcher.

## Import

```aivi
use aivi.clipboard (
    ClipboardError
    ClipboardContent
    ClipboardTask
    ClipboardWriteTask
)
```

## Overview

| Item | Type | Description |
|------|------|-------------|
| `ClipboardError` | type | Things that can go wrong when reading or writing clipboard data |
| `ClipboardContent` | type | Tagged clipboard contents: text, URIs, image bytes, HTML, or empty |
| `ClipboardTask A` | `Task ClipboardError A` | Task alias for clipboard-related work |
| `ClipboardWriteTask` | `Task ClipboardError Unit` | Task alias for clipboard writes |
| `clipboard.watch` | `Signal (Result ClipboardError ClipboardContent)` | Reactive watcher shape documented in the module comments |

## Types

### ClipboardError

```aivi
type ClipboardError =
  | ClipboardUnavailable
  | ClipboardEmpty
  | ClipboardTypeMismatch Text
  | ClipboardWriteFailed Text
```

These variants explain why clipboard access failed.

- `ClipboardUnavailable` — no clipboard service is available
- `ClipboardEmpty` — the clipboard currently has no value to read
- `ClipboardTypeMismatch` — the clipboard has data, but not in the form you expected
- `ClipboardWriteFailed` — a write attempt failed with a message from the backend

### ClipboardContent

```aivi
type ClipboardContent =
  | TextContent Text
  | UriListContent (List Text)
  | ImageContent Bytes
  | HtmlContent Text
  | EmptyClipboard
```

`ClipboardContent` is a tagged value, so you always know what kind of data the clipboard is
holding before you try to use it.

- `TextContent` — plain text
- `UriListContent` — one or more copied URIs, such as file paths or links
- `ImageContent` — raw image bytes
- `HtmlContent` — formatted HTML text
- `EmptyClipboard` — an explicit empty state

```aivi
use aivi.clipboard (
    ClipboardContent
    TextContent
    UriListContent
    ImageContent
    HtmlContent
    EmptyClipboard
)

type ClipboardContent -> Text
func clipboardSummary = content => content
 ||> TextContent text    -> text
 ||> UriListContent uris -> "Copied links"
 ||> ImageContent bytes  -> "Copied image"
 ||> HtmlContent html    -> "Copied rich text"
 ||> EmptyClipboard      -> "Clipboard is empty"
```

### ClipboardTask

```aivi
type ClipboardTask A = (Task ClipboardError A)
```

Convenience name for clipboard operations that may fail with `ClipboardError`.

### ClipboardWriteTask

```aivi
type ClipboardWriteTask = (Task ClipboardError Unit)
```

Convenience name for clipboard write operations.

At the time of writing, this module does not export a concrete write function. The alias is
still useful because it documents the task shape other clipboard APIs are expected to use.

## Documented source shape

The stdlib module comments document the following watcher shape:

```aivi
@source clipboard.watch
signal clipboardContent : Signal (Result ClipboardError ClipboardContent)
```

This module does not currently export a direct clipboard write helper.
