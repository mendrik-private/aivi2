# Markup

AIVI markup is a tree of GTK widget declarations. It looks like JSX, but compiles to
native GTK4/libadwaita widgets — no web rendering, no virtual DOM, no Electron.

## Basic tags

```text
-- render a Window titled "Hello AIVI"
-- containing a vertical Box with 8px spacing
-- with a Label showing "Welcome!" and a Button labeled "Click me"
```

Tags are PascalCase GTK widget names. Attributes set widget properties.
Self-closing tags (`<Label ... />`) have no children.

The outer `<Window>` is the root widget. Every AIVI application has exactly one `export main`
with a `<Window>` at the top.

## String interpolation

Use `{expression}` inside a double-quoted string to embed a value:

```text
-- declare 'score' as 42
-- declare 'msg' interpolating score into a text message
-- render a Window with a title that includes the score value
-- containing a Label whose text interpolates the score value
```

The interpolation works in both `val` strings and markup attribute strings.

## Binding signals to attributes

When an attribute value is wrapped in `{...}` with a signal, the widget re-renders automatically
when the signal changes:

```text
-- declare a signal 'count' starting at 0
-- derive 'labelText' from count, formatted as "Clicked N times"
-- render a Window titled "Counter" with a Label whose text is bound to labelText
-- the Label updates automatically whenever count changes
```

The `<Label>` text updates every time `labelText` changes — which happens whenever `count`
changes. There is no explicit update call.

## The each tag

`<each>` renders a list of items. It requires a `key` attribute to help the runtime identify
stable items across updates:

```text
-- declare a product type 'User' with integer id and text name
-- declare a signal 'users' holding a list of Users
-- render a Window titled "Users" with a vertical Box
-- iterate over the users list, keying each item by user id
-- render a Label showing each user's name
```

- `of={users}` — the list signal to iterate.
- `as={user}` — the name bound to each item inside the block.
- `key={user.id}` — a stable unique identifier for each item.

The `key` attribute is required. It allows the runtime to reuse widgets for unchanged items
rather than rebuilding the whole list.

## Nested each

```text
-- declare a signal 'boardRows' holding a list of rows
-- render a vertical Box for the board
-- iterate over each row, keyed by row id
-- for each row render a horizontal Box
-- iterate over each cell in the row, keyed by cell id
-- render a Label showing the cell's glyph
```

Each row is a horizontal `<Box>`, and each cell inside it is a `<Label>`.
This is the exact structure in the Snake demo.

## The match tag

`<match>` and `<case>` are markup-level pattern matching. They render different widget trees
based on a value:

```text
-- declare a signal 'status' of type Status
-- render different widget trees based on the value of status
-- when Running, show a Label "Game is running"
-- when GameOver, show a Label "Game over!"
```

Like `\|\|>`, `<match>` is exhaustive — all variants must be covered.

## The show tag

`<show>` renders its children only when a condition is true:

```text
-- declare a signal 'isLoggedIn' of type Bool
-- render a "Log out" Button only when isLoggedIn is True
-- when isLoggedIn is False, the button is absent from the widget tree
```

When `isLoggedIn` is `False`, the `<Button>` is removed from the widget tree.

## Orientation and spacing

GTK `Box` is the main layout widget. `orientation` takes `Vertical` or `Horizontal`
(both are AIVI values of type `Orientation`). `spacing` is an `Int` in pixels.

```text
-- render a vertical Box with 12px spacing
-- containing a Label "First" and a Label "Second"
```

## Attribute expressions

Attribute values can be any AIVI expression:

```text
-- declare cellSize as 32
-- render a horizontal Box with spacing equal to cellSize
-- containing a Label showing the board width
```

## Summary

- Tags are GTK widget names in PascalCase.
- `{signal}` binds an attribute to a live signal.
- String interpolation: `"Hello {name}"`.
- `<each of={list} as={item} key={item.id}>` iterates a list signal.
- `<match on={signal}>` with `<case pattern=...>` arms for conditional rendering.
- `<show when={boolSignal}>` for presence/absence toggling.

[Next: Type Classes →](/tour/08-typeclasses)
