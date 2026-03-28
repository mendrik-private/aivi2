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
type RangeInt = { start: Int, end: Int }
```

An inclusive integer range. A range where `start > end` is considered **empty**.

---

## Construction

### `make : Int -> Int -> RangeInt`

Create a range from `start` to `end` (inclusive).

```aivi
use aivi.core.range (make)

value r = make 1 10  -- [1, 10]
```

---

## Querying

### `isEmpty : RangeInt -> Bool`

Returns `True` when `start > end`.

```aivi
make 5 3 |> isEmpty  -- True
make 1 1 |> isEmpty  -- False
```

### `contains : RangeInt -> Int -> Bool`

Returns `True` when `n` is within the range (inclusive on both ends).

```aivi
make 0 100 |> contains 50  -- True
make 0 100 |> contains 101 -- False
```

### `length : RangeInt -> Int`

Returns the number of integers in the range. Empty ranges have length `0`.

```aivi
make 1 5  |> length  -- 5  (1, 2, 3, 4, 5)
make 7 7  |> length  -- 1
make 10 5 |> length  -- 0  (empty)
```

### `startOf : RangeInt -> Int` / `endOf : RangeInt -> Int`

Extract the start or end bound.

```aivi
make 3 9 |> startOf  -- 3
make 3 9 |> endOf    -- 9
```

---

## Operations

### `clampTo : RangeInt -> Int -> Int`

Clamp a value to the range boundaries.

```aivi
make 0 255 |> clampTo 300  -- 255
make 0 255 |> clampTo (-5) -- 0
make 0 255 |> clampTo 128  -- 128
```

### `shift : Int -> RangeInt -> RangeInt`

Translate the entire range by a delta.

```aivi
make 0 10 |> shift 5   -- RangeInt { start: 5, end: 15 }
make 0 10 |> shift (-3) -- RangeInt { start: -3, end: 7 }
```

### `overlaps : RangeInt -> RangeInt -> Bool`

Returns `True` when two ranges share at least one integer. Empty ranges never overlap.

```aivi
overlaps (make 1 5) (make 3 8)   -- True  (share 3..5)
overlaps (make 1 5) (make 6 10)  -- False
overlaps (make 5 1) (make 1 5)   -- False (first is empty)
```

### `intersect : RangeInt -> RangeInt -> RangeInt`

Returns the intersection of two ranges. If the ranges do not overlap the result is an empty range (`start > end`).

```aivi
intersect (make 1 8) (make 5 12)  -- RangeInt { start: 5, end: 8 }
intersect (make 0 3) (make 7 10)  -- RangeInt { start: 7, end: 3 } (empty)
```

---

## Real-world example

```aivi
use aivi.core.range (make, contains, clampTo, length)

type Viewport = { visible: RangeInt, total: Int }

fun visibleRows:(List Int) viewport:Viewport rows:(List Int) =>
    rows |> filter (contains viewport.visible)

fun scrollProgress:Float viewport:Viewport =>
    viewport.visible.start |> toFloat |> divide (toFloat viewport.total)
```
