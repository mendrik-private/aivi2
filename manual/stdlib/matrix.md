# aivi.matrix

Rectangular two-dimensional collections.

`aivi.matrix` provides a generic `Matrix A` type for NxM data laid out by zero-based `x` and `y`
coordinates. It is meant for boards, tiles, seat maps, and other rectangular grids rather than
numeric linear algebra.

## Import

```aivi
use aivi.matrix (
    Matrix
    MatrixError
    init
    fromRows
    width
    height
    rows
    row
    at
    replaceAt
)
```

## Overview

| Name | Type | Description |
| --- | --- | --- |
| `Matrix A` | opaque generic type | A rectangular row-major grid of values |
| `MatrixError` | sum type | Constructor/validation errors |
| `init` | `Int -> Int -> (Int -> Int -> A) -> Result MatrixError (Matrix A)` | Build a matrix from `x` and `y` coordinates |
| `fromRows` | `List (List A) -> Result MatrixError (Matrix A)` | Validate an existing nested-list shape |
| `width` | `Matrix A -> Int` | Number of columns |
| `height` | `Matrix A -> Int` | Number of rows |
| `rows` | `Matrix A -> List (List A)` | Expose the row-major carrier |
| `row` | `Matrix A -> Int -> Option (List A)` | Read one zero-based row |
| `at` | `Matrix A -> Int -> Int -> Option A` | Read one cell by `x` then `y` |
| `replaceAt` | `Matrix A -> (Int, Int) -> A -> Option (Matrix A)` | Replace one cell, preserving rectangular shape |

## Error type

```aivi
type MatrixError =
  | NegativeWidth Int
  | NegativeHeight Int
  | RaggedRows Int Int Int
```

- `NegativeWidth w` means `init` was called with a negative width.
- `NegativeHeight h` means `init` was called with a negative height.
- `RaggedRows rowIndex expected actual` means `fromRows` found a row whose length did not match the
  first row. `rowIndex` is zero-based.

## `init`

```aivi
init : Int -> Int -> (Int -> Int -> A) -> Result MatrixError (Matrix A)
```

`init width height build` calls `build x y` for every zero-based coordinate in the rectangle.
`x` is the column index and `y` is the row index.

```aivi
use aivi.matrix (
    Matrix
    MatrixError
    init
    at
)

type Int -> Int -> Int
func seatNumber x y =>
    x + y * 100

value seats : Result MatrixError (Matrix Int) =
    init 3 2 seatNumber

value middleSeat : Result MatrixError (Option Int) = seats
 ||> Err error  -> Err error
 ||> Ok matrix  -> Ok (at matrix 1 1)
```

## `fromRows`

```aivi
fromRows : List (List A) -> Result MatrixError (Matrix A)
```

Use `fromRows` when you already have nested lists and want to verify that every row has the same
length.

```aivi
use aivi.matrix (
    Matrix
    MatrixError
    fromRows
)

value board : Result MatrixError (Matrix Text) =
    fromRows [
        ["A", "B", "C"],
        ["D", "E", "F"]
    ]
```

## Dimensions and access

```aivi
width : Matrix A -> Int
height : Matrix A -> Int
rows : Matrix A -> List (List A)
row : Matrix A -> Int -> Option (List A)
at : Matrix A -> Int -> Int -> Option A
```

`width` and `height` report the current rectangular shape. `row` and `at` return `None` when the
requested index is out of bounds.

```aivi
use aivi.matrix (
    Matrix
    MatrixError
    init
    width
    height
    row
    at
)

type Int -> Int -> Int
func cell x y =>
    x + y * 10

value board : Result MatrixError (Matrix Int) = init 4 3 cell

value boardWidth : Result MatrixError Int = board
 ||> Err error -> Err error
 ||> Ok matrix -> Ok (width matrix)

value boardHeight : Result MatrixError Int = board
 ||> Err error -> Err error
 ||> Ok matrix -> Ok (height matrix)

value firstRow : Result MatrixError (Option (List Int)) = board
 ||> Err error -> Err error
 ||> Ok matrix -> Ok (row matrix 0)

value corner : Result MatrixError (Option Int) = board
 ||> Err error -> Err error
 ||> Ok matrix -> Ok (at matrix 3 2)
```

## `replaceAt`

```aivi
replaceAt : Matrix A -> (Int, Int) -> A -> Option (Matrix A)
```

`replaceAt` returns `Some updatedMatrix` when both coordinates are in bounds, otherwise `None`.

```aivi
use aivi.matrix (
    Matrix
    MatrixError
    init
    replaceAt
)

type Int -> Int -> Bool
func isWall x y =>
    x == 0 or y == 0

value board : Result MatrixError (Matrix Bool) = init 3 3 isWall

value patched : Result MatrixError (Option (Matrix Bool)) = board
 ||> Err error -> Err error
 ||> Ok matrix -> Ok (replaceAt matrix (1, 1) True)
```

## Notes

- Coordinates are zero-based.
- Matrices are row-major: `rows matrix` returns the carrier as `List (List A)`.
- `init 0 height ...` and `init width 0 ...` are valid and produce empty columns or rows; only
  negative dimensions are rejected.
