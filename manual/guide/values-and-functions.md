# Values & Functions

At the top level, AIVI keeps values and functions separate:

- `value` declares a named constant expression
- `func` declares a named pure function

That split keeps intent obvious in larger modules and matches the current surface language directly.

## Values

A `value` binds a name to a single expression:

```aivi
value answer : Int = 42
value greeting : Text = "Hello"
value isReady : Bool = True
```

Type annotations are optional when the compiler can infer them, but they are useful in public modules and documentation.

Values can refer to earlier values:

```aivi
value width : Int = 24
value height : Int = 20
value cellCount : Int = width * height
```

Values can also hold records and lists:

```aivi
type BoardSize = {
    width: Int,
    height: Int
}

value boardSize : BoardSize = {
    width: 24,
    height: 20
}

value checkpoints : List Int = [
    4,
    8,
    12
]
```

## Functions

A `func` declaration names a pure function:

```aivi
type Int -> Int -> Int
func add = x y =>
    x + y

value total = add 3 4
```

Function signatures live on a preceding `type` line, and the `func` header keeps parameters unannotated.

```aivi
type Text -> Text
func greet = name =>
    "Hello, {name}!"

value greetingText = greet "Ada"
```

## Multiple parameters

Parameters are separated by spaces, not commas:

```aivi
type Int -> Int -> Int -> Bool
func between = low high n =>
    n >= low and n <= high

value scoreAllowed = between 0 100 42
```

## Multi-line bodies

Function bodies are still just expressions, so multi-line definitions usually lean on pipes:

```aivi
type Int -> Text
func describeScore = score =>
    score >= 50 T|> "good"
     F|> "keep going"

value scoreLabel = describeScore 88
```

## Unary subject sugar

When a unary function starts from its argument directly, `func name = .` is shorthand for a single implicit argument whose initial body/head is that argument:

```aivi
type Text -> Text
func trimStatus = .
 ||> " ready " -> "ready"
 ||> _         -> .
```

## Calling functions

Call a function by writing the function name followed by its arguments:

```aivi
type Int -> Int -> Int
func area = width height =>
    width * height

value roomArea = area 5 8
```

If an argument is itself an expression, wrap it in parentheses:

```aivi
type Int -> Int -> Int
func area = width height =>
    width * height

value adjustedArea = area (2 + 3) (4 * 2)
```

That includes negative literals in call position, for example `abs (-3)` rather than `abs -3`.

## Partial application

Functions can be partially applied. Supplying fewer arguments returns another function:

```aivi
type Int -> Int -> Int
func multiply = left right =>
    left * right

value double = multiply 2
value ten = double 5
```

## Named helpers instead of inline lambdas

For examples and playground-friendly snippets, prefer a named helper over an inline anonymous function:

```aivi
type Text -> Text
func trimStatus = .
 ||> " ready " -> "ready"
 ||> _         -> .

type Text -> Text
func decorateStatus = status =>
    "[{status}]"

value shownStatus = " ready "
  |> trimStatus
  |> decorateStatus
```

That style gives each step a reusable name and stays inside the current language surface.

## Structural patches

Use `<|` to produce an updated value without mutating the original:

```aivi
type User = {
    name: Text,
    isAdmin: Bool
}

value user : User = {
    name: "Ada",
    isAdmin: False
}

value promoted : User =
    user <| {
        name: "Grace",
        isAdmin: True,
    }
```

`patch { ... }` builds a reusable same-shape update function:

```aivi
value promote : (User -> User) =
    patch {
        isAdmin: True,
    }
```

Selectors are relative to the current focus:

- record roots accept either `field` or `.<field>`; nested field steps still use dots, as in `profile.name`
- `[*]` traverses `List` elements or `Map` values
- `[predicate]` filters `List` elements
- `["key"]` or `[.key == "id-1"]` selects `Map` entries

Inside patch predicates, use dot-prefixed projections such as `.active`, `.price`, `.key`, and `.value`.

The current checked slice also accepts constructor focus through `Some`, `Ok`, `Err`, `Valid`, `Invalid`, and same-module constructors with exactly one payload field.

`:=` stores a function value as data instead of applying it during patch execution.

```aivi
type Int -> Int
func increment = n =>
    n + 1

type Counter = {
    step: Int -> Int
}

value keepCounter : (Counter -> Counter) =
    patch {
        step: := increment,
    }
```

Current limitation: structural removal syntax (`field: -`, or equivalently `.field: -`) is parsed but still rejected later in the compiler pipeline because result-type-changing patch elaboration is not wired through the executable slice yet.

## Type annotations

Both `value` and `func` use `:` for type annotations:

```aivi
value count : Int = 0

type Int -> Int
func negate = n =>
    0 - n
```

## Summary

| Form | Example |
| --- | --- |
| Value | `value answer:Int = 42` |
| Function | `type Int -> Int -> Int` / `func add = x y => x + y` |
| Function call | `add 3 4` |
| Partial application | `value double = multiply 2` |
| Patch apply | `value promoted = user <| { isAdmin: True }` |
