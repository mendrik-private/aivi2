# aivi.pair

Utilities for working with two-element tuples. Pairs are written `(A, B)` and are the primary way to group two values of potentially different types.

```aivi
use aivi.pair (fst, snd, swap, mapFst, mapSnd, mapBoth, fromPair, toPair, duplicate)
```

---

## fst

Extracts the first element of a pair.

```
fst : (A, B) -> A
```

```aivi
use aivi.pair (fst)

fun getKey:Text entry:(Text, Int) =>
    fst entry
```

---

## snd

Extracts the second element of a pair.

```
snd : (A, B) -> B
```

```aivi
use aivi.pair (snd)

fun getValue:Int entry:(Text, Int) =>
    snd entry
```

---

## swap

Swaps the two elements of a pair, returning `(B, A)` from `(A, B)`.

```
swap : (A, B) -> (B, A)
```

```aivi
use aivi.pair (swap)

fun flipEntry:(Int, Text) entry:(Text, Int) =>
    swap entry
```

---

## mapFst

Applies a function to the first element, leaving the second unchanged.

```
mapFst : (A -> C) -> (A, B) -> (C, B)
```

```aivi
use aivi.pair (mapFst)
use aivi.math (square)

fun squareFst:(Int, Text) pair:(Int, Text) =>
    mapFst square pair
```

---

## mapSnd

Applies a function to the second element, leaving the first unchanged.

```
mapSnd : (B -> C) -> (A, B) -> (A, C)
```

```aivi
use aivi.pair (mapSnd)
use aivi.math (abs)

fun absValue:(Text, Int) entry:(Text, Int) =>
    mapSnd abs entry
```

---

## mapBoth

Applies one function to the first element and another to the second.

```
mapBoth : (A -> C) -> (B -> D) -> (A, B) -> (C, D)
```

```aivi
use aivi.pair (mapBoth)
use aivi.math (abs, negate)

fun normalizePair:(Int, Int) pair:(Int, Int) =>
    mapBoth abs negate pair
```

---

## fromPair

Constructs a pair from two separate values.

```
fromPair : A -> B -> (A, B)
```

```aivi
use aivi.pair (fromPair)

fun makeEntry:(Text, Int) label:Text score:Int =>
    fromPair label score
```

---

## toPair

Constructs a pair from two separate values. Useful as a named combinator when pairing results in a pipeline.

```
toPair : A -> B -> (A, B)
```

```aivi
use aivi.pair (toPair)

fun labelScore:(Text, Int) label:Text score:Int =>
    toPair label score
```

---

## duplicate

Creates a pair where both elements are the same value.

```
duplicate : A -> (A, A)
```

```aivi
use aivi.pair (duplicate)

fun mirror:(Int, Int) n:Int =>
    duplicate n
```
