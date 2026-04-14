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
func previewOpacity = arg1 => arg1
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

`when` accepts `Bool`, and it also accepts the canonical truthy/falsy carriers `Option`, `Result`,
and `Validation` (plus one outer `Signal`). That means request helpers such as
`usersResult.error` or `usersResult.success` can plug straight into markup:

```aivi
value fetchError = Some "offline"

value view =
    <Window title="Status">
        <show when={fetchError}>
            <Label text="Failed" />
        </show>
    </Window>
```

`keepMounted` stays a plain `Bool`.

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
value view =
    <Window title="App" defaultWidth={800} defaultHeight={600}>
        <Window.titlebar>
            <HeaderBar />
        </Window.titlebar>
        <Label text="Hello, world!" />
    </Window>
```

**Properties:** `title`, `defaultWidth`, `defaultHeight`, `resizable`, `modal`, `visible`, `sensitive`, `opacity`, `hexpand`, `vexpand`, `halign`, `valign`, `widthRequest`, `heightRequest`, `marginStart`, `marginEnd`, `marginTop`, `marginBottom`, `tooltip`, `cssClasses`, `animateOpacity`

### `HeaderBar`

Title bar with optional start/end widget slots.

```aivi
value view =
    <Window title="App">
        <Window.titlebar>
            <HeaderBar>
                <HeaderBar.start>
                    <Button label="Back" />
                </HeaderBar.start>
                <HeaderBar.end>
                    <Button label="Menu" />
                </HeaderBar.end>
            </HeaderBar>
        </Window.titlebar>
        <Label text="Content" />
    </Window>
```

### `Box`

Linear layout container.

```aivi
value view =
    <Window title="App">
        <Box orientation="Vertical" spacing={8}>
            <Label text="First" />
            <Label text="Second" />
        </Box>
    </Window>
```

**Properties:** `orientation` (`Horizontal`|`Vertical`), `spacing`, `homogeneous`

### `ScrolledWindow`

Adds scrollbars to an inner widget.

```aivi
value view =
    <Window title="App">
        <ScrolledWindow hexpand={True} vexpand={True}>
            <Box orientation="Vertical" spacing={4}>
                <Label text="Row 1" />
                <Label text="Row 2" />
            </Box>
        </ScrolledWindow>
    </Window>
```

### `Paned`

Two-pane resizable splitter.

```aivi
value view =
    <Window title="App">
        <Paned orientation="Horizontal" hexpand={True} vexpand={True}>
            <Paned.start>
                <Label text="Left panel" />
            </Paned.start>
            <Paned.end>
                <Label text="Right panel" />
            </Paned.end>
        </Paned>
    </Window>
```

### `ToolbarView`

Adwaita toolbar container with top/bottom bars and content.

```aivi
value view =
    <Window title="App">
        <ToolbarView>
            <ToolbarView.top>
                <HeaderBar />
            </ToolbarView.top>
            <Label text="Content" vexpand={True} />
        </ToolbarView>
    </Window>
```

### `Label`

Text display widget.

```aivi
value view =
    <Window title="App">
        <Label text="Hello, world!" wrap={True} halign="Start" />
    </Window>
```

### `Button`

Clickable button.

```aivi
value view =
    <Window title="App">
        <Button label="Click me" halign="Center" />
    </Window>
```

### `Entry`

Single-line text input.

```aivi
value view =
    <Window title="App">
        <Entry placeholder="Type something..." hexpand={True} />
    </Window>
```

### `Switch`

Toggle switch.

```aivi
value view =
    <Window title="App">
        <Switch active={True} halign="Center" valign="Center" />
    </Window>
```

### `CheckButton`

Checkbox with label.

```aivi
value view =
    <Window title="App">
        <CheckButton label="Enable feature" active={False} />
    </Window>
```

### `ToggleButton`

Button that stays pressed.

```aivi
value view =
    <Window title="App">
        <ToggleButton label="Bold" active={False} />
    </Window>
```

### `SpinButton`

Numeric spinner input.

```aivi
value view =
    <Window title="App">
        <SpinButton value={5.0} min={0.0} max={10.0} step={1.0} digits={0} />
    </Window>
```

### `Scale`

Slider for numeric values.

```aivi
value view =
    <Window title="App">
        <Scale value={50.0} min={0.0} max={100.0} step={1.0} drawValue={True} hexpand={True} />
    </Window>
```

### `Image`

Image display.

```aivi
value view =
    <Window title="App">
        <Image iconName="folder-symbolic" pixelSize={48} halign="Center" />
    </Window>
```

### `Spinner`

Activity indicator.

```aivi
value view =
    <Window title="App">
        <Spinner spinning={True} halign="Center" valign="Center" />
    </Window>
```

### `ProgressBar`

Progress display.

```aivi
value view =
    <Window title="App">
        <ProgressBar fraction={0.6} hexpand={True} />
    </Window>
```

### `Revealer`

Animated visibility toggle.

```aivi
value view =
    <Window title="App">
        <Revealer revealed={True} transitionType="slide-down">
            <Label text="Now you see me" />
        </Revealer>
    </Window>
```

### `Separator`

Visual divider line.

```aivi
value view =
    <Window title="App">
        <Box orientation="Vertical">
            <Label text="Above" />
            <Separator orientation="Horizontal" />
            <Label text="Below" />
        </Box>
    </Window>
```

### `StatusPage`

Placeholder page with icon and description.

```aivi
value view =
    <Window title="App">
        <StatusPage title="Nothing here yet" description="Add items to get started" iconName="folder-open-symbolic" />
    </Window>
```

### `Clamp`

Width-constraining container.

```aivi
value view =
    <Window title="App">
        <Clamp maximumSize={600} tighteningThreshold={400}>
            <Label text="Content constrained to 600 px" wrap={True} />
        </Clamp>
    </Window>
```

### `Banner`

Informational banner strip.

```aivi
value view =
    <Window title="App">
        <Box orientation="Vertical">
            <Banner title="You are offline" buttonLabel="Retry" revealed={True} />
            <Label text="Main content" vexpand={True} />
        </Box>
    </Window>
```

### `Frame`

Container with an optional label border.

```aivi
value view =
    <Window title="App">
        <Frame label="Details">
            <Label text="Inside the frame" marginTop={8} />
        </Frame>
    </Window>
```

### `Viewport`

Low-level scrollable viewport.

```aivi
value view =
    <Window title="App">
        <ScrolledWindow vexpand={True}>
            <Viewport>
                <Label text="Scrollable content" />
            </Viewport>
        </ScrolledWindow>
    </Window>
```

---

### Adwaita preference rows

These widgets are designed for settings/preferences UIs. They extend `gtk::ListBoxRow` and should be placed inside a `ListBox`.

#### `ActionRow`

A row with title, optional subtitle, and suffix widgets.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <ActionRow title="Notifications" subtitle="Allow alerts">
                <ActionRow.suffix>
                    <Switch active={True} valign="Center" />
                </ActionRow.suffix>
            </ActionRow>
        </ListBox>
    </Window>
```

**Properties:** `title`, `subtitle`, `activatable`  
**Events:** `onActivated` (Unit)  
**Children:** `suffix` (sequence)

#### `ExpanderRow`

An expandable row that reveals child rows.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <ExpanderRow title="Advanced" subtitle="More options" expanded={False}>
                <ExpanderRow.rows>
                    <ActionRow title="Option A" />
                    <ActionRow title="Option B" />
                </ExpanderRow.rows>
            </ExpanderRow>
        </ListBox>
    </Window>
```

**Properties:** `title`, `subtitle`, `expanded`  
**Children:** `rows` (sequence)

#### `SwitchRow`

A preference row with an embedded switch.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <SwitchRow title="Dark mode" subtitle="Use dark colour scheme" active={False} />
        </ListBox>
    </Window>
```

**Properties:** `title`, `subtitle`, `active`  
**Events:** `onToggled` (Bool — new active state)

#### `SpinRow`

A preference row with an embedded spin button.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <SpinRow title="Font size" value={12.0} min={8.0} max={32.0} step={1.0} />
        </ListBox>
    </Window>
```

**Properties:** `title`, `subtitle`, `value`, `min`, `max`, `step`  
**Events:** `onValueChanged` (Float)

#### `EntryRow`

A preference row with an embedded text entry.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <EntryRow title="Username" text="" />
        </ListBox>
    </Window>
```

**Properties:** `title`, `text`  
**Events:** `onChange` (Text), `onActivated` (Unit)

---

### List and selection

#### `ListBox`

A vertical list container for rows.

```aivi
value view =
    <Window title="App">
        <ListBox selectionMode="None">
            <ListBoxRow>
                <Label text="Row one" />
            </ListBoxRow>
            <ListBoxRow>
                <Label text="Row two" />
            </ListBoxRow>
        </ListBox>
    </Window>
```

**Properties:** `selectionMode` (`None`|`Single`|`Browse`|`Multiple`)  
**Events:** `onRowActivated` (Int — zero-based row index)  
**Children:** `children` (sequence)

#### `ListBoxRow`

A single row in a `ListBox`.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <ListBoxRow activatable={True}>
                <Label text="Clickable row" marginStart={12} marginEnd={12} />
            </ListBoxRow>
        </ListBox>
    </Window>
```

**Properties:** `activatable`  
**Events:** `onActivated` (Unit)  
**Children:** `child` (single)

#### `DropDown`

A dropdown selector from a comma-separated list of strings.

```aivi
value view =
    <Window title="App">
        <DropDown items="Red,Green,Blue" selected={0} halign="Start" />
    </Window>
```

**Properties:** `items` (comma-separated text), `selected` (Int)  
**Events:** `onSelectionChanged` (Int — selected index)

---

### Utility

#### `SearchEntry`

A text entry styled for search input with debounced `onSearchChanged`.

```aivi
value view =
    <Window title="App">
        <SearchEntry placeholder="Search..." hexpand={True} />
    </Window>
```

**Properties:** `text`, `placeholder`  
**Events:** `onChange` (Text), `onActivated` (Unit), `onSearchChanged` (Text — debounced)

#### `Expander`

A collapsible container with a label toggle.

```aivi
value view =
    <Window title="App">
        <Expander label="Details" expanded={False}>
            <Label text="Hidden until expanded" />
        </Expander>
    </Window>
```

**Properties:** `label`, `expanded`  
**Children:** `child` (single)

---

### Navigation and overlay

#### `NavigationView`

Adwaita push-based navigation stack. Children must be `NavigationPage` widgets.

```aivi
value view =
    <Window title="App">
        <NavigationView>
            <NavigationPage title="Home" tag="home">
                <Label text="Home page" halign="Center" valign="Center" vexpand={True} />
            </NavigationPage>
            <NavigationPage title="Details" tag="details">
                <Label text="Details page" halign="Center" valign="Center" vexpand={True} />
            </NavigationPage>
        </NavigationView>
    </Window>
```

**Children:** `pages` (sequence of `NavigationPage`)

#### `NavigationPage`

A page within a `NavigationView`.

```aivi
value view =
    <Window title="App">
        <NavigationView>
            <NavigationPage title="Profile" tag="profile">
                <ToolbarView>
                    <ToolbarView.top>
                        <HeaderBar />
                    </ToolbarView.top>
                    <Label text="Profile content" halign="Center" valign="Center" vexpand={True} />
                </ToolbarView>
            </NavigationPage>
        </NavigationView>
    </Window>
```

**Properties:** `title`, `tag`  
**Children:** `content` (single)

#### `ToastOverlay`

An overlay that can display transient toast notifications (toasts are shown at runtime via the signal engine, not via markup children).

```aivi
value view =
    <Window title="App">
        <ToastOverlay>
            <ToastOverlay.content>
                <Label text="Main content" halign="Center" valign="Center" vexpand={True} />
            </ToastOverlay.content>
        </ToastOverlay>
    </Window>
```

**Children:** `content` (single)

---

### Preferences and forms

#### `PreferencesWindow`

An Adwaita preferences window with a searchable page switcher.

```aivi
value view =
    <PreferencesWindow title="Settings" defaultWidth={600} defaultHeight={400} searchEnabled={True}>
        <PreferencesPage title="General" iconName="preferences-system-symbolic">
            <PreferencesGroup title="Appearance" description="Adjust how the app looks">
                <SwitchRow title="Dark mode" />
            </PreferencesGroup>
        </PreferencesPage>
    </PreferencesWindow>
```

**Properties:** `title`, `defaultWidth` (Int), `defaultHeight` (Int), `searchEnabled` (Bool)  
**Children:** `pages` (sequence of `PreferencesPage`)

#### `PreferencesPage`

A named page inside a `PreferencesWindow`.

```aivi
value view =
    <PreferencesWindow title="Settings">
        <PreferencesPage title="General" iconName="preferences-system-symbolic">
            <PreferencesGroup title="Behaviour">
                <SwitchRow title="Auto-save" active={True} />
            </PreferencesGroup>
        </PreferencesPage>
    </PreferencesWindow>
```

**Properties:** `title`, `iconName`  
**Children:** `children` (sequence of `PreferencesGroup`)

#### `PreferencesGroup`

A titled group of preference rows within a `PreferencesPage`.

```aivi
value view =
    <PreferencesWindow title="Settings">
        <PreferencesPage title="General" iconName="preferences-system-symbolic">
            <PreferencesGroup title="Privacy" description="Control data usage">
                <SwitchRow title="Analytics" active={False} />
                <SwitchRow title="Crash reports" active={True} />
            </PreferencesGroup>
        </PreferencesPage>
    </PreferencesWindow>
```

**Properties:** `title`, `description`  
**Children:** `children` (sequence — any preference rows or widgets)

#### `ComboRow`

A preference row with a drop-down selector.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <ComboRow title="Theme" items="Light,Dark,System" selected={2} />
        </ListBox>
    </Window>
```

**Properties:** `title`, `subtitle`, `items` (comma-separated), `selected` (Int)  
**Events:** `onSelectionChanged` (Int — zero-based index of selected item)

#### `PasswordEntryRow`

A preference row styled for password input with reveal toggle.

```aivi
value view =
    <Window title="App">
        <ListBox>
            <PasswordEntryRow title="Password" text="" />
        </ListBox>
    </Window>
```

**Properties:** `title`, `text`  
**Events:** `onChange` (Text), `onActivated` (Unit)

---

### Layout overlays

#### `Overlay`

Stacks widgets on top of a main content widget. Use `content` for the base layer and `overlay` for widgets that float above it.

```aivi
value view =
    <Window title="App">
        <Overlay>
            <Label text="Base content" halign="Center" valign="Center" hexpand={True} vexpand={True} />
            <Overlay.overlay>
                <Label text="Floating label" halign="End" valign="End" marginEnd={12} marginBottom={12} />
            </Overlay.overlay>
        </Overlay>
    </Window>
```

**Children:** `content` (single — base widget), `overlay` (sequence — overlaid widgets)

#### `MultilineEntry`

A multi-line text editor backed by `gtk::TextView`.

```aivi
value view =
    <Window title="App">
        <MultilineEntry text="" editable={True} monospace={False} topMargin={8} bottomMargin={8} vexpand={True} />
    </Window>
```

**Properties:** `text`, `editable` (Bool), `monospace` (Bool), `topMargin` (Int), `bottomMargin` (Int), `leftMargin` (Int), `rightMargin` (Int)  
**Events:** `onChange` (Text), `onFocusIn` (Unit), `onFocusOut` (Unit)

#### `Picture`

Displays an image from a file path or a GResource path, with configurable content-fit
behaviour. Use `Picture` instead of `Image` when you need layout-aware scaling.

```aivi
value view =
    <Window title="App">
        <Picture filename="/usr/share/pixmaps/logo.png" contentFit="contain" canShrink={True} hexpand={True} vexpand={True} />
    </Window>
```

**Properties:**
- `filename` (Text) — absolute file path to load; empty string clears the image
- `resource` (Text) — GResource path (e.g. `/com/example/app/logo.png`); empty clears
- `contentFit` (Text) — one of `contain` (default), `fill`, `cover`, `scale-down`
- `altText` (Text) — accessibility description
- `canShrink` (Bool) — allow the picture to shrink below its natural size (default `True`)

### ViewStack + ViewSwitcher

`ViewStack` is the primary Adwaita page-navigation container. `ViewSwitcher` is a tab bar
that presents the pages of a `ViewStack`. Since AIVI uses reactive state instead of
cross-widget references, both widgets share state through a common `visibleChildName`
signal rather than a direct object link.

```aivi
```

**ViewStack properties:**
- `visibleChildName` (Text) — name of the currently visible page

**ViewStack events:**
- `onSwitch` (Text) — fires with the name of the newly visible page when navigation changes

**ViewStack children:**
- `pages` — sequence of child widgets; each is added as a named page via `add_named`.
  To assign a name to a page, set the child widget's GObject `name` property via the
  `visibleChildName` setter on the stack or use reactive state.

**ViewSwitcher properties:**
- `policy` (Text) — `"Narrow"` (icon-only) or `"Wide"` (icon + label); default `Narrow`

> **Note:** ViewSwitcher does not automatically link to a ViewStack in AIVI's declarative
> model. Connect them reactively: bind the `ViewStack.visibleChildName` signal to both
> the stack and a switcher state signal.

### AlertDialog

`AlertDialog` is a modal Adwaita dialog with configurable response buttons. Present and
dismiss it by binding a Bool signal to the `visible` property.

```aivi
```

**Properties:**
- `visible` (Bool) — `True` presents the dialog, `False` closes it
- `heading` (Text) — dialog heading text
- `body` (Text) — dialog body text
- `defaultResponse` (Text) — ID of the response activated by pressing Enter
- `closeResponse` (Text) — ID of the response used when the dialog is closed by the user
- `responses` (Text) — pipe-separated list of `id:Label` or `id:Label:appearance` entries,
  where appearance is one of `default`, `suggested`, `destructive`.
  Example: `"ok:OK:suggested|cancel:Cancel"`

**Events:**
- `onResponse` (Text) — fires with the response ID when the user activates a response
