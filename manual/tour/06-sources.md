# Sources

Signals describe *how* a value is computed. Sources describe *where* values come from.

A source is an external event stream — a keyboard, a timer, a network response, a file watcher.
Sources are attached to signals using the `@source` decorator.

## The @source decorator

```aivi
type Key = Key Text

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
sig keyDown : Signal Key
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

```aivi
@source timer.every 160 with {
    immediate: True,
    coalesce: True
}
sig tick : Signal Unit
```

- `timer.every 160` fires every 160 milliseconds.
- `immediate: True` fires once immediately on activation (useful for initial render).
- `coalesce: True` drops ticks that accumulate while the handler is busy.

The snake game uses this to drive the game loop:

```aivi
type Status = Running | GameOver

type Pixel = Pixel Int Int

type Direction =
  | Up
  | Down
  | Left
  | Right

type BoardSize = {
    width: Int,
    height: Int
}

type Game = {
    snake: List Pixel,
    food: Pixel,
    score: Int,
    status: Status,
    seed: Int
}

val boardSize:BoardSize = {
    width: 12,
    height: 10
}

val initialGame:Game = {
    snake: [
        Pixel 6 5,
        Pixel 5 5,
        Pixel 4 5
    ],
    food: Pixel 10 1,
    score: 0,
    status: Running,
    seed: 2463534242
}

val direction:Direction = Right

fun stepGame:Game #size:BoardSize #direction:Direction #game:Game =>
    game

@source timer.every 160 with {
    immediate: True,
    coalesce: True
}
sig game : Signal Game =
    initialGame
     @|> stepGame boardSize direction
     <|@ stepGame boardSize direction
```

Every 160 ms, `stepGame` runs and the `game` signal updates, which cascades to `board`,
`boardRows`, `scoreLine`, and everything else derived from `game`.

## HTTP source

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

@source http.get "/api/user/1"
sig userData : Signal (Result HttpError User)
```

The signal starts empty (`None` or a loading state depending on the source type).
When the HTTP response arrives, the signal fires with `Ok user` or `Err message`.

## Button click source

```aivi
sig submitClicked : Signal Unit
```

This is an input signal — it has no body and is driven externally. In markup, connect it via
`onClick={submitClicked}` on a `<Button>` element.

This is a direct widget binding. Unlike the provider-based `@source button.clicked "id"` pattern
used for recurrent signals, `onClick={submitClicked}` does not need a separate `id` or `@source`
declaration:

```aivi
sig submitClicked : Signal Unit

val main =
    <Window title="Form">
        <Button label="Submit" onClick={submitClicked} />
    </Window>

export main
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
