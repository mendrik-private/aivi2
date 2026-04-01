# aivi.portal

Types for working with desktop portals.

Portals are the standard Linux desktop way to ask the system for privileged actions such as
opening a file chooser, opening a URI, or taking a screenshot. They are especially useful
for sandboxed apps.

This module currently exports the result and error types used by portal-backed features. The
stdlib comments also document the source names for several portal operations.

## Import

```aivi
use aivi.portal (
    PortalError
    PortalFileFilter
    PortalFileSelection
    PortalUriResult
    PortalScreenshotResult
    PortalTask
)
```

## Overview

| Item | Type | Description |
|------|------|-------------|
| `PortalError` | type | Things that can go wrong while talking to a portal |
| `PortalFileFilter` | record | One file chooser filter |
| `PortalFileSelection` | type | Result of choosing files |
| `PortalUriResult` | type | Result of asking the desktop to open a URI |
| `PortalScreenshotResult` | type | Result of requesting a screenshot |
| `PortalTask A` | `Task PortalError A` | Generic portal task alias |
| `portal.openFile` | `Signal (Result PortalError PortalFileSelection)` | Documented file chooser source shape |
| `portal.openUri` | `Signal (Result PortalError PortalUriResult)` | Documented URI-opening source shape |
| `portal.screenshot` | `Signal (Result PortalError PortalScreenshotResult)` | Documented screenshot source shape |

## Types

### PortalError

```aivi
type PortalError =
  | PortalUnavailable
  | UserCancelled
  | PortalPermissionDenied Text
  | PortalDecodeFailed Text
```

These variants describe why a portal request could not complete.

- `PortalUnavailable` — no usable portal backend is available
- `UserCancelled` — the request was cancelled before a usable result was produced
- `PortalPermissionDenied` — the desktop rejected the request
- `PortalDecodeFailed` — the portal replied, but the data could not be decoded cleanly

### PortalFileFilter

```aivi
type PortalFileFilter = {
    name: Text,
    patterns: List Text
}
```

One filter option for a file chooser.

- `name` — label shown to the user
- `patterns` — filename patterns such as `"*.aivi"` or `"*.png"`

```aivi
use aivi.portal (PortalFileFilter)

value aiviFiles : PortalFileFilter = {
    name: "AIVI Files",
    patterns: ["*.aivi"]
}
```

### PortalFileSelection

```aivi
type PortalFileSelection =
  | SingleFile Text
  | MultipleFiles (List Text)
  | SelectionCancelled
```

Result of a file chooser operation.

- `SingleFile` — one selected path or URI
- `MultipleFiles` — several selected paths or URIs
- `SelectionCancelled` — chooser closed without a selection

```aivi
use aivi.portal (
    PortalFileSelection
    SingleFile
    MultipleFiles
    SelectionCancelled
)

type PortalFileSelection -> Text
func selectionSummary = selection => selection
 ||> SingleFile path      -> path
 ||> MultipleFiles paths  -> "Several files selected"
 ||> SelectionCancelled   -> "No file selected"
```

### PortalUriResult

```aivi
type PortalUriResult =
  | UriOpened Text
  | UriOpenCancelled
  | UriOpenFailed Text
```

Result of asking the desktop to open a URI.

- `UriOpened` — the request was accepted for the given URI
- `UriOpenCancelled` — the user or system cancelled the request
- `UriOpenFailed` — the request failed with a message

### PortalScreenshotResult

```aivi
type PortalScreenshotResult =
  | ScreenshotBytes Bytes
  | ScreenshotCancelled
```

Result of a screenshot request.

- `ScreenshotBytes` — screenshot bytes were returned
- `ScreenshotCancelled` — the request was cancelled

### PortalTask

```aivi
type PortalTask A = (Task PortalError A)
```

Generic alias for portal-related tasks.

## Documented source shapes

The stdlib module comments document the following source-backed patterns:

```aivi
@source portal.openFile {
    filters: [{ name: "AIVI Files", patterns: ["*.aivi"] }]
}
signal openedFile : Signal (Result PortalError PortalFileSelection)

@source portal.openUri "https://example.com"
signal uriResult : Signal (Result PortalError PortalUriResult)

@source portal.screenshot
signal screenshot : Signal (Result PortalError PortalScreenshotResult)
```

This module does not currently export direct `openFile`, `openUri`, or `screenshot`
functions.
