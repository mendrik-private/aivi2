# Sources

Sources are how AIVI programs interact with the outside world. They represent effects: network requests, timers, keyboard input, file system events, and more. Each source is declared explicitly, typed completely, and managed by the runtime.

## The `@source` Decorator

A source is attached to a signal using the `@source` decorator on the line immediately before the `signal` declaration:

```aivi
@source timer.every 200 with {
    immediate: False,
    coalesce: True
}
signal tick: Signal Unit
```

This declares `tick` as a signal that fires every 200 milliseconds. The `with { ... }` block supplies options to the source.

## Source Identity

The runtime uses the source's arguments and options to determine its **identity**. Two signals with identical identities share the same underlying acquisition — there is no duplicate work. If a new request has the same identity as one already in-flight, the runtime deduplicates it.

## Built-In Sources

### `timer.every`

Fires on a regular interval:

```aivi
@source timer.every 200 with {
    immediate: False,
    coalesce: True
}
signal tick: Signal Unit
```

| Option | Type | Description |
|---|---|---|
| `immediate` | `Bool` | Fire immediately on first evaluation |
| `coalesce` | `Bool` | Merge multiple pending firings into one |

### `window.keyDown`

Emits keyboard events from the application window:

```aivi
type Key = Key Text

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown: Signal Key
```

| Option | Type | Description |
|---|---|---|
| `repeat` | `Bool` | Include auto-repeated key events |
| `focusOnly` | `Bool` | Only fire when the window has focus |

Each `Key` carries the key name as a `Text` value, e.g. `Key "ArrowUp"`, `Key " "`.

### `http.get`

Fetches a URL and decodes the response:

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

type Map K V = | EmptyMap

domain Duration over Int
    literal s: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry

value authHeaders: Map Text Text = EmptyMap
signal apiHost: Signal Text = "https://api.example.com"

@source http.get "{apiHost}/users" with {
    headers: authHeaders,
    decode: Strict,
    retry: 3x,
    timeout: 5s
}
signal users: Signal (Result HttpError (List User))
```

The URL can interpolate signals using `{signalName}`. When `apiHost` changes, the source re-fetches.

| Option | Type | Description |
|---|---|---|
| `headers` | `Map Text Text` | HTTP request headers |
| `decode` | `DecodeMode` | `Strict` or `Permissive` |
| `retry` | `Retry` | Number of retries (e.g. `3x`) |
| `timeout` | `Duration` | Request timeout (e.g. `5s`) |

### `fs.watch`

Watches a path for file system events:

```aivi
type FsWatchEvent =
  | Created
  | Changed
  | Deleted

@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
signal fileEvents: Signal FsWatchEvent
```

### `process.spawn`

Spawns a subprocess and streams its output:

```aivi
type ProcessEvent =
  | Spawned

type StreamMode =
  | Ignore
  | Lines
  | Bytes

@source process.spawn "rg" ["TODO", "."] with {
    stdout: Lines,
    stderr: Ignore
}
signal grepEvents: Signal ProcessEvent
```

| Option | Type | Description |
|---|---|---|
| `stdout` | `StreamMode` | How to handle stdout |
| `stderr` | `StreamMode` | How to handle stderr |

## Source State and Errors

Every source-backed signal exposes meta-state through its fields:

```aivi
signal users: Signal (Result HttpError (List User)) = ...

signal isLoading: Signal Bool       = users.running
signal loadFailed: Signal (Option Error) = users.error
```

When the source is in-flight, `users.running` is `True`. When it completes successfully, `users.done` is `True` and the signal carries `Ok (List User)`. If it fails, `users.error` holds the error.

## Actions `.do`

Sources can also expose **actions** — typed operations that perform an effect when invoked. Actions are attached to a signal and invoked through `.do`:

```aivi
signal submitResult = form.do.submit
```

Actions are typed:

```aivi
signal.do.action : (Input?) -> ActionResult E A
```

Where `A` is the success value and `E` is the failure type. Invoking an action returns a new signal — the result integrates into the signal graph automatically.

## Custom Providers

You can declare custom provider contracts for domain-specific sources:

```aivi
provider custom.feed
    argument path: Text
    option timeout: Duration
    wakeup: providerTrigger

provider custom.timer
    option activeWhen: Signal Bool
    wakeup: timer
```

- `argument` — a required positional argument
- `option` — an optional configuration value
- `wakeup` — specifies how the provider triggers re-evaluation

## Runtime Guarantees

The source system provides several guarantees:

- **Deduplication**: identical sources are not duplicated; in-flight requests are shared
- **Scheduling**: sources run on worker threads; they never block the GTK main thread
- **Cancellation**: when a source's inputs change and a new request is needed, the old one is cancelled
- **Retries**: the `retry` option handles transient failures transparently
- **Stale-while-revalidate**: a cached value can be served while a refresh is in-flight

## Example: Combining Sources

Here is a signal that combines a timer with keyboard input:

```aivi
@source timer.every 500 with {
    immediate: True,
    coalesce: True
}
signal pollTick: Signal Unit

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown: Signal Key

signal refreshCount: Signal Int =
    keyDown
     |> scan 0 (\count _ => count + 1)

signal autoRefreshCount: Signal Int =
    pollTick
     |> scan 0 (\count _ => count + 1)
```

Both signals accumulate independently. The runtime manages their lifecycles separately.
