# aivi.gresource

Types for working with GTK and GNOME GResources.

If you have not used GResources before, think of them as read-only files bundled inside
your application binary or resource package. They are commonly used for CSS, UI XML, icons,
and other assets that should ship with the app.

This module currently exports the path type, error type, and task aliases used by
resource-loading APIs. The stdlib comments also document resource loading source shapes.

Current status: this is shared vocabulary around a host-backed resource provider. The target
architecture is to keep bundled resource loads under provider capabilities rather than parallel
task-only APIs.

## Import

```aivi
use aivi.gresource (
    ResourceError
    ResourcePath
    ResourceTask
    ResourceTextTask
    ResourceBytesTask
    ResourceListTask
)
```

## Overview

| Item | Type | Description |
|------|------|-------------|
| `ResourceError` | type | Things that can go wrong when resolving or decoding a resource |
| `ResourcePath` | domain over `Text` | Checked resource path value |
| `ResourceTask A` | `Task ResourceError A` | Generic resource task alias |
| `ResourceTextTask` | `Task ResourceError Text` | Resource task that returns text |
| `ResourceBytesTask` | `Task ResourceError Bytes` | Resource task that returns bytes |
| `ResourceListTask` | `Task ResourceError (List Text)` | Resource task that returns a list of text values |
| `resource.text` | `Signal (Result ResourceError Text)` | Documented source shape for text resources |
| `resource.bytes` | `Signal (Result ResourceError Bytes)` | Documented source shape for byte resources |

## Types

### ResourceError

```aivi
type ResourceError =
  | ResourceNotFound Text
  | ResourceDecodeFailed Text
  | ResourceUnavailable
```

These variants describe the common failure cases when loading a bundled resource.

- `ResourceNotFound` — the path does not exist in the resource bundle
- `ResourceDecodeFailed` — the bytes were found, but could not be decoded as requested
- `ResourceUnavailable` — resource access is not available in the current runtime

### ResourcePath

```aivi
domain ResourcePath over Text = {
    type Text -> Result ResourceError ResourcePath
    parse
}
```

`ResourcePath` is a dedicated type for paths such as `"/com/example/app/style.css"`.
Using a domain instead of plain `Text` makes it clearer when a value is supposed to point to
bundled app resources.

```aivi
use aivi.gresource (ResourcePath)

value cssPath : Result ResourceError ResourcePath = parse "/com/example/app/style.css"
```

### ResourceTask

```aivi
type ResourceTask A = (Task ResourceError A)
```

Generic alias for resource-related tasks.

### ResourceTextTask

```aivi
type ResourceTextTask = (Task ResourceError Text)
```

Alias for resource operations that return decoded text, such as CSS or UI markup.

### ResourceBytesTask

```aivi
type ResourceBytesTask = (Task ResourceError Bytes)
```

Alias for resource operations that return raw bytes, such as images or other binary data.

### ResourceListTask

```aivi
type ResourceListTask = (Task ResourceError (List Text))
```

Alias for resource-related tasks that return a list of text values.

## Documented source shapes

The stdlib module comments document the following source-backed patterns:

```aivi
@source resource.text "/com/example/app/style.css"
signal appCss : Signal (Result ResourceError Text)

@source resource.bytes "/com/example/app/icon.png"
signal iconData : Signal (Result ResourceError Bytes)
```

This module does not currently export direct `readText` or `readBytes` functions.
