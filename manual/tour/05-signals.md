# Signals

A `Signal A` is a value of type `A` that changes over time. `val` is stable; `sig` participates in the reactive graph.

## Derived signals

```aivi
type NamePair =
  | NamePair Text Text

sig firstName = "Ada"
sig lastName = "Lovelace"

sig namePair =
  &|> firstName
  &|> lastName
  |> NamePair
```

## Folding wakeups with `scan`

`scan` is the normal way to turn wakeups into evolving state.

```aivi
fun step:Int tick:Unit current:Int =>
    current + 1

@source timer.every 120 with {
    immediate: True
}
sig tick: Signal Unit

sig counter: Signal Int =
    tick
     |> scan 0 step
```

## Explicit recurrence

When the scheduler itself drives the next step, use recurrence decorators and recurrence pipes.

```aivi
domain Duration over Int
    literal s: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry

fun step value =>
    value

@recur.timer 5s
sig polled: Signal Int =
    0
     @|> step
     <|@ step

@recur.backoff 3x
val retried: Task Int Int =
    0
     @|> step
     <|@ step
```

Signals are not general-purpose mutable cells. Prefer deriving new signals from existing ones instead of treating them like imperative state variables.
