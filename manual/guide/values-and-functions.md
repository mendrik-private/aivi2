# Values & Functions

AIVI has two kinds of named definitions at the top level: **values** (constants) and **functions**.

## Values

A `value` declaration binds a name to an expression that is computed once. It never changes.

```aivi
value answer = 42
value greeting = "Hello, world"
value pi: Float = 3.14159
value boardSize: BoardSize = { width: 24, height: 20 }
```

You can optionally annotate the type after the name with a colon. If you omit it, the compiler infers the type.

Values can refer to other values:

```aivi
value width = 24
value height = 20
value cellCount = width * height
```

Values can hold any expression — including records, lists, and markup:

```aivi
value initialSnake: List Pixel = [
    Pixel 6 10,
    Pixel 5 10,
    Pixel 4 10
]
```

## Functions

A `fun` declaration defines a named pure function with one or more parameters.

```aivi
fun add: Int x: Int y: Int =>
    x + y
```

The return type comes immediately after the function name. Each parameter is written as `name: Type`. The body follows `=>`.

```aivi
fun greet: Text name: Text =>
    "Hello, {name}!"
```

Functions can call other functions:

```aivi
fun square: Int n: Int =>
    n * n

fun sumOfSquares: Int a: Int b: Int =>
    square a + square b
```

### Multiple Parameters

Parameters are separated by spaces — no commas, no parentheses:

```aivi
fun clamp: Int low: Int high: Int value: Int =>
    value
     T|> low   -- if value < low
     F|> value
```

Wait — that example uses pipes. Here is a simpler one using arithmetic:

```aivi
fun between: Bool low: Int high: Int n: Int =>
    n >= low and n <= high
```

### Multi-Line Bodies

When the body spans multiple lines, indent it consistently:

```aivi
fun describeScore: Text score: Int =>
    score
     ||> _ if score >= 100 -> "excellent"
     ||> _ if score >= 50  -> "good"
     ||> _                 -> "keep going"
```

## Lambdas

You can write anonymous functions inline using `=>`:

```aivi
value double: List Int =
    [1, 2, 3]
     |> map (x => x * 2)
```

### Dot Shorthand

The `.` character is a shorthand for a lambda that accesses a field or applies a function to the implicit argument:

```aivi
fun getNames: List Text users: List User =>
    users
     |> map .name
```

`.name` is equivalent to `u => u.name`.

You can also chain dot access for field projection:

```aivi
fun getShippingStatus: Text order: Order =>
    order
     |> .shipping
     |> .status
```

## Type Annotations

All type annotations use `:` (a colon):

```aivi
value count: Int = 0
fun negate: Int n: Int => 0 - n
```

When the compiler can infer the type, the annotation is optional. For top-level declarations it is good practice to include it for documentation.

## Calling Functions

Functions are called by juxtaposition — put the function name first, then its arguments, separated by spaces:

```aivi
fun area: Int w: Int h: Int =>
    w * h

value roomArea = area 5 8   -- result: 40
```

When passing a complex expression as an argument, wrap it in parentheses:

```aivi
value result = area (2 + 3) (4 * 2)
```

## Partial Application

Functions can be partially applied. Supplying fewer arguments than required returns a new function waiting for the rest:

```aivi
fun multiply: Int a: Int b: Int =>
    a * b

value double = multiply 2     -- a function Int -> Int
value ten    = double 5       -- 10
```

This is especially useful with pipes:

```aivi
value numbers = [1, 2, 3, 4, 5]
value doubled =
    numbers
     |> map (multiply 2)
```

## Summary

| Form | Syntax |
|---|---|
| Constant | `value name: Type = expr` |
| Function | `fun name: ReturnType param: Type => body` |
| Lambda | `x => expr` |
| Dot shorthand | `.field` or `.method` |
| Function call | `f arg1 arg2` |
