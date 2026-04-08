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
func userName = .name

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

type UserId = UserId Int

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

## Companion helpers on closed sums

Closed sums can keep total helper functions next to their constructors:

```aivi
type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human

    type Player -> Text
    label = .
     ||> Human    -> "You"
     ||> Computer -> "Computer"
}

value next : Player = opponent Human
value shown : Text = label Computer
```

These companion bindings are still ordinary functions. You call them by name, and you import or
export them the same way as any other function.

The `type` line spells the full function type, including the receiver. In the example above,
`type Player -> Player` and `type Player -> Text` are written explicitly. When the receiver is the
only parameter, `name = .` is shorthand for `name = self => ...`.

The brace form is reserved for companion sums only when the first significant entry is a constructor
line beginning with `|`. Ordinary record declarations still use the same `type Name = { field: T }`
syntax as before.

## Product types with positional arguments

Constructors can take multiple positional arguments. When a type has a single constructor with fields, the leading `|` is optional:

```aivi
type Vec2 = Vec2 Int Int

type Cell = Cell Int Int
```

Multi-variant types still use `|`:

```aivi
type Shape =
  | Circle Int
  | Rect Int Int
```

### Named fields

Field labels can be added for documentation and diagnostics. Names are declaration-only metadata — construction stays positional:

```aivi
type Date =
  Date year:Year month:Month day:Day

type TimeOfDay =
  TimeOfDay hour:Hour minute:Minute second:Second
```

Named and anonymous fields may be mixed, though named fields are recommended for readability when a constructor has more than two fields.

Construction is curried: apply the constructor to each argument left-to-right:

```aivi
value origin = Vec2 0 0
value corner = Vec2 10 20
value today = Date 2024 6 15
```

Under-application is legal — a partially applied constructor is a function:

```aivi
value mkRow = Cell 5
value cell = mkRow 3
```

Pattern matching gives each positional field a name at the use site:

```aivi
type Vec2 -> Vec2 -> Vec2
func addVec = a b => (a, b)
 ||> (Vec2 ax ay, Vec2 bx by) -> Vec2 (ax + bx) (ay + by)

type Cell -> Int
func cellX = .
 ||> Cell x _ -> x

type Cell -> Int
func cellY = .
 ||> Cell _ y -> y
```

Use `_` for fields you do not need. The constructor name must appear in every arm that matches it.

Multi-argument constructors differ from records: their fields are **positional** and **anonymous**. Records carry named fields; product constructors carry ordered slots. Use records when field names aid readability; use product constructors for compact, well-understood tuples like coordinates or identifiers.

## Matching typed values

Because constructors are part of the type, pattern matching stays precise:

```aivi
type LoadState =
  | NotAsked
  | Loading
  | Loaded Text
  | Failed Text

type LoadState -> Text
func loadLabel = .
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
func firstInt = .
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

The pipe form is only syntax sugar. The type on the left is passed as the final argument to the transform on the right:

```aivi
type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }
```

The pipe form is preferred because it reads left-to-right in application order.

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
