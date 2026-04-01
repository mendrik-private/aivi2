# aivi.result

Utilities for working with `Result E A` — a value that is either a success (`Ok`) or a failure (`Err`). All functions are pure and can be freely composed with pipes.

```aivi
use aivi.result (
    isOk
    isErr
    mapErr
    withDefault
    orElse
    flatMap
    flatten
    toOption
    toList
    map
    mapBoth
    fold
    fromOption
)
```

---

## isOk

Returns `True` if the result is `Ok`.

**Type:** `opt:(Result E A) -> Bool`

```aivi
use aivi.result (isOk)

type Result Text Int -> Bool
func succeeded = result =>
    isOk result
```

---

## isErr

Returns `True` if the result is `Err`.

**Type:** `opt:(Result E A) -> Bool`

```aivi
use aivi.result (isErr)

type Result Text Int -> Bool
func failed = result =>
    isErr result
```

---

## mapErr

Transforms the error inside `Err`, leaving `Ok` untouched.

**Type:** `transform:(E1 -> E2) -> opt:(Result E1 A) -> Result E2 A`

```aivi
use aivi.result (mapErr)

type Text -> Int
func toCode = message =>
    42

type Result Text Int -> (Result Int Int)
func withErrorCode = r => r
  |> mapErr toCode
```

---

## withDefault

Extracts the value from `Ok`, or returns the fallback if `Err`.

**Type:** `fallback:A -> opt:(Result E A) -> A`

```aivi
use aivi.result (withDefault)

type Result Text Int -> Int
func safeScore = result =>
    withDefault 0 result
```

---

## orElse

Returns the result unchanged if it is `Ok`, otherwise returns the fallback result.

**Type:** `fallback:(Result E A) -> opt:(Result E A) -> Result E A`

```aivi
use aivi.result (orElse)

type Result Text Int -> (Result Text Int) -> (Result Text Int)
func withFallback = primary secondary => primary
  |> orElse secondary
```

---

## flatMap

Chains a `Result`-returning function over an `Ok` value. Propagates `Err` without calling the function.

**Type:** `next:(A -> Result E B) -> opt:(Result E A) -> Result E B`

```aivi
use aivi.result (flatMap)

type Int -> (Result Text Int)
func ensurePositive = n => n > 0
 T|> Ok n
 F|> Err "must be positive"

type Result Text Int -> (Result Text Int)
func validateCount = result => result
  |> flatMap ensurePositive
```

---

## flatten

Removes one layer of nesting from a `Result E (Result E A)`.

**Type:** `opt:(Result E (Result E A)) -> Result E A`

```aivi
use aivi.result (flatten)

type Result Text (Result Text Int) -> (Result Text Int)
func unwrapNested = r =>
    flatten r
```

---

## toOption

Converts a `Result` to an `Option`, discarding the error. `Ok value` becomes `Some value`; `Err` becomes `None`.

**Type:** `opt:(Result E A) -> Option A`

```aivi
use aivi.result (toOption)

type Result Text Int -> (Option Int)
func justValue = result =>
    toOption result
```

---

## toList

Converts `Ok value` to a one-element list, or `Err` to an empty list.

**Type:** `opt:(Result E A) -> List A`

```aivi
use aivi.result (toList)

type Result Text Int -> (List Int)
func resultItems = result =>
    toList result
```

---

## map

Transforms the value inside `Ok` using a function, leaving `Err` untouched.

**Type:** `transform:(A -> B) -> opt:(Result E A) -> Result E B`

```aivi
use aivi.result (map)

type Int -> Int
func double = n =>
    n * 2

type Result Text Int -> (Result Text Int)
func doubleResult = result => result
  |> map double
```

---

## mapBoth

Transforms both sides of a `Result` simultaneously: `onErr` for `Err`, `onOk` for `Ok`.

**Type:** `onErr:(E1 -> E2) -> onOk:(A -> B) -> opt:(Result E1 A) -> Result E2 B`

```aivi
use aivi.result (mapBoth)

type Text -> Int
func toCode = message =>
    500

type Int -> Int
func double = n =>
    n * 2

type Result Text Int -> (Result Int Int)
func normalise = result =>
    mapBoth toCode double result
```

---

## fold

Collapses a `Result` to a single value by applying `onOk` to `Ok` or `onErr` to `Err`.

**Type:** `onErr:(E -> B) -> onOk:(A -> B) -> opt:(Result E A) -> B`

```aivi
use aivi.result (fold)

type Text -> Int
func zero = ignored =>
    0

type Int -> Int
func identity = n =>
    n

type Result Text Int -> Int
func resultToInt = result =>
    fold zero identity result
```

---

## fromOption

Converts an `Option` to a `Result`. `Some value` becomes `Ok value`; `None` becomes `Err error`.

**Type:** `error:E -> opt:(Option A) -> Result E A`

```aivi
use aivi.result (fromOption)

type Option Int -> (Result Text Int)
func requireAge = opt =>
    fromOption "Age is required" opt
```
