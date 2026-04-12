# aivi.order

Utilities for ordering and comparison.

The canonical surface is `Ord`-driven:

- `min`, `max`, `minOf`, `maxOf`, and `clamp` work directly for any `Ord` type
- explicit custom orderings use the `...By` variants
- `reversed` and `comparing` help build those custom comparators

```aivi
use aivi.order (
    min
    max
    minOf
    maxOf
    clamp
    minBy
    reversed
    comparing
)
```

An explicit comparator for integers in ascending order looks like this:

```aivi
type Int -> Int -> Bool
func byInt = a b =>
    a < b
```

Pass this (or any equivalent) to the `...By` helpers when you need a custom ordering.

---

## min

Returns the smaller of two `Ord` values.

**Type:** `left:A -> right:A -> A`

```aivi
use aivi.order (min)

type Int -> Int -> Int
func smaller = a b =>
    min a b
```

---

## max

Returns the larger of two `Ord` values.

**Type:** `left:A -> right:A -> A`

```aivi
use aivi.order (max)

type Int -> Int -> Int
func larger = a b =>
    max a b
```

---

## minOf

Returns the smallest value in a non-empty collection represented as a first element and a rest list.

**Type:** `first:A -> rest:(List A) -> A`

```aivi
use aivi.order (minOf)

type Int -> (List Int) -> Int
func smallest = first rest =>
    minOf first rest
```

---

## maxOf

Returns the largest value in a non-empty collection represented as a first element and a rest list.

**Type:** `first:A -> rest:(List A) -> A`

```aivi
use aivi.order (maxOf)

type Int -> (List Int) -> Int
func largest = first rest =>
    maxOf first rest
```

---

## clamp

Constrains an `Ord` value to the inclusive range `[low, high]`. If the value is below `low` it returns `low`; if above `high` it returns `high`; otherwise it returns the value unchanged.

**Type:** `low:A -> high:A -> value:A -> A`

```aivi
use aivi.order (clamp)

type Int -> Int
func clampScore = score =>
    clamp 0 100 score
```

---

## minBy

Returns the smaller of two values according to an explicit comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> A`

```aivi
use aivi.order (minBy)

type Int -> Int -> Bool
func byInt = a b =>
    a < b

type Int -> Int -> Int
func smaller = a b =>
    minBy byInt a b
```

## maxBy / minOfBy / maxOfBy / clampBy

The remaining `...By` helpers are the explicit-comparator counterparts of the canonical `Ord`-driven surface:

- `maxBy : (A -> A -> Bool) -> A -> A -> A`
- `minOfBy : (A -> A -> Bool) -> A -> List A -> A`
- `maxOfBy : (A -> A -> Bool) -> A -> List A -> A`
- `clampBy : (A -> A -> Bool) -> A -> A -> A -> A`

Use them when you need a custom ordering instead of the ambient `Ord` instance.

---

## reversed

Flips a comparator so it produces the opposite ordering. Useful for sorting in descending order without writing a separate comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> Bool`

```aivi
use aivi.order (reversed)

type Int -> Int -> Bool
func byInt = a b =>
    a < b

type Int -> Int -> Bool
func descending = a b =>
    reversed byInt a b
```

Passing `descending` wherever a comparator is expected will produce largest-first results — for example with `minBy`, `maxBy`, `sortBy`, or `maximumBy`.

---

## comparing

Lifts a comparator on `B` to a comparator on `A` by first projecting each value with a function `project:(A -> B)`. Useful for comparing records by a single field.

**Type:** `project:(A -> B) -> compare:(B -> B -> Bool) -> left:A -> right:A -> Bool`

```aivi
use aivi.order (comparing)

type Person = {
    name: Text,
    age: Int
}

type Int -> Int -> Bool
func byInt = a b =>
    a < b

type Person -> Int
func ageOf = person =>
    person.age

type Person -> Person -> Bool
func youngerFirst = p1 p2 =>
    comparing ageOf byInt p1 p2
```

The `.age` shorthand projects a `Person` to its `age` field. You can pass `youngerFirst` anywhere a `(Person -> Person -> Bool)` comparator is expected — for example as the `compare` argument to `minOfBy` or `clampBy`.
