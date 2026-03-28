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

fun succeeded:Bool result: (Result Text Int) =>
    isOk result
```

---

## isErr

Returns `True` if the result is `Err`.

**Type:** `opt:(Result E A) -> Bool`

```aivi
use aivi.result (isErr)

fun failed:Bool result: (Result Text Int) =>
    isErr result
```

---

## mapErr

Transforms the error inside `Err`, leaving `Ok` untouched.

**Type:** `transform:(E1 -> E2) -> opt:(Result E1 A) -> Result E2 A`

```aivi
use aivi.result (mapErr)

fun toCode:Int message:Text =>
    42

fun withErrorCode: (Result Int Int) r: (Result Text Int) => r
  |> mapErr toCode
```

---

## withDefault

Extracts the value from `Ok`, or returns the fallback if `Err`.

**Type:** `fallback:A -> opt:(Result E A) -> A`

```aivi
use aivi.result (withDefault)

fun safeScore:Int result: (Result Text Int) =>
    withDefault 0 result
```

---

## orElse

Returns the result unchanged if it is `Ok`, otherwise returns the fallback result.

**Type:** `fallback:(Result E A) -> opt:(Result E A) -> Result E A`

```aivi
use aivi.result (orElse)

fun withFallback: (Result Text Int) primary: (Result Text Int) secondary: (Result Text Int) => primary
  |> orElse secondary
```

---

## flatMap

Chains a `Result`-returning function over an `Ok` value. Propagates `Err` without calling the function.

**Type:** `next:(A -> Result E B) -> opt:(Result E A) -> Result E B`

```aivi
use aivi.result (flatMap)

fun ensurePositive: (Result Text Int) n:Int => n > 0
  T|> Ok n
  F|> Err "must be positive"

fun validateCount: (Result Text Int) result: (Result Text Int) => result
  |> flatMap ensurePositive
```

---

## flatten

Removes one layer of nesting from a `Result E (Result E A)`.

**Type:** `opt:(Result E (Result E A)) -> Result E A`

```aivi
use aivi.result (flatten)

fun unwrapNested: (Result Text Int) r: (Result Text (Result Text Int)) =>
    flatten r
```

---

## toOption

Converts a `Result` to an `Option`, discarding the error. `Ok value` becomes `Some value`; `Err` becomes `None`.

**Type:** `opt:(Result E A) -> Option A`

```aivi
use aivi.result (toOption)

fun justValue: (Option Int) result: (Result Text Int) =>
    toOption result
```

---

## toList

Converts `Ok value` to a one-element list, or `Err` to an empty list.

**Type:** `opt:(Result E A) -> List A`

```aivi
use aivi.result (toList)

fun resultItems: (List Int) result: (Result Text Int) =>
    toList result
```

---

## map

Transforms the value inside `Ok` using a function, leaving `Err` untouched.

**Type:** `transform:(A -> B) -> opt:(Result E A) -> Result E B`

```aivi
use aivi.result (map)

fun double:Int n:Int =>
    n * 2

fun doubleResult: (Result Text Int) result: (Result Text Int) => result
  |> map double
```

---

## mapBoth

Transforms both sides of a `Result` simultaneously: `onErr` for `Err`, `onOk` for `Ok`.

**Type:** `onErr:(E1 -> E2) -> onOk:(A -> B) -> opt:(Result E1 A) -> Result E2 B`

```aivi
use aivi.result (mapBoth)

fun toCode:Int message:Text =>
    500

fun double:Int n:Int =>
    n * 2

fun normalise: (Result Int Int) result: (Result Text Int) =>
    mapBoth toCode double result
```

---

## fold

Collapses a `Result` to a single value by applying `onOk` to `Ok` or `onErr` to `Err`.

**Type:** `onErr:(E -> B) -> onOk:(A -> B) -> opt:(Result E A) -> B`

```aivi
use aivi.result (fold)

fun zero:Int ignored:Text =>
    0

fun identity:Int n:Int =>
    n

fun resultToInt:Int result: (Result Text Int) =>
    fold zero identity result
```

---

## fromOption

Converts an `Option` to a `Result`. `Some value` becomes `Ok value`; `None` becomes `Err error`.

**Type:** `error:E -> opt:(Option A) -> Result E A`

```aivi
use aivi.result (fromOption)

fun requireAge: (Result Text Int) opt: (Option Int) =>
    fromOption "Age is required" opt
```
