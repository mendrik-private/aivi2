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
| `Task E A` | A one-shot async computation description |

## Type Classes

| Class | Description |
|-------|-------------|
| `Eq A` | Equality comparison |
| `Ord A` | Ordering and comparison via `compare : A -> A -> Ordering`; ordinary `<`, `>`, `<=`, and `>=` derive from this member |
| `Default A` | A default value |
| `Functor F` | Mappable container |
| `Apply F` | Effectful function application |
| `Applicative F` | Applicative functor |
| `Chain M` | Dependent sequencing without changing the carrier family |
| `Monad F` | Monadic sequencing (`List`, `Option`, `Result`, and `Task` are the builtin executable carriers today) |
| `Foldable F` | Foldable container |
| `Traversable F` | Traversable container |
| `Filterable F` | Filterable container |
| `Semigroup A` | Associative combination |
| `Monoid A` | Semigroup with identity |
| `Bifunctor F` | Mappable over both type parameters |

For the current higher-kinded hierarchy, the canonical executable support reference, and the current
unary imported-instance slice for user-authored higher-kinded classes and instances, see
[Typeclasses & Higher-Kinded Support](/guide/typeclasses). For the law contract behind those classes,
see [Class Laws & Design Boundaries](/guide/class-laws). Parser or checker acceptance alone does not
imply executable runtime support.

## Option Functions

```aivi
use aivi.prelude (
    Option
    Text
    Bool
    getOrElse
    isSome
    isSomeAnd
    mapOr
    textNonEmpty
)

value name : Option Text = Some "Ada"
value displayName : Text = getOrElse "guest" name
value hasName : Bool = isSome name

type Text -> Text
func punctuate = name =>
    append name "!"

value foldedName : Text = mapOr "guest" punctuate name
value checkedName : Bool = isSomeAnd textNonEmpty name
```

## Result Functions

```aivi
value age : Result Text Int = Ok 30
value ageValue : Int = withDefault 0 age
value succeeded : Bool = isOk age
```

## Validation Functions

```aivi
use aivi.prelude (
    Text
    Validation
    isValid
    validationToResult
    validationGetOrElse
)

value checked : Validation Text Text = Valid "Ada"
value passed : Bool = isValid checked
value checkedText : Text = validationGetOrElse "guest" checked
value checkedResult : Result Text Text = validationToResult checked
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

`Ord.compare` is the primitive ordering member in the prelude. Any type with an `Ord` instance can use `min`, `max`, `minOf`, and the ordinary ordering operators directly.

```aivi
value smallest : Int = min 5 3
value greatest : Int = max 5 3

value leastOf : Int =
    minOf 10 [
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
