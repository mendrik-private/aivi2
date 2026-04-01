# aivi.option

Utilities for working with `Option A` — a value that may or may not be present. All functions are pure and can be freely composed with pipes.

```aivi
use aivi.option (
    isSome
    isNone
    getOrElse
    orElse
    flatMap
    flatten
    toList
    toResult
    map
    filter
    zip
    fromBool
)
```

---

## isSome

Returns `True` if the option holds a value.

**Type:** `opt:(Option A) -> Bool`

```aivi
use aivi.option (isSome)

type Option Int -> Bool
func hasValue = opt =>
    isSome opt
```

---

## isNone

Returns `True` if the option is empty.

**Type:** `opt:(Option A) -> Bool`

```aivi
use aivi.option (isNone)

type Option Text -> Bool
func isMissing = opt =>
    isNone opt
```

---

## getOrElse

Extracts the value from `Some`, or returns the fallback if `None`.

**Type:** `fallback:A -> opt:(Option A) -> A`

```aivi
use aivi.option (getOrElse)

type Option Text -> Text
func displayName = opt =>
    getOrElse "Anonymous" opt
```

---

## orElse

Returns the option unchanged if it is `Some`, otherwise returns the fallback option.

**Type:** `fallback:(Option A) -> opt:(Option A) -> Option A`

```aivi
use aivi.option (orElse)

type Option Text -> (Option Text) -> (Option Text)
func firstAvailable = primary secondary => primary
  |> orElse secondary
```

---

## flatMap

Chains an `Option`-returning function over a `Some` value. Returns `None` when the input is `None` or when the function returns `None`.

**Type:** `next:(A -> Option B) -> opt:(Option A) -> Option B`

```aivi
use aivi.option (flatMap)

type Int -> (Option Int)
func parsePositive = n => n > 0
 T|> Some n
 F|> None

type Option Int -> (Option Int)
func parseAndFilter = opt => opt
  |> flatMap parsePositive
```

---

## flatten

Removes one layer of nesting from an `Option (Option A)`.

**Type:** `opt:(Option (Option A)) -> Option A`

```aivi
use aivi.option (flatten)

type Option (Option Int) -> (Option Int)
func unwrapNested = opt =>
    flatten opt
```

---

## toList

Converts `Some value` to a one-element list, or `None` to an empty list.

**Type:** `opt:(Option A) -> List A`

```aivi
use aivi.option (toList)

type Option Int -> (List Int)
func optionItems = opt =>
    toList opt
```

---

## toResult

Converts an `Option` to a `Result`. `Some value` becomes `Ok value`; `None` becomes `Err error`.

**Type:** `error:E -> opt:(Option A) -> Result E A`

```aivi
use aivi.option (toResult)

type Option Text -> (Result Text Text)
func requireName = opt =>
    toResult "Name is required" opt
```

---

## map

Transforms the value inside `Some` using a function, leaving `None` untouched.

**Type:** `transform:(A -> B) -> opt:(Option A) -> Option B`

```aivi
use aivi.option (map)

type Int -> Int
func double = n =>
    n * 2

type Option Int -> (Option Int)
func doubleOpt = opt => opt
  |> map double
```

---

## filter

Keeps the `Some` value only if it satisfies the predicate; otherwise returns `None`.

**Type:** `predicate:(A -> Bool) -> opt:(Option A) -> Option A`

```aivi
use aivi.option (filter)

type Int -> Bool
func isPositive = n =>
    n > 0

type Option Int -> (Option Int)
func keepPositive = opt => opt
  |> filter isPositive
```

---

## zip

Pairs two options together. Produces `Some (a, b)` only when both inputs are `Some`.

**Type:** `left:(Option A) -> right:(Option B) -> Option (A, B)`

```aivi
use aivi.option (zip)

type Option Int -> (Option Text) -> (Option (Int, Text))
func pairIfBoth = count label =>
    zip count label
```

---

## fromBool

Wraps `item` in `Some` when `b` is `True`, or returns `None` when `b` is `False`.

**Type:** `item:A -> b:Bool -> Option A`

```aivi
use aivi.option (fromBool)

type Bool -> Text -> (Option Text)
func whenEnabled = enabled label =>
    fromBool label enabled
```
