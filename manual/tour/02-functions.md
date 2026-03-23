# Functions

Functions in AIVI are declared with `fun`. They are pure by default — a function depends only
on its explicit parameters and always returns the same result for the same inputs.

## Basic syntax

```text
-- declare a pure function 'add' returning Int
-- takes two integer parameters x and y
-- returns their sum
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

```text
-- declare a function 'greet' returning Text, taking a name and a title
-- returns a greeting string combining title and name
-- call greet with "Lovelace" and "Ms", binding the result to 'msg'
```

Function calls are positional juxtaposition. You pass arguments in the order the parameters
are declared. The `#` sigil on the parameter declaration signals that this is a labeled
parameter, which is visible in syntax highlighting — labeled params appear in a distinct colour.

## Multi-parameter functions

```text
-- declare a function 'clamp' returning Int, taking lo, hi, and value
-- if value is less than lo, return lo
-- otherwise if value is greater than hi, return hi
-- otherwise return value unchanged
```

Functions can have as many labeled parameters as needed.

## Calling functions

Pass arguments positionally after the function name:

```text
-- call clamp with bounds 0–100 and value 42, result is 42
-- call clamp with bounds 0–100 and value -5, result is 0 (clamped to lower bound)
```

When the argument is a complex expression, wrap it in parentheses:

```text
-- call nextHead with direction and snake's head position, bind result to 'moved'
-- call willEat with boardSize, direction, food, and snake, bind result to 'eaten'
```

Note that in AIVI, function application is juxtaposition (no parentheses for the call itself,
only for grouping subexpressions). `nextHead direction snake.head` calls `nextHead` with two
arguments: `direction` and `snake.head`.

## Functions as values

Functions in AIVI are first-class. You can pass a function as an argument:

```text
-- declare a function 'applyTwice' that takes a function from Int to Int and an integer x
-- applies the function to x twice, returning the final Int
-- call applyTwice with "add 1" and 5, result is 7
```

The `->` in `(Int -> Int)` is a function type: a function that takes `Int` and returns `Int`.

## Anonymous functions (lambdas)

```text
-- declare an anonymous function 'double' that multiplies its argument by 2
-- declare an anonymous function 'add5' that adds 5 to its argument
```

Lambdas use the `\` syntax. They appear frequently in pipe chains:

```text
-- derive 'labelText' from the signal 'count', formatting it as "You clicked N times"
```

## Pure by default

Every `fun` is pure. It cannot read from disk, make network calls, or mutate state.
Side effects live in `sig` declarations with `@source` decorators.

This means AIVI functions are easy to reason about and easy to test:
- given the same inputs, they always return the same output
- they have no hidden dependencies
- they can be called in any order without affecting each other

```text
-- declare a pure function 'double' that returns x multiplied by 2
-- declare a pure function 'square' that returns x multiplied by itself
-- call double with 5, always produces 10
-- call square with 4, always produces 16
```

## Summary

- `fun name:ReturnType #param:Type => body`
- Return type comes immediately after the name, before parameters.
- All parameters are labeled with `#`.
- Function application is juxtaposition; parentheses group subexpressions.
- Lambdas: `\param => body`.
- Functions are pure — no side effects.

[Next: Pipes →](/tour/03-pipes)
