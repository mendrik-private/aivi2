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

fun circleArea:Float radius:Float =>
    pi * radius * radius

fun circleCircumference:Float radius:Float =>
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

fun hypotenuse:Float a:Float b:Float =>
    sqrt (a * a + b * b)

fun roundTrip:Float n:Int =>
    fromInt n
```

---

## negate

Negates a float. AIVI has no prefix minus on literals, so `negate` fills that gap.

```
negate : Float -> Float
```

```aivi
use aivi.core.float (negate)

fun flipSign:Float n:Float =>
    negate n
```

---

## max / min

Return the larger or smaller of two floats.

```
max : Float -> Float -> Float
min : Float -> Float -> Float
```

```aivi
use aivi.core.float (
    max
    min
)

fun boundedProgress:Float progress:Float =>
    min 1.0 (max 0.0 progress)
```

---

## clamp

Clamps a value to the inclusive range `[lo, hi]`.

```
clamp : Float -> Float -> Float -> Float
```

```aivi
use aivi.core.float (clamp)

fun normalizedVolume:Float raw:Float =>
    clamp 0.0 1.0 raw
```

---

## lerp

Linear interpolation between `a` and `b`. `lerp a b 0.0` returns `a`, `lerp a b 1.0` returns `b`.

```
lerp : Float -> Float -> Float -> Float
```

```aivi
use aivi.core.float (lerp)

fun blend:Float from:Float to:Float t:Float =>
    lerp from to t
```

---

## sign

Returns `-1.0`, `0.0`, or `1.0` depending on the sign of `n`.

```
sign : Float -> Float
```

```aivi
use aivi.core.float (sign)

fun moveDirection:Float velocity:Float =>
    sign velocity
```

---

## between

Returns `True` if `n` is in the closed interval `[lo, hi]`.

```
between : Float -> Float -> Float -> Bool
```

```aivi
use aivi.core.float (between)

fun isValidRatio:Bool ratio:Float =>
    between 0.0 1.0 ratio
```

---

## Predicates

```
isZero     : Float -> Bool
isPositive : Float -> Bool
isNegative : Float -> Bool
```

```aivi
use aivi.core.float (
    isPositive
    isNegative
)

fun describeNonPositive:Text n:Float => isNegative n
  T|> "negative"
  F|> "zero"

fun signum:Text n:Float => isPositive n
  T|> "positive"
  F|> describeNonPositive n
```

---

## square

Multiplies a float by itself.

```
square : Float -> Float
```

```aivi
use aivi.core.float (square)

fun addFloats:Float left:Float right:Float =>
    left + right

fun distanceSquared:Float dx:Float dy:Float =>
    addFloats (square dx) (square dy)
```

---

## toRadians / toDegrees

Convert between degrees and radians.

```
toRadians : Float -> Float
toDegrees : Float -> Float
```

```aivi
use aivi.core.float (
    toRadians
    toDegrees
)

fun halfCircleInRadians:Float ignored:Unit =>
    toRadians 180.0

fun rightAngleInDegrees:Float ignored:Unit =>
    toDegrees 1.5707963267948966
```
