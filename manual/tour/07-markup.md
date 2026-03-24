# Markup

AIVI markup is a tree of GTK widget declarations. It looks like JSX, but compiles to
native GTK4/libadwaita widgets — no web rendering, no virtual DOM, no Electron.

## Basic tags

```text
// TODO: add a verified AIVI example here
```

Tags are PascalCase GTK widget names. Attributes set widget properties.
The current executable widget catalog ships `Window`, `Box`, `Label`, and `Button`: `Window`
accepts one child, `Box` accepts a list of children, and `Label`/`Button` are leaf widgets.
Self-closing tags (`<Label ... />`) have no children.

The outer `<Window>` is the root widget. Every AIVI application has exactly one `export main`
with a `<Window>` at the top.

## String interpolation

Use `{expression}` inside a double-quoted string to embed a value:

```text
// TODO: add a verified AIVI example here
```

The interpolation works in both `val` strings and markup attribute strings.

## Binding signals to attributes

When an attribute value is wrapped in `{...}` with a signal, the widget re-renders automatically
when the signal changes:

```text
// TODO: add a verified AIVI example here
```

The `<Label>` text updates every time `labelText` changes — which happens whenever `count`
changes. There is no explicit update call.

## The each tag

`<each>` renders a list of items. It requires a `key` attribute to help the runtime identify
stable items across updates:

```text
// TODO: add a verified AIVI example here
```

- `of={users}` — the list signal to iterate.
- `as={user}` — the name bound to each item inside the block.
- `key={user.id}` — a stable unique identifier for each item.

The `key` attribute is required. It allows the runtime to reuse widgets for unchanged items
rather than rebuilding the whole list.

## Nested each

```text
// TODO: add a verified AIVI example here
```

Each row is a horizontal `<Box>`, and each cell inside it is a `<Label>`.
This is the exact structure in the Snake demo.

## The match tag

`<match>` and `<case>` are markup-level pattern matching. They render different widget trees
based on a value:

```text
// TODO: add a verified AIVI example here
```

Like `||>`, `<match>` is exhaustive — all variants must be covered.

## The show tag

`<show>` renders its children only when a condition is true:

```text
// TODO: add a verified AIVI example here
```

When `isLoggedIn` is `False`, the `<Button>` is removed from the widget tree.

## Orientation and spacing

GTK `Box` is the main layout widget. `orientation` takes `Vertical` or `Horizontal`
(both are AIVI values of type `Orientation`). `spacing` is an `Int` in pixels.

```text
// TODO: add a verified AIVI example here
```

## Attribute expressions

Attribute values can be any AIVI expression:

```text
// TODO: add a verified AIVI example here
```

## Summary

- Tags are GTK widget names in PascalCase.
- `{signal}` binds an attribute to a live signal.
- String interpolation: `"Hello {name}"`.
- `<each of={list} as={item} key={item.id}>` iterates a list signal.
- `<match on={signal}>` with `<case pattern=...>` arms for conditional rendering.
- `<show when={boolSignal}>` for presence/absence toggling.

[Next: Type Classes →](/tour/08-typeclasses)
