# Functions

Functions in AIVI are declared with `fun`. They are pure by default — a function depends only
on its explicit parameters and always returns the same result for the same inputs.

## Basic syntax

```aivi
fun add:Int #x:Int #y:Int =>
    x + y
```

Breaking this down:

| Part | Meaning |
|---|---|
| `fun` | declaration keyword |
| `add` | function name |
| `:Int` | **return type** (comes before parameters) |
| `#x:Int` | labeled parameter named `x` of type `Int` |
| `#y:Int` | labeled parameter named `y` of type `Int` |
| `=>` | separates signature from body |
| `x + y` | function body — any expression |

The return type prefix (`add:Int`) is one of AIVI's deliberate design choices:
reading left to right, you see the name and return type before the parameters.

## Labeled parameters

All parameters in AIVI are labeled with `#`. This means you always know what an argument
represents at the call site:

```aivi
fun greet:Text #name:Text #title:Text =>
    "{title} {name}"

val msg:Text = greet "Lovelace" "Ms"
```

Function calls are positional juxtaposition. You pass arguments in the order the parameters
are declared. The `#` sigil on the parameter declaration signals that this is a labeled
parameter, which is visible in syntax highlighting — labeled params appear in a distinct colour.

## Multi-parameter functions

```aivi
fun clampLow:Int #lo:Int #value:Int =>
    value < lo
     T|> lo
     F|> value

fun clamp:Int #lo:Int #hi:Int #value:Int =>
    value > hi
     T|> hi
     F|> clampLow lo value
```

Functions can have as many labeled parameters as needed.

## Calling functions

Pass arguments positionally after the function name:

```aivi
fun clampLow:Int #lo:Int #value:Int =>
    value < lo
     T|> lo
     F|> value

fun clamp:Int #lo:Int #hi:Int #value:Int =>
    value > hi
     T|> hi
     F|> clampLow lo value

val r1:Int = clamp 0 100 42
val r2:Int = clamp 0 100 150
```

When the argument is a complex expression, wrap it in parentheses:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

type Pixel = { x: Int, y: Int }

type Snake = {
    head: Pixel,
    body: List Pixel
}

fun nextHead:Pixel #direction:Direction #head:Pixel =>
    head

fun movedHead:Pixel #direction:Direction #snake:Snake =>
    nextHead direction snake.head
```

Note that in AIVI, function application is juxtaposition (no parentheses for the call itself,
only for grouping subexpressions). `nextHead direction snake.head` calls `nextHead` with two
arguments: `direction` and `snake.head`.

## Functions as values

Functions in AIVI are first-class. You can pass a function as an argument:

```aivi
fun addOne:Int #n:Int =>
    n + 1

fun applyTwice:Int #f:(Int -> Int) #x:Int =>
    f (f x)

val result:Int = applyTwice addOne 5
```

The `->` in `(Int -> Int)` is a function type: a function that takes `Int` and returns `Int`.

## Anonymous functions (lambdas)

> **Note:** Lambda syntax (`\param => body`) is a planned feature not yet in the current implementation. Use named functions instead.

```text
// planned syntax (not yet implemented):
val double = \x => x * 2
val add5 = \x => x + 5
```

The equivalent in current AIVI uses named functions:

```aivi
fun double:Int #x:Int =>
    x * 2

fun add5:Int #x:Int =>
    x + 5
```

Named functions appear frequently in pipe chains and as first-class arguments:

```aivi
sig count : Signal Int = 0
sig labelText : Signal Text = "You clicked {count} times"
```

## Pure by default

Every `fun` is pure. It cannot read from disk, make network calls, or mutate state.
Side effects live in `sig` declarations with `@source` decorators.

This means AIVI functions are easy to reason about and easy to test:
- given the same inputs, they always return the same output
- they have no hidden dependencies
- they can be called in any order without affecting each other

```aivi
fun double:Int #x:Int =>
    x * 2

fun square:Int #x:Int =>
    x * x
```

## Summary

- `fun name:ReturnType #param:Type => body`
- Return type comes immediately after the name, before parameters.
- All parameters are labeled with `#`.
- Function application is juxtaposition; parentheses group subexpressions.
- Lambdas: `\param => body`.
- Functions are pure — no side effects.

[Next: Pipes →](/tour/03-pipes)
