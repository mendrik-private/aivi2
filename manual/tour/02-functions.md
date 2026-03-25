# Functions

Functions in AIVI are declared with `fun`. They are pure by default — a function depends only
on its explicit parameters and always returns the same result for the same inputs.

## Basic syntax

```text
// TODO: add a verified AIVI example here
```

Breaking this down:

| Part | Meaning |
|---|---|
| `fun` | declaration keyword |
| `add` | function name |
| `:Int` | **return type** (comes before parameters) |
| `x:Int` | parameter named `x` of type `Int` |
| `y:Int` | parameter named `y` of type `Int` |
| `=>` | separates signature from body |
| `x + y` | function body — any expression |

The return type prefix (`add:Int`) is one of AIVI's deliberate design choices:
reading left to right, you see the name and return type before the parameters.

## Parameters

Parameters are written directly in the function head:

```text
// TODO: add a verified AIVI example here
```

Function calls are positional juxtaposition. You pass arguments in the order the parameters
are declared.

## Multi-parameter functions

```text
// TODO: add a verified AIVI example here
```

Functions can have as many parameters as needed.

## Calling functions

Pass arguments positionally after the function name:

```text
// TODO: add a verified AIVI example here
```

When the argument is a complex expression, wrap it in parentheses:

```text
// TODO: add a verified AIVI example here
```

Note that in AIVI, function application is juxtaposition (no parentheses for the call itself,
only for grouping subexpressions). `nextHead direction snake.head` calls `nextHead` with two
arguments: `direction` and `snake.head`.

## Functions as values

Functions in AIVI are first-class. You can pass a function as an argument:

```text
// TODO: add a verified AIVI example here
```

The `->` in `(Int -> Int)` is a function type: a function that takes `Int` and returns `Int`.

## Anonymous functions (lambdas)

Anonymous functions use `param =>` for single-argument lambdas or `_ op value` for simple operator sections:

```text
// single parameter lambda
val double = x => x * 2

// operator section (implicit argument)
val add5 = _ + 5
```

The equivalent using named functions:

```text
// TODO: add a verified AIVI example here
```

Named functions appear frequently in pipe chains and as first-class arguments:

```text
// TODO: add a verified AIVI example here
```

## Pure by default

Every `fun` is pure. It cannot read from disk, make network calls, or mutate state.
Side effects live in `sig` declarations with `@source` decorators.

This means AIVI functions are easy to reason about and easy to test:
- given the same inputs, they always return the same output
- they have no hidden dependencies
- they can be called in any order without affecting each other

```text
// TODO: add a verified AIVI example here
```

## Summary

- `fun name:ReturnType param:Type => body`
- Return type comes immediately after the name, before parameters.
- All parameters are labeled with `#`.
- Function application is juxtaposition; parentheses group subexpressions.
- Lambdas: `param => body` or `_ op value` for operator sections.
- Functions are pure — no side effects.

[Next: Pipes →](/tour/03-pipes)
