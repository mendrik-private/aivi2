# aivi.image

Types for loaded images and image-loading errors.

This module does not currently export a concrete image-loading function. Instead, it defines
the value shapes used by image-loading integrations, including the source shape documented in
the stdlib comments.

## Import

```aivi
use aivi.image (
    ImageError
    ImageFormat
    ImageSize
    ImageMetadata
    ImageTask
    ImageData
)
```

## Overview

| Item | Type | Description |
|------|------|-------------|
| `ImageError` | type | Things that can go wrong while loading or decoding an image |
| `ImageFormat` | type | Known image formats |
| `ImageSize` | record | Width and height |
| `ImageMetadata` | record | Format, size, and alpha information without carrying the bytes |
| `ImageTask A` | `Task ImageError A` | Generic image task alias |
| `ImageData` | record | Loaded image bytes plus format and size |
| `image.load` | `Signal (Result ImageError ImageData)` | Documented source shape for reactive image loading |

## Types

### ImageError

```aivi
type ImageError =
  | ImageNotFound Text
  | ImageDecodeFailed Text
  | UnsupportedFormat Text
  | ImageUnavailable
```

These variants explain why image loading failed.

- `ImageNotFound` — the image path could not be found
- `ImageDecodeFailed` — bytes were found, but could not be decoded into an image
- `UnsupportedFormat` — the format is recognised as unsupported
- `ImageUnavailable` — image loading is not available in the current runtime

### ImageFormat

```aivi
type ImageFormat =
  | Png
  | Jpeg
  | Webp
  | Svg
  | Gif
  | UnknownFormat Text
```

A small tagged type for the image format.

- Use `UnknownFormat text` when the loader has a format label but not a dedicated variant.

```aivi
use aivi.image (
    ImageFormat
    Png
    Jpeg
    Webp
    Svg
    Gif
    UnknownFormat
)

type ImageFormat -> Text
func formatLabel = format => format
 ||> Png             -> "PNG"
 ||> Jpeg            -> "JPEG"
 ||> Webp            -> "WebP"
 ||> Svg             -> "SVG"
 ||> Gif             -> "GIF"
 ||> UnknownFormat s -> s
```

### ImageSize

```aivi
type ImageSize = {
    width: Int,
    height: Int
}
```

Pixel dimensions of an image.

### ImageMetadata

```aivi
type ImageMetadata = {
    format: ImageFormat,
    size: ImageSize,
    hasAlpha: Bool
}
```

Summary information about an image without carrying the raw bytes.

- `format` — the decoded image format
- `size` — width and height
- `hasAlpha` — `True` when the image includes an alpha channel

### ImageTask

```aivi
type ImageTask A = (Task ImageError A)
```

Generic alias for image-related tasks.

### ImageData

```aivi
type ImageData = {
    format: ImageFormat,
    size: ImageSize,
    bytes: Bytes
}
```

Full image payload.

- `format` — decoded format tag
- `size` — width and height
- `bytes` — raw image bytes

```aivi
use aivi.image (ImageData)

type ImageData -> Int
func imageWidth = image =>
    image.size.width
```

## Documented source shape

The stdlib module comments document the following source pattern:

```aivi
@source image.load "/app/icons/logo.png"
signal appLogo : Signal (Result ImageError ImageData)
```

The module comments also note a future one-shot task helper, but that function is not
exported here today.
