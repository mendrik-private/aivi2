# Sources

Signals describe *how* a value is computed. Sources describe *where* values come from.

A source is an external event stream — a keyboard, a timer, a network response, a file watcher.
Sources are attached to signals using the `@source` decorator.

## The @source decorator

```text
@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig keyDown : Signal Key
```

`@source` names the provider (`window.keyDown`) and passes a configuration record in
`with { ... }`. The decorator binds to the **next** `sig` declaration below it.

## Source lifecycle

AIVI manages source lifecycle automatically:

1. When the component that owns the signal is mounted, the source is activated.
2. Each event from the source drives the signal's recurrence.
3. When the component is unmounted, the source is torn down.

You never subscribe or unsubscribe manually.

## `@recur.timer` — periodic signals

`@recur.timer` drives a recurrent signal at a fixed interval:

```text
@recur.timer 160ms
sig game : Signal Game =
    initialGame
     @|> stepGame boardSize direction
     <|@ stepGame boardSize direction
```

The interval uses the `Duration` domain literal (`ms`, `sec`, `min`). On every tick the
recurrence step runs, producing the next accumulated state.

Options on the accompanying `@source timer.every N with { ... }` block:

| Option | Type | Meaning |
|---|---|---|
| `immediate` | `Bool` | Fire once on activation before the first tick |
| `coalesce` | `Bool` | Drop accumulated ticks when the handler is busy |

## `@recur.backoff` — retry with back-off

`@recur.backoff` drives a `Task E A` recurrence that retries on failure with exponential
back-off:

```text
@recur.backoff 3x
val fetched : Task HttpError User =
    initialState
     @|> fetchUser
     <|@ fetchUser
```

The retry count uses the `Retry` domain literal (`x`).

## `window.keyDown` — keyboard events

```text
@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig keyDown : Signal Key
```

Emits a `Key` value on every key press. `repeat: False` suppresses held-key repeats.
`focusOnly: True` only fires when the window has focus.

## `http.get` / `http.post` — HTTP requests

```text
@source http.get "{apiHost}/users" with {
    headers: authHeaders,
    decode: Strict,
    retry: 3x,
    timeout: 5s
}
sig users : Signal (Result HttpError (List User))
```

The signal type is `Signal (Result HttpError A)`. It holds the latest response, or the
latest error if the request failed. `decode` controls JSON decoding strictness (`Strict`
or `Permissive`). `retry` and `timeout` use domain literals from `aivi.http`.

## `fs.watch` — filesystem events

```text
@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
sig fileEvents : Signal FsEvent
```

Emits `FsEvent` values (`Created`, `Changed`, `Deleted`) as the watched path changes.
Import `FsEvent` and its constructors from `aivi.fs`.

## `process.spawn` — subprocess output

```text
@source process.spawn "rg" ["TODO", "."] with {
    stdout: Lines,
    stderr: Ignore
}
sig grepEvents : Signal ProcessEvent
```

Spawns a child process and streams its output as `ProcessEvent` values. `stdout` and
`stderr` accept `Lines`, `Bytes`, or `Ignore`.

## Source configuration reference

| Source | Emits | Key options |
|---|---|---|
| `timer.every N` | `TimerTick` | `immediate`, `coalesce` |
| `window.keyDown` | `Key` | `repeat`, `focusOnly` |
| `http.get "url"` | `Result HttpError A` | `headers`, `decode`, `retry`, `timeout` |
| `http.post "url"` | `Result HttpError A` | `body`, `headers`, `decode`, `retry`, `timeout` |
| `fs.watch "path"` | `FsEvent` | `events` |
| `process.spawn "cmd" args` | `ProcessEvent` | `stdout`, `stderr` |

## How sources feed signals

1. `@source` / `@recur.timer` declares the external event stream.
2. The `sig` body with `@|>` / `<|@` describes how each event updates the signal.
3. Derived signals (`|>` chains) recompute automatically when their dependency changes.
4. Markup binds to signals with `{signalName}` attributes.
5. GTK widgets re-render when bound signals change.

Everything between step 1 and step 5 is managed by the AIVI runtime.

## Stale suppression

If a source fires faster than the handler can process, `coalesce: True` drops intermediate
events and delivers only the latest. Essential for high-frequency timers.

## Summary

- `@source provider.name config` decorates the next `sig` declaration.
- `@recur.timer Nms` drives a periodic recurrent signal.
- `@recur.backoff Nx` drives a retrying `Task`.
- Sources are activated on mount and torn down on unmount automatically.
- All source types emit typed values — no raw events reach user code.

[Next: Markup →](/tour/07-markup)
