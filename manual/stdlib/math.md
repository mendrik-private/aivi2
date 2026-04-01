# aivi.math

Integer arithmetic utilities. Provides common numeric helpers including absolute value, sign detection, parity tests, clamping, and divisibility.

```aivi
use aivi.math (
    abs
    negate
    sign
    isEven
    isOdd
    square
    clamp
    between
    divides
)
```

---

## abs

Returns the absolute value of an integer.

```aivi
abs : Int -> Int
```

```aivi
use aivi.math (abs)

type Int -> Int -> Int
func distance = a b =>
    abs (a - b)
```

---

## negate

Negates an integer: `negate n` is equivalent to `0 - n`.

```aivi
negate : Int -> Int
```

```aivi
use aivi.math (negate)

type Int -> Int
func flipSign = n =>
    negate n
```

---

## sign

Returns the sign of an integer as `-1`, `0`, or `1`.

```aivi
sign : Int -> Int
```

```aivi
use aivi.math (sign)

type Int -> Int
func direction = velocity =>
    sign velocity
```

---

## isEven

Returns `True` if the integer is divisible by 2.

```aivi
isEven : Int -> Bool
```

```aivi
use aivi.list (filter)

use aivi.math (isEven)

type List Int -> List Int
func evensOnly = numbers =>
    filter isEven numbers
```

---

## isOdd

Returns `True` if the integer is not divisible by 2.

```aivi
isOdd : Int -> Bool
```

```aivi
use aivi.list (filter)

use aivi.math (isOdd)

type List Int -> List Int
func oddsOnly = numbers =>
    filter isOdd numbers
```

---

## square

Multiplies an integer by itself.

```aivi
square : Int -> Int
```

```aivi
use aivi.math (square)

type Int -> Int
func areaOfSquare = side =>
    square side
```

---

## clamp

Constrains a value to lie within `[low, high]`. If `n < low`, returns `low`; if `n > high`, returns `high`; otherwise returns `n`.

```aivi
clamp : Int -> Int -> Int -> Int
```

```aivi
use aivi.math (clamp)

type Int -> Int
func normalizedVolume = raw =>
    clamp 0 100 raw
```

---

## between

Returns `True` if `n` is within the inclusive range `[low, high]`.

```aivi
between : Int -> Int -> Int -> Bool
```

```aivi
use aivi.math (between)

type Int -> Bool
func isValidAge = age =>
    between 0 150 age
```

---

## divides

Returns `True` if `divisor` evenly divides `n` (i.e. `n % divisor == 0`).

```aivi
divides : Int -> Int -> Bool
```

```aivi
use aivi.math (divides)

type Int -> Bool
func isMultipleOfThree = n =>
    divides 3 n
```
