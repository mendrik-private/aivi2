# aivi.bigint

Arbitrary-size integer helpers for numbers that may grow past the normal `Int` range.

All functions in this module are synchronous and pure — they return values directly and do not
perform I/O. Operations that may not produce a value, such as parsing invalid text, converting a
very large number back to `Int`, or dividing by zero, return `Option`.

## Import

```aivi
use aivi.bigint (
    fromInt
    fromText
    toInt
    toText
    add
    sub
    mul
    div
    bigMod
    pow
    neg
    bigAbs
    cmp
    bigEq
    gt
    lt
    zero
    one
    factorial
)
```

Friendly alias names such as `parse`, `plus`, `minus`, `times`, `dividedBy`, `remainder`,
`raiseTo`, `negate`, `absolute`, `equals`, `greaterThan`, and `lessThan` are also exported.

## Overview

### Parsing and conversion

| Name | Type | Description |
|------|------|-------------|
| `fromInt` / `fromInteger` | `Int -> BigInt` | Convert a normal `Int` to `BigInt` |
| `fromText` / `parse` | `Text -> Option BigInt` | Parse decimal text into `BigInt` |
| `toInt` | `BigInt -> Option Int` | Convert back to `Int` when the value fits |
| `toText` | `BigInt -> Text` | Render a decimal string |

### Arithmetic

| Name | Type | Description |
|------|------|-------------|
| `add` / `plus` | `BigInt -> BigInt -> BigInt` | Add two big integers |
| `sub` / `minus` | `BigInt -> BigInt -> BigInt` | Subtract the right value from the left |
| `mul` / `times` | `BigInt -> BigInt -> BigInt` | Multiply two big integers |
| `div` / `dividedBy` | `BigInt -> BigInt -> Option BigInt` | Integer division, or `None` for zero divisors |
| `bigMod` / `remainder` | `BigInt -> BigInt -> Option BigInt` | Remainder, or `None` for zero divisors |
| `pow` / `raiseTo` | `BigInt -> Int -> BigInt` | Raise a value to a whole-number power |
| `neg` / `negate` | `BigInt -> BigInt` | Change the sign |
| `bigAbs` / `absolute` | `BigInt -> BigInt` | Absolute value |
| `factorial` | `Int -> BigInt` | Factorial as a `BigInt` result |

### Comparison and checks

| Name | Type | Description |
|------|------|-------------|
| `cmp` | `BigInt -> BigInt -> Int` | Compare two values and return `-1`, `0`, or `1` |
| `bigEq` / `equals` | `BigInt -> BigInt -> Bool` | Exact equality |
| `gt` / `greaterThan` | `BigInt -> BigInt -> Bool` | Greater-than check |
| `lt` / `lessThan` | `BigInt -> BigInt -> Bool` | Less-than check |
| `greaterOrEqual` | `BigInt -> BigInt -> Bool` | Greater-than-or-equal check |
| `lessOrEqual` | `BigInt -> BigInt -> Bool` | Less-than-or-equal check |
| `isZero` | `BigInt -> Bool` | Check for zero |
| `isPositive` | `BigInt -> Bool` | Check for values above zero |
| `isNegative` | `BigInt -> Bool` | Check for values below zero |

### Constants

| Name | Type | Description |
|------|------|-------------|
| `zero` | `BigInt` | `0` as a `BigInt` |
| `one` | `BigInt` | `1` as a `BigInt` |
| `negOne` | `BigInt` | `-1` as a `BigInt` |

## Functions

### fromInt / fromInteger

```aivi
fromInt : Int -> BigInt
fromInteger : Int -> BigInt
```

Convert a normal machine-sized `Int` into `BigInt`. Use this when you want to move into
big-integer arithmetic before the value grows large.

```aivi
use aivi.bigint (fromInt)

value startCount = fromInt 42
```

### fromText / parse

```aivi
fromText : Text -> Option BigInt
parse : Text -> Option BigInt
```

Parse decimal text into `BigInt`. Surrounding whitespace is ignored. Returns `None` when the text
is not a valid integer.

```aivi
use aivi.bigint (fromText)

value customerId = fromText "90071992547409931234567890"
```

### toInt

```aivi
toInt : BigInt -> Option Int
```

Try to convert a `BigInt` back to plain `Int`. Returns `Some n` when the value fits in `Int`, or
`None` when it is too large or too small.

```aivi
use aivi.bigint (
    fromText
    toInt
)

func toMachineInt = raw =>
```

### toText

```aivi
toText : BigInt -> Text
```

Render a `BigInt` as decimal text. This is the easiest way to show a large number in the UI or
store it in text-based formats.

```aivi
use aivi.bigint (
    factorial
    toText
)

value rendered = toText (factorial 30)
```

### add / plus

```aivi
add : BigInt -> BigInt -> BigInt
plus : BigInt -> BigInt -> BigInt
```

Add two `BigInt` values.

### sub / minus

```aivi
sub : BigInt -> BigInt -> BigInt
minus : BigInt -> BigInt -> BigInt
```

Subtract the right value from the left.

### mul / times

```aivi
mul : BigInt -> BigInt -> BigInt
times : BigInt -> BigInt -> BigInt
```

Multiply two `BigInt` values.

### div / dividedBy

```aivi
div : BigInt -> BigInt -> Option BigInt
dividedBy : BigInt -> BigInt -> Option BigInt
```

Integer division. Any remainder is discarded. Returns `None` when the divisor is zero.

```aivi
use aivi.bigint (
    div
    fromInt
)

value maybePages = div (fromInt 120) (fromInt 10)
```

### bigMod / remainder

```aivi
bigMod : BigInt -> BigInt -> Option BigInt
remainder : BigInt -> BigInt -> Option BigInt
```

Return the remainder after integer division. Returns `None` when the divisor is zero.

### pow / raiseTo

```aivi
pow : BigInt -> Int -> BigInt
raiseTo : BigInt -> Int -> BigInt
```

Raise a `BigInt` to a whole-number power. The exponent is a normal `Int`. Negative exponents are
currently treated as `0`, so the result is `1`.

### neg / negate

```aivi
neg : BigInt -> BigInt
negate : BigInt -> BigInt
```

Flip the sign of a `BigInt`.

### bigAbs / absolute

```aivi
bigAbs : BigInt -> BigInt
absolute : BigInt -> BigInt
```

Return the absolute value of a `BigInt`.

### cmp

```aivi
cmp : BigInt -> BigInt -> Int
```

Compare two `BigInt` values. The result is `-1` when the left value is smaller, `0` when both
values are equal, and `1` when the left value is larger.

### bigEq / equals

```aivi
bigEq : BigInt -> BigInt -> Bool
equals : BigInt -> BigInt -> Bool
```

Check whether two `BigInt` values are exactly equal.

### gt / greaterThan

```aivi
gt : BigInt -> BigInt -> Bool
greaterThan : BigInt -> BigInt -> Bool
```

Return `True` when the left value is greater than the right value.

### lt / lessThan

```aivi
lt : BigInt -> BigInt -> Bool
lessThan : BigInt -> BigInt -> Bool
```

Return `True` when the left value is less than the right value.

### greaterOrEqual / lessOrEqual

```aivi
greaterOrEqual : BigInt -> BigInt -> Bool
lessOrEqual : BigInt -> BigInt -> Bool
```

Inclusive comparison helpers built from the basic comparison functions.

### zero / one / negOne

```aivi
zero : BigInt
one : BigInt
negOne : BigInt
```

Ready-made `BigInt` constants for common starting values.

### isZero / isPositive / isNegative

```aivi
isZero : BigInt -> Bool
isPositive : BigInt -> Bool
isNegative : BigInt -> Bool
```

Small sign-check helpers for common conditions.

### factorial

```aivi
factorial : Int -> BigInt
```

Compute `n!` as a `BigInt`. `factorial 0` returns `one`, and negative input currently also returns
`one`.

```aivi
use aivi.bigint (
    factorial
    toText
)

value reportSize = toText (factorial 50)
```

## Example — parse, add, and render a large total

```aivi
use aivi.bigint (
    fromText
    add
    toText
)

func combineTotals = left right =>
```

## Example — compare large identifiers safely

```aivi
use aivi.bigint (
    cmp
    fromText
)

func newerId = left right =>
```
