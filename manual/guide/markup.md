# Markup & UI

AIVI uses a JSX-like markup syntax for building GTK4 user interfaces. Widget trees are written as first-class expressions and bound directly to signals.

## Basic Widgets

A widget is written like an XML element with a capital letter. Attributes are passed as `name={value}`:

```aivi
value main =
    <Window title="AIVI App">
        <Box orientation={Vertical} spacing={8}>
            <Label text="Hello, world!" />
            <Button label="Click me" />
        </Box>
    </Window>
```

Self-closing tags use `/>`. Tags with children use a closing tag.

## Binding Signals

Bind a signal directly to an attribute using `{signalName}`. The widget automatically updates when the signal changes:

```aivi
signal count: Signal Int = ...
signal message: Signal Text = "You have pressed {count} times"

value main =
    <Window title="Counter">
        <Box orientation={Vertical} spacing={8}>
            <Label text={message} />
        </Box>
    </Window>
```

Whenever `count` changes, `message` recomputes, and the label updates automatically. There is no manual DOM diffing or re-render call.

## Orientations and Spacing

`Box` lays out its children in a row or column:

```aivi
value layout =
    <Box orientation={Horizontal} spacing={4}>
        <Label text="Left" />
        <Label text="Right" />
    </Box>
```

Common values for `orientation`: `Vertical`, `Horizontal`.

## Conditional Rendering with `<show>`

Show or hide a widget based on a boolean signal:

```aivi
signal hasError: Signal Bool = ...
signal errorText: Signal Text = ...

value errorView =
    <show when={hasError}>
        <Label text={errorText} />
    </show>
```

When `hasError` is `False`, the inner content is not rendered.

## Pattern Matching with `<match>` and `<case>`

Render different widgets depending on the value of a signal:

```aivi
type Screen =
  | Loading
  | Ready (List Item)
  | Failed Text

signal screen: Signal Screen = ...

value screenView =
    <match on={screen}>
        <case pattern={Loading}>
            <Label text="Loading..." />
        </case>
        <case pattern={Ready items}>
            <each of={items} as={item} key={item.id}>
                <Row title={item.title} />
            </each>
        </case>
        <case pattern={Failed reason}>
            <Label text="Error: {reason}" />
        </case>
    </match>
```

- `<match on={signal}>` selects on the signal's current value
- `<case pattern={...}>` matches a constructor pattern
- Variables bound in the pattern are in scope inside the case body

## Lists with `<each>`

Render a widget for each item in a list signal:

```aivi
type Item = {
    id: Int,
    title: Text
}

signal items: Signal (List Item) = ...

value listView =
    <each of={items} as={item} key={item.id}>
        <Label text={item.title} />
    </each>
```

- `of` — the list signal to iterate over
- `as` — the name to bind each element to
- `key` — a stable unique identifier for reconciliation (required)

### Empty State

Use `<empty>` inside `<each>` to render something when the list is empty:

```aivi
value listView =
    <each of={items} as={item} key={item.id}>
        <Row title={item.title} />
        <empty>
            <Label text="No items yet" />
        </empty>
    </each>
```

## Fragments

Use `<fragment>` to group multiple widgets without a wrapping container:

```aivi
value headerGroup =
    <fragment>
        <Label text="Title" />
        <Label text="Subtitle" />
    </fragment>
```

## The `with` Pattern

`<with let={binding}>` binds a value or signal for use inside the block:

```aivi
value profileCard =
    <with let={currentUser}>
        <Box orientation={Vertical} spacing={4}>
            <Label text={currentUser.name} />
            <Label text={currentUser.email} />
        </Box>
    </with>
```

## Monospace Text

Labels can be made monospace for displaying code or fixed-width content:

```aivi
signal boardText: Signal Text = ...

value boardView =
    <Label text={boardText} monospace />
```

Boolean attributes like `monospace` can be written without a value.

## A Complete Example

The following renders a game board with score and status:

```aivi
signal boardText: Signal Text  = ...
signal scoreLine: Signal Text  = ...
signal statusLine: Signal Text = ...
signal dirLine: Signal Text    = ...

value main =
    <Window title="AIVI Snake">
        <Box orientation={Vertical} spacing={8}>
            <Label text={dirLine} />
            <Label text={scoreLine} />
            <Label text={statusLine} />
            <Label text={boardText} monospace />
        </Box>
    </Window>

export main
```

All four labels update independently as their respective signals change. The runtime only redraws the widgets whose signal values actually changed.

## Summary

| Element | Purpose |
|---|---|
| `<Window>` | Top-level application window |
| `<Box>` | Linear layout container |
| `<Label>` | Text display |
| `<Button>` | Clickable button |
| `<show when={bool}>` | Conditional rendering |
| `<match on={signal}>` | Pattern-based rendering |
| `<case pattern={P}>` | One arm of a `<match>` |
| `<each of={list} as={x} key={x.id}>` | List rendering |
| `<empty>` | Fallback inside `<each>` |
| `<fragment>` | Group without container |
| `<with let={binding}>` | Local binding |
