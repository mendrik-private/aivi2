# Sources

Signals describe *how* a value is computed. Sources describe *where* values come from.

A source is an external event stream — a keyboard, a timer, a network response, a file watcher.
Sources are attached to signals using the `@source` decorator.

## The @source decorator

```text
-- declare a signal 'keyDown' driven by keyboard key-down events from the window
-- configured to ignore key-repeat events and only fire when the window has focus
```

`@source` names the source (`window.keyDown`) and passes a configuration record.
The signal `keyDown` will fire whenever the user presses a key, emitting a `Key` value.

The source is declared *above* the `sig` it decorates — the decorator binds to the next
`sig` declaration.

## Source lifecycle

AIVI manages the source lifecycle automatically:

1. When the UI component that owns the signal is mounted, the source is activated.
2. While active, each event from the source drives the signal's recurrence.
3. When the component is unmounted, the source is torn down.

You never unsubscribe manually. The runtime handles it.

## Timer source

The `timer.every` source fires at a fixed interval:

```text
-- declare a signal 'tick' driven by a timer firing every 160 milliseconds
-- fires once immediately on activation
-- drops queued ticks if the handler is busy (coalesce)
```

- `timer.every 160` fires every 160 milliseconds.
- `immediate: True` fires once immediately on activation (useful for initial render).
- `coalesce: True` drops ticks that accumulate while the handler is busy.

The snake game uses this to drive the game loop:

```text
-- bind 'game' to a timer firing every 160 ms, with an immediate first tick and coalescing
-- game starts at the initial game state
-- on each timer tick, apply stepGame with boardSize and direction to advance the game state
```

Every 160 ms, `stepGame` runs and the `game` signal updates, which cascades to `board`,
`boardRows`, `scoreLine`, and everything else derived from `game`.

## HTTP source

```text
-- declare a signal 'userData' driven by an HTTP GET request to the given URL
-- the signal carries either a successfully parsed User or an HttpError
```

The signal starts empty (`None` or a loading state depending on the source type).
When the HTTP response arrives, the signal fires with `Ok user` or `Err message`.

## Button click source

```text
-- declare a signal 'submitClicked' that fires when the button with id "submit" is clicked
```

The source name `"submit"` corresponds to the `id` attribute on a `<Button>` in markup:

```text
-- render a Window titled "Form" containing a Button labeled "Submit" with id "submit"
```

## Source configuration

Sources accept configuration via the `with { ... }` block.
Each source type documents its own options.

| Source | Common options |
|---|---|
| `timer.every N` | `immediate`, `coalesce` |
| `window.keyDown` | `repeat`, `focusOnly` |
| `button.clicked "id"` | — |
| `http.get "url"` | `headers`, `refreshOn` |
| `http.post "url"` | `body`, `headers`, `refreshOn` |

## How sources feed signals

The full picture:

1. `@source` declares the external event stream.
2. The `sig` declaration with `@\|>...<\|@` says how each event updates the signal.
3. Derived signals (using `\|>`) update automatically whenever their dependency changes.
4. Markup binds to signals with `{signalName}` attributes.
5. GTK widgets re-render when the signals they are bound to change.

Everything between step 1 (external event) and step 5 (widget update) is managed by the AIVI
runtime. User code is a pure description of transformations.

## Stale suppression

If a source fires faster than the signal can process, `coalesce: True` drops the intermediate
events and only delivers the latest. This is important for timers at high frequency — without
coalescing, a slow step function could cause event queue buildup.

## Summary

- `@source provider.method config` decorates the next `sig` declaration.
- Sources are activated when the owning component mounts and deactivated on unmount.
- `timer.every N` drives periodic updates.
- `window.keyDown`, `button.clicked` respond to user interaction.
- `http.get` fetches data asynchronously.
- `coalesce: True` prevents event queue buildup.

[Next: Markup →](/tour/07-markup)
