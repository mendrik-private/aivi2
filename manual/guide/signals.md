# Signals

Signals are the reactive core of AIVI. A signal is a named value in the dependency graph: when its inputs change, the signal is recomputed by the runtime.

## Declaring a signal

```aivi
signal count = 21
```

This declares a reactive value named `count`.

## Deriving from another signal

Signals are often defined from earlier signals with pipes:

```aivi
fun double: Int n:Int =>
    n * 2

signal count = 21

signal doubledCount =
    count
     |> double
```

## Boolean gating

Signals can branch just like ordinary values:

```aivi
signal ready = True

signal statusText =
    ready
     T|> "ready"
     F|> "waiting"
```

## Filtering with `?|>`

`?|>` turns a value into `Option` when a predicate may reject it:

```aivi
type User = {
    active: Bool,
    email: Text
}

type Session = { user: User }

value seed: User = {
    active: True,
    email: "ada@example.com"
}

signal sessions: Signal Session = {
    user: seed
}

signal activeUsers: Signal User =
    sessions
     |> .user
     ?|> .active
```

## Previous and diff

The language has dedicated pipes for time-oriented signal transformations:

```aivi
signal score = 10

signal previousScore =
    score
     ~|> 0

signal scoreDelta =
    score
     -|> 0
```

## Shaping signal outputs

Signals can still produce richer values without leaving the ordinary expression model:

```aivi
type NamePair = {
    first: Text,
    last: Text
}

signal firstName = "Ada"
signal lastName = "Lovelace"

signal namePair = {
    first: firstName,
    last: lastName
}
```

## Signals versus values

| Form | Meaning |
| --- | --- |
| `value answer = 42` | Fixed expression |
| `signal count = 21` | Reactive graph node |

Use `value` when something does not participate in reactive recomputation. Use `signal` when it should.
