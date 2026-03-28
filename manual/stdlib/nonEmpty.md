# aivi.nonEmpty

Types and utilities for non-empty lists — collections that are **guaranteed to have at least one element**. This guarantee is enforced at the type level, making `head` and `last` always safe without returning `Option`.

```aivi
use aivi.nonEmpty (NonEmpty, NonEmptyList, singleton, cons, head, last, length, toList, fromNonEmpty, mapNel, fromList, appendNel)
```

---

## NonEmpty

A structural non-empty container holding a head element and a (possibly empty) tail list.

```aivi
type NonEmpty A =
  | MkNonEmpty A (List A)
```

`NonEmpty A` is the generic non-empty container. Use `fromNonEmpty` to convert it to a `NonEmptyList A`.

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
use aivi.nonEmpty (singleton)

fun wrapOne:(NonEmptyList Text) label:Text =>
    singleton label
```

---

## cons

Prepends an element to a `NonEmptyList`.

**Type:** `item:A -> nel:(NonEmptyList A) -> NonEmptyList A`

```aivi
use aivi.nonEmpty (singleton, cons)

fun buildList:(NonEmptyList Int) first:Int second:Int =>
    cons first (singleton second)
```

---

## head

Returns the first element of a `NonEmptyList`. Always safe — no `Option` required.

**Type:** `nel:(NonEmptyList A) -> A`

```aivi
use aivi.nonEmpty (head, singleton)

fun firstOf:(Int) nel:(NonEmptyList Int) =>
    head nel
```

---

## last

Returns the last element of a `NonEmptyList`. Always safe — no `Option` required.

**Type:** `nel:(NonEmptyList A) -> A`

```aivi
use aivi.nonEmpty (last, singleton)

fun finalItem:Int nel:(NonEmptyList Int) =>
    last nel
```

---

## length

Returns the number of elements in the list.

**Type:** `nel:(NonEmptyList A) -> Int`

```aivi
use aivi.nonEmpty (length, singleton, cons)

fun countItems:Int nel:(NonEmptyList Int) =>
    length nel
```

---

## toList

Converts a `NonEmptyList` to a regular `List`.

**Type:** `nel:(NonEmptyList A) -> List A`

```aivi
use aivi.nonEmpty (toList, singleton)

fun asRegularList:(List Int) nel:(NonEmptyList Int) =>
    toList nel
```

---

## fromNonEmpty

Converts a `NonEmpty A` to a `NonEmptyList A`.

**Type:** `ne:(NonEmpty A) -> NonEmptyList A`

```aivi
use aivi.nonEmpty (fromNonEmpty)

fun toNEL:(NonEmptyList Text) ne:(NonEmpty Text) =>
    fromNonEmpty ne
```

---

## mapNel

Applies a function to every element, producing a new `NonEmptyList`. The non-empty guarantee is preserved.

**Type:** `transform:(A -> B) -> nel:(NonEmptyList A) -> NonEmptyList B`

```aivi
use aivi.nonEmpty (mapNel)

fun double:Int n:Int =>
    n * 2

fun doubleAll:(NonEmptyList Int) nel:(NonEmptyList Int) =>
    mapNel double nel
```

---

## fromList

Attempts to convert a regular `List` to a `NonEmptyList`. Returns `None` if the list is empty.

**Type:** `items:(List A) -> Option (NonEmptyList A)`

```aivi
use aivi.nonEmpty (fromList)

fun safeFromList:(Option (NonEmptyList Int)) items:(List Int) =>
    fromList items
```

Use this when constructing a `NonEmptyList` from data whose size is not statically known, then handle the `None` case for empty input.

---

## appendNel

Concatenates two `NonEmptyList`s into one. The result is always non-empty.

**Type:** `left:(NonEmptyList A) -> right:(NonEmptyList A) -> NonEmptyList A`

```aivi
use aivi.nonEmpty (appendNel, singleton)

fun combineErrors:(NonEmptyList Text) a:(NonEmptyList Text) b:(NonEmptyList Text) =>
    appendNel a b
```

`appendNel` is used internally by `aivi.validation` to merge error lists from both sides of a failed `zipValidation`.
