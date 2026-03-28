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

```
identity : A -> A
```

```aivi
use aivi.core.fn (identity)

fun keepAsIs:Int n:Int =>
    identity n
```

---

## const

Returns a function that always returns its first argument, ignoring the second. Useful for discarding an input in a pipeline step.

```
const : A -> B -> A
```

```aivi
use aivi.core.fn (const)

fun alwaysForty:Int ignored:Text =>
    const 42 ignored
```

---

## flip

Reverses the order of the first two arguments of a two-argument function.

```
flip : (A -> B -> C) -> B -> A -> C
```

```aivi
use aivi.core.fn (flip)

use aivi.math (clamp)

fun clampFlipped:Int high:Int low:Int n:Int =>
    flip clamp high low n
```

---

## compose

Composes two functions, applying `g` first and then `f`. `compose f g x` is equivalent to `f (g x)`.

```
compose : (B -> C) -> (A -> B) -> A -> C
```

```aivi
use aivi.core.fn (compose)

use aivi.math (
    negate
    abs
)

fun negAbs:Int n:Int =>
    compose negate abs n
```

---

## andThen

Applies `f` first and then `g`. The reverse of `compose`. `andThen f g x` is equivalent to `g (f x)`. Often called "left-to-right composition" or `>>>`.

```
andThen : (A -> B) -> (B -> C) -> A -> C
```

```aivi
use aivi.core.fn (andThen)

use aivi.math (
    abs
    negate
)

fun absNeg:Int n:Int =>
    andThen abs negate n
```

---

## always

Returns a function that ignores its argument and always returns the given value. Equivalent to `const` with argument order swapped.

```
always : A -> B -> A
```

```aivi
use aivi.core.fn (always)

fun constantZero:Int ignored:Text =>
    always 0 ignored
```

---

## on

Applies a transformation `f` to both arguments before combining them with `combine`. Useful for comparing or combining values after mapping.

```
on : (B -> B -> C) -> (A -> B) -> A -> A -> C
```

```aivi
use aivi.core.fn (on)

use aivi.math (abs)

fun byInt:Bool left:Int right:Int =>
    left < right

fun absCompare:Bool x:Int y:Int =>
    on byInt abs x y
```

---

## applyTo

Applies a function to a value. `applyTo x f` is equivalent to `f x`. Useful for making value-first pipelines.

```
applyTo : A -> (A -> B) -> B
```

```aivi
use aivi.core.fn (applyTo)

use aivi.math (abs)

fun applyAbs:Int n:Int =>
    applyTo n abs
```

---

## applyTwice

Applies a function to itself twice: `applyTwice f x` is equivalent to `f (f x)`.

```
applyTwice : (A -> A) -> A -> A
```

```aivi
use aivi.core.fn (applyTwice)

use aivi.math (square)

fun fourthPower:Int n:Int =>
    applyTwice square n
```
