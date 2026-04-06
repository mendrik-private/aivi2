# aivi.core.float

IEEE 754 double-precision floating-point helpers. The built-in `Float` type supports `+`, `-`, `*`, `/`, `<`, `>`, `<=`, `>=`, `==`, and `!=` directly. This module adds commonly needed pure helpers on top.

The low-level math intrinsics (`floor`, `ceil`, `round`, `sqrt`, `abs`, `toInt`, `fromInt`, `toText`, `parseText`) are available via the compiler catalog:

```aivi
use aivi.core.float (
    floor
    ceil
    round
    sqrt
    abs
    toInt
    fromInt
    toText
    parseText
)
```

Pure helpers are imported the same way:

```aivi
use aivi.core.float (
    pi
    e
    tau
    negate
    absHelper
    max
    min
    clamp
    lerp
    sign
    between
    isZero
    isPositive
    isNegative
    square
    toRadians
    toDegrees
)
```

---

## Constants

| Name  | Value                  | Description                  |
|-------|------------------------|------------------------------|
| `pi`  | `3.141592653589793`    | π — ratio of circumference to diameter |
| `e`   | `2.718281828459045`    | Euler's number               |
| `tau` | `6.283185307179586`    | τ = 2π — full circle in radians |

```aivi
use aivi.core.float (
    pi
    tau
)

type Float -> Float
func circleArea = radius =>
    pi * radius * radius

type Float -> Float
func circleCircumference = radius =>
    tau * radius
```

---

## Intrinsics

These are handled by the compiler directly. Import them from `aivi.core.float`.

| Name        | Signature             | Description                        |
|-------------|-----------------------|------------------------------------|
| `floor`     | `Float -> Float`      | Round down to nearest whole number |
| `ceil`      | `Float -> Float`      | Round up to nearest whole number   |
| `round`     | `Float -> Float`      | Round to nearest whole number      |
| `sqrt`      | `Float -> Float`      | Square root                        |
| `abs`       | `Float -> Float`      | Absolute value                     |
| `toInt`     | `Float -> Int`        | Truncate to integer                |
| `fromInt`   | `Int -> Float`        | Convert integer to float           |
| `toText`    | `Float -> Text`       | Convert to text representation     |
| `parseText` | `Text -> Option Float`| Parse text as float; `None` if invalid |

```aivi
use aivi.core.float (
    sqrt
    toInt
    fromInt
    abs
)

type Float -> Float -> Float
func hypotenuse = a b =>
    sqrt (a * a + b * b)

type Int -> Float
func roundTrip = n =>
    fromInt n
```

---

## negate

Negates a float. AIVI has no prefix minus on literals, so `negate` fills that gap.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (negate)

type Float -> Float
func flipSign = n =>
    negate n
```

---

## max / min

Return the larger or smaller of two floats.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (
    max
    min
)

type Float -> Float
func boundedProgress = progress =>
    min 1.0 (max 0.0 progress)
```

---

## clamp

Clamps a value to the inclusive range `[lo, hi]`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (clamp)

type Float -> Float
func normalizedVolume = raw =>
    clamp 0.0 1.0 raw
```

---

## lerp

Linear interpolation between `a` and `b`. `lerp a b 0.0` returns `a`, `lerp a b 1.0` returns `b`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (lerp)

type Float -> Float -> Float -> Float
func blend = from to t =>
    lerp from to t
```

---

## sign

Returns `-1.0`, `0.0`, or `1.0` depending on the sign of `n`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (sign)

type Float -> Float
func moveDirection = velocity =>
    sign velocity
```

---

## between

Returns `True` if `n` is in the closed interval `[lo, hi]`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (between)

type Float -> Bool
func isValidRatio = ratio =>
    between 0.0 1.0 ratio
```

---

## Predicates

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (
    isPositive
    isNegative
)

type Float -> Text
func describeNonPositive = n => isNegative n
 T|> "negative"
 F|> "zero"

type Float -> Text
func signum = n => isPositive n
 T|> "positive"
 F|> describeNonPositive n
```

---

## square

Multiplies a float by itself.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (square)

type Float -> Float -> Float
func addFloats = left right =>
    left + right

type Float -> Float -> Float
func distanceSquared = dx dy =>
    addFloats (square dx) (square dy)
```

---

## toRadians / toDegrees

Convert between degrees and radians.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.float (
    toRadians
    toDegrees
)

type Unit -> Float
func halfCircleInRadians = ignored =>
    toRadians 180.0

type Unit -> Float
func rightAngleInDegrees = ignored =>
    toDegrees 1.5707963267948966
```
