# Sources

Signals describe *how* a value is computed. Sources describe *where* values come from.

A source is an external event stream — a keyboard, a timer, a network response, a file watcher.
Sources are attached to signals using the `@source` decorator.

## The @source decorator

```aivi
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

## Headless host-context sources

`aivi execute` also exposes one-shot host-context providers for CLI programs. These publish once
at startup, commit a snapshot, and then stay stable for the rest of the run.

```aivi
@source process.args
sig args : Signal (List Text)

@source env.get "ACCESS_TOKEN"
sig token : Signal (Option Text)

@source path.configHome
sig configHome : Signal Text
```

Available execute-time providers:

| Source | Emits | Meaning |
|---|---|---|
| `process.args` | `List Text` | Positional arguments after `aivi execute <path> --` |
| `process.cwd` | `Text` | Current working directory for the launched CLI process |
| `env.get "NAME"` | `Option Text` | Environment variable lookup |
| `stdio.read` | `Text` | Entire stdin payload read once before execution |
| `path.home` | `Text` | User home directory |
| `path.configHome` | `Text` | XDG config home |
| `path.dataHome` | `Text` | XDG data home |
| `path.cacheHome` | `Text` | XDG cache home |
| `path.tempDir` | `Text` | Host temporary directory |

## `@recur.timer` — periodic signals

`@recur.timer` drives a recurrent signal at a fixed interval:

```aivi
@recur.timer 160ms
sig game : Signal Game =
    initialGame
     @|> stepGame boardSize direction
     <|@ stepGame boardSize direction
```

The interval uses the `Duration` domain literal (`ms`, `sec`, `min`). The decorator provides
the timer wakeups; on each tick the recurrent stages run and produce the next accumulated state.

Options for `@source timer.every N` (the expanded form of `@recur.timer`):

| Option | Type | Meaning |
|---|---|---|
| `immediate` | `Bool` | Fire once on activation before the first tick |
| `coalesce` | `Bool` | Drop accumulated ticks when the handler is busy |

## `@recur.backoff` — retry with back-off

`@recur.backoff` drives a `Task E A` recurrence that retries on failure with exponential
back-off:

```aivi
@recur.backoff 3x
val fetched : Task HttpError User =
    initialState
     @|> fetchUser
     <|@ fetchUser
```

The retry count uses the `Retry` domain literal (`x`).

## `window.keyDown` — keyboard events

```aivi
@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig keyDown : Signal Key
```

Emits a `Key` value on every key press. `repeat: False` suppresses held-key repeats.
`focusOnly: True` only fires when the window has focus.

## `http.get` / `http.post` — HTTP requests

```aivi
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

```aivi
@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
sig fileEvents : Signal FsEvent
```

Emits `FsEvent` values (`Created`, `Changed`, `Deleted`) as the watched path changes.
Import `FsEvent` and its constructors from `aivi.fs`.

## `process.spawn` — subprocess output

```aivi
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
| `process.args` | `List Text` | none |
| `process.cwd` | `Text` | none |
| `env.get "NAME"` | `Option Text` | none |
| `stdio.read` | `Text` | none |
| `path.home` | `Text` | none |
| `path.configHome` | `Text` | none |
| `path.dataHome` | `Text` | none |
| `path.cacheHome` | `Text` | none |
| `path.tempDir` | `Text` | none |
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
- `aivi execute` adds one-shot process/stdin/path providers for headless programs.
- Sources are activated on mount and torn down on unmount automatically.
- All source types emit typed values — no raw events reach user code.

[Next: Markup →](/tour/07-markup)
