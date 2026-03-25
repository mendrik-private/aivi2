# Signals

A signal is a value that changes over time.

If `val` is a snapshot — a value fixed at declaration time — then `sig` is a live value that
is always current. When a signal's dependencies change, the signal recomputes automatically.

Think of it like a spreadsheet cell: when a cell it depends on changes, it updates immediately.

## Declaring a signal

```aivi
// TODO: add a verified AIVI example here
```

This declares a signal named `count` that holds an `Int`. Its initial value is `0`.

A signal that derives from another signal uses `|>`:

```aivi
// TODO: add a verified AIVI example here
```

`doubled` is always `count * 2`. You do not manually update it; the runtime maintains the
dependency.

## Signals from signals

Any pipe chain that starts with a signal produces a new signal:

```aivi
// TODO: add a verified AIVI example here
```

`scoreLine` recomputes whenever `game` changes. The `|>` pipes you already know work
identically on signals.

## Recurrence: `@|>` and `<|@`

The recurrence pattern is how a signal accumulates state over time.
The shape is always: **seed → enter → guards → step**.

```aivi
// TODO: add a verified AIVI example here
```

Reading this:

- `initial` — the seed value before any events arrive.
- `@|> cursor` — enter the recurrence driven by `cursor` (the source that wakes the loop).
- `?|> cursor.hasNext` — an optional guard; the step is skipped when the predicate is false.
- `<|@ cursor.next` — the step function: receives the current state, returns the next state.

## Example: direction signal in Snake

```aivi
// TODO: add a verified AIVI example here
```

`Right` is the seed. On each `keyDown` event, `<|@ updateDirection keyDown` computes
the next direction from the current one.

## Example: game state signal

```aivi
// TODO: add a verified AIVI example here
```

Every 160 ms the timer fires. `stepGame` runs with the current `direction`, producing the next
`game` state. The entire game loop is two lines.

## Recurrence guards

A `?|>` between `@|>` and `<|@` acts as a guard. If the predicate is false, the current
iteration is skipped:

```aivi
// TODO: add a verified AIVI example here
```

Here `?|> .hasNext` skips the step once the recurrent state no longer has a next element.

## Signals are values, not variables

A key distinction: `sig count` does not declare a mutable variable. It declares a node in the
signal dependency graph. The runtime owns the actual storage; AIVI code only describes the
relationships.

You cannot write to a signal from user code. Only declared sources (`@source`, `@recur.timer`)
can drive a recurrence.

## Derived signals vs recurrent signals

A derived signal has no memory — it is a pure transformation.
A recurrent signal has memory — it folds over a stream of events.

## Summary

- `sig name : Signal T = initialValue` declares a time-varying value.
- Derived signals use `|>` chains; they recompute automatically.
- Signals form a dependency graph maintained by the runtime.
- You never write to a signal; you only declare how it is computed.
- `@|>` enters the recurrent loop: seed on the left, driver (cursor or source) on the right.
- `<|@` advances the loop: applies the step function to the current state, returning the next state.
- `?|>` between `@|>` and `<|@` acts as a guard, skipping the step when false.
- Recurrent signals have memory (they fold over events); derived signals do not.

[Next: Sources →](/tour/06-sources)
