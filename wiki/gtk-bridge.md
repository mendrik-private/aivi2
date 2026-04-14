# GTK Bridge

The GTK bridge lowers AIVI markup expressions into a live GTK4/libadwaita widget tree. It is the only layer that touches GTK widgets, and it runs exclusively on the GLib main thread.

## Architecture

**Sources**: `crates/aivi-gtk/src/`

```
HIR markup expression
    │ lower.rs: lower_markup_root() / lower_markup_expr()
    ▼
WidgetPlan (aivi-gtk/plan.rs)
    │ runtime_adapter.rs: assemble_widget_runtime()
    ▼
WidgetRuntimeAssembly
    │ executor.rs: GtkExecutor
    ▼
Live GTK widget tree (main thread)
    │ host.rs: GtkConcreteHost
    ▼
GtkConcreteWidget instances
```

## Markup Lowering

**Source**: `lower.rs`

`lower_markup_root()` and `lower_markup_expr()` translate HIR `MarkupNode` trees into `WidgetPlan` graphs:

- `LoweringOptions` controls lowering behaviour
- `LoweringError` covers unsupported widget kinds, property types, or event signals

The lowerer consults `schema.rs` to resolve widget names, property names, and event signal names against the GTK widget catalog.

## Widget Schema / Catalog

**Source**: `schema.rs`

The widget catalog is the authoritative list of supported GTK/libadwaita widgets and their properties and events.

Key lookup functions:
- `lookup_widget_schema(kind)` — look up a `GtkWidgetSchema` by `GtkConcreteWidgetKind`
- `lookup_widget_schema_by_name(name)` — look up by string name
- `lookup_widget_property(schema, name)` — look up a `GtkPropertyDescriptor`
- `lookup_widget_event(schema, name)` — look up a `GtkEventDescriptor`
- `supported_widget_schemas()` — full list of supported widgets

**Widget kinds** (`GtkConcreteWidgetKind`): 45 supported GTK4 and Adwaita widgets as of this writing. Includes: Window, HeaderBar, Paned, Box, ScrolledWindow, Frame, Viewport, Label, Button, Entry, Switch, CheckButton, ToggleButton, SpinButton, Scale, Image, Spinner, ProgressBar, Revealer, Separator, StatusPage, Clamp, Banner, ToolbarView, ActionRow, ExpanderRow, SwitchRow, SpinRow, EntryRow, ListBox, ListBoxRow, DropDown, SearchEntry, Expander, NavigationView, NavigationPage, ToastOverlay, PreferencesGroup, PreferencesPage, PreferencesWindow, ComboRow, PasswordEntryRow, Overlay, MultilineEntry, Picture.

**Property setters** (`GtkPropertySetter`):
- `GtkTextPropertySetter` — sets a string property
- `GtkBoolPropertySetter` — sets a bool property
- `GtkI64PropertySetter` — sets an integer property
- `GtkF64PropertySetter` — sets a float property
- `GtkTextOrI64PropertySetter` — union setter

### 2026-04-08 widget note

- Buttons now support `focusable={Bool}` in the GTK schema/host path.
- Reversi uses `focusable={False}` on board cells so clicks do not show a transient focus-state flash before the red-stone paint lands.

**Event signals** (`GtkEventSignal`): maps AIVI event names to GObject signal names.

## Widget Plan

**Source**: `plan.rs`

`WidgetPlan` is the stable intermediate representation between the lowerer and the runtime adapter:

- `RuntimeWidgetNode` — a concrete widget with property bindings and event hookups
- `RuntimeShowNode` — conditional show/hide (maps to `visible` property + `ShowMountPolicy`)
- `RuntimeEachNode` — dynamic list rendering (fanout over a signal)
- `RuntimeCaseNode` / `RuntimeCaseBranch` — pattern-matched widget subtree
- `RuntimeMatchNode` — structural match over a value
- `RuntimeWithNode` — local binding in the widget tree
- `RuntimeFragmentNode` — a named reusable fragment

## Runtime Adapter

**Source**: `runtime_adapter.rs`

`assemble_widget_runtime()` translates the `WidgetPlan` into a `WidgetRuntimeAssembly` — a set of `HirSignalBinding`-compatible owner/input pairs that the signal graph can own:

- `RuntimePropertyBinding` — wires a signal to a widget property setter
- `RuntimeEventBinding` — wires a GObject signal to an AIVI input
- `RuntimeExprInput` — an expression evaluated per tick as a widget input
- `RuntimeChildOp` — child widget add/remove operations
- `RuntimeSetterBinding` — imperative property setter

`WidgetRuntimeAdapterError` / `WidgetRuntimeAdapterErrors` report assembly failures.

## Executor

**Source**: `executor.rs`

`GtkExecutor` drives the widget lifecycle on the main thread:
- Creates `GtkConcreteWidget` instances for each `RuntimeWidgetNode`
- Registers property change callbacks on the scheduler's reactive clause outputs
- Hooks GObject signals to `WorkerPublicationSender` to feed input handles
- Handles `RuntimeShowNode` visibility switching
- Handles `RuntimeEachNode` dynamic child list management

## Host

**Source**: `host.rs`

`GtkConcreteHost` is the boundary between the AIVI runtime and raw GTK widget objects:

- `GtkConcreteWidget` — wraps a `gtk::Widget` with stable identity
- `GtkHostValue` — a typed value the host can set on a widget property
- `GtkQueuedEvent` — a queued GTK event (for test/introspection)
- `GtkQueuedWindowKeyEvent` — a queued keyboard event

`concrete_widget_is_window()`, `concrete_supports_property()`, `concrete_event_payload()` are host-level capability queries.

## Thread Safety

**All GTK operations must happen on the GLib main thread.**

- Widget creation: main thread only
- Property mutation: main thread only
- GObject signal emission: main thread only
- Source providers and task executors run on worker threads and publish into scheduler queues
- The scheduler drives reactive clause outputs on the main thread via `GlibSchedulerDriver`

*See also: [runtime.md](runtime.md), [signal-model.md](signal-model.md)*
