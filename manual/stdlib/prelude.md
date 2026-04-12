# aivi.prelude

The `aivi.prelude` module is AIVI's convenience layer — it re-exports the most commonly used functions from across the standard library.

Because `aivi.prelude` declares `hoist`, all of its exports are automatically available in every AIVI file. You do not need a `use aivi.prelude (...)` declaration. Types like `Int`, `Bool`, `Text`, `List`, `Option`, `Result`, and type class constraints like `Eq`, `Ord`, `Functor`, and more are simply in scope everywhere.

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
| `Ord A` | Ordering and comparison via `compare : A -> A -> Ordering`; ordinary `<`, `>`, `<=`, and `>=` derive from this member |
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

For the current higher-kinded hierarchy, the executable builtin carrier matrix, and the current unary imported-instance slice for user-authored higher-kinded classes and instances, see [Typeclasses & Higher-Kinded Support](/guide/typeclasses). Parser or checker acceptance alone does not imply executable runtime support.

## Option Functions

```aivi
use aivi.prelude (
    Option
    Text
    Bool
    getOrElse
    isSome
    foldOption
    isSomeAnd
    textNonEmpty
)

value name : Option Text = Some "Ada"
value displayName : Text = getOrElse "guest" name
value hasName : Bool = isSome name

type Text -> Text
func punctuate = name =>
    append name "!"

value foldedName : Text = foldOption "guest" punctuate name
value checkedName : Bool = isSomeAnd textNonEmpty name
```

## Result Functions

```aivi
value age : Result Text Int = Ok 30
value ageValue : Int = withDefault 0 age
value succeeded : Bool = isOk age
```

## List Functions

```aivi
use aivi.prelude (
    Int
    Text
    Bool
    List
    length
    head
    isEmpty
    indexed
)

value items : List Text = [
    "Ada",
    "Grace",
    "Hedy"
]

value count : Int = length items
value first : Option Text = head items
value empty : Bool = isEmpty []
value indexedItems : List (Int, Text) = indexed items
```

```aivi
use aivi.prelude (
    Int
    mapWithIndex
    reduceWithIndex
)

type Int -> Int -> Int
func addIndex = index item =>
    index + item

type Int -> Int -> Int -> Int
func addIndexed = total index item =>
    total + index + item

value adjusted : List Int =
    mapWithIndex addIndex [
        10,
        20,
        30
    ]

value indexedTotal : Int =
    reduceWithIndex addIndexed 0 [
        10,
        20,
        30
    ]
```

## Order Functions

`Ord.compare` is the primitive ordering member in the prelude. Any type with an `Ord` instance can use the ordinary ordering operators and sections directly.

```aivi
type Int -> Int -> Bool
func earlier = a b =>
    a < b

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
value absolute : Int = abs (-5)
value flipped : Int = negate 7
value even : Bool = isEven 4
value clamped : Int = clamp 0 100 150
value inRange : Bool = between 1 10 5
```

## Bool Functions

```aivi
value inverted : Bool = not True
value exclusive : Bool = xor True False
```

## Pair Functions

```aivi
value pair : (Int, Text) = (
    42,
    "hello"
)

value firstValue : Int = first pair
value secondValue : Text = second pair
value swapped : (Text, Int) = swap pair
```
