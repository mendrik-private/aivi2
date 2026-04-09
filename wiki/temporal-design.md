# Triggered temporal design

## Question

How should AIVI express "when signal `s` fires, emit after a delay" or "emit `n` times every
`d` and then stop"?

## Current model

- `@source` is the external boundary: outside world -> `@source` -> signal -> pure derivations ->
  UI. Sources are for keyboard input, HTTP, files, timers, and other runtime-facing observables
  (`manual/guide/sources.md`).
- Pipes are the preferred surface when the story is "take this value and keep transforming it"
  rather than "merge external events" (`manual/guide/pipes.md`).
- Recurrent pipes already have an internal scheduler-owned handoff with explicit wakeup proofs and
  non-source timer/backoff wakeups (`crates/aivi-hir/src/recurrence_elaboration.rs`,
  `crates/aivi-typing/src/recurrence.rs`).

## Design options

### 1. Source-shaped solution

Example direction:

```aivi
@source timer.sequence with {
    trigger: placeCoord,
    after: 1ms
}
signal snapshotReady : Signal Coord
```

Pros:

- Reuses existing source lifecycle, trigger, and cancellation machinery.
- Pragmatic if the goal is to ship quickly with minimal new surface syntax.

Cons:

- The trigger signal is not an external boundary; it is already inside the reactive graph.
- This pushes temporal scheduling into the source layer, which makes `@source` do double duty as
  both I/O boundary and internal time-transform surface.
- Payload-preserving replay also needs extra typing work because built-in source contracts do not
  currently relate config to emitted signal type.

### 2. Temporal signal / recurrence solution

Example direction:

```aivi
signal snapshotReady = placeCoord
 |> delay 1ms

signal computerFlashTick = flashStart
 |> burst 200ms 3times
```

Or, if the existing recurrence family stays visible:

```aivi
@recur.timer 200ms
signal flashes : Signal FlashState = seed
 @|> start
 <|@ step
```

Pros:

- More aligned with AIVI's "signals + pipes + scheduler-owned recurrence" model.
- Treats delayed / repeated emission as an internal temporal transform of an existing signal rather
  than a new external source.
- Fits the current recurrence architecture, which already models explicit timer/backoff wakeups.

Cons:

- Requires language design work on temporal pipe/recurrence syntax and lowering, not just a new
  built-in provider.
- May be a larger first implementation than a source-shaped stopgap.

## Recommendation

The **more AIVI-like long-term design** is option 2: make this a temporal signal transform
(probably pipe- or recurrence-shaped), not a new `@source`.

The **most practical short-term implementation** is still option 1 if the immediate goal is to
remove hacks like Reversi's 1ms snapshot timer without opening a wider surface-language design
thread.

So the best sequencing is:

1. If optimizing for language coherence: design a temporal pipe/recurrence surface first.
2. If optimizing for delivery speed: ship a narrow source-shaped helper now, but treat it as a
   stepping stone rather than the final semantic home.
