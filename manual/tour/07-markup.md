# Markup

AIVI markup is a tree of GTK widget declarations. It looks like JSX, but compiles to
native GTK4/libadwaita widgets — no web rendering, no virtual DOM, no Electron.

## Basic tags

```aivi
type Orientation =
  | Vertical
  | Horizontal

val main =
    <Window title="Hello AIVI">
        <Box orientation={Vertical} spacing={8}>
            <Label text="Welcome!" />
            <Button label="Click me" />
        </Box>
    </Window>

export main
```

Tags are PascalCase GTK widget names. Attributes set widget properties.
The current executable widget catalog ships `Window`, `Box`, `Label`, and `Button`: `Window`
accepts one child, `Box` accepts a list of children, and `Label`/`Button` are leaf widgets.
Self-closing tags (`<Label ... />`) have no children.

The outer `<Window>` is the root widget. Every AIVI application has exactly one `export main`
with a `<Window>` at the top.

## String interpolation

Use `{expression}` inside a double-quoted string to embed a value:

```aivi
val score:Int = 42
val msg:Text = "Current score: {score}"

type Orientation =
  | Vertical
  | Horizontal

val main =
    <Window title="Score: {score}">
        <Label text={msg} />
    </Window>

export main
```

The interpolation works in both `val` strings and markup attribute strings.

## Binding signals to attributes

When an attribute value is wrapped in `{...}` with a signal, the widget re-renders automatically
when the signal changes:

```aivi
type Orientation =
  | Vertical
  | Horizontal

provider button.clicked
    wakeup: sourceEvent
    argument id: Text

fun addOne:Int #n:Int =>
    n + 1

@source button.clicked "inc"
sig count : Signal Int =
    0
     @|> addOne
     <|@ addOne

sig labelText : Signal Text = "Clicked {count} times"

val main =
    <Window title="Counter">
        <Label text={labelText} />
    </Window>

export main
```

The `<Label>` text updates every time `labelText` changes — which happens whenever `count`
changes. There is no explicit update call.

## The each tag

`<each>` renders a list of items. It requires a `key` attribute to help the runtime identify
stable items across updates:

```aivi
type User = {
    id: Int,
    name: Text
}

type Orientation =
  | Vertical
  | Horizontal

sig users : Signal (List User) = []

val main =
    <Window title="Users">
        <Box orientation={Vertical} spacing={4}>
            <each of={users} as={user} key={user.id}>
                <Label text={user.name} />
            </each>
        </Box>
    </Window>

export main
```

- `of={users}` — the list signal to iterate.
- `as={user}` — the name bound to each item inside the block.
- `key={user.id}` — a stable unique identifier for each item.

The `key` attribute is required. It allows the runtime to reuse widgets for unchanged items
rather than rebuilding the whole list.

## Nested each

```aivi
type CellKind =
  | SnakeHead
  | SnakeBody
  | Food
  | Empty

type BoardCell = {
    id: Int,
    kind: CellKind
}

type BoardRow = {
    id: Int,
    cells: List BoardCell
}

type Orientation =
  | Vertical
  | Horizontal

fun cellGlyph:Text #kind:CellKind =>
    kind
     ||> SnakeHead => "@"
     ||> SnakeBody => "o"
     ||> Food      => "*"
     ||> Empty     => "."

sig boardRows : Signal (List BoardRow) = []

val main =
    <Window title="Board">
        <Box orientation={Vertical} spacing={2}>
            <each of={boardRows} as={row} key={row.id}>
                <Box orientation={Horizontal} spacing={2}>
                    <each of={row.cells} as={cell} key={cell.id}>
                        <Label text={cellGlyph cell.kind} />
                    </each>
                </Box>
            </each>
        </Box>
    </Window>

export main
```

Each row is a horizontal `<Box>`, and each cell inside it is a `<Label>`.
This is the exact structure in the Snake demo.

## The match tag

`<match>` and `<case>` are markup-level pattern matching. They render different widget trees
based on a value:

```aivi
type Status = Running | GameOver

type Orientation =
  | Vertical
  | Horizontal

sig status : Signal Status = Running

val main =
    <Window title="Status">
        <match on={status}>
            <case pattern={Running}>
                <Label text="Game is running" />
            </case>
            <case pattern={GameOver}>
                <Label text="Game over!" />
            </case>
        </match>
    </Window>

export main
```

Like `\|\|>`, `<match>` is exhaustive — all variants must be covered.

## The show tag

`<show>` renders its children only when a condition is true:

```aivi
type Orientation =
  | Vertical
  | Horizontal

sig isLoggedIn : Signal Bool = False

val main =
    <Window title="App">
        <show when={isLoggedIn}>
            <Button label="Log out" />
        </show>
    </Window>

export main
```

When `isLoggedIn` is `False`, the `<Button>` is removed from the widget tree.

## Orientation and spacing

GTK `Box` is the main layout widget. `orientation` takes `Vertical` or `Horizontal`
(both are AIVI values of type `Orientation`). `spacing` is an `Int` in pixels.

```aivi
type Orientation =
  | Vertical
  | Horizontal

val layout =
    <Box orientation={Vertical} spacing={12}>
        <Label text="First" />
        <Label text="Second" />
    </Box>
```

## Attribute expressions

Attribute values can be any AIVI expression:

```aivi
type Orientation =
  | Vertical
  | Horizontal

val cellSize:Int = 32

type Game = { width: Int }

val game:Game = { width: 12 }

val view =
    <Box orientation={Horizontal} spacing={cellSize}>
        <Label text="{game.width}" />
    </Box>
```

## Summary

- Tags are GTK widget names in PascalCase.
- `{signal}` binds an attribute to a live signal.
- String interpolation: `"Hello {name}"`.
- `<each of={list} as={item} key={item.id}>` iterates a list signal.
- `<match on={signal}>` with `<case pattern=...>` arms for conditional rendering.
- `<show when={boolSignal}>` for presence/absence toggling.

[Next: Type Classes →](/tour/08-typeclasses)
