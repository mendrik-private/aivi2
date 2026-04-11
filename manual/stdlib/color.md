# aivi.color

A small domain type for UI colors.

`Color` wraps an `Int`, so you can pass colors around as a named type instead of a raw
number.

## Import

```aivi
use aivi.color (Color)
```

## Overview

| Name | Type | Description |
|------|------|-------------|
| `Color` | domain over `Int` | A packed color value |
| `.carrier` | `Color -> Int` | The raw packed ARGB integer |

## Domain

```aivi
domain Color over Int
    argb : Int -> Color
    red : Color -> Int
    green : Color -> Int
    blue : Color -> Int
    alpha : Color -> Int
    withAlpha : Color -> Int -> Color
    withRed : Color -> Int -> Color
    withGreen : Color -> Int -> Color
    withBlue : Color -> Int -> Color
    blend : Color -> Color -> Float -> Color
```

`Color` is useful when a field should clearly mean "this is a color" rather than "this is
just some integer".

```aivi
use aivi.color (Color)

type Theme = {
    accent: Color,
    warning: Color
}
```

## `.carrier`

Access the packed ARGB integer backing a `Color` value.

```aivi
use aivi.color (Color)

type Color -> Int
func toArgb = color =>
    color.carrier
```

## Domain members

The members declared inside the `Color` domain — `argb`, `red`, `green`, `blue`, `alpha`,
`withAlpha`, `withRed`, `withGreen`, `withBlue`, `blend` — are part of the domain's internal
implementation. They are used by the runtime's theming layer and are not individually
importable from user code.

To manipulate colors in your own module, extract the packed integer with `.carrier`, apply
arithmetic, and reconstruct as needed, or receive `Color` values from GTK theme lookups.

## Notes

This module does not currently include named colors or a text parser such as `#RRGGBB`.

Current limits:

- no lightness / hue / saturation algebra
- no domain operators for adjustments such as `+ 10lightness` or `- 20hue`
- no alternate color-space helpers such as HSL or OKLCH
