# Signals

A signal is a value that changes over time.

If `val` is a snapshot — a value fixed at declaration time — then `sig` is a live value that
is always current. When a signal's dependencies change, the signal recomputes automatically.

Think of it like a spreadsheet cell: when a cell it depends on changes, it updates immediately.

## Declaring a signal

```text
// TODO: add a verified AIVI example here
```

This declares a signal named `count` that holds an `Int`. Its initial value is `0`.

A signal that derives from another signal uses `|>`:

```text
// TODO: add a verified AIVI example here
```

`doubled` is always `count * 2`. You do not manually update it; the runtime maintains the
dependency.

## Signals from signals

Any pipe chain that starts with a signal produces a new signal:

```text
// TODO: add a verified AIVI example here
```

`scoreLine` recomputes whenever `game` changes. The `|>` pipes you already know work
identically on signals.

## Recurrence: `@|>` and `<|@`

The recurrence pattern is how a signal accumulates state over time.
The shape is always: **seed → enter → guards → step**.

```text
// TODO: add a verified AIVI example here
```

Reading this:

- `initial` — the seed value before any events arrive.
- `@|> cursor` — enter the recurrence driven by `cursor` (the source that wakes the loop).
- `?|> cursor.hasNext` — an optional guard; the step is skipped when the predicate is false.
- `<|@ cursor.next` — the step function: receives the current state, returns the next state.

## Example: direction signal in Snake

```text
// TODO: add a verified AIVI example here
```

`Right` is the seed. On each `keyDown` event, `<|@ updateDirection keyDown` computes
the next direction from the current one.

## Example: game state signal

```text
// TODO: add a verified AIVI example here
```

Every 160 ms the timer fires. `stepGame` runs with the current `direction`, producing the next
`game` state. The entire game loop is two lines.

## Recurrence guards

A `?|>` between `@|>` and `<|@` acts as a guard. If the predicate is false, the current
iteration is skipped:

```text
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

| Form | Meaning |
|---|---|
| `sig x = someSignal \|> f` | Derives from another signal; no local state |
| `sig x = init @\|> step src <\|@ step src` | Accumulates state over time |

A derived signal has no memory — it is a pure transformation.
A recurrent signal has memory — it folds over a stream of events.

## Summary

- `sig name : Signal T = initialValue` declares a time-varying value.
- Derived signals use `|>` chains; they recompute automatically.
- Recurrent signals: `seed @|> source` enters the loop; `?|>` guards are optional; `<|@ step` updates the state on each wakeup.
- Signals form a dependency graph maintained by the runtime.
- You never write to a signal; you only declare how it is computed.

[Next: Sources →](/tour/06-sources)
