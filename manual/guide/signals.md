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
type Int -> Int
func double n =>
    n * 2

signal count = 21

signal doubledCount = count
  |> double
```

## Boolean gating

Signals can branch just like ordinary values:

```aivi
signal ready = True

signal statusText = ready
 T|> "ready"
 F|> "waiting"
```

## Filtering with `?|>`

On signals, `?|>` filters updates whose predicate fails while keeping the `Signal A` carrier:

```aivi
type User = {
    active: Bool,
    email: Text
}

type Session = { user: User }

value seed : User = {
    active: True,
    email: "ada@example.com"
}

signal sessions : Signal Session = {
    user: seed
}

signal activeUsers : Signal User = sessions
  |> .user
 ?|> .active
```

For ordinary non-signal values, the same operator returns `Option A`.

## Reactive update clauses with `when`

You can also attach top-level reactive updates to an already declared signal.

The guarded form uses an ordinary boolean expression:

```aivi
signal left = 20
signal right = 22
signal total = 0
signal ready = True
signal enabled = True

when ready => total <- left + right
when ready and enabled => total <-
    result {
        next <- Ok left
        next + right
    }
```

You can also match a subject value directly and route each matching arm into an existing signal:

```aivi
type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down
signal heading = Up
signal tickSeen = False

when event
  ||> Turn dir => heading <- dir
  ||> Tick => tickSeen <- True
```

These forms mean:

- the guarded form uses an ordinary boolean expression
- the pattern-armed form matches each `||>` arm against the subject expression
- any binders introduced by an arm, like `dir`, are only in scope for that arm body
- the target must be a previously declared signal
- the right-hand side is an ordinary expression with direct signal references
- unlike a pipe, there is no ambient subject value inside the body
- if a guarded clause is false when it fires, the target keeps its previous committed value
- if multiple `when` clauses write the same signal in one tick, later clauses win by source order

Guards like `status.done` are fine too, but only when ordinary expression typing already proves that member access is a `Bool`.

Use `when` when you want event-shaped reactive commits into an existing signal. Use pipes when you want to transform the current subject flowing through one expression spine.

Reactive update self-reference rules are unchanged. A target signal still cannot read itself from its own `when` guard or body.

## Previous and diff

The language has dedicated pipes for time-oriented signal transformations:

```aivi
signal score = 10

signal previousScore = score
 ~|> 0

signal scoreDelta = score
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
