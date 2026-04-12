# aivi.bits

Compiler-backed bitwise integer intrinsics.

This is a low-level module for bitwise work on `Int` values.

```aivi
use aivi.bits (
    and
    or
    xor
    not
    shiftLeft
    shiftRight
    shiftRightUnsigned
)
```

## Exports

| Name | Type |
| --- | --- |
| `and` | `Int -> Int -> Int` |
| `or` | `Int -> Int -> Int` |
| `xor` | `Int -> Int -> Int` |
| `not` | `Int -> Int` |
| `shiftLeft` | `Int -> Int -> Int` |
| `shiftRight` | `Int -> Int -> Int` |
| `shiftRightUnsigned` | `Int -> Int -> Int` |

```aivi
use aivi.bits (
    and
    or
    xor
    not as bitNot
    shiftLeft
    shiftRight
)

value masked : Int = and 15 10
value combined : Int = or 5 10
value toggled : Int = xor 15 3
value inverted : Int = bitNot 0
value widened : Int = shiftLeft 1 4
value narrowed : Int = shiftRight 32 2
```
