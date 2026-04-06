# Markup & UI

AIVI markup is an expression language for UI trees. It looks XML-like, but it is part of the language surface and participates in ordinary AIVI binding rules.

## Basic widgets

```aivi
value view =
    <Window title="Greeting">
        <Box>
            <Label text="Hello" />
            <Label text="World" />
        </Box>
    </Window>
```

Capitalised tags name widgets or components. Attributes can be plain text or embedded expressions.

## Binding expressions into attributes

```aivi
value header = "Users"

value view =
    <Window title="Directory">
        <Label text={header} />
    </Window>
```

Anything inside `{...}` is an AIVI expression.

## Typed widget properties

Widget attributes are schema-checked host properties, not an open CSS map. That means you bind ordinary expressions into the properties a widget exposes today.

```aivi
type Bool -> Float
func previewOpacity = .
 T|> 1.0
 F|> 0.25

value previewVisible = True

value view =
    <Window title="Preview">
        <Button label="Blue" animateOpacity={True} opacity={previewOpacity previewVisible} />
    </Window>
```

`Button.opacity` takes a `Float` and the GTK host clamps it into the `0.0` to `1.0` range. `Button.animateOpacity` takes a `Bool` and enables the built-in opacity transition class, so changes to `opacity` fade instead of snapping.

Because these are ordinary attributes, they can be driven by any signal-backed or source-backed expression. Arbitrary `cssProps` maps are not part of the current AIVI surface yet.

## Conditional rendering with `<show>`

```aivi
value isVisible = True

value view =
    <Window title="Status">
        <Box>
            <show when={isVisible} keepMounted={True}>
                <Label text="Visible" />
            </show>
        </Box>
    </Window>
```

`<show>` renders its body only when the condition holds.

## Local bindings with `<with>`

```aivi
type Item = {
    id: Int,
    title: Text
}

type Screen = Ready (List Item)

value screen =
    Ready [
        { id: 1, title: "Alpha" },
        { id: 2, title: "Beta" }
    ]

value view =
    <Window title="Items">
        <Box>
            <with value={screen} as={currentScreen}>
                <match on={currentScreen}>
                    <case pattern={Ready items}>
                        <each of={items} as={item} key={item.id}>
                            <Label text={item.title} />
                        </each>
                    </case>
                </match>
            </with>
        </Box>
    </Window>
```

`<with>` binds a value for the nested subtree.

## Pattern matching with `<match>`

Markup has control nodes for the same pattern-oriented style used elsewhere in the language:

```aivi
type Item = {
    id: Int,
    title: Text
}

type Screen =
  | Loading
  | Ready (List Item)
  | Failed Text

value screen =
    Ready [
        { id: 1, title: "Alpha" },
        { id: 2, title: "Beta" }
    ]

value view =
    <Window title="Items">
        <Box>
            <match on={screen}>
                <case pattern={Loading}>
                    <Label text="Loading..." />
                </case>
                <case pattern={Ready items}>
                    <each of={items} as={item} key={item.id}>
                        <Label text={item.title} />
                        <empty>
                            <Label text="No items" />
                        </empty>
                    </each>
                </case>
                <case pattern={Failed reason}>
                    <Label text={reason} />
                </case>
            </match>
        </Box>
    </Window>
```

## Iteration with `<each>`

`<each>` renders one subtree per list item and can include an `<empty>` fallback when the list is empty.

## Summary

| Element | Meaning |
| --- | --- |
| `<Label ... />` | A widget node |
| `<fragment>...</fragment>` | Group children without an outer widget |
| `<show when={...}>` | Conditional rendering |
| `<with value={...} as={...}>` | Bind a value in markup |
| `<match on={...}>` | Pattern-based rendering |
| `<case pattern={...}>` | One match branch |
| `<each of={...} as={...} key={...}>` | Iterate a list |
| `<empty>` | Empty-list fallback inside `<each>` |

---

**See also:** [Building Snake](building-snake.md) — a complete application that uses every markup feature together

---

## Widget catalog

### `Window`

Top-level application window.

```aivi
// <unparseable item>
```

**Properties:** `title`, `defaultWidth`, `defaultHeight`, `resizable`, `modal`, `visible`, `sensitive`, `opacity`, `hexpand`, `vexpand`, `halign`, `valign`, `widthRequest`, `heightRequest`, `marginStart`, `marginEnd`, `marginTop`, `marginBottom`, `tooltip`, `cssClasses`, `animateOpacity`

### `HeaderBar`

Title bar with optional start/end widget slots.

```aivi
// <unparseable item>
```

### `Box`

Linear layout container.

```aivi
// <unparseable item>
```

**Properties:** `orientation` (`Horizontal`|`Vertical`), `spacing`, `homogeneous`

### `ScrolledWindow`

Adds scrollbars to an inner widget.

```aivi
// <unparseable item>
```

### `Paned`

Two-pane resizable splitter.

```aivi
// <unparseable item>
```

### `ToolbarView`

Adwaita toolbar container with top/bottom bars and content.

```aivi
// <unparseable item>
```

### `Label`

Text display widget.

```aivi
// <unparseable item>
```

### `Button`

Clickable button.

```aivi
// <unparseable item>
```

### `Entry`

Single-line text input.

```aivi
// <unparseable item>
```

### `Switch`

Toggle switch.

```aivi
// <unparseable item>
```

### `CheckButton`

Checkbox with label.

```aivi
// <unparseable item>
```

### `ToggleButton`

Button that stays pressed.

```aivi
// <unparseable item>
```

### `SpinButton`

Numeric spinner input.

```aivi
// <unparseable item>
```

### `Scale`

Slider for numeric values.

```aivi
// <unparseable item>
```

### `Image`

Image display.

```aivi
// <unparseable item>
```

### `Spinner`

Activity indicator.

```aivi
// <unparseable item>
```

### `ProgressBar`

Progress display.

```aivi
// <unparseable item>
```

### `Revealer`

Animated visibility toggle.

```aivi
// <unparseable item>
```

### `Separator`

Visual divider line.

```aivi
// <unparseable item>
```

### `StatusPage`

Placeholder page with icon and description.

```aivi
// <unparseable item>
```

### `Clamp`

Width-constraining container.

```aivi
// <unparseable item>
```

### `Banner`

Informational banner strip.

```aivi
// <unparseable item>
```

### `Frame`

Container with an optional label border.

```aivi
// <unparseable item>
```

### `Viewport`

Low-level scrollable viewport.

```aivi
// <unparseable item>
```

---

### Adwaita preference rows

These widgets are designed for settings/preferences UIs. They extend `gtk::ListBoxRow` and should be placed inside a `ListBox`.

#### `ActionRow`

A row with title, optional subtitle, and suffix widgets.

```aivi
// <unparseable item>
```

**Properties:** `title`, `subtitle`, `activatable`  
**Events:** `onActivated` (Unit)  
**Children:** `suffix` (sequence)

#### `ExpanderRow`

An expandable row that reveals child rows.

```aivi
// <unparseable item>
```

**Properties:** `title`, `subtitle`, `expanded`  
**Children:** `rows` (sequence)

#### `SwitchRow`

A preference row with an embedded switch.

```aivi
// <unparseable item>
```

**Properties:** `title`, `subtitle`, `active`  
**Events:** `onToggled` (Bool — new active state)

#### `SpinRow`

A preference row with an embedded spin button.

```aivi
// <unparseable item>
```

**Properties:** `title`, `subtitle`, `value`, `min`, `max`, `step`  
**Events:** `onValueChanged` (Float)

#### `EntryRow`

A preference row with an embedded text entry.

```aivi
// <unparseable item>
```

**Properties:** `title`, `text`  
**Events:** `onChange` (Text), `onActivated` (Unit)

---

### List and selection

#### `ListBox`

A vertical list container for rows.

```aivi
// <unparseable item>
```

**Properties:** `selectionMode` (`None`|`Single`|`Browse`|`Multiple`)  
**Events:** `onRowActivated` (Int — zero-based row index)  
**Children:** `children` (sequence)

#### `ListBoxRow`

A single row in a `ListBox`.

```aivi
// <unparseable item>
```

**Properties:** `activatable`  
**Events:** `onActivated` (Unit)  
**Children:** `child` (single)

#### `DropDown`

A dropdown selector from a comma-separated list of strings.

```aivi
// <unparseable item>
```

**Properties:** `items` (comma-separated text), `selected` (Int)  
**Events:** `onSelectionChanged` (Int — selected index)

---

### Utility

#### `SearchEntry`

A text entry styled for search input with debounced `onSearchChanged`.

```aivi
// <unparseable item>
```

**Properties:** `text`, `placeholder`  
**Events:** `onChange` (Text), `onActivated` (Unit), `onSearchChanged` (Text — debounced)

#### `Expander`

A collapsible container with a label toggle.

```aivi
// <unparseable item>
```

**Properties:** `label`, `expanded`  
**Children:** `child` (single)

---

### Navigation and overlay

#### `NavigationView`

Adwaita push-based navigation stack. Children must be `NavigationPage` widgets.

```aivi
// <unparseable item>
```

**Children:** `pages` (sequence of `NavigationPage`)

#### `NavigationPage`

A page within a `NavigationView`.

```aivi
// <unparseable item>
```

**Properties:** `title`, `tag`  
**Children:** `content` (single)

#### `ToastOverlay`

An overlay that can display transient toast notifications (toasts are shown at runtime via the signal engine, not via markup children).

```aivi
// <unparseable item>
```

**Children:** `content` (single)
