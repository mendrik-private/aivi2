# Signals

Signals are the reactive core of AIVI. A signal is a value that changes over time. When a signal's inputs change, it automatically recomputes. The runtime builds a dependency graph and schedules updates efficiently.

## Declaring a Signal

```aivi
signal count: Signal Int = 0
```

This declares a signal `count` with initial value `0`. The type `Signal Int` says it carries integers.

When another signal depends on `count`, it automatically re-evaluates whenever `count` changes:

```aivi
signal doubled: Signal Int = count * 2
signal message: Signal Text = "Count is {count}"
```

## Deriving Signals

Signals can be derived by transforming another signal through a pipeline:

```aivi
signal score: Signal Int = ...

signal grade: Signal Text =
    score
     ||> _ if score >= 90 -> "A"
     ||> _ if score >= 75 -> "B"
     ||> _ if score >= 60 -> "C"
     ||> _                -> "F"
```

Any pipe operator works — the signal automatically re-evaluates each time the source signal emits.

## The `scan` Function

`scan` is the primary way to accumulate state from a signal over time. It folds each incoming value into an accumulated state:

```aivi
signal count: Signal Int =
    keyPresses
     |> scan 0 (state _ => state + 1)
```

- The first argument is the **initial state** (`0`)
- The second argument is the **step function** — it receives the current state and the new value, and returns the next state

Using a named function:

```aivi
fun countStep: Int state: Int _: Key =>
    state + 1

signal count: Signal Int =
    keyPresses
     |> scan 0 countStep
```

A more complex example that tracks a running direction:

```aivi
fun updateDirection: Direction key: Key current: Direction =>
    arrowKey key
     |> filterDirection current

signal direction: Signal Direction =
    keyDown
     |> scan Right updateDirection
```

## Signal Meta-State

Every signal exposes reactive meta-state signals as fields:

| Field | Type | Description |
|---|---|---|
| `.running` | `Signal Bool` | `True` while the signal is being evaluated or a source is in-flight |
| `.done` | `Signal Bool` | `True` once the signal has settled with a value |
| `.stale` | `Signal Bool` | `True` before the first settled value is available |
| `.error` | `Signal (Option Error)` | `Some e` if the signal has failed, `None` otherwise |

```aivi
signal users: Signal (Result HttpError (List User)) = ...

signal isLoading: Signal Bool = users.running
signal hasError: Signal Bool  = users.error *|> isSome
```

### Initial State

When a signal is first created:
- `.running` is `False`
- `.done` is `False`
- `.stale` is `True`
- `.error` is `None`

Once the signal settles with a value, `.done` becomes `True` and `.stale` becomes `False`. `.done` never goes back to `False` within the same epoch.

## Signal Graph Guarantees

The AIVI runtime provides strong guarantees about signal evaluation:

- **Glitch-free**: a downstream signal never sees an inconsistent combination of old and new upstream values
- **Batched**: all changes in a single event are processed together before the UI updates
- **Topologically ordered**: signals always evaluate after all their dependencies
- **Minimal**: a signal only re-evaluates if its inputs actually changed

## Combining Signals

Use `&|>` to combine multiple signals. The combined signal emits when any input changes:

```aivi
signal firstName: Signal Text = ...
signal lastName: Signal Text  = ...

signal fullName: Signal Text =
 &|> firstName
 &|> lastName
  |> (\first last => "{first} {last}")
```

## Filtering with `?|>`

`?|>` turns a signal into `Signal (Option A)` — it emits `Some value` when the predicate holds, and `None` when it does not:

```aivi
type User = { active: Bool, name: Text }

signal activeUser: Signal (Option User) =
    userSignal
     ?|> .active
```

## Accumulation with `+|>`

`+|>` is the signal version of a stateful fold. It accumulates state across emissions:

```aivi
signal total: Signal Int =
    priceSignal
     +|> 0 (state price => state + price)
```

Shorthand using `prev` and `.`:

```aivi
signal total: Signal Int =
    priceSignal
     +|> prev + .
```

## The Previous Value with `~|>`

`~|>` pairs each new emission with the previous one:

```aivi
signal transition: Signal (Status, Status) =
    statusSignal
     ~|> Idle
```

`Idle` is the initial "previous" value before the first real emission. After that, the signal emits `(previousStatus, currentStatus)`.

## Signals vs Values

| | `value` | `signal` |
|---|---|---|
| Computed | Once, at startup | Every time inputs change |
| Type | `T` | `Signal T` |
| Can use sources | No | Yes |
| Reactive | No | Yes |

Values are constants. Signals are reactive. Use a `value` when the result never needs to change; use a `signal` when it must respond to external events or other signals.

## Full Example

Here is the signal pipeline from the snake game:

```aivi
@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown: Signal Key

signal direction: Signal Direction =
    keyDown
     |> scan Right updateDirectionOrRestart

signal restartCount: Signal Int =
    keyDown
     |> scan 0 updateRestartCount

@source timer.every 200 with {
    immediate: False,
    coalesce: True
}
signal tick: Signal Unit

signal gameState: Signal GameTickState =
    tick
     |> scan initialGameTickState stepOnTick

signal game: Signal Game =
    gameState
     |> gameValue

signal board: Signal GameBoard =
    game
     |> toBoard boardSize

signal boardText: Signal Text =
    board
     |> boardTextFor
```

Each signal declares its dependency explicitly. The runtime builds the graph, schedules evaluation, and ensures the UI always reflects a consistent state.
