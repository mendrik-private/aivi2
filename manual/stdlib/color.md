# aivi.color

A small domain type for UI colors.

`Color` wraps an `Int`, so you can pass colors around as a named type instead of a raw
number. Importing the `Color` domain also brings its helper names into scope.

## Import

```aivi
use aivi.color (Color)
```

## Overview

| Name | Type | Description |
|------|------|-------------|
| `Color` | domain over `Int` | A packed color value |
| `argb` | `Int -> Color` | Build a color from one packed ARGB integer |
| `red` / `green` / `blue` / `alpha` | `Color -> Int` | Read one color channel |
| `withAlpha` / `withRed` / `withGreen` / `withBlue` | `Color -> Int -> Color` | Replace one channel |
| `blend` | `Color -> Color -> Float -> Color` | Mix two colors |

## Domain

```aivi
domain Color over Int = {
    type Int -> Color
    argb
    type Color -> Int
    red
    type Color -> Int
    green
    type Color -> Int
    blue
    type Color -> Int
    alpha
    type Color -> Int -> Color
    withAlpha
    type Color -> Int -> Color
    withRed
    type Color -> Int -> Color
    withGreen
    type Color -> Int -> Color
    withBlue
    type Color -> Color -> Float -> Color
    blend
}
```

`Color` is useful when a field should clearly mean “this is a color” rather than “this is
just some integer”.

```aivi
use aivi.color (Color)

type Theme = {
    accent: Color,
    warning: Color
}
```

### argb

```aivi
```

Construct a `Color` from one packed ARGB integer. This is the low-level constructor exposed
by the module today.

### red / green / blue / alpha

```aivi
```

Read one channel from a color.

### withAlpha / withRed / withGreen / withBlue

```aivi
```

Return a new color with one channel replaced.

```aivi
use aivi.color (Color)

type Theme = { accent: Color }

type Theme -> Color
func dimAccent = theme =>
    theme.accent.withAlpha 180
```

### blend

```aivi
```

Blend two colors together. The `Float` controls how far the result moves from the first
color toward the second.

```aivi
use aivi.color (Color)

type Theme = {
    accent: Color,
    background: Color
}

type Theme -> Color
func hoverColor = theme =>
    theme.background.blend theme.accent 0.15
```

## Notes

This module does not currently include named colors or a text parser such as `#RRGGBB`.
Work with the packed integer form, or convert from text before reaching this module.

Current limits:

- no lightness / hue / saturation algebra
- no domain operators for adjustments such as `+ 10lightness` or `- 20hue`
- no alternate color-space helpers such as HSL or OKLCH
