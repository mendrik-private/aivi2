# aivi.portal

Types for working with desktop portals.

Portals are the standard Linux desktop way to ask the system for privileged actions such as
opening a file chooser, opening a URI, or taking a screenshot. They are especially useful
for sandboxed apps.

This module exports the result and error types used by the built-in desktop portal sources.

Portal work stays under provider-backed `@source` variants rather than a separate task-first
portal handle family.

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
| `portal.openFile` | `Signal (Result PortalError PortalFileSelection)` | Built-in file chooser source |
| `portal.openUri` | `Signal (Result PortalError PortalUriResult)` | Built-in URI-opening source |
| `portal.screenshot` | `Signal (Result PortalError PortalScreenshotResult)` | Built-in screenshot source |

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
 ||> SingleFile path     -> path
 ||> MultipleFiles paths -> "Several files selected"
 ||> SelectionCancelled  -> "No file selected"
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

## Built-in source shapes

These source variants are implemented today:

```aivi
@source portal.openFile {
    title: "Open attachment"
    multiple: true
    filters: [{ name: "AIVI Files", patterns: ["*.aivi"] }]
}
signal openedFile : Signal (Result PortalError PortalFileSelection)

@source portal.openUri "https://example.com" with { ask: true }
signal uriResult : Signal (Result PortalError PortalUriResult)

@source portal.screenshot with { interactive: true }
signal screenshot : Signal (Result PortalError PortalScreenshotResult)
```

### `portal.openFile`

**Form:** `@source portal.openFile config`

`config` is currently a record with these supported fields:

| Field | Type | Meaning |
|------|------|---------|
| `title` | `Text` | Dialog title. Default: `"Open File"` |
| `acceptLabel` | `Text` | Custom accept button label |
| `modal` | `Bool` | Modal hint. Default: `True` |
| `multiple` | `Bool` | Allow multiple selection |
| `directory` | `Bool` | Pick folders instead of files |
| `currentFolder` | `Text` | Suggested starting folder path |
| `filters` | `List PortalFileFilter` | Glob-based chooser filters |

`portal.openFile` returns:

- `Ok (SingleFile uri)` for one selected file
- `Ok (MultipleFiles uris)` for multi-select
- `Ok SelectionCancelled` if chooser is cancelled
- `Err PortalUnavailable | PortalPermissionDenied | PortalDecodeFailed` for runtime/protocol failures

### `portal.openUri`

**Form:** `@source portal.openUri uri`

Supported options:

| Option | Type | Meaning |
|------|------|---------|
| `ask` | `Bool` | Ask desktop to prompt for app choice |
| `writable` | `Bool` | Request writable handoff where supported |
| `activationToken` | `Text` | Forward activation token to launched app |
| `refreshOn` | `Signal A` | Standard request retrigger |
| `activeWhen` | `Signal Bool` | Standard lifecycle gate |

`portal.openUri` returns `Ok (UriOpened uri)`, `Ok UriOpenCancelled`, or `Ok (UriOpenFailed message)`.

### `portal.screenshot`

**Form:** `@source portal.screenshot`

Supported options:

| Option | Type | Meaning |
|------|------|---------|
| `interactive` | `Bool` | Ask portal backend for interactive capture UI |
| `modal` | `Bool` | Modal hint |
| `refreshOn` | `Signal A` | Standard request retrigger |
| `activeWhen` | `Signal Bool` | Standard lifecycle gate |

`portal.screenshot` reads the portal-provided file URI and returns `Ok (ScreenshotBytes bytes)` or
`Ok ScreenshotCancelled`.
