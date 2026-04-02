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
| `unwrap` | `Color -> Int` | Get the packed integer back |
| `red` / `green` / `blue` / `alpha` | `Color -> Int` | Read one color channel |
| `withAlpha` / `withRed` / `withGreen` / `withBlue` | `Color -> Int -> Color` | Replace one channel |
| `blend` | `Color -> Color -> Float -> Color` | Mix two colors |

## Domain

```aivi
domain Color over Int = {
    type Int -> Color
    argb
    type Color -> Int
    unwrap
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
argb : Int -> Color
```

Construct a `Color` from one packed ARGB integer. This is the low-level constructor exposed
by the module today.

### unwrap

```aivi
unwrap : Color -> Int
```

Extract the packed integer again.

### red / green / blue / alpha

```aivi
red : Color -> Int
green : Color -> Int
blue : Color -> Int
alpha : Color -> Int
```

Read one channel from a color.

### withAlpha / withRed / withGreen / withBlue

```aivi
withAlpha : Color -> Int -> Color
withRed : Color -> Int -> Color
withGreen : Color -> Int -> Color
withBlue : Color -> Int -> Color
```

Return a new color with one channel replaced.

```aivi
use aivi.color (Color)

type Theme = {
    accent: Color
}

type Theme -> Color
func dimAccent = theme =>
    withAlpha theme.accent 180
```

### blend

```aivi
blend : Color -> Color -> Float -> Color
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
    blend theme.background theme.accent 0.15
```

## Notes

This module does not currently include named colors or a text parser such as `#RRGGBB`.
Work with the packed integer form, or convert from text before reaching this module.

Current limits:

- no lightness / hue / saturation algebra
- no domain operators for adjustments such as `+ 10lightness` or `- 20hue`
- no alternate color-space helpers such as HSL or OKLCH
