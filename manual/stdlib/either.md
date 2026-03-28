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

```
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

fun describeResult:Text result: (Either Text Int) => result
  ||> Left msg -> "Error: {msg}"
  ||> Right n  -> "Got {n}"
```

---

## mapRight

Transforms the `Right` value, leaving `Left` unchanged.

```
mapRight : (R -> R2) -> Either L R -> Either L R2
```

```aivi
use aivi.core.either (
    Either
    mapRight
)

fun double:Int n:Int =>
    n * 2

fun doubleRight: (Either Text Int) result: (Either Text Int) =>
    mapRight double result
```

---

## mapLeft

Transforms the `Left` value, leaving `Right` unchanged.

```
mapLeft : (L -> L2) -> Either L R -> Either L2 R
```

```aivi
use aivi.core.either (
    Either
    mapLeft
)

fun toMessage:Text code:Int =>
    "Error {code}"

fun wrapError: (Either Text Int) result: (Either Int Int) =>
    mapLeft toMessage result
```

---

## mapBoth

Transforms both sides independently.

```
mapBoth : (L -> L2) -> (R -> R2) -> Either L R -> Either L2 R2
```

```aivi
use aivi.core.either (
    Either
    mapBoth
)

use aivi.math (negate)

use aivi.text (surround)

fun transformBoth: (Either Text Int) e: (Either Text Int) =>
    mapBoth (surround "[" "]") negate e
```

---

## fold

Reduces an `Either` to a single value by applying the appropriate function.

```
fold : (L -> C) -> (R -> C) -> Either L R -> C
```

```aivi
use aivi.core.either (
    Either
    fold
)

fun whenLeft:Int ignored:Text =>
    0

fun whenRight:Int ignored:Text =>
    1

fun toLength:Int e: (Either Text Text) =>
    fold whenLeft whenRight e
```

---

## isLeft / isRight

Predicates that test which case an `Either` holds.

```
isLeft  : Either L R -> Bool
isRight : Either L R -> Bool
```

```aivi
use aivi.core.either (
    Either
    isLeft
    isRight
)

fun hasError:Bool e: (Either Text Int) =>
    isLeft e
```

---

## fromLeft / fromRight

Extract the value from the expected case, returning a default if the other case is held.

```
fromLeft  : L -> Either L R -> L
fromRight : R -> Either L R -> R
```

```aivi
use aivi.core.either (
    Either
    fromRight
)

fun getValueOrZero:Int e: (Either Text Int) =>
    fromRight 0 e
```

---

## swap

Swaps the `Left` and `Right` cases.

```
swap : Either L R -> Either R L
```

```aivi
use aivi.core.either (
    Either
    swap
)

fun flipEither: (Either Int Text) e: (Either Text Int) =>
    swap e
```

---

## toOption

Converts to `Option`, keeping only `Right` values.

```
toOption : Either L R -> Option R
```

```aivi
use aivi.core.either (
    Either
    toOption
)

fun rightOrNone: (Option Int) e: (Either Text Int) =>
    toOption e
```

---

## fromResult

Converts a `Result E A` into an `Either E A`. `Ok value` becomes `Right value`; `Err error` becomes `Left error`.

```
fromResult : Result E A -> Either E A
```

```aivi
use aivi.core.either (
    Either
    fromResult
)

fun resultToEither: (Either Text Int) result: (Result Text Int) =>
    fromResult result
```

---

## partitionEithers

Splits a list of `Either` values into a tuple of lefts and rights.

```
partitionEithers : List (Either L R) -> (List L, List R)
```

```aivi
use aivi.core.either (
    Either
    partitionEithers
)

fun takeLefts: (List Text) parts: ((List Text, List Int)) => parts
  ||> (lefts, ignored) -> lefts

fun splitResults: (List Text) items: (List (Either Text Int)) =>
    takeLefts (partitionEithers items)
```
