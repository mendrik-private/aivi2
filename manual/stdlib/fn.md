# aivi.core.fn

Higher-order function combinators. This module provides building blocks for working with functions as first-class values — composing, flipping, and applying them in pipelines.

```aivi
use aivi.core.fn (
    identity
    const
    flip
    compose
    andThen
    always
    on
    applyTo
    applyTwice
)
```

---

## identity

Returns its argument unchanged. Useful as a no-op transformer in pipelines.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (identity)

type Int -> Int
func keepAsIs = n =>
    identity n
```

---

## const

Returns a function that always returns its first argument, ignoring the second. Useful for discarding an input in a pipeline step.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (const)

type Text -> Int
func alwaysForty = ignored =>
    const 42 ignored
```

---

## flip

Reverses the order of the first two arguments of a two-argument function.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (flip)

use aivi.math (clamp)

type Int -> Int -> Int -> Int
func clampFlipped = high low n =>
    flip clamp high low n
```

---

## compose

Composes two functions, applying `g` first and then `f`. `compose f g x` is equivalent to `f (g x)`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (compose)

use aivi.math (
    negate
    abs
)

type Int -> Int
func negAbs = n =>
    compose negate abs n
```

---

## andThen

Applies `f` first and then `g`. The reverse of `compose`. `andThen f g x` is equivalent to `g (f x)`. Often called "left-to-right composition" or `>>>`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (andThen)

use aivi.math (
    abs
    negate
)

type Int -> Int
func absNeg = n =>
    andThen abs negate n
```

---

## always

Returns a function that ignores its argument and always returns the given value. Equivalent to `const` with argument order swapped.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (always)

type Text -> Int
func constantZero = ignored =>
    always 0 ignored
```

---

## on

Applies a transformation `f` to both arguments before combining them with `combine`. Useful for comparing or combining values after mapping.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (on)

use aivi.math (abs)

type Int -> Int -> Bool
func byInt = left right =>
    left < right

type Int -> Int -> Bool
func absCompare = x y =>
    on byInt abs x y
```

---

## applyTo

Applies a function to a value. `applyTo x f` is equivalent to `f x`. Useful for making value-first pipelines.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (applyTo)

use aivi.math (abs)

type Int -> Int
func applyAbs = n =>
    applyTo n abs
```

---

## applyTwice

Applies a function to itself twice: `applyTwice f x` is equivalent to `f (f x)`.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.fn (applyTwice)

use aivi.math (square)

type Int -> Int
func fourthPower = n =>
    applyTwice square n
```
