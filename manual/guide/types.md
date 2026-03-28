# Types

AIVI is statically typed. Every value, function, and signal has a type known at compile time. There are no implicit conversions, no null, and no runtime type confusion.

## Primitive Types

| Type | Description | Example |
|---|---|---|
| `Int` | Whole number | `42`, `0`, `-7` |
| `Float` | Floating-point number | `3.14`, `-0.5` |
| `Bool` | Boolean | `True`, `False` |
| `Text` | UTF-8 string | `"hello"` |
| `Unit` | The "nothing" type — exactly one value | `()` |

## Type Aliases

Give a name to any type:

```aivi
type Key = Key Text
type Score = Int
type UserId = Int
```

The first form (`Key Text`) is a **newtype** — it wraps `Text` in a named constructor `Key` so the two cannot be mixed up accidentally.

## Union Types

A union type (also called an algebraic data type or ADT) has several named constructors, only one of which is active at a time:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right
```

Constructors can carry data:

```aivi
type Status =
  | Running
  | GameOver

type LoadState =
  | NotAsked
  | Loading
  | Loaded User
  | Failed Text
```

`Loaded User` carries a value of type `User`. `Failed Text` carries an error message.

Compact single-line form is also allowed:

```aivi
type Status = Running | GameOver
```

### Using Constructors

Constructors are used as values. If a constructor carries data, apply it like a function:

```aivi
value state = NotAsked
value loaded = Loaded { id: 1, name: "Ada" }
value failed = Failed "network error"
```

## Record Types

Records are product types — they hold several named fields at once:

```aivi
type User = {
    id: Int,
    name: Text,
    email: Text
}
```

Create a record with `{ field: value, ... }`:

```aivi
value user: User = {
    id: 1,
    name: "Ada",
    email: "ada@example.com"
}
```

Access fields with `.`:

```aivi
fun getUserName: Text user: User =>
    user.name
```

### Record Shorthand

When a local variable has the same name as a field, you can omit the value:

```aivi
value name = "Ada"
value email = "ada@example.com"

value user: User = {
    id: 1,
    name,       -- same as name: name
    email       -- same as email: email
}
```

### Record Update

Copy a record and change some fields using `{ existing | field: newValue }`:

```aivi
fun withScore: Game game: Game score: Int =>
    { game | score }
```

Wait — update syntax is `{ record | field: newValue }`:

```aivi
fun resetScore: Game game: Game =>
    { game | score: 0 }
```

## Parameterised Types

Types can take type parameters (written after the type name):

```aivi
type Option A =
  | None
  | Some A

type Result E A =
  | Err E
  | Ok A

type List A = ...   -- built-in
```

Use them by supplying the type argument:

```aivi
value maybeName: Option Text = Some "Ada"
value noName: Option Text = None

value success: Result Text Int = Ok 42
value failure: Result Text Int = Err "not found"
```

## Tuples

A tuple holds a fixed number of values of potentially different types:

```aivi
value pair: (Int, Text) = (1, "one")
value triple: (Bool, Int, Text) = (True, 0, "zero")
```

Access tuple elements through pattern matching:

```aivi
fun fst: Int pair: (Int, Int) =>
    pair
     ||> (first, _) -> first
```

## Type Parameters in Functions

Functions can be polymorphic — they work for any type that satisfies a constraint:

```aivi
fun identity: A x: A =>
    x
```

Here `A` is a type variable. The compiler infers what `A` is at each call site.

## The Option Type

`Option A` represents a value that may or may not be present. It replaces null.

```aivi
value foundUser: Option User = Some { id: 1, name: "Ada", email: "ada@example.com" }
value noUser: Option User = None
```

Use pattern matching to handle both cases:

```aivi
fun userDisplayName: Text opt: Option User =>
    opt
     ||> Some user -> user.name
     ||> None      -> "Guest"
```

## The Result Type

`Result E A` represents either success (`Ok A`) or failure (`Err E`). It replaces exceptions.

```aivi
fun safeDivide: Result Text Int a: Int b: Int =>
    b == 0
     T|> Err "division by zero"
     F|> Ok (a / b)
```

## Lists

Lists are homogeneous sequences. All elements must have the same type.

```aivi
value primes: List Int = [2, 3, 5, 7, 11]
value names: List Text = ["Ada", "Grace", "Alan"]
value empty: List Int = []
```

Ranges create a list of consecutive integers:

```aivi
value indices = [0..9]    -- [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
```

Use `append` to build lists:

```aivi
fun prepend: List Int item: Int xs: List Int =>
    append [item] xs
```

## Strings

Text literals use double quotes. Interpolate expressions with `{expr}`:

```aivi
value name = "Ada"
value score = 42
value message = "Hello, {name}! Your score is {score}."
```

Any expression can appear inside `{}`. The result is automatically converted to text.

### Regular Expressions

Regex literals use the `rx` prefix:

```aivi
value datePattern = rx"\d{4}-\d{2}-\d{2}"
value slugPattern = rx"[a-z0-9]+(-[a-z0-9]+)*"
```

## Summary

| Form | Syntax |
|---|---|
| Union type | `type Name = Con1 \| Con2 T` |
| Record type | `type Name = { field: Type, ... }` |
| Newtype | `type Name = Name WrappedType` |
| Option | `Some value` / `None` |
| Result | `Ok value` / `Err error` |
| List | `[a, b, c]` |
| Tuple | `(a, b)` |
| Range | `[low..high]` |
| String interpolation | `"text {expr} more text"` |
| Regex | `rx"pattern"` |
