# aivi.nonEmpty

Types and utilities for non-empty lists — collections that are **guaranteed to have at least one element**. This guarantee is enforced at the type level, making `head` and `last` always safe without returning `Option`.

```aivi
use aivi.nonEmpty (
    NonEmpty
    NonEmptyList
    singleton
    cons
    head
    last
    length
    toList
    mapNel
    fromList
    appendNel
)
```

---

## NonEmpty

A structural non-empty container holding a head element and a (possibly empty) tail list.

```aivi
type NonEmpty A =
  | MkNonEmpty A (List A)
```

`NonEmpty A` is the generic non-empty container. Use `singleton`, `cons`, or `fromList` when you need to build a `NonEmptyList A`.

---

## NonEmptyList

The primary non-empty list type used throughout the standard library, including as the error carrier in `aivi.validation`.

```aivi
type NonEmptyList A =
  | MkNEL A (List A)
```

Construct values using `singleton`, `cons`, or `fromList`.

---

## singleton

Creates a `NonEmptyList` with exactly one element.

**Type:** `item:A -> NonEmptyList A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    singleton
)

type Text -> (NonEmptyList Text)
func wrapOne = label=>    singleton label
```

---

## cons

Prepends an element to a `NonEmptyList`.

**Type:** `item:A -> nel:(NonEmptyList A) -> NonEmptyList A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    singleton
    cons
)

type Int -> Int -> (NonEmptyList Int)
func buildList = first second=>    cons first (singleton second)
```

---

## head

Returns the first element of a `NonEmptyList`. Always safe — no `Option` required.

**Type:** `nel:(NonEmptyList A) -> A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    head
    singleton
)

type (NonEmptyList Int) -> Int
func firstOf = nel=>    head nel
```

---

## last

Returns the last element of a `NonEmptyList`. Always safe — no `Option` required.

**Type:** `nel:(NonEmptyList A) -> A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    last
    singleton
)

type (NonEmptyList Int) -> Int
func finalItem = nel=>    last nel
```

---

## length

Returns the number of elements in the list.

**Type:** `nel:(NonEmptyList A) -> Int`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    length
    singleton
    cons
)

type (NonEmptyList Int) -> Int
func countItems = nel=>    length nel
```

---

## toList

Converts a `NonEmptyList` to a regular `List`.

**Type:** `nel:(NonEmptyList A) -> List A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    toList
    singleton
)

type (NonEmptyList Int) -> (List Int)
func asRegularList = nel=>    toList nel
```

---

## NonEmpty conversion

There is currently no exported `fromNonEmpty` helper. Build a `NonEmptyList A` directly with `singleton`, `cons`, or `fromList`.

**Related constructors:** `singleton`, `cons`, `fromList`

```
use aivi.nonEmpty (
    singleton
    fromList
)
```

---

## mapNel

Applies a function to every element, producing a new `NonEmptyList`. The non-empty guarantee is preserved.

**Type:** `transform:(A -> B) -> nel:(NonEmptyList A) -> NonEmptyList B`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    mapNel
)

type Int -> Int
func double = n=>    n * 2

type (NonEmptyList Int) -> (NonEmptyList Int)
func doubleAll = nel=>    mapNel double nel
```

---

## fromList

Attempts to convert a regular `List` to a `NonEmptyList`. Returns `None` if the list is empty.

**Type:** `items:(List A) -> Option (NonEmptyList A)`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    fromList
)

type (List Int) -> (Option (NonEmptyList Int))
func safeFromList = items=>    fromList items
```

Use this when constructing a `NonEmptyList` from data whose size is not statically known, then handle the `None` case for empty input.

---

## appendNel

Concatenates two `NonEmptyList`s into one. The result is always non-empty.

**Type:** `left:(NonEmptyList A) -> right:(NonEmptyList A) -> NonEmptyList A`

```aivi
use aivi.nonEmpty (
    NonEmptyList
    appendNel
    singleton
)

type (NonEmptyList Text) -> (NonEmptyList Text) -> (NonEmptyList Text)
func combineErrors = a b=>    appendNel a b
```

`appendNel` is used internally by `aivi.validation` to merge error lists from both sides of a failed `zipValidation`.
