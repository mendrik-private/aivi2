# Values and Types

AIVI is explicit about declarations. `val` binds values, `type` defines closed types, and annotations use `:`.

## Basic values

```aivi
val answer = 42
val title: Text = "Inbox"
val ready: Bool = True

val names: List Text = [
    "Ada",
    "Grace"
]
```

## Sum types

Use closed constructors when the set of states matters.

```aivi
type Screen =
  | Loading
  | Ready Text
  | Failed Text

val current: Screen = Ready "Users"
```

## Records and constructor products

Records name their fields. Constructors carry positional payloads.

```aivi
use aivi.defaults (Option)

type User = {
    name: Text,
    nickname: Option Text,
    email: Option Text
}

type Vec2 = Vec2 Int Int

val user: User = { name: "Ada" }
val origin: Vec2 = Vec2 0 0
```

## Absence and failure are typed

There is no `null` or `undefined`. Model absence with `Option` and failures with `Result` or `Validation`.

```aivi
use aivi (
    Err
    None
    Ok
    Option
    Result
    Some
)

val maybeName: Option Text = Some "Ada"
val missingName: Option Text = None
val loaded: Result Text Int = Ok 2
val failed: Result Text Int = Err "offline"
```

Parametric types use normal application syntax, such as `Option Text` and `Result HttpError User`.
