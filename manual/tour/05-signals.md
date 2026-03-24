# Signals

A signal is a value that changes over time.

If `val` is a snapshot — a value fixed at declaration time — then `sig` is a live value that
is always current. When a signal's dependencies change, the signal recomputes automatically.

Think of it like a spreadsheet cell: when a cell it depends on changes, it updates immediately.

## Declaring a signal

```aivi
sig count : Signal Int = 0
```

This declares a signal named `count` that holds an `Int`. Its initial value is `0`.

A signal that derives from another signal uses `\|>`:

```aivi
fun timesTwo:Int #n:Int =>
    n * 2

sig count : Signal Int = 0

sig doubled : Signal Int =
    count
     |> timesTwo
```

`doubled` is always `count * 2`. You do not manually update it; the runtime maintains the
dependency.

## Signals from signals

Any pipe chain that starts with a signal produces a new signal:

```aivi
type Status = Running | GameOver

type Game = {
    score: Int,
    status: Status
}

sig game : Signal Game = {
    score: 0,
    status: Running
}

fun formatScore:Text #n:Int =>
    "Score: {n}"

sig scoreLine : Signal Text =
    game
     |> .score
     |> formatScore
```

`scoreLine` recomputes whenever `game` changes. The `\|>` pipes you already know work
identically on signals.

## Recurrence: @\|>...<\|@

The recurrence pattern is how signals accumulate state over time.
`@\|>` starts the recurrent flow; `<\|@` is the recurrence step.

```aivi
fun addOne:Int #n:Int =>
    n + 1

provider button.clicked
    wakeup: sourceEvent
    argument id: Text

@source button.clicked "inc"
sig count : Signal Int =
    0
     @|> addOne
     <|@ addOne
```

Reading this:

- `0` — the initial value (the seed of the accumulator).
- `@\|>` — recurrent flow start: when the source fires, begin the accumulation.
- `<\|@` — recurrence step: apply the step function to the current accumulated value.
- `add 1` is partially applied — the step function receives the current `count` as its last
  argument each time the source fires.

## Example: direction signal in Snake

```aivi
type Key = Key Text

type Direction =
  | Up
  | Down
  | Left
  | Right

fun updateDirection:Direction #key:Key #current:Direction =>
    current

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig keyDown : Signal Key

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig direction : Signal Direction =
    Right
     @|> updateDirection keyDown
     <|@ updateDirection keyDown
```

On each `keyDown` event, `@\|>` starts the recurrence and `<\|@` applies `keepDirection keyDown`
to the current direction, storing the result as the new direction.

## Example: game state signal

```aivi
type Status = Running | GameOver

type Pixel = Pixel Int Int

type Direction =
  | Up
  | Down
  | Left
  | Right

type BoardSize = {
    width: Int,
    height: Int
}

type Game = {
    snake: List Pixel,
    food: Pixel,
    score: Int,
    status: Status,
    seed: Int
}

val boardSize:BoardSize = {
    width: 12,
    height: 10
}

val initialGame:Game = {
    snake: [
        Pixel 6 5,
        Pixel 5 5,
        Pixel 4 5
    ],
    food: Pixel 10 1,
    score: 0,
    status: Running,
    seed: 2463534242
}

fun stepGame:Game #size:BoardSize #direction:Direction #game:Game =>
    game

sig direction : Signal Direction = Right

@source timer.every 160 with {
    immediate: True,
    coalesce: True
}
sig game : Signal Game =
    initialGame
     @|> stepGame boardSize direction
     <|@ stepGame boardSize direction
```

Every 160 ms the timer fires. `stepGame` runs with the current `direction`, producing the next
`game` state. The entire game loop is two lines.

## Multiple step sources

`<|@` can introduce a different source from `@|>`. A counter with two buttons:

```aivi
type Msg = Increment | Decrement

fun update:Int #msg:Msg #n:Int =>
    msg
     ||> Increment => n + 1
     ||> Decrement => n - 1

provider button.clicked
    wakeup: sourceEvent
    argument id: Text

@source button.clicked "increment"
sig increment : Signal Unit

@source button.clicked "decrement"
sig decrement : Signal Unit

@source button.clicked "increment"
sig count : Signal Int =
    0
     @|> update Increment
     <|@ update Decrement
```

`@|>` opens the recurrence triggered by `increment`; `<|@` adds `decrement` as a second
recurse point. Each event folds through `update` into the current count.

## Signals are values, not variables

A key distinction: `sig count` does not declare a mutable variable. It declares a node in the
signal dependency graph. The runtime owns the actual storage; AIVI code only describes the
relationships.

You cannot write to a signal from user code. Only declared sources (`@source`, `@recur.timer`)
can drive a recurrence.

## Derived signals vs recurrent signals

| Form | Meaning |
|---|---|
| `sig x = someSignal \|> f` | Derives from another signal; no local state |
| `sig x = init @\|> step src <\|@ step src` | Accumulates state over time |

A derived signal has no memory — it is a pure transformation.
A recurrent signal has memory — it folds over a stream of events.

## Summary

- `sig name : Signal T = initialValue` declares a time-varying value.
- Derived signals use `\|>` chains; they recompute automatically.
- Recurrent signals use `@\|>` (flow start) and `<\|@` (recurrence step) to fold events into accumulated state.
- Signals form a dependency graph maintained by the runtime.
- You never write to a signal; you only declare how it is computed.

[Next: Sources →](/tour/06-sources)
