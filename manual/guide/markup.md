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

**Properties:** `title`, `defaultWidth`, `defaultHeight`, `resizable`, `modal`, `maximized`, `fullscreen`, `decorated`, `hideOnClose`, `visible`, `sensitive`, `opacity`, `hexpand`, `vexpand`, `halign`, `valign`, `widthRequest`, `heightRequest`, `marginStart`, `marginEnd`, `marginTop`, `marginBottom`, `tooltip`, `cssClasses`, `animateOpacity`

- `maximized` (Bool) — maximize or unmaximize the window
- `fullscreen` (Bool) — enter or exit fullscreen mode
- `decorated` (Bool) — show or hide window decorations (title bar, borders)
- `hideOnClose` (Bool) — hide rather than destroy the window when closed

**Events:** `onCloseRequest` (Unit), `onMaximize` (Bool), `onFullscreen` (Bool)

- `onMaximize` fires with `True` when maximized, `False` when restored
- `onFullscreen` fires with `True` when entering fullscreen, `False` when leaving

### `HeaderBar`

Title bar with optional start/end widget slots.

```aivi
value view =
    <Window title="App">
        <Window.titlebar>
            <HeaderBar decorationLayout="close:">
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

**Properties:** `showTitleButtons`, `decorationLayout`, `centeringPolicy`

- `decorationLayout` (Text) — overrides the system button layout; format is `"left-buttons:right-buttons"` where tokens include `close`, `maximize`, `minimize`, `icon` (e.g. `"close:"`, `"menu:minimize,maximize,close"`)
- `centeringPolicy` (Text) — controls title centering; `"Loose"` (default) centres the title when space allows, `"Strict"` always centres it even when the start/end widgets are asymmetric

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
        <Label text="Hello, world!" wrap={True} halign="Start" xalign={0.0} />
    </Window>
```

**Properties (extra):** `xalign`, `yalign`, `widthChars`, `singleLineMode`

- `xalign` (F64) — horizontal text alignment within the widget; `0.0` = left, `0.5` = center, `1.0` = right
- `yalign` (F64) — vertical text alignment within the widget
- `widthChars` (I64) — minimum width in characters; useful for stable layouts
- `singleLineMode` (Bool) — when `True`, forces text to render as a single line (truncates or scrolls)

### `Button`

Clickable button.

```aivi
value view =
    <Window title="App">
        <Button label="Click me" halign="Center" />
    </Window>
```

**Extra properties:** `iconName`, `useUnderline`, `receivesDefault`

- `iconName` (Text) — symbolic icon name; when set alongside `label`, produces an icon+label button; when set without `label`, produces an icon-only button (e.g. `"list-add-symbolic"`)
- `useUnderline` (Bool) — when `True`, a `_` in `label` marks the next character as a mnemonic accelerator
- `receivesDefault` (Bool) — when `True`, this button activates when Enter is pressed in its window (if no focused widget intercepts the key)

**Gesture events (also available on `Box`, `Label`, `Image`):**

- `onSecondaryClick` (Unit) — fires on right-button / secondary-button click
- `onLongPress` (Unit) — fires after a long press gesture is held
- `onSwipeLeft` (Unit) — fires when a left-ward swipe is detected
- `onSwipeRight` (Unit) — fires when a right-ward swipe is detected

### `Entry`

Single-line text input.

```aivi
value view =
    <Window title="App">
        <Entry placeholder="Type something..." hexpand={True} primaryIconName="system-search-symbolic" />
    </Window>
```

**Extra properties:** `primaryIconName`, `secondaryIconName`

- `primaryIconName` (Text) — icon name for the leading (start) icon slot; use a symbolic icon name such as `"system-search-symbolic"`
- `secondaryIconName` (Text) — icon name for the trailing (end) icon slot; commonly used for a clear button (`"edit-clear-symbolic"`)

**Events:** `onChange` (Text), `onActivated` (Unit), `onFocusIn` (Unit), `onFocusOut` (Unit)

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
        <Scale value={50.0} min={0.0} max={100.0} step={1.0} drawValue={True} valuePos="Bottom" hexpand={True} />
    </Window>
```

**Extra properties:** `valuePos`, `fillLevel`

- `valuePos` (Text) — position of the numeric label when `drawValue` is `True`; one of `"Top"`, `"Bottom"`, `"Left"`, `"Right"`
- `fillLevel` (F64) — fills the track up to this value (also enables the fill-level indicator automatically); useful for buffered-progress UIs

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
**Children:** `prefix` (sequence — widgets placed before the title), `suffix` (sequence — widgets placed after the title)

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
**Events:** `onExpanded` (Bool) — fires each time the row is expanded or collapsed  
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
**Events:** `onChange` (Text), `onActivated` (Unit), `onFocusIn` (Unit), `onFocusOut` (Unit)

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

**Properties:** `selectionMode` (`None`|`Single`|`Browse`|`Multiple`), `showSeparators` (Bool)  
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

#### `ListView`

A factory-backed GTK list for large item sequences. It accepts ordinary child widgets, but mounts them through GTK list items so scrolling stays efficient. Wrap it in a `ScrolledWindow` when the content should scroll.

```aivi
value view =
    <Window title="App">
        <ScrolledWindow vexpand={True}>
            <ListView showSeparators={True} singleClickActivate={True} onActivate={index}>
                <Label text="Thread A" />
                <Label text="Thread B" />
                <Label text="Thread C" />
            </ListView>
        </ScrolledWindow>
    </Window>
```

**Properties:** `showSeparators`, `enableRubberband`, `singleClickActivate` (Bool)  
**Events:** `onActivate` (Int — zero-based item index)  
**Children:** `children` (sequence — default)

#### `GridView`

A factory-backed GTK grid for large tile collections. Like `ListView`, it accepts ordinary child widgets and virtualizes scrolling/layout through GTK’s grid item machinery.

```aivi
value view =
    <Window title="App">
        <ScrolledWindow vexpand={True}>
            <GridView minColumns={2} maxColumns={4} singleClickActivate={True} onActivate={index}>
                <Button label="Card 1" />
                <Button label="Card 2" />
                <Button label="Card 3" />
                <Button label="Card 4" />
            </GridView>
        </ScrolledWindow>
    </Window>
```

**Properties:** `enableRubberband`, `singleClickActivate` (Bool), `minColumns`, `maxColumns` (Int)  
**Events:** `onActivate` (Int — zero-based item index)  
**Children:** `children` (sequence — default)

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
**Events:** `onExpanded` (Bool) — fires each time the expander is expanded or collapsed  
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
**Events:** `onShowing` (Unit) — fired when the page is pushed onto the navigation stack; `onHiding` (Unit) — fired when the page is popped  
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
**Events:** `onChange` (Text), `onActivated` (Unit), `onFocusIn` (Unit), `onFocusOut` (Unit)

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

#### `WebView`

Strict embedded HTML viewer backed by WebKitGTK. Load HTML through `html`. Safe defaults stay on: JavaScript disabled, session/storage ephemeral, permission requests denied, and navigation/new windows blocked. When links should open externally, use `portal.openUri` from app state instead of relying on in-widget browsing.

```aivi
value view =
    <Window title="App">
        <WebView html={"<article><h1>Hello</h1><p>Rendered by WebKit.</p></article>"} hexpand={True} vexpand={True} />
    </Window>
```

**Properties:** `html` (Text)

### ViewStack + ViewSwitcher

`ViewStack` is the primary Adwaita page-navigation container. `ViewSwitcher` is a tab bar
that presents the pages of a `ViewStack`. Since AIVI uses reactive state instead of
cross-widget references, both widgets share state through a common `visibleChildName`
signal rather than a direct object link.

```aivi
signal activePage : Text = "home"

value view =
    <Window title="App">
        <ToolbarView>
            <ToolbarView.top>
                <HeaderBar>
                    <ViewSwitcher policy="Wide" />
                </HeaderBar>
            </ToolbarView.top>
            <ViewStack visibleChildName={activePage}>
                <ViewStack.pages>
                    <ViewStackPage name="home" title="Home" iconName="go-home-symbolic">
                        <Label text="Home page" halign="Center" valign="Center" vexpand={True} />
                    </ViewStackPage>
                    <ViewStackPage name="search" title="Search" iconName="edit-find-symbolic">
                        <Label text="Search page" halign="Center" valign="Center" vexpand={True} />
                    </ViewStackPage>
                </ViewStack.pages>
            </ViewStack>
        </ToolbarView>
    </Window>
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
signal showConfirm : Bool = False
signal lastResponse : Text = ""

value view =
    <Window title="App">
        <AlertDialog visible={showConfirm} heading="Delete item?" body="This action cannot be undone." defaultResponse="delete" closeResponse="cancel" responses="delete:Delete:destructive|cancel:Cancel"></AlertDialog>
        <Button label="Delete" cssClasses="destructive-action"></Window>
    </Window>
``` — `True` presents the dialog, `False` closes it
- `heading` (Text) — dialog heading text
- `body` (Text) — dialog body text
- `defaultResponse` (Text) — ID of the response activated by pressing Enter
- `closeResponse` (Text) — ID of the response used when the dialog is closed by the user
- `responses` (Text) — pipe-separated list of `id:Label` or `id:Label:appearance` entries,
  where appearance is one of `default`, `suggested`, `destructive`.
  Example: `"ok:OK:suggested|cancel:Cancel"`

**Events:**
- `onResponse` (Text) — fires with the response ID when the user activates a response

### `Calendar`

Month calendar with optional day selection.

```aivi
signal selectedDay : Int = 1
signal selectedMonth : Int = 0
signal selectedYear : Int = 2025

value view =
    <Window title="App">
        <Calendar year={selectedYear} month={selectedMonth} day={selectedDay} onDaySelected={.}></Window>
    </Window>
```

**Properties:** `year`, `month`, `day`

- `year` (I64) — displayed year
- `month` (I64) — displayed month, **0-based** (0 = January, 11 = December)
- `day` (I64) — selected day of the month (1–31)

**Events:**
- `onDaySelected` (Unit) — fires when the user clicks a day

### `FlowBox` + `FlowBoxChild`

Reflowing grid of child widgets. Children wrap to the next row when the available width is exhausted.

```aivi
value view =
    <Window title="App">
        <ScrolledWindow hexpand={True} vexpand={True}>
            <FlowBox selectionMode="Single" rowSpacing={6} columnSpacing={6}>
                <FlowBoxChild>
                    <Label text="Item A" />
                </FlowBoxChild>
                <FlowBoxChild>
                    <Label text="Item B" />
                </FlowBoxChild>
                <FlowBoxChild>
                    <Label text="Item C" />
                </FlowBoxChild>
            </FlowBox>
        </ScrolledWindow>
    </Window>
```

**FlowBox properties:** `selectionMode`, `rowSpacing`, `columnSpacing`

- `selectionMode` (Text) — `"None"`, `"Single"`, `"Browse"`, or `"Multiple"`
- `rowSpacing` (I64) — pixels of space between rows
- `columnSpacing` (I64) — pixels of space between columns

**FlowBox events:**
- `onChildActivated` (Unit) — fires when a child is activated (double-clicked or Enter)

**FlowBox children:** `children` — sequence of `FlowBoxChild` widgets

**FlowBoxChild children:** `content` — single child slot (the widget displayed inside the cell)

### `MenuButton`

A button that opens a `Popover` when clicked.

```aivi
signal menuOpen : Bool = False

value view =
    <Window title="App">
        <Window.titlebar>
            <HeaderBar>
                <HeaderBar.end>
                    <MenuButton iconName="open-menu-symbolic" onToggled={open => menuOpen}>
                        <MenuButton.popover>
                            <Popover>
                                <Popover.content>
                                    <Box orientation="Vertical" spacing={4} marginTop={8} marginBottom={8} marginStart={8} marginEnd={8}>
                                        <Button label="Settings" />
                                        <Button label="About" />
                                    </Box>
                                </Popover.content>
                            </Popover>
                        </MenuButton.popover>
                    </MenuButton>
                </HeaderBar.end>
            </HeaderBar>
        </Window.titlebar>
        <Label text="Content" />
    </Window>
```

**Properties:** `label`, `iconName`, `active`, `useUnderline`

- `label` (Text) — button label text (use `iconName` for icon-only buttons)
- `iconName` (Text) — icon name from the icon theme (e.g. `"open-menu-symbolic"`)
- `active` (Bool) — whether the popover is currently open
- `useUnderline` (Bool) — interpret `_` in the label as a mnemonic underline

**Events:**
- `onToggled` (Bool) — fires with `True` when the popover opens, `False` when it closes

**Children:** `popover` — single `Popover` child slot

### `Popover`

A floating overlay widget anchored to its parent. Typically used as the child of `MenuButton`.

```aivi
value view =
    <Window title="App">
        <MenuButton label="Options">
            <MenuButton.popover>
                <Popover autohide={True} hasArrow={True} onClosed={.}>
                    <Popover.content>
                        <Label text="Hello from popover!" marginStart={8} marginEnd={8} marginTop={8} marginBottom={8} />
                    </Popover.content>
                </Popover>
            </MenuButton.popover>
        </MenuButton>
    </Window>
```

**Properties:** `autohide`, `hasArrow`

- `autohide` (Bool) — close the popover automatically when focus leaves it (default `True`)
- `hasArrow` (Bool) — draw a pointing arrow towards the anchor widget (default `True`)

**Events:**
- `onClosed` (Unit) — fires when the popover is dismissed

**Children:** `content` — single child slot for the popover body widget

---

### `CenterBox`

A three-slot horizontal container that aligns a center child between optional start and end children.

```aivi
value view =
    <Window title="App">
        <CenterBox hexpand={True}>
            <CenterBox.start>
                <Button label="Back" />
            </CenterBox.start>
            <CenterBox.center>
                <Label text="Title" />
            </CenterBox.center>
            <CenterBox.end>
                <Button label="Forward" />
            </CenterBox.end>
        </CenterBox>
    </Window>
```

**Children:** `start` (single), `center` (single — default), `end` (single)

---

### `AboutDialog`

Adwaita About dialog showing application metadata. Acts as a top-level window; set `visible` reactively to show or hide it.

```aivi
value view =
    <Window title="App">
        <AboutDialog visible={showAbout} appName="My App" version="1.0.0" developerName="Jane Doe" website="https://example.com" issueUrl="https://github.com/example/issues" licenseType="MIT" applicationIcon="my-app" />
        <Button label="About" onClick={True}></Window>
    </Window>
```

**Properties:** `visible`, `appName`, `version`, `developerName`, `comments`, `website`, `issueUrl`, `licenseType`, `applicationIcon`

- `licenseType` (Text) — one of the GLib/GTK license identifiers: `"MIT"`, `"GPL-2.0"`, `"GPL-3.0"`, `"LGPL-2.1"`, `"LGPL-3.0"`, `"AGPL-3.0"`, `"Apache-2.0"`, `"MPL-2.0"`, `"Custom"`, `"Unknown"`

---

### `SplitButton`

A combined button + dropdown arrow that opens a `Popover` for secondary actions.

```aivi
value view =
    <Window title="App">
        <SplitButton label="Save" onClick={.} hexpand={False}>
            <SplitButton.popover>
                <Popover>
                    <Popover.content>
                        <Box orientation="Vertical" spacing={4}>
                            <Button label="Save As…" />
                            <Button label="Export…" />
                        </Box>
                    </Popover.content>
                </Popover>
            </SplitButton.popover>
        </SplitButton>
    </Window>
```

**Properties:** `label`, `iconName`

- `label` (Text) — text for the main button half
- `iconName` (Text) — icon name alternative to `label`

**Events:** `onClick` (Unit) — fires when the main button half is clicked

**Children:** `popover` (single — a `Popover` widget for the dropdown)

---

### `NavigationSplitView`

An adaptive two-panel layout (sidebar + content) that collapses to a single column on narrow displays. Both panels should contain `NavigationPage` widgets.

```aivi
value view =
    <Window title="App" defaultWidth={900} defaultHeight={600}>
        <NavigationSplitView showContent={True} sidebarWidthFraction={0.3}>
            <NavigationSplitView.sidebar>
                <NavigationPage title="Sidebar">
                    <Label text="Sidebar content" />
                </NavigationPage>
            </NavigationSplitView.sidebar>
            <NavigationSplitView.content>
                <NavigationPage title="Content">
                    <Label text="Main content" />
                </NavigationPage>
            </NavigationSplitView.content>
        </NavigationSplitView>
    </Window>
```

**Properties:** `showContent`, `sidebarWidthFraction`, `minSidebarWidth`, `maxSidebarWidth`, `sidebarPosition`

- `showContent` (Bool) — when collapsed, whether the content panel is shown (vs. the sidebar)
- `sidebarWidthFraction` (Float) — sidebar width as a fraction of the total width (default `0.25`)
- `minSidebarWidth` / `maxSidebarWidth` (Float) — clamp sidebar width in pixels
- `sidebarPosition` (Text) — `"Start"` (default) or `"End"`

**Events:** `onShowContent` (Bool) — fires when `showContent` changes

**Children:** `sidebar` (single), `content` (single — default)

---

### `OverlaySplitView`

Like `NavigationSplitView` but the sidebar slides over the content rather than pushing it.

```aivi
value view =
    <Window title="App" defaultWidth={800} defaultHeight={600}>
        <OverlaySplitView showSidebar={sidebarOpen}>
            <OverlaySplitView.sidebar>
                <Box orientation="Vertical" spacing={8} marginStart={12} marginTop={12}>
                    <Label text="Navigation" />
                </Box>
            </OverlaySplitView.sidebar>
            <OverlaySplitView.content>
                <Label text="Content" halign="Center" valign="Center" />
            </OverlaySplitView.content>
        </OverlaySplitView>
    </Window>
```

**Properties:** `showSidebar`, `sidebarPosition`, `sidebarWidthFraction`, `minSidebarWidth`, `maxSidebarWidth`

- `showSidebar` (Bool) — show or hide the overlay sidebar
- `sidebarPosition` (Text) — `"Start"` (default) or `"End"`

**Events:** `onShowSidebar` (Bool) — fires when `showSidebar` changes

**Children:** `sidebar` (single), `content` (single — default)

---

### `TabView` + `TabPage` + `TabBar`

A tabbed interface. `TabView` holds pages; `TabBar` provides the tab strip.

```aivi
value view =
    <Window title="App">
        <Box orientation="Vertical">
            <TabView selectedPage={activeTab} onSelectedPageChanged={.} hexpand={True} vexpand={True}>
                <TabView.tabBar>
                    <TabBar autohide={False} expandTabs={True} />
                </TabView.tabBar>
                <TabView.pages>
                    <TabPage title="Documents">
                        <TabPage.content>
                            <Label text="Documents tab" halign="Center" valign="Center" />
                        </TabPage.content>
                    </TabPage>
                    <TabPage title="Settings" needsAttention={True}>
                        <TabPage.content>
                            <Label text="Settings tab" halign="Center" valign="Center" />
                        </TabPage.content>
                    </TabPage>
                </TabView.pages>
            </TabView>
        </Box>
    </Window>
```

#### `TabView`

**Properties:** `selectedPage` (Int — zero-based index)  
**Events:** `onPageAdded` (Unit), `onPageClosed` (Unit), `onSelectedPageChanged` (Unit)  
**Children:** `pages` (sequence of `TabPage` — default), `tabBar` (single `TabBar`)

#### `TabPage`

**Properties:** `visible`, `title`, `needsAttention` (Bool — shows a dot indicator), `loading` (Bool — shows a spinner)  
**Children:** `content` (single — default)

#### `TabBar`

**Properties:** `autohide` (Bool — hides bar when there is only one page), `expandTabs` (Bool — tabs expand to fill the bar width)

---

### `Carousel` + indicators

A swipe carousel backed by `adw::Carousel`. Attach `CarouselIndicatorDots` or `CarouselIndicatorLines` as the `dots`/`lines` child to get navigation indicators.

```aivi
value view =
    <Window title="App">
        <Box orientation="Vertical" spacing={8} hexpand={True} vexpand={True}>
            <Carousel spacing={16} interactive={True} hexpand={True} vexpand={True} onPageChanged={.} onSwipeLeft={.} onSwipeRight={.}>
                <Carousel.dots>
                    <CarouselIndicatorDots />
                </Carousel.dots>
                <Carousel.pages>
                    <Label text="Page 1" halign="Center" valign="Center" />
                    <Label text="Page 2" halign="Center" valign="Center" />
                    <Label text="Page 3" halign="Center" valign="Center" />
                </Carousel.pages>
            </Carousel>
        </Box>
    </Window>
```

**Carousel properties:** `spacing` (Int), `revealDuration` (Int — ms), `interactive` (Bool)  
**Carousel events:** `onPageChanged` (Int — zero-based page index), `onSwipeLeft` (Unit), `onSwipeRight` (Unit)  
**Carousel children:** `pages` (sequence — default), `dots` (single `CarouselIndicatorDots`), `lines` (single `CarouselIndicatorLines`)

`CarouselIndicatorDots` and `CarouselIndicatorLines` have no widget-specific properties; they automatically link to their parent Carousel.

---

### `Grid` + `GridChild`

A two-dimensional grid layout. Each child must be wrapped in a `GridChild` that specifies its position.

```aivi
value view =
    <Window title="App">
        <Grid rowSpacing={8} columnSpacing={8} marginStart={12} marginEnd={12} marginTop={12} marginBottom={12}>
            <GridChild column={0} row={0} columnSpan={2}>
                <GridChild.content>
                    <Label text="Wide header" hexpand={True} />
                </GridChild.content>
            </GridChild>
            <GridChild column={0} row={1}>
                <GridChild.content>
                    <Button label="A" />
                </GridChild.content>
            </GridChild>
            <GridChild column={1} row={1}>
                <GridChild.content>
                    <Button label="B" />
                </GridChild.content>
            </GridChild>
        </Grid>
    </Window>
```

#### `Grid`

**Properties:** `rowSpacing`, `columnSpacing` (Int), `rowHomogeneous`, `columnHomogeneous` (Bool)  
**Children:** `children` (sequence of `GridChild` — default)

#### `GridChild`

**Properties:** `column`, `row`, `columnSpan`, `rowSpan` (Int — default span is 1)  
**Children:** `content` (single — default)

---

### `FileDialog`

A native file-chooser dialog backed by `gtk::FileChooserNative`. Set `visible` to `True` to show the dialog; the response is delivered via `onResponse`.

```aivi
value view =
    <Window title="App">
        <FileDialog visible={dialogOpen} title="Open file" mode="Open" acceptLabel="Open" cancelLabel="Cancel" onResponse={code}>
            <Button label="Open file…" onClick={True}></Window>
        </FileDialog>
    </Window>
```

**Properties:**

| Property | Type | Description |
|---|---|---|
| `visible` | Bool | Show (`True`) or hide (`False`) the dialog |
| `title` | Text | Window title of the chooser |
| `mode` | Text | `"Open"` (default) or `"Save"` |
| `acceptLabel` | Text | Label for the accept button |
| `cancelLabel` | Text | Label for the cancel button |

**Events:** `onResponse` (Int) — response code; `1` = accepted, `0` = cancelled (matches `gtk::ResponseType`)
