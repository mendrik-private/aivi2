# Built-in Source Catalog

This page is the dedicated reference for the current `@source` surface. It is intentionally conservative: it follows the compiler and runtime that ship today, not deferred RFC ideas or older examples.

Use [Sources](/guide/sources) for the tutorial overview. Use this page when you need the trustworthy answer to:

- which built-in source kinds exist today,
- which positional arguments they take,
- which named options are actually supported,
- and which options are only partially wired or still intentionally limited.

This page documents the **current shipped** low-level source variants. Public code should prefer
provider capability handles such as `@source fs`, `@source http`, `@source env`, and `@source path`
when a built-in family exposes one. Direct `@source provider.variant ...` remains the underlying
form and is still the right choice when you need variant-specific options that are not yet surfaced
as handle members.

## Reading the catalog

- Arguments are positional values written after the source kind.
- Options are named fields inside `with { ... }`.
- `activeWhen`, `refreshOn`, `reloadOn`, and `restartOn` are lifecycle/trigger options: they are part of the accepted source contract, but their behavior is planned in the compiler/runtime lifecycle machinery rather than by the provider-specific parser alone.
- “Not executed yet” means the option is in the current contract surface but the runtime still rejects or narrows it at provider startup.

## Common current rules

- `http.get`, `http.post`, `fs.read`, `socket.connect`, and `mailbox.subscribe` currently expect a `Signal (Result E A)` output shape.
- `fs.watch` currently decodes to `Text` or to a payloadless sum such as `Created | Changed | Deleted`.
- `window.keyDown` currently decodes to `Text`, a payloadless key sum, or a single text-wrapping key constructor.
- `dbus.signal` and `dbus.method` currently decode to specific record shapes described below.
- Scheduler-owned recurrence work is still being tightened across the pipeline, so this page calls out where a contract option exists before the runtime fully executes it.

## Unified capability families

These families now have a preferred public handle surface:

| Family | Preferred public surface | Lowered implementation |
| --- | --- | --- |
| File system | `@source fs ...` + `FsSource` | `fs.read`, `fs.watch`, plus handle-member tasks such as `files.delete` |
| HTTP | `@source http ...` + `HttpSource` | `http.get`, `http.post`, plus handle-member request tasks such as `api.get` |
| OpenAPI | `@source api "spec.yaml" with { baseUrl: url, ... }` + `ApiSource` | `api.get`, `api.post`, `api.put`, `api.patch`, `api.delete` |
| Database | `@source db ...` + `DbSource` | `db.connect`, `db.live`, plus handle-member `database.query` / `database.commit` tasks |
| Environment | `@source env` + `EnvSource` | `env.get` plus handle-member environment snapshots/listing |
| Logging / stdio | `@source log`, `@source stdio` | log/stdio handle members plus `stdio.read` |
| Randomness | `@source random` + `RandomSource` | handle-member random tasks such as `entropy.bytes` |
| Process / path / D-Bus | `@source process`, `@source path`, `@source dbus` | existing built-in providers plus handle-member host snapshots |

Incoming payloads decode directly into the annotated target type. JSON-as-text helpers are no
longer part of the public external boundary.

## Timers

### `timer.every`

**Form:** `@source timer.every interval`

| Option | Type | Current support |
| --- | --- | --- |
| `immediate` | `Bool` | Supported. Publishes once immediately before the repeating cadence starts. |
| `jitter` | `Duration` | Supported. Adds a random positive offset (from 0 up to the jitter value) to each tick interval. The jitter must not exceed the base interval. |
| `coalesce` | `Bool` | Supported. `True` (default) coalesces missed ticks into a single event per sleep cycle. `False` fires all overdue ticks individually. |
| `restartOn` | `Signal A` | Supported as an explicit trigger option. Reconfigures the timer and restarts its cadence. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |

**Notes**

- The interval is the single positional argument.
- The runtime still accepts a bare integer timer argument as a legacy milliseconds path, but new documentation should prefer `Duration` values.

### `timer.after`

**Form:** `@source timer.after delay`

Uses the same option surface and current limitations as `timer.every`, but fires once instead of repeating. `restartOn` re-arms the one-shot delay each time its trigger fires.

## HTTP

### `http.get`

**Form:** `@source http.get url`

| Option | Type | Current support |
| --- | --- | --- |
| `headers` | `Map Text Text` | Supported. |
| `query` | `Map Text Text` | Supported. |
| `decode` | `DecodeMode` | Supported through the decode pipeline. |
| `timeout` | `Duration` | Supported. |
| `retry` | `Retry` | Supported. |
| `refreshOn` | `Signal B` | Supported as an explicit trigger option. |
| `refreshEvery` | `Duration` | Supported as polling cadence input. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |
| `body` | `A` | Supported. Sends the value as the request body (text or JSON-encoded). While uncommon, RFC 9110 permits request bodies on GET. |

**Notes**

- `http.get` is request-like: newer generations supersede older ones.
- Repeated requests still need an explicit wakeup such as reactive inputs, `refreshOn`, `refreshEvery`, or `retry`.

### `http.post`

**Form:** `@source http.post url`

Uses the same option surface as `http.get`, plus:

| Option | Type | Current support |
| --- | --- | --- |
| `body` | `A` | Supported for `http.post` only. |

## OpenAPI

The `api` family provides a typed capability handle backed by an OpenAPI 3.x spec file. Member
access on the handle is validated against the spec at compile time when the spec path is a static
literal.

Read operations (`listPets`, `showPetById`, etc.) lower to `api.get` signal providers. Write
operations (`createPet`, `deletePet`, etc.) lower to `api.post` / `api.put` / `api.patch` /
`api.delete` intrinsic value calls.

Generate AIVI type declarations from the spec with:

```
aivi openapi-gen ./spec.yaml -o types/api.aivi
```

### `api.get` / `api.post` / `api.put` / `api.patch` / `api.delete`

**Handle form:**

```aivi
@source api "./spec.yaml" with {
    baseUrl: serverUrl,
    auth: BearerToken apiToken,
    timeout: 30sec
}
signal petstore : ApiSource
```

**Option surface** (same as `http.get` / `http.post`, plus):

| Option | Type | Current support |
| --- | --- | --- |
| `baseUrl` | `Text` | Required. Base URL prepended to every operation path. |
| `auth` | `ApiAuth` | Supported. Injects `Authorization` or `X-API-Key` headers from the value. |
| `headers` | `Map Text Text` | Supported. Additional HTTP headers. |
| `body` | `A` | Supported for mutation operations. |
| `timeout` | `Duration` | Supported. |
| `retry` | `Retry` | Supported. |
| `refreshEvery` | `Duration` | Supported. |
| `decode` | `DecodeMode` | Supported through the decode pipeline. |
| `refreshOn` | `Signal A` | Supported as an explicit trigger signal. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |

**Notes**

- The spec path (first argument) is only used at compile time for operationId validation. It is not used at runtime.
- The final request URL is `baseUrl` + the operation's path from the spec.
- Auth variants: `BearerToken Text`, `BasicAuth Text Text`, `ApiKey Text`, `ApiKeyQuery Text`, `OAuth2 Text`.
- `ApiKeyQuery` injects the key as a query parameter rather than a header (not yet applied at runtime).

## File system

### `fs.watch`

**Form:** `@source fs.watch path`

| Option | Type | Current support |
| --- | --- | --- |
| `events` | `List FsWatchEvent` | Supported. Defaults to `Created`, `Changed`, and `Deleted` when omitted. |
| `recursive` | `Bool` | Supported. When `True`, watches all files in the directory tree. When `False` (default), watches only the specified path. |

**Notes**

- `fs.watch` reports file-system events only. It does not read file contents.

### `fs.read`

**Form:** `@source fs.read path`

| Option | Type | Current support |
| --- | --- | --- |
| `decode` | `DecodeMode` | Supported through the decode pipeline. |
| `reloadOn` | `Signal A` | Supported as an explicit trigger option. |
| `debounce` | `Duration` | Supported. |
| `readOnStart` | `Bool` | Supported. Defaults to `True`. |

**Notes**

- `fs.read` is the snapshot-reading companion to `fs.watch`.
- `debounce` and `readOnStart` affect runtime reads; `reloadOn` remains the explicit trigger mechanism.

## Streams and messaging

### `socket.connect`

**Form:** `@source socket.connect address`

| Option | Type | Current support |
| --- | --- | --- |
| `decode` | `DecodeMode` | Supported through the decode pipeline. |
| `buffer` | `Int` | Supported. |
| `reconnect` | `Bool` | Supported. |
| `heartbeat` | `Duration` | Supported. Spawns a keepalive writer thread that periodically sends a newline to prevent idle timeouts. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |

**Notes**

- `socket.connect` currently supports only `tcp://host:port` URLs.
- It is a raw TCP line-stream surface, not a WebSocket or general framed protocol surface.

### `mailbox.subscribe`

**Form:** `@source mailbox.subscribe mailbox`

| Option | Type | Current support |
| --- | --- | --- |
| `decode` | `DecodeMode` | Supported through the decode pipeline. |
| `buffer` | `Int` | Supported. |
| `reconnect` | `Bool` | Supported. When `True`, the subscriber retries after a disconnection. |
| `heartbeat` | `Duration` | Supported. Publishes periodic `Unit` heartbeat events at the specified interval. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |

**Notes**

- `mailbox.subscribe` is a process-local text bus.

## Processes and host context

### `process.spawn`

**Form:** `@source process.spawn command` or `@source process.spawn command args`

The second positional argument, when present, is a `List Text`.

| Option | Type | Current support |
| --- | --- | --- |
| `cwd` | `Path` | Supported. |
| `env` | `Map Text Text` | Supported. |
| `stdout` | `StreamMode` | Supported. `Ignore` discards output, `Lines` publishes text lines, `Bytes` publishes raw byte chunks. |
| `stderr` | `StreamMode` | Supported. `Ignore` discards output, `Lines` publishes text lines, `Bytes` publishes raw byte chunks. |
| `restartOn` | `Signal A` | Supported as an explicit trigger option. |

**Notes**

- The output type must currently be a sum-shaped `ProcessEvent`-style signal.
- Recognized event variants are `Spawned`, `Stdout`, `Stderr`, `Exited`, and `Failed`.
- `stdout: Lines` or `stdout: Bytes` requires a `Stdout` variant in the output type, and `stderr: Lines` or `stderr: Bytes` requires a `Stderr` variant.

### Immediate host-context sources

These built-ins publish one host-context snapshot when the source starts. They do not take options.

| Kind | Positional arguments | Published shape | Current notes |
| --- | --- | --- | --- |
| `process.args` | none | `List Text` | Supported. Publishes the current process arguments. |
| `process.cwd` | none | `Text` | Supported. Publishes the current working directory text. |
| `env.get` | `key` | `Option Text` | Supported. Publishes `Some value` when the environment variable exists, otherwise `None`. |
| `stdio.read` | none | `Text` | Supported. Reads stdin once and publishes the text snapshot. |
| `path.home` | none | `Text` | Supported. Fails if `HOME` is missing. |
| `path.configHome` | none | `Text` | Supported. Uses `XDG_CONFIG_HOME`, or falls back to `$HOME/.config`. |
| `path.dataHome` | none | `Text` | Supported. Uses `XDG_DATA_HOME`, or falls back to `$HOME/.local/share`. |
| `path.cacheHome` | none | `Text` | Supported. Uses `XDG_CACHE_HOME`, or falls back to `$HOME/.cache`. |
| `path.tempDir` | none | `Text` | Supported. Publishes the current process temp directory text. |

## Database

### `db.connect`

**Form:** `@source db.connect config`

| Option | Type | Current support |
| --- | --- | --- |
| `pool` | `Int` | Accepted and validated. The current runtime still opens a single SQLite connection probe per source instance; pool sizing is reserved for later provider-owned pooling work. |
| `activeWhen` | `Signal Bool` | Supported as a lifecycle gate. |

**Notes**

- `config` must currently evaluate to either a database path `Text` or a record containing `database: Text`.
- Relative database paths are resolved against the runtime working directory before publication.
- The current runtime validates that SQLite can open the target and then publishes the normalized `Connection` record.
- The published `Connection.database` field is the normalized path the runtime actually opened.
- The intended result shape is `Signal (Result DbError Connection)`.

### `db.live`

**Form:** `@source db.live query`

| Option | Type | Current support |
| --- | --- | --- |
| `refreshOn` | `Signal B` | Supported through the existing source reconfiguration lifecycle, including pre-elaborated `.changed` projections. |
| `debounce` | `Duration` | Supported for refresh reconfiguration; activation still loads immediately. |
| `optimistic` | `Bool` | Supported. When `True`, the provider may publish the previous known-good value immediately while the query runs. Default: `False`. |
| `onRollback` | `Signal DbError` | Supported as a lifecycle option. The runtime accepts this signal reference for rollback notification when an optimistic update is reverted. |
| `activeWhen` | `Signal Bool` | Supported through the existing source activation lifecycle. |

**Notes**

- The compiler now recognizes `db.live` as a built-in provider key.
- Runtime execution now runs the query task on a worker thread and republishes on activation or refresh.
- The intended result shape is `Signal (Result DbError A)`.
- Successful `db.commit` tasks now advance matching input-backed `.changed` signals using the current `Connection.database` path plus changed table names, so `db.live refreshOn` paths refresh automatically after commits.
- `refreshOn` is the whole refresh boundary in the current slice. A `users.changed` projection is accepted, but it is still just an explicit trigger signal routed through that same path.
- `commit()` now drives `TableRef.changed`-style refreshes automatically when the commit plan names the changed tables and the table handle resolves to the same normalized `Connection.database` path.
- Invalidation is still coarse at the table level in this slice. Row-scoped `watch` behavior remains future work.

## GTK input

### `window.keyDown`

**Form:** `@source window.keyDown`

| Option | Type | Current support |
| --- | --- | --- |
| `capture` | `Bool` | Supported. When `True`, the event controller uses the capture propagation phase. Default: `False`. |
| `repeat` | `Bool` | Supported. |
| `focusOnly` | `Bool` | Supported. When `False`, key events are captured even when the window is not focused. Default: `True`. |

**Notes**

- Delivery comes from the focused window key controller.

### `gtk.darkMode`

**Form:** `@source gtk.darkMode`

No options.  The output must decode to `Bool`.

Emits the current system dark-mode state (`True` = dark) once at startup and again
each time the user changes the appearance preference in GNOME Settings.  Backed by
`adw::StyleManager::is_dark()`.

**Example**

```aivi
type Theme = Light | Dark

@source gtk.darkMode
signal rawDark : Bool

signal theme : Theme
```

**Notes**

- The source fires one initial value before the first render tick, so `theme` is
  always populated when the UI first appears.
- Uses `adw::StyleManager::default()`, so it reflects the system-wide GNOME preference
  or any app-level override set via `adw::StyleManager::set_color_scheme()`.

### `clipboard.changed`

**Form:** `@source clipboard.changed`

No options.  The output must decode to `Text`.

Emits the current clipboard text once at startup and again each time the GDK clipboard
content changes.  Backed by `gdk::Display::default().clipboard()`.  Non-text clipboard
contents (images, files) yield an empty string.

**Example**

```aivi
@source clipboard.changed
signal clipboardText : Text

value view =
    <Window title="Clipboard Watcher">
        <Label text={clipboardText} wrap={True} halign="Start" marginStart={12} marginEnd={12} marginTop={12} marginBottom={12} />
    </Window>
```

**Notes**

- The source fires one initial value before the first render tick, so `clipboardText`
  is always populated when the UI first appears.
- Only the latest clipboard text is kept per tick (coalescing queue); rapid clipboard
  changes between scheduler ticks collapse to a single update.
- Reads happen asynchronously on the GLib main thread; the signal updates on the next
  scheduler tick after the read completes.

### `window.size`

**Form:** `@source window.size`

No options.  The output must decode to `{ width: Int, height: Int }`.

Emits the current window dimensions once at startup and again each time the width or
height of the application's root window changes.

**Example**

```aivi
@source window.size
signal windowDimensions : { width: Int, height: Int }

value view =
    <Window title="App">
        <Label text={"W=" + Int.toText windowDimensions.width + " H=" + Int.toText windowDimensions.height} halign="Center" valign="Center" />
    </Window>
```

**Notes**

- Fires one initial value at startup before the first render tick.
- Width and height changes from the same scheduler tick are coalesced into a single update.

### `window.focus`

**Form:** `@source window.focus`

No options.  The output must decode to `Bool`.

Emits `True` when the application window gains focus and `False` when it loses focus.
Fires once at startup with the initial focus state.

**Example**

```aivi
@source window.focus
signal hasFocus : Bool

value view =
    <Window title="App">
        <Label text={if hasFocus "Focused" "Unfocused"} halign="Center" valign="Center" />
    </Window>
```

**Notes**

- Backed by `GtkWindow::is-active` property notifications.
- Fires one initial value at startup.

## D-Bus

### `dbus.ownName`

**Form:** `@source dbus.ownName wellKnownName`

| Option | Type | Current support |
| --- | --- | --- |
| `bus` | `Text` | Supported. Current accepted values are `"session"` and `"system"`. Defaults to the session bus. |
| `address` | `Text` | Supported. |
| `flags` | `List BusNameFlag` | Supported with `AllowReplacement`, `ReplaceExisting`, and `DoNotQueue`. |

**Notes**

- The output must currently decode to `Text` or to a payloadless `BusNameState` sum with exactly `Owned`, `Queued`, and `Lost`.

### `dbus.signal`

**Form:** `@source dbus.signal path`

| Option | Type | Current support |
| --- | --- | --- |
| `bus` | `Text` | Supported. Current accepted values are `"session"` and `"system"`. Defaults to the session bus. |
| `address` | `Text` | Supported. |
| `interface` | `Text` | Supported. |
| `member` | `Text` | Supported. |

**Notes**

- The output must currently be a record with fields `path`, `interface`, `member`, and `body`.
- Header fields must decode as `Text`.
- The `body` field may currently be `Text` or `List DbusValue`.
- `Maybe` payloads and floating-point D-Bus payloads are still outside the current runtime slice.

### `dbus.method`

**Form:** `@source dbus.method destination`

| Option | Type | Current support |
| --- | --- | --- |
| `bus` | `Text` | Supported. Current accepted values are `"session"` and `"system"`. Defaults to the session bus. |
| `address` | `Text` | Supported. |
| `path` | `Text` | Supported. |
| `interface` | `Text` | Supported. |
| `member` | `Text` | Supported. |

**Notes**

- The output must currently be a record with fields `destination`, `path`, `interface`, `member`, and `body`.
- Header fields must decode as `Text`.
- The `body` field may currently be `Text` or `List DbusValue`.
- The runtime currently replies on the D-Bus wire with `Unit` immediately; non-`Unit` reply payloads are still deferred.

## Custom source providers

Custom `@source` uses are supported through top-level `provider qualified.name` declarations.

Current declaration reality:

- provider names are qualified top-level names such as `custom.feed`,
- supported declaration fields are `argument`, `option`, and `wakeup`,
- `wakeup:` currently accepts `timer`, `backoff`, `sourceEvent`, or `providerTrigger`,
- argument and option schemas stay within the current closed proof surface,
- custom providers do **not** inherit built-in option semantics unless the custom contract declares them.

That means there is no fixed compiler-owned catalog for custom provider keys. Each custom `@source some.name ...` site is checked against the matching in-program `provider some.name` contract.
