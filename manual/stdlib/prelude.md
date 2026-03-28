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
| `Monad F` | Monadic chaining |
| `Foldable F` | Foldable container |
| `Traversable F` | Traversable container |
| `Filterable F` | Filterable container |
| `Semigroup A` | Associative combination |
| `Monoid A` | Semigroup with identity |
| `Bifunctor F` | Mappable over both type parameters |

## Option Functions

```aivi
use aivi.prelude (getOrElse, isSome, isNone, mapOption, filterOption)

value name: Option Text = Some "Ada"

value displayName: Text = getOrElse "guest" name
// "Ada"

value hasName: Bool = isSome name
// True
```

## Result Functions

```aivi
use aivi.prelude (withDefault, isOk, isErr)

value age: Result Text Int = Ok 30

value ageValue: Int = withDefault 0 age
// 30

value succeeded: Bool = isOk age
// True
```

## List Functions

```aivi
use aivi.prelude (length, head, isEmpty, nonEmpty, reverse, take)

value items: List Text = ["Ada", "Grace", "Hedy"]

value count: Int = length items
// 3

value first: Option Text = head items
// Some "Ada"

value empty: Bool = isEmpty []
// True
```

## Order Functions

```aivi
use aivi.prelude (min, max, minOf)

fun earlier:Bool a:Int b:Int =>
    a < b

value smallest: Int = min earlier 5 3
// 3

value greatest: Int = max earlier 5 3
// 5

value leastOf: Int = minOf earlier 10 [7, 4, 9]
// 4
```

## Text Functions

```aivi
use aivi.prelude (join, concat, surround)

value csv: Text = join ", " ["Ada", "Grace", "Hedy"]
// "Ada, Grace, Hedy"

value combined: Text = concat ["Hello", " ", "World"]
// "Hello World"

value wrapped: Text = surround "(" ")" "AIVI"
// "(AIVI)"
```

## Math Functions

```aivi
use aivi.prelude (abs, negate, isEven, clamp, between)

value absolute: Int = abs -5
// 5

value flipped: Int = negate 7
// -7

value even: Bool = isEven 4
// True

value clamped: Int = clamp 0 100 150
// 100

value inRange: Bool = between 1 10 5
// True
```

## Bool Functions

```aivi
use aivi.prelude (not, xor, implies)

value inverted: Bool = not True
// False

value exclusive: Bool = xor True False
// True
```

## Pair Functions

```aivi
use aivi.prelude (fst, snd, swap)

value pair: (Int, Text) = (42, "hello")

value first: Int = fst pair
// 42

value second: Text = snd pair
// "hello"

value swapped: (Text, Int) = swap pair
// ("hello", 42)
```
