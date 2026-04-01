# aivi.core.range

Integer range utilities with an inclusive `[start, end]` interval type. All operations are pure AIVI — no I/O, no intrinsics.

```aivi
use aivi.core.range (
    RangeInt
    make
    isEmpty
    contains
    length
    overlaps
    clampTo
    startOf
    endOf
    shift
    intersect
)
```

---

## Type

### `RangeInt`

```aivi
type RangeInt = {
    start: Int,
    end: Int
}
```

An inclusive integer range. A range where `start > end` is considered **empty**.

---

## Construction

### `make : Int -> Int -> RangeInt`

Create a range from `start` to `end` (inclusive).

```aivi
use aivi.core.range (make)

value r = make 1 10
```

---

## Querying

### `isEmpty : RangeInt -> Bool`

Returns `True` when `start > end`.

```aivi
use aivi.core.range (
    isEmpty
    make
)

value none : Bool = isEmpty (make 3 1)
```

### `contains : RangeInt -> Int -> Bool`

Returns `True` when `n` is within the range (inclusive on both ends).

```aivi
use aivi.core.range (
    contains
    make
)

value hasFive : Bool = contains (make 1 10) 5
```

### `length : RangeInt -> Int`

Returns the number of integers in the range. Empty ranges have length `0`.

```aivi
use aivi.core.range (
    length
    make
)

value count : Int = length (make 4 7)
```

### `startOf : RangeInt -> Int` / `endOf : RangeInt -> Int`

Extract the start or end bound.

```aivi
use aivi.core.range (
    startOf
    endOf
    make
)

value start : Int = startOf (make 4 7)
value finish : Int = endOf (make 4 7)
```

---

## Operations

### `clampTo : RangeInt -> Int -> Int`

Clamp a value to the range boundaries.

```aivi
use aivi.core.range (
    clampTo
    make
)

value clamped : Int = clampTo (make 1 10) 15
```

### `shift : Int -> RangeInt -> RangeInt`

Translate the entire range by a delta.

```aivi
use aivi.core.range (
    RangeInt
    shift
    make
)

value shifted : RangeInt = shift 3 (make 1 5)
```

### `overlaps : RangeInt -> RangeInt -> Bool`

Returns `True` when two ranges share at least one integer. Empty ranges never overlap.

```aivi
use aivi.core.range (
    make
    overlaps
)

value sharesValues : Bool = overlaps (make 1 5) (make 4 8)
```

### `intersect : RangeInt -> RangeInt -> RangeInt`

Returns the intersection of two ranges. If the ranges do not overlap the result is an empty range (`start > end`).

```aivi
use aivi.core.range (
    RangeInt
    intersect
    make
)

value shared : RangeInt = intersect (make 1 5) (make 4 8)
```

---

## Real-world example

```aivi
use aivi.core.range (
    RangeInt
    clampTo
    length
)

type Viewport = {
    visible: RangeInt,
    total: Int
}

type Viewport -> RangeInt
func visibleRange = viewport => viewport
 ||> { visible, total } -> visible

type Viewport -> Int
func visibleCount = viewport =>
    length (visibleRange viewport)

type Viewport -> Int -> Int
func clampSelection = viewport row =>
    clampTo (visibleRange viewport) row
```
