# aivi.order

Utilities for ordering and comparison. All functions are parameterised by a **comparator** — a function `compare:(A -> A -> Bool)` that returns `True` when the first argument should come before the second (i.e. is "less than").

```aivi
use aivi.order (
    min
    max
    minOf
    maxOf
    clamp
    reversed
    comparing
)
```

A comparator for integers in ascending order looks like this:

```aivi
type Int -> Int -> Bool
func byInt = a b=>    a < b
```

Pass this (or any equivalent) as the `compare` argument to all functions in this module.

---

## min

Returns the smaller of two values according to the comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> A`

```aivi
use aivi.order (min)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> Int -> Int
func smaller = a b=>    min byInt a b
```

---

## max

Returns the larger of two values according to the comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> A`

```aivi
use aivi.order (max)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> Int -> Int
func larger = a b=>    max byInt a b
```

---

## minOf

Returns the smallest value in a non-empty collection represented as a first element and a rest list.

**Type:** `compare:(A -> A -> Bool) -> first:A -> rest:(List A) -> A`

```aivi
use aivi.order (minOf)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> (List Int) -> Int
func smallest = first rest=>    minOf byInt first rest
```

---

## maxOf

Returns the largest value in a non-empty collection represented as a first element and a rest list.

**Type:** `compare:(A -> A -> Bool) -> first:A -> rest:(List A) -> A`

```aivi
use aivi.order (maxOf)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> (List Int) -> Int
func largest = first rest=>    maxOf byInt first rest
```

---

## clamp

Constrains a value to the inclusive range `[low, high]`. If the value is below `low` it returns `low`; if above `high` it returns `high`; otherwise it returns the value unchanged.

**Type:** `compare:(A -> A -> Bool) -> low:A -> high:A -> value:A -> A`

```aivi
use aivi.order (clamp)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> Int
func clampScore = score=>    clamp byInt 0 100 score
```

---

## reversed

Flips a comparator so it produces the opposite ordering. Useful for sorting in descending order without writing a separate comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> Bool`

```aivi
use aivi.order (reversed)

type Int -> Int -> Bool
func byInt = a b=>    a < b

type Int -> Int -> Bool
func descending = a b=>    reversed byInt a b
```

Passing `descending` wherever a comparator is expected will produce largest-first results.

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
func byInt = a b=>    a < b

type Person -> Int
func ageOf = person=>    person.age

type Person -> Person -> Bool
func youngerFirst = p1 p2=>    comparing ageOf byInt p1 p2
```

The `.age` shorthand projects a `Person` to its `age` field. You can pass `youngerFirst` anywhere a `(Person -> Person -> Bool)` comparator is expected — for example as the `compare` argument to `minOf` or `clamp`.
