# aivi.core.either

Disjoint union type for values that can be one of two alternatives. By convention `Left` holds an error or secondary value and `Right` holds the primary or success value.

```aivi
use aivi.core.either (
    Either
    mapRight
    mapLeft
    mapBoth
    fold
    isLeft
    isRight
    fromLeft
    fromRight
    swap
    toOption
    toResult
    fromResult
    partitionEithers
)
```

---

## Either

```aivi
type Either L R =
  | Left L
  | Right R
```

A value of type `Either L R` is either a `Left L` or a `Right R`. Use `||>` to branch on which case you have:

```aivi
use aivi.core.either (
    Either
    Left
    Right
)

type Either Text Int -> Text
func describeResult = result => result
 ||> Left msg -> "Error: {msg}"
 ||> Right n  -> "Got {n}"
```

---

## mapRight

Transforms the `Right` value, leaving `Left` unchanged.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    mapRight
)

type Int -> Int
func double = n =>
    n * 2

type Either Text Int -> (Either Text Int)
func doubleRight = result =>
    mapRight double result
```

---

## mapLeft

Transforms the `Left` value, leaving `Right` unchanged.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    mapLeft
)

type Int -> Text
func toMessage = code =>
    "Error {code}"

type Either Int Int -> (Either Text Int)
func wrapError = result =>
    mapLeft toMessage result
```

---

## mapBoth

Transforms both sides independently.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    mapBoth
)

use aivi.math (negate)

use aivi.text (surround)

type Either Text Int -> (Either Text Int)
func transformBoth = e =>
    mapBoth (surround "[" "]") negate e
```

---

## fold

Reduces an `Either` to a single value by applying the appropriate function.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    fold
)

type Text -> Int
func whenLeft = ignored =>
    0

type Text -> Int
func whenRight = ignored =>
    1

type Either Text Text -> Int
func toLength = e =>
    fold whenLeft whenRight e
```

---

## isLeft / isRight

Predicates that test which case an `Either` holds.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    isLeft
    isRight
)

type Either Text Int -> Bool
func hasError = e =>
    isLeft e
```

---

## fromLeft / fromRight

Extract the value from the expected case, returning a default if the other case is held.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    fromRight
)

type Either Text Int -> Int
func getValueOrZero = e =>
    fromRight 0 e
```

---

## swap

Swaps the `Left` and `Right` cases.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    swap
)

type Either Text Int -> (Either Int Text)
func flipEither = e =>
    swap e
```

---

## toOption

Converts to `Option`, keeping only `Right` values.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    toOption
)

type Either Text Int -> (Option Int)
func rightOrNone = e =>
    toOption e
```

---

## fromResult

Converts a `Result E A` into an `Either E A`. `Ok value` becomes `Right value`; `Err error` becomes `Left error`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    fromResult
)

type Result Text Int -> (Either Text Int)
func resultToEither = result =>
    fromResult result
```

---

## partitionEithers

Splits a list of `Either` values into a tuple of lefts and rights.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.either (
    Either
    partitionEithers
)

type ((List Text, List Int)) -> (List Text)
func takeLefts = parts => parts
 ||> (lefts, ignored) -> lefts

type List (Either Text Int) -> (List Text)
func splitResults = items =>
    takeLefts (partitionEithers items)
```
