# aivi.option

Utilities for working with `Option A` — a value that may or may not be present. All functions are pure and can be freely composed with pipes.

```aivi
use aivi.option (isSome, isNone, getOrElse, orElse, flatMap, flatten, toList, toResult, map, filter, zip, fromBool)
```

---

## isSome

Returns `True` if the option holds a value.

**Type:** `opt:(Option A) -> Bool`

```aivi
use aivi.option (isSome)

fun hasValue:Bool opt:(Option Int) =>
    isSome opt
```

---

## isNone

Returns `True` if the option is empty.

**Type:** `opt:(Option A) -> Bool`

```aivi
use aivi.option (isNone)

fun isMissing:Bool opt:(Option Text) =>
    isNone opt
```

---

## getOrElse

Extracts the value from `Some`, or returns the fallback if `None`.

**Type:** `fallback:A -> opt:(Option A) -> A`

```aivi
use aivi.option (getOrElse)

fun displayName:Text opt:(Option Text) =>
    getOrElse "Anonymous" opt
```

---

## orElse

Returns the option unchanged if it is `Some`, otherwise returns the fallback option.

**Type:** `fallback:(Option A) -> opt:(Option A) -> Option A`

```aivi
use aivi.option (orElse)

fun firstAvailable:(Option Text) primary:(Option Text) secondary:(Option Text) =>
    primary |> orElse secondary
```

---

## flatMap

Chains an `Option`-returning function over a `Some` value. Returns `None` when the input is `None` or when the function returns `None`.

**Type:** `next:(A -> Option B) -> opt:(Option A) -> Option B`

```aivi
use aivi.option (flatMap)

fun parsePositive:(Option Int) n:Int =>
    n
     ||> _ if n > 0 -> Some n
     ||> _          -> None

fun parseAndFilter:(Option Int) opt:(Option Int) =>
    opt |> flatMap parsePositive
```

---

## flatten

Removes one layer of nesting from an `Option (Option A)`.

**Type:** `opt:(Option (Option A)) -> Option A`

```aivi
use aivi.option (flatten)

fun unwrapNested:(Option Int) opt:(Option (Option Int)) =>
    flatten opt
```

---

## toList

Converts `Some value` to a one-element list, or `None` to an empty list.

**Type:** `opt:(Option A) -> List A`

```aivi
use aivi.option (toList)

fun optionItems:(List Int) opt:(Option Int) =>
    toList opt
```

---

## toResult

Converts an `Option` to a `Result`. `Some value` becomes `Ok value`; `None` becomes `Err error`.

**Type:** `error:E -> opt:(Option A) -> Result E A`

```aivi
use aivi.option (toResult)

fun requireName:(Result Text Text) opt:(Option Text) =>
    toResult "Name is required" opt
```

---

## map

Transforms the value inside `Some` using a function, leaving `None` untouched.

**Type:** `transform:(A -> B) -> opt:(Option A) -> Option B`

```aivi
use aivi.option (map)

fun double:Int n:Int =>
    n * 2

fun doubleOpt:(Option Int) opt:(Option Int) =>
    opt |> map double
```

---

## filter

Keeps the `Some` value only if it satisfies the predicate; otherwise returns `None`.

**Type:** `predicate:(A -> Bool) -> opt:(Option A) -> Option A`

```aivi
use aivi.option (filter)

fun isPositive:Bool n:Int =>
    n > 0

fun keepPositive:(Option Int) opt:(Option Int) =>
    opt |> filter isPositive
```

---

## zip

Pairs two options together. Produces `Some (a, b)` only when both inputs are `Some`.

**Type:** `left:(Option A) -> right:(Option B) -> Option (A, B)`

```aivi
use aivi.option (zip)

fun pairIfBoth:(Option (Int, Text)) count:(Option Int) label:(Option Text) =>
    zip count label
```

---

## fromBool

Wraps `item` in `Some` when `b` is `True`, or returns `None` when `b` is `False`.

**Type:** `item:A -> b:Bool -> Option A`

```aivi
use aivi.option (fromBool)

fun whenEnabled:(Option Text) enabled:Bool label:Text =>
    fromBool label enabled
```
