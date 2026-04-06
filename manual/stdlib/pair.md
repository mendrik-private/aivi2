# aivi.pair

Utilities for working with two-element tuples. Pairs are written `(A, B)` and are the primary way to group two values of potentially different types.

```aivi
use aivi.pair (
    fst
    snd
    swap
    mapFst
    mapSnd
    mapBoth
    fromPair
    toPair
    duplicate
)
```

---

## fst

Extracts the first element of a pair.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (fst)

type (Text, Int) -> Text
func getKey = entry =>
    fst entry
```

---

## snd

Extracts the second element of a pair.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (snd)

type (Text, Int) -> Int
func getValue = entry =>
    snd entry
```

---

## swap

Swaps the two elements of a pair, returning `(B, A)` from `(A, B)`.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (swap)

type (Text, Int) -> (Int, Text)
func flipEntry = entry =>
    swap entry
```

---

## mapFst

Applies a function to the first element, leaving the second unchanged.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (mapFst)

use aivi.math (square)

type (Int, Text) -> (Int, Text)
func squareFst = pair =>
    mapFst square pair
```

---

## mapSnd

Applies a function to the second element, leaving the first unchanged.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (mapSnd)

use aivi.math (abs)

type (Text, Int) -> (Text, Int)
func absValue = entry =>
    mapSnd abs entry
```

---

## mapBoth

Applies one function to the first element and another to the second.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (mapBoth)

use aivi.math (
    abs
    negate
)

type (Int, Int) -> (Int, Int)
func normalizePair = pair =>
    mapBoth abs negate pair
```

---

## fromPair

Constructs a pair from two separate values.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (fromPair)

type Text -> Int -> (Text, Int)
func makeEntry = label score =>
    fromPair label score
```

---

## toPair

Constructs a pair from two separate values. Useful as a named combinator when pairing results in a pipeline.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (toPair)

type Text -> Int -> (Text, Int)
func labelScore = label score =>
    toPair label score
```

---

## duplicate

Creates a pair where both elements are the same value.

```aivi
// <unparseable item>
```

```aivi
use aivi.pair (duplicate)

type Int -> (Int, Int)
func mirror = n =>
    duplicate n
```
