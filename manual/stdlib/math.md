# aivi.math

Integer arithmetic utilities. Provides common numeric helpers including absolute value, sign detection, parity tests, clamping, and divisibility.

```aivi
use aivi.math (abs, negate, sign, isEven, isOdd, square, clamp, between, divides)
```

---

## abs

Returns the absolute value of an integer.

```
abs : Int -> Int
```

```aivi
use aivi.math (abs)

fun distance:Int a:Int b:Int =>
    abs (a - b)
```

---

## negate

Negates an integer: `negate n` is equivalent to `0 - n`.

```
negate : Int -> Int
```

```aivi
use aivi.math (negate)

fun flipSign:Int n:Int =>
    negate n
```

---

## sign

Returns the sign of an integer as `-1`, `0`, or `1`.

```
sign : Int -> Int
```

```aivi
use aivi.math (sign)

fun direction:Int velocity:Int =>
    sign velocity
```

---

## isEven

Returns `True` if the integer is divisible by 2.

```
isEven : Int -> Bool
```

```aivi
use aivi.math (isEven)

fun evensOnly:List Int numbers:List Int =>
    filter isEven numbers
```

---

## isOdd

Returns `True` if the integer is not divisible by 2.

```
isOdd : Int -> Bool
```

```aivi
use aivi.math (isOdd)

fun oddsOnly:List Int numbers:List Int =>
    filter isOdd numbers
```

---

## square

Multiplies an integer by itself.

```
square : Int -> Int
```

```aivi
use aivi.math (square)

fun areaOfSquare:Int side:Int =>
    square side
```

---

## clamp

Constrains a value to lie within `[low, high]`. If `n < low`, returns `low`; if `n > high`, returns `high`; otherwise returns `n`.

```
clamp : Int -> Int -> Int -> Int
```

```aivi
use aivi.math (clamp)

fun normalizedVolume:Int raw:Int =>
    clamp 0 100 raw
```

---

## between

Returns `True` if `n` is within the inclusive range `[low, high]`.

```
between : Int -> Int -> Int -> Bool
```

```aivi
use aivi.math (between)

fun isValidAge:Bool age:Int =>
    between 0 150 age
```

---

## divides

Returns `True` if `divisor` evenly divides `n` (i.e. `n % divisor == 0`).

```
divides : Int -> Int -> Bool
```

```aivi
use aivi.math (divides)

fun isMultipleOfThree:Bool n:Int =>
    divides 3 n
```
