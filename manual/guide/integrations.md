# External Integrations

AIVI apps stay pure inside and talk to the outside world through a single, uniform boundary: **sources**. Every integration — HTTP, files, timers, databases, D-Bus, custom services — follows the same pattern:

```
Declare a source  →  Annotate the type  →  Derive signals  →  Present in UI
```

Because the outside world feeds in through a typed boundary, your application logic stays pure: no callbacks, no promise chains, no manual error plumbing. The runtime handles the decode, the lifecycle, and the threading.

This page shows efficient integration patterns for the most common cases.

---

## The integration pattern

Every integration has the same four steps:

```aivi
// 1. Declare the source
@source http.get "https://api.example.com/users"
signal usersRaw : Signal (Result HttpError (List User))

signal users : Signal (List User) = usersRaw
  |> .ok
  |> withDefault []

signal userCount : Signal Text = users
  |> length
  |> "Users: {.}"

value main =
    <Window title="Users">
        <Label text={userCount} />
    </Window>

export main
```

Steps 1 and 2 are cleanly separated. The source annotation describes **what** to fetch and **when** to re-fetch. The signals describe **what to do with it** — in pure functions, with no I/O.

---

## Timers

Timers are the simplest source — no external service, no decode.

```aivi
// Fire every 500ms, do not fire on start-up
@source timer.every 500ms with {
    immediate: False,
    coalesce: True
}
signal tick : Signal Unit

signal count : Signal Int
```

Use `coalesce: True` when a slow UI should not queue up a backlog of ticks. The runtime discards any un-processed ticks rather than piling them up.

```aivi
// Fire once after a 2-second delay
@source timer.after 2s
signal startupDone : Signal Unit
```

---

## HTTP

### One-shot fetch on startup

```aivi
@source http.get "https://api.example.com/config"
signal configResponse : Signal (Result HttpError AppConfig)

signal config : Signal AppConfig = configResponse
  |> withDefault defaultConfig
```

The response body is decoded directly into `AppConfig` using the structural decode rules. If your type annotation is a `Result`, decode errors surface as `Err`; if it is a plain type, a decode error produces a runtime diagnostic.

### Polling with a timer

```aivi
@source timer.every 30s with {
    immediate: True,
    coalesce: True
}
signal refreshTick : Signal Unit

@source http.get "https://api.example.com/feed" with {
    refreshOn: refreshTick,
    timeout: 10s
}
signal feedResponse : Signal (Result HttpError (List FeedItem))
```

`immediate: True` fires on startup so the first fetch happens immediately, not 30 seconds later. `coalesce: True` on the timer prevents request queuing if a slow response is still in-flight.

### POST with a payload

```aivi
signal submitPayload : Signal CreateUserRequest

@source http.post "https://api.example.com/users" with {
    refreshOn: submitPayload,
    body: submitPayload
}
signal createResult : Signal (Result HttpError User)
```

The `body` option sends the current value of `submitPayload` as a JSON-encoded request body on each tick.

---

## Filesystem

### Read a file once

```aivi
value configPath = "/etc/myapp/config.json"

@source fs.read configPath
signal configText : Signal (Result FsError AppConfig)
```

### Watch for changes

```aivi
@source fs.watch "/home/user/notes" with {
    recursive: False
}
signal notesChanged : Signal FsEvent

@source fs.read "/home/user/notes/index.md" with {
    reloadOn: notesChanged
}
signal notesIndex : Signal (Result FsError Text)
```

`FsEvent` carries information about what changed (file created, modified, deleted). Derive signals from `notesChanged` to filter by event kind before triggering expensive re-reads.

### Efficient partial reads

If a file is large, annotate the signal type with a narrower record and the runtime will decode only what you need:

```aivi
type AppConfig = {
    theme: Text,
    fontSize: Int
}

@source fs.read "/etc/myapp/config.json"
signal config : Signal (Result FsError AppConfig)
```

---

## Database

### Connect and query

```aivi
@source db.connect "sqlite:///var/lib/myapp/data.db"
signal db : Signal (Result DbError DbHandle)

@source db.live db "SELECT id, name FROM users ORDER BY name"
signal users : Signal (Result DbError (List User))
```

`db.live` re-executes the query whenever the database is written to. The result decodes into `List User` using the structural decode rules (field names match column names).

### Parameterised queries

```aivi
signal selectedTag : Signal Text

@source db.live db "SELECT * FROM posts WHERE tag = $1" with {
    refreshOn: selectedTag
}
signal taggedPosts : Signal (Result DbError (List Post))
```

`refreshOn` re-executes the query whenever the trigger signal fires. Embed query parameters as literals in the SQL string, or construct the query string reactively from a derived signal.

---

## D-Bus

### Subscribe to a signal

```aivi
@source dbus.signal "/org/freedesktop/NetworkManager" with {
    interface: "org.freedesktop.DBus.Properties",
    member: "PropertiesChanged"
}
signal nmProperties : Signal (Result DbusError DbusMessage)
```

The object path is the positional argument to `dbus.signal`. `interface` and `member` are named options.

### Call a method

```aivi
@source dbus.method "org.freedesktop.NetworkManager" with {
    path: "/org/freedesktop/NetworkManager",
    interface: "org.freedesktop.NetworkManager",
    member: "GetDevices"
}
signal devices : Signal (Result DbusError (List Text))
```

The D-Bus destination (bus name) is the positional argument to `dbus.method`. Object path, interface, and member are named options.

D-Bus method sources call the method on startup (or on a trigger) and decode the reply. Use `dbus.signal` for subscriptions to broadcasts; use `dbus.method` for one-shot or triggered queries.

### Decode the reply

Annotate the signal type and the runtime decodes D-Bus variant values into AIVI types:

```aivi
type NetworkState = {
    connectivity: Int,
    state: Int
}

@source dbus.method with {}
signal networkState : Signal (Result DbusError NetworkState)
```

---

## Environment and process context

```aivi
// Snapshot env on startup — no reactive re-reads needed
@source env.get "HOME"
signal homeDir : Signal (Result EnvError Text)

@source env.getAll
signal envMap : Signal (Result EnvError (Dict Text Text))
```

---

## Combining multiple sources

Use signal merge to combine several sources into a single event stream:

```aivi
signal refreshClick : Signal Unit

@source timer.every 60s with {
    immediate: True,
    coalesce: True
}
signal autoRefresh : Signal Unit

signal refresh : Signal Unit = refreshClick | autoRefresh
  ||> _ _ => ()

@source http.get "https://api.example.com/data" with {
    refreshOn: refresh
}
signal data : Signal (Result HttpError (List Item))
```

This wires a manual refresh button and an automatic 60-second refresh into a single trigger for the HTTP source.

---

## Custom source providers

When no built-in source fits, declare a custom provider with a contract:

```aivi
// <unparseable item>
@source BluetoothScanner with {
    scanDurationMs: 5000,
    nameFilter: Some "MyDevice"
}
signal device : Signal (Result ScanError BluetoothDevice)
```

The contract declares what options the provider accepts, what type it emits, and when it wakes up. The implementation lives in a Rust provider crate. The AIVI side stays declarative.

---

## Patterns and tips

### Decode early, derive late

Always annotate source signals with the narrowest useful type. Let the runtime do the decode work at the boundary:

```aivi
// Good — decode at the source boundary
@source http.get url
signal user : Signal (Result HttpError User)

@source http.get url
signal userJson : Signal (Result HttpError Text)

signal user = userJson
  |> map parseUser
```

### Coalesce high-frequency sources

For timers driving network requests or expensive computations, always set `coalesce: True`:

```aivi
@source timer.every 100ms with {
    immediate: False,
    coalesce: True
}
signal tick : Signal Unit
```

Without coalescing, a slow downstream will cause ticks to queue.

### Gate expensive signals with `?|>`

If a derived signal is expensive and only meaningful in certain states, gate it:

```aivi
signal isLoggedIn : Signal Bool

signal userProfile : Signal Profile = authToken
 ?|> isLoggedIn
  |> fetchProfile
```

The `fetchProfile` derivation only runs when `isLoggedIn` is `True`.

### Lift errors to the UI

Every I/O signal should carry a `Result`. Lift errors into the UI explicitly rather than silently defaulting:

```aivi
signal configResult : Signal (Result FsError AppConfig)

signal configError : Signal (Option Text) = configResult
 ||> Ok _  -> None
 ||> Err e -> Some (renderFsError e)

value main =
    <Window title="App">
        <show when={configError |> isSome}>
            <Label text={configError |> withDefault ""} />
        </show>
    </Window>
```

---

*See also: [sources.md](sources.md) for the source mechanism, [source-catalog.md](source-catalog.md) for the full reference, [signals.md](signals.md) for the reactive graph.*
