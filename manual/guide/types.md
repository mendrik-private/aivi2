# Types

AIVI is statically typed. The compiler knows the type of every expression before the program runs, so mistakes are caught early and values do not silently change shape at runtime.

## Primitive types

| Type | Meaning | Example |
| --- | --- | --- |
| `Int` | Whole numbers | `42`, `0`, `0 - 7` |
| `Float` | Floating-point numbers | `3.14`, `0.5` |
| `Bool` | Booleans | `True`, `False` |
| `Text` | UTF-8 text | `"hello"` |
| `Unit` | A type with one value | `()` |

## `type` for aliases and records

Use `type` when you want a plain alias or a record shape:

```aivi
type Score = Int

type User = {
    id: Int,
    name: Text,
    email: Text
}

value bestScore: Score = 42

value ada: User = {
    id: 1,
    name: "Ada",
    email: "ada@example.com"
}
```

Records carry several named fields at once:

```aivi
type User = {
    id: Int,
    name: Text,
    email: Text
}

fun userName: Text user:User =>
    user.name

value shownName =
    userName {
        id: 1,
        name: "Ada",
        email: "ada@example.com"
    }
```

## `data` for constructors and tagged values

Use `data` for algebraic data types: enums, tagged unions, and constructor-backed wrappers.

```aivi
data Direction =
  | Up
  | Down
  | Left
  | Right

data UserId =
  | UserId Int

value facing = Left
value currentUser = UserId 7
```

Constructors can carry payloads:

```aivi
data LoadState =
  | NotAsked
  | Loading
  | Loaded Text
  | Failed Text

value readyState = Loaded "Ada"
value failedState = Failed "offline"
```

## Matching typed values

Because constructors are part of the type, pattern matching stays precise:

```aivi
data LoadState =
  | NotAsked
  | Loading
  | Loaded Text
  | Failed Text

fun loadLabel: Text state:LoadState =>
    state
     ||> NotAsked      -> "idle"
     ||> Loading       -> "loading"
     ||> Loaded name   -> "ready {name}"
     ||> Failed reason -> "failed {reason}"

value currentLabel = loadLabel (Loaded "Grace")
```

## Built-in parameterised types

AIVI ships with several useful generic types. You use them by supplying the concrete payload type:

```aivi
type User = {
    id: Int,
    name: Text
}

value maybeUser: (Option User) =
    Some {
        id: 1,
        name: "Ada"
    }

value noUser: (Option User) = None
value success: (Result Text Int) = Ok 42
value failure: (Result Text Int) = Err "not found"
```

Lists are homogeneous, so every element has the same type:

```aivi
value primes: List Int = [
    2,
    3,
    5,
    7,
    11
]

value names: List Text = [
    "Ada",
    "Grace",
    "Alan"
]

value emptyNumbers: List Int = []
```

## Tuples

Tuples hold a fixed number of values, often with different types:

```aivi
value pair: (Int, Text) = (
    1,
    "one"
)

value triple: (Bool, Int, Text) = (
    True,
    0,
    "zero"
)
```

You usually unpack tuples with pattern matching:

```aivi
fun firstInt: Int pair:(Int, Int) =>
    pair
     ||> (first, _) -> first

value firstValue =
    firstInt (
        4,
        9
    )
```

## Text interpolation

Interpolation lets typed values appear inside text literals:

```aivi
value name: Text = "Ada"
value score: Int = 42
value message: Text = "Hello, {name}! Your score is {score}."
```

## Summary

| Form | Use it for |
| --- | --- |
| `type Name = Alias` | Plain aliases |
| `type Name = { ... }` | Records |
| `data Name = Con1 \| Con2` | Tagged unions |
| `data Name = Name Wrapped` | Constructor-backed wrappers |
| `Option A` | Value may be present or absent |
| `Result E A` | Success or failure |
| `List A` | Homogeneous sequences |
| `(A, B)` | Fixed-size tuples |
