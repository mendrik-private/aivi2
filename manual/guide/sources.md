# Sources

Sources are how AIVI connects the reactive graph to the outside world. Timers, HTTP requests, keyboard events, file watching, and subprocess events are all modeled as source-backed signals.

## Source-backed signals with `@source`

Today, built-in sources are attached with the `@source` decorator immediately before the signal declaration:

```aivi
@source timer.every 120 with {
    immediate: True,
    coalesce: True
}
signal tick: Signal Unit

value view =
    <Window title="Timer">
        <Label text="Timer source active" />
    </Window>
```

That defines `tick` as a timer-driven signal.

## Window input

```aivi
data Key =
  | Key Text

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown: Signal Key

value view =
    <Window title="Keys">
        <Label text="Press a key" />
    </Window>
```

## HTTP requests

```aivi
data HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

data DecodeMode =
  | Strict
  | Permissive

data Map K V =
  | EmptyMap

domain Duration over Int
    literal sec: Int -> Duration

domain Retry over Int
    literal rt: Int -> Retry

value authHeaders: Map Text Text = EmptyMap

signal apiHost = "https://api.example.com"

@source http.get "{apiHost}/users" with {
    headers: authHeaders,
    decode: Strict,
    retry: 3rt,
    timeout: 5sec
}
signal users: Signal (Result HttpError (List User))

value view =
    <Window title="Users">
        <Label text="Loading users" />
    </Window>
```

## File watching

```aivi
data FsWatchEvent =
  | Created
  | Changed
  | Deleted

@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
signal fileEvents: Signal FsWatchEvent

value view =
    <Window title="Watcher">
        <Label text="Watching files" />
    </Window>
```

## Spawning a process

```aivi
data StreamMode =
  | Ignore
  | Lines
  | Bytes

data ProcessEvent =
  | Spawned

@source process.spawn "rg" ["TODO", "."] with {
    stdout: Lines,
    stderr: Ignore
}
signal grepEvents: Signal ProcessEvent

value view =
    <Window title="Search">
        <Label text="Running rg" />
    </Window>
```

## Custom providers

You can also declare a provider contract:

```aivi
data Mode =
  | Stream

domain Duration over Int
    literal ms: Int -> Duration

provider custom.feed
    argument path: Text
    option timeout: Duration
    option mode: Mode
    wakeup: providerTrigger

@source custom.feed "/tmp/demo.txt" with {
    timeout: 5ms,
    mode: Stream
}
signal updates: Signal Int

value view =
    <Window title="Feed">
        <Label text="Custom provider" />
    </Window>
```

## Summary

| Form | Meaning |
| --- | --- |
| `@source timer.every ...` | Timer-backed signal |
| `@source window.keyDown ...` | Window input signal |
| `@source http.get ...` | HTTP-backed signal |
| `@source fs.watch ...` | File watch signal |
| `@source process.spawn ...` | Process-backed signal |
| `provider custom.feed` | Custom source contract |
