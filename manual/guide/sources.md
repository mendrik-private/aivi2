# Sources

Pure functions cannot read files, make HTTP requests, or listen for keyboard input. They take values and return values — that is their strength.

But a desktop application needs to talk to the outside world. **Sources** are how AIVI bridges that gap. A source is a typed, declared entry point that feeds external data into the reactive graph:

```
Outside world  →  @source  →  Signal  →  Pure derivations  →  UI
  (keyboard,      (typed      (reactive    (your functions)    (GTK
   HTTP,           boundary)   graph)                          widgets)
   timers,
   files)
```

Inside the boundary, everything is deterministic. Outside, the runtime handles the mess.

For the current compiler-and-runtime-backed reference of every built-in source kind and option, see the [Built-in Source Catalog](/guide/source-catalog).

## Unified external boundary

Built-in capability handles are now the public external surface for the built-in families that have
both reactive reads and one-shot commands. Modules such as `aivi.fs`, `aivi.http`, `aivi.env`,
`aivi.log`, `aivi.stdio`, `aivi.random`, and `aivi.data.json` remain as shared type/helper
vocabularies, but they no longer expose parallel effectful entry points.

Current shape:

```aivi
use aivi.fs (FsSource, FsError, FsEvent)

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError AppConfig) = files.read configPath
signal changes : Signal FsEvent = files.watch configPath
value cleanup : Task Text Unit = files.delete "cache.txt"
```

In that model:

- reads, watches, queries, and subscriptions stay source/reactive
- mutations become explicit provider-owned commands on the same capability
- incoming data decodes directly into the annotated target type
- host snapshots such as environment/process/XDG data use the same provider boundary
- sink-style effects such as logging, stdio writes, D-Bus method calls, and outbound sends do too
- raw JSON-as-text helper workflows are not the public external design anymore

## Built-in capability handles

Built-in provider families now support bodyless handle anchors plus direct top-level member use.
The compiler lowers those forms onto the existing built-in source providers, task intrinsics, and
pure host-context intrinsics:

```aivi
use aivi.fs (FsSource, FsError)

signal projectRoot : Signal Text = "/tmp/demo"

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
value cleanup = files.delete "cache.txt"
```

Today this lowering is implemented for `fs`, `http`, `db`, `env`, `log`, `stdio`, `random`,
`process`, `path`, and `dbus`.

Current rules:

- handle anchors must stay bodyless and use a nominal non-`Signal` annotation such as `FsSource`
- direct `signal name : Signal T = handle.member ...` forms lower to ordinary bodyless source
  bindings with synthesized `@source provider.variant ...` metadata
- direct `value name = handle.member ...` forms lower through the built-in handle task path for
  commands, queries, and host snapshots
- capability handles are compile-time anchors, not exported runtime signals
- custom provider contracts may declare `operation` and `command` members already
- direct `signal name : Signal T = handle.member ...` lowering now works for custom provider
  operations too; those lower to member-qualified custom source bindings such as
  `@source custom.feed.read ...`
- direct custom command handle values are still pending a generic task/runtime bridge

## Source-backed signals with `@source`

Today, built-in sources are attached with the `@source` decorator immediately before the signal declaration:

```aivi
@source timer.every 120 with {
    immediate: True,
    coalesce: True
}
signal tick : Signal Unit

value view =
    <Window title="Timer">
        <Label text="Timer source active" />
    </Window>
```

That defines `tick` as a timer-driven signal.

## Window input

```aivi
type Key =
  | Key Text

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown : Signal Key

value view =
    <Window title="Keys">
        <Label text="Press a key" />
    </Window>
```

## HTTP requests

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

type DecodeMode =
  | Strict
  | Permissive

type Map K V =
  | EmptyMap

domain Duration over Int

domain Retry over Int

value authHeaders : Map Text Text = EmptyMap

signal apiHost = "https://api.example.com"

@source http.get "{apiHost}/users" with {
    headers: authHeaders,
    decode: Strict,
    retry: 3times,
    timeout: 5sec
}
signal users : Signal (Result HttpError (List User))

value view =
    <Window title="Users">
        <Label text="Loading users" />
    </Window>
```

## File watching

```aivi
type FsWatchEvent =
  | Created
  | Changed
  | Deleted

@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
signal fileEvents : Signal FsWatchEvent

value view =
    <Window title="Watcher">
        <Label text="Watching files" />
    </Window>
```

## Spawning a process

```aivi
type StreamMode =
  | Ignore
  | Lines
  | Bytes

type ProcessEvent =
  | Spawned

@source process.spawn "rg" ["TODO", "."] with {
    stdout: Lines,
    stderr: Ignore
}
signal grepEvents : Signal ProcessEvent

value view =
    <Window title="Search">
        <Label text="Running rg" />
    </Window>
```

## Custom providers

You can also declare a provider contract. Argument and option declarations still describe the
`@source` boundary itself; `operation` and `command` declarations preserve the capability-member
surface in HIR so later custom-provider handle lowering can target one provider-owned API:

```aivi
type Mode =
  | Stream

domain Duration over Int

provider custom.feed
    argument path : Text
    option timeout : Duration
    option mode : Mode
    operation read : Text -> Signal Int
    command delete : Text -> Task Text Unit
    wakeup: providerTrigger

@source custom.feed "/tmp/demo.txt" with {
    timeout: 5ms,
    mode: Stream
}
signal updates : Signal Int

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
| `provider custom.feed` | Custom source/capability contract |
| [Built-in Source Catalog](/guide/source-catalog) | Current source-kind and option reference |
