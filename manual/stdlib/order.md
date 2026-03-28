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
fun byInt:Bool a:Int b:Int =>
    a < b
```

Pass this (or any equivalent) as the `compare` argument to all functions in this module.

---

## min

Returns the smaller of two values according to the comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> A`

```aivi
use aivi.order (min)

fun byInt:Bool a:Int b:Int =>
    a < b

fun smaller:Int a:Int b:Int =>
    min byInt a b
```

---

## max

Returns the larger of two values according to the comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> A`

```aivi
use aivi.order (max)

fun byInt:Bool a:Int b:Int =>
    a < b

fun larger:Int a:Int b:Int =>
    max byInt a b
```

---

## minOf

Returns the smallest value in a non-empty collection represented as a first element and a rest list.

**Type:** `compare:(A -> A -> Bool) -> first:A -> rest:(List A) -> A`

```aivi
use aivi.order (minOf)

fun byInt:Bool a:Int b:Int =>
    a < b

fun smallest:Int first:Int rest: (List Int) =>
    minOf byInt first rest
```

---

## maxOf

Returns the largest value in a non-empty collection represented as a first element and a rest list.

**Type:** `compare:(A -> A -> Bool) -> first:A -> rest:(List A) -> A`

```aivi
use aivi.order (maxOf)

fun byInt:Bool a:Int b:Int =>
    a < b

fun largest:Int first:Int rest: (List Int) =>
    maxOf byInt first rest
```

---

## clamp

Constrains a value to the inclusive range `[low, high]`. If the value is below `low` it returns `low`; if above `high` it returns `high`; otherwise it returns the value unchanged.

**Type:** `compare:(A -> A -> Bool) -> low:A -> high:A -> value:A -> A`

```aivi
use aivi.order (clamp)

fun byInt:Bool a:Int b:Int =>
    a < b

fun clampScore:Int score:Int =>
    clamp byInt 0 100 score
```

---

## reversed

Flips a comparator so it produces the opposite ordering. Useful for sorting in descending order without writing a separate comparator.

**Type:** `compare:(A -> A -> Bool) -> left:A -> right:A -> Bool`

```aivi
use aivi.order (reversed)

fun byInt:Bool a:Int b:Int =>
    a < b

fun descending:Bool a:Int b:Int =>
    reversed byInt a b
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

fun byInt:Bool a:Int b:Int =>
    a < b

fun ageOf:Int person:Person =>
    person.age

fun youngerFirst:Bool p1:Person p2:Person =>
    comparing ageOf byInt p1 p2
```

The `.age` shorthand projects a `Person` to its `age` field. You can pass `youngerFirst` anywhere a `(Person -> Person -> Bool)` comparator is expected — for example as the `compare` argument to `minOf` or `clamp`.
