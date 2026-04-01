# aivi.prelude

The `aivi.prelude` module is AIVI's convenience layer — it re-exports the most commonly used functions from across the standard library so you can access them all from a single import.

It also serves as the source of built-in types and type classes such as `Int`, `Bool`, `Text`, `List`, `Option`, `Result`, and type class constraints like `Eq`, `Ord`, `Functor`, and more.

## Usage

```aivi
use aivi.prelude (
    Int
    Bool
    Text
    List
    getOrElse
    withDefault
    length
    head
    min
    max
    join
)
```

## Built-in Types

These types are always available and can be imported from `aivi.prelude`:

| Type | Description |
|------|-------------|
| `Int` | 64-bit signed integer |
| `Float` | 64-bit floating point |
| `Decimal` | Arbitrary-precision decimal |
| `BigInt` | Arbitrary-precision integer |
| `Bool` | Boolean: `True` or `False` |
| `Text` | Unicode text string |
| `Unit` | The unit type (no value) |
| `Ordering` | Result of comparison: `Less`, `Equal`, or `Greater` |
| `List A` | Ordered collection |
| `Option A` | Optional value: `Some value` or `None` |
| `Result E A` | Success or failure: `Ok value` or `Err error` |
| `Validation E A` | Accumulating validation: `Valid value` or `Invalid errors` |
| `Signal A` | A reactive value that changes over time |
| `Task E A` | An async computation |

## Type Classes

| Class | Description |
|-------|-------------|
| `Eq A` | Equality comparison |
| `Ord A` | Ordering and comparison |
| `Default A` | A default value |
| `Functor F` | Mappable container |
| `Applicative F` | Applicative functor |
| `Monad F` | Monadic chaining (`List`, `Option`, and `Result` are the builtin executable carriers today) |
| `Foldable F` | Foldable container |
| `Traversable F` | Traversable container |
| `Filterable F` | Filterable container |
| `Semigroup A` | Associative combination |
| `Monoid A` | Semigroup with identity |
| `Bifunctor F` | Mappable over both type parameters |

For the current higher-kinded hierarchy, the executable builtin carrier matrix, and the current same-module-only limits for user-authored higher-kinded classes and instances, see [Typeclasses & Higher-Kinded Support](/guide/typeclasses). Parser or checker acceptance alone does not imply executable runtime support.

## Option Functions

```aivi
use aivi.prelude (
    getOrElse
    isSome
    isNone
    mapOption
    filterOption
)

value name : Option Text = Some "Ada"
value displayName : Text = getOrElse "guest" name
value hasName : Bool = isSome name
```

## Result Functions

```aivi
use aivi.prelude (
    withDefault
    isOk
    isErr
)

value age : Result Text Int = Ok 30
value ageValue : Int = withDefault 0 age
value succeeded : Bool = isOk age
```

## List Functions

```aivi
use aivi.prelude (
    length
    head
    isEmpty
    nonEmpty
    reverse
    take
)

value items : List Text = [
    "Ada",
    "Grace",
    "Hedy"
]

value count : Int = length items
value first : Option Text = head items
value empty : Bool = isEmpty []
```

## Order Functions

```aivi
use aivi.prelude (
    min
    max
    minOf
)

type Int -> Int -> Bool
func earlier = a b=>    a < b

value smallest : Int = min earlier 5 3
value greatest : Int = max earlier 5 3

value leastOf : Int =
    minOf earlier 10 [
        7,
        4,
        9
    ]
```

## Text Functions

```aivi
use aivi.prelude (
    join
    surround
)

value csv : Text =
    join ", " [
        "Ada",
        "Grace",
        "Hedy"
    ]

value combined : Text =
    join "" [
        "Hello",
        " ",
        "World"
    ]

value wrapped : Text = surround "(" ")" "AIVI"
```

## Math Functions

```aivi
use aivi.prelude (
    abs
    negate
    isEven
    clamp
    between
)

value absolute : Int = abs (-5)
value flipped : Int = negate 7
value even : Bool = isEven 4
value clamped : Int = clamp 0 100 150
value inRange : Bool = between 1 10 5
```

## Bool Functions

```aivi
use aivi.prelude (
    not
    xor
    implies
)

value inverted : Bool = not True
value exclusive : Bool = xor True False
```

## Pair Functions

```aivi
use aivi.prelude (
    fst
    snd
    swap
)

value pair : (Int, Text) = (
    42,
    "hello"
)

value first : Int = fst pair
value second : Text = snd pair
value swapped : (Text, Int) = swap pair
```
