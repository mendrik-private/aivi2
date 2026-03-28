# Values & Functions

At the top level, AIVI keeps values and functions separate:

- `value` declares a named constant expression
- `fun` declares a named pure function

That split keeps intent obvious in larger modules and matches the current surface language directly.

## Values

A `value` binds a name to a single expression:

```aivi
value answer: Int = 42
value greeting: Text = "Hello"
value isReady: Bool = True
```

Type annotations are optional when the compiler can infer them, but they are useful in public modules and documentation.

Values can refer to earlier values:

```aivi
value width: Int = 24
value height: Int = 20
value cellCount: Int = width * height
```

Values can also hold records and lists:

```aivi
type BoardSize = {
    width: Int,
    height: Int
}

value boardSize: BoardSize = {
    width: 24,
    height: 20
}

value checkpoints: List Int = [
    4,
    8,
    12
]
```

## Functions

A `fun` declaration names a pure function:

```aivi
fun add: Int x:Int y:Int =>
    x + y

value total = add 3 4
```

The return type comes immediately after the function name. Parameters follow as `name:Type`.

```aivi
fun greet: Text name:Text =>
    "Hello, {name}!"

value greetingText = greet "Ada"
```

## Multiple parameters

Parameters are separated by spaces, not commas:

```aivi
fun between: Bool low:Int high:Int n:Int =>
    n >= low and n <= high

value scoreAllowed = between 0 100 42
```

## Multi-line bodies

Function bodies are still just expressions, so multi-line definitions usually lean on pipes:

```aivi
fun describeScore: Text score:Int =>
    score >= 50
     T|> "good"
     F|> "keep going"

value scoreLabel = describeScore 88
```

## Calling functions

Call a function by writing the function name followed by its arguments:

```aivi
fun area: Int width:Int height:Int =>
    width * height

value roomArea = area 5 8
```

If an argument is itself an expression, wrap it in parentheses:

```aivi
fun area: Int width:Int height:Int =>
    width * height

value adjustedArea = area (2 + 3) (4 * 2)
```

## Partial application

Functions can be partially applied. Supplying fewer arguments returns another function:

```aivi
fun multiply: Int left:Int right:Int =>
    left * right

value double = multiply 2
value ten = double 5
```

## Named helpers instead of inline lambdas

For examples and playground-friendly snippets, prefer a named helper over an inline anonymous function:

```aivi
fun trimStatus: Text status:Text =>
    status
     ||> " ready " -> "ready"
     ||> _         -> status

fun decorateStatus: Text status:Text =>
    "[{status}]"

value shownStatus =
    " ready "
     |> trimStatus
     |> decorateStatus
```

That style gives each step a reusable name and stays inside the current language surface.

## Type annotations

Both `value` and `fun` use `:` for type annotations:

```aivi
value count: Int = 0

fun negate: Int n:Int =>
    0 - n
```

## Summary

| Form | Example |
| --- | --- |
| Value | `value answer:Int = 42` |
| Function | `fun add:Int x:Int y:Int => x + y` |
| Function call | `add 3 4` |
| Partial application | `value double = multiply 2` |
