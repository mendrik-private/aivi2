# Signals

A signal is a value that changes over time.

If `val` is a snapshot — a value fixed at declaration time — then `sig` is a live value that
is always current. When a signal's dependencies change, the signal recomputes automatically.

Think of it like a spreadsheet cell: when a cell it depends on changes, it updates immediately.

## Declaring a signal

```text
-- declare a signal 'count' of type Int starting at 0
```

This declares a signal named `count` that holds an `Int`. Its initial value is `0`.

A signal that derives from another signal uses `\|>`:

```text
-- derive a signal 'doubled' from count, always equal to count multiplied by 2
```

`doubled` is always `count * 2`. You do not manually update it; the runtime maintains the
dependency.

## Signals from signals

Any pipe chain that starts with a signal produces a new signal:

```text
-- derive 'scoreLine' from the game signal
-- extract the score field
-- format it as the text "Score: N"
-- recomputes whenever game changes
```

`scoreLine` recomputes whenever `game` changes. The `\|>` pipes you already know work
identically on signals.

## Recurrence: @\|>...<\|@

The recurrence pattern is how signals accumulate state over time.
`@\|>` starts the recurrent flow; `<\|@` is the recurrence step.

```text
-- declare a helper function 'add' that adds x to n
-- bind the signal 'count' to the "inc" button click event
-- count starts at 0
-- each time the button fires, fold "add 1" into the accumulated count
```

Reading this:

- `0` — the initial value (the seed of the accumulator).
- `@\|>` — recurrent flow start: when the source fires, begin the accumulation.
- `<\|@` — recurrence step: apply the step function to the current accumulated value.
- `add 1` is partially applied — the step function receives the current `count` as its last
  argument each time the source fires.

## Example: direction signal in Snake

```text
-- bind 'direction' to keyboard key-down events, ignoring key repeats, only when focused
-- direction starts as Right
-- on each key-down event, apply keepDirection with the key press to compute the new direction
```

On each `keyDown` event, `@\|>` starts the recurrence and `<\|@` applies `keepDirection keyDown`
to the current direction, storing the result as the new direction.

## Example: game state signal

```text
-- bind 'game' to a timer that fires every 160 milliseconds, firing once immediately and coalescing rapid ticks
-- game starts at the initial game state
-- on each timer tick, apply stepGame with boardSize and current direction to advance the game
```

Every 160 ms the timer fires. `stepGame` runs with the current `direction`, producing the next
`game` state. The entire game loop is two lines.

## Multiple step sources

`<|@` can introduce a different source from `@|>`. A counter with two buttons:

```text
-- declare a message type 'Msg' with variants Increment and Decrement
-- declare a function 'update' that increments or decrements count based on a message
-- bind 'increment' signal to the "increment" button, emitting the Increment message
-- bind 'decrement' signal to the "decrement" button, emitting the Decrement message
-- count starts at 0
-- on each increment event, fold update with Increment into the accumulated count
-- on each decrement event, fold update with Decrement into the accumulated count
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
