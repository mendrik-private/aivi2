# aivi.arithmetic

Compiler-backed integer arithmetic intrinsics.

This is a low-level module. The everyday arithmetic surface in AIVI is still the ordinary operators
`+`, `-`, `*`, `/`, and the helpers in [`aivi.math`](math.md). Reach for `aivi.arithmetic` when you
need named integer operations as first-class functions.

```aivi
use aivi.arithmetic (
    add
    sub
    mul
    div
    mod
    neg
)
```

## Exports

| Name | Type |
| --- | --- |
| `add` | `Int -> Int -> Int` |
| `sub` | `Int -> Int -> Int` |
| `mul` | `Int -> Int -> Int` |
| `div` | `Int -> Int -> Int` |
| `mod` | `Int -> Int -> Int` |
| `neg` | `Int -> Int` |

```aivi
use aivi.arithmetic (
    add
    sub
    mul
    div
    mod
    neg
)

value total : Int = add 20 22
value delta : Int = sub 10 3
value scaled : Int = mul 6 7
value quotient : Int = div 21 3
value remainder : Int = mod 22 5
value flipped : Int = neg 8
```
