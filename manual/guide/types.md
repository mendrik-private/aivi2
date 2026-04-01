# Types

AIVI is statically typed. The compiler knows the type of every expression before the program runs, so mistakes are caught early and values do not silently change shape at runtime.

## Primitive types

| Type | Meaning | Example |
| --- | --- | --- |
| `Int` | Whole numbers | `42`, `0`, `-7` |
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

value bestScore : Score = 42

value ada : User = {
    id: 1,
    name: "Ada",
    email: "ada@example.com"
}
```

Use `type` for named algebraic data types, records, and aliases.

Records carry several named fields at once:

```aivi
type User = {
    id: Int,
    name: Text,
    email: Text
}

type User -> Text
func userName = user=>    user.name

value shownName =
    userName {
        id: 1,
        name: "Ada",
        email: "ada@example.com"
    }
```

## `type` for constructors and tagged values

Use `type` for algebraic data types too: enums, tagged unions, and constructor-backed wrappers.

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

type UserId =
  | UserId Int

value facing = Left
value currentUser = UserId 7
```

Constructors can carry payloads:

```aivi
type LoadState =
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
type LoadState =
  | NotAsked
  | Loading
  | Loaded Text
  | Failed Text

type LoadState -> Text
func loadLabel = state=> state ||> NotAsked      -> "idle"
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

value maybeUser : (Option User) =
    Some {
        id: 1,
        name: "Ada"
    }

value noUser : (Option User) = None
value success : (Result Text Int) = Ok 42
value failure : (Result Text Int) = Err "not found"
```

Lists are homogeneous, so every element has the same type:

```aivi
value primes : List Int = [
    2,
    3,
    5,
    7,
    11
]

value names : List Text = [
    "Ada",
    "Grace",
    "Alan"
]

value emptyNumbers : List Int = []
```

## Tuples

Tuples hold a fixed number of values, often with different types:

```aivi
value pair : (Int, Text) = (
    1,
    "one"
)

value triple : (Bool, Int, Text) = (
    True,
    0,
    "zero"
)
```

You usually unpack tuples with pattern matching:

```aivi
type (Int, Int) -> Int
func firstInt = pair=> pair ||> (first, _) -> first

value firstValue =
    firstInt (
        4,
        9
    )
```

## Text interpolation

Interpolation lets typed values appear inside text literals:

```aivi
value name : Text = "Ada"
value score : Int = 42
value message : Text = "Hello, {name}! Your score is {score}."
```

## Record row transforms

Record row transforms derive a new closed record type from an existing closed record type.

They are useful when one canonical record needs closely related variants for create inputs, patch inputs, or public API responses:

```aivi
type User = {
    id: Int,
    name: Text,
    nickname: Option Text,
    createdAt: Text,
    isAdmin: Bool
}

type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }

type UserPatch = User |> Omit (createdAt, isAdmin) |> Optional (name, nickname)
```

Available transforms:

| Transform | Meaning |
| --- | --- |
| `Pick (f1, ..., fn) R` | Keep exactly the listed fields |
| `Omit (f1, ..., fn) R` | Remove the listed fields |
| `Optional (f1, ..., fn) R` | Wrap listed fields in `Option` when they are not already optional |
| `Required (f1, ..., fn) R` | Remove one `Option` layer from listed fields when present |
| `Defaulted (f1, ..., fn) R` | Same resulting shape as `Optional`, but for fields expected to be supplied later by a defaulting or codec step |
| `Rename { old: new } R` | Rename fields without changing their types |

Rules:

- the source must be a closed record type
- every referenced field must exist
- `Optional` and `Defaulted` do not create nested `Option (Option A)`
- `Required` removes at most one `Option` layer
- `Rename` must not produce field-name collisions

The pipe form is only syntax sugar. This:

```aivi
type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }
```

means the same thing as:

```aivi
type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }
```

## Summary

| Form | Use it for |
| --- | --- |
| `type Name = Alias` | Plain aliases |
| `type Name = { ... }` | Records |
| `type Name = Con1 \| Con2` | Tagged unions |
| `type Name = Name Wrapped` | Constructor-backed wrappers |
| `Option A` | Value may be present or absent |
| `Result E A` | Success or failure |
| `List A` | Homogeneous sequences |
| `(A, B)` | Fixed-size tuples |
| `Pick` / `Omit` / `Optional` / `Required` / `Defaulted` / `Rename` | Deriving related closed record types |
