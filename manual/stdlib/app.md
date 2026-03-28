# aivi.app

Application framework types for AIVI desktop apps.

The `aivi.app` module provides the core data types for building structured GTK4/libadwaita
applications: lifecycle state, actions, undo/redo history, and notifications.

## Import

```aivi
use aivi.app.lifecycle (
    AppLifecycle
    AppActionResult
    AppCommand
    UndoState
    NotificationLevel
    AppNotification
)
```

## Types

### AppLifecycle

```aivi
type AppLifecycle =
  | Starting
  | Running
  | Suspended
  | Stopping
  | Stopped
```

Represents the current phase of an application's main loop. AIVI apps transition through
these states as the OS or user triggers lifecycle events:

| State       | Description                                      |
|-------------|--------------------------------------------------|
| `Starting`  | Initialisation in progress                       |
| `Running`   | App is active and accepting user input           |
| `Suspended` | App is in background (GNOME session suspend)     |
| `Stopping`  | Graceful shutdown initiated                      |
| `Stopped`   | App has exited all active work                   |

```aivi
use aivi.app.lifecycle (AppLifecycle)

fun describeLifecycle:Text state:AppLifecycle => state
  ||> Starting  -> "starting"
  ||> Running   -> "running"
  ||> Suspended -> "suspended"
  ||> Stopping  -> "stopping"
  ||> Stopped   -> "stopped"
```

### AppActionResult

```aivi
type AppActionResult A =
  | ActionOk A
  | ActionFailed Text
  | ActionCancelled
```

The outcome of any app action or command. Use this instead of `Result` when an action
can also be cancelled by the user (e.g. a file dialog dismissed without selecting a file).

```aivi
use aivi.app.lifecycle (AppActionResult)

fun handleSave:Text result: (AppActionResult Text) => result
  ||> ActionOk v      -> "saved: {v}"
  ||> ActionFailed e  -> "failed: {e}"
  ||> ActionCancelled -> "cancelled"
```

### AppCommand

```aivi
type AppCommand = {
    label: Text,
    description: Text,
    shortcut: Option Text
}
```

A named app command suitable for display in menus, toolbars, or a command palette.

- `label` — short display name (e.g. `"New File"`)
- `description` — longer tooltip or help text
- `shortcut` — optional keyboard shortcut string (e.g. `"Ctrl+N"`)

```aivi
use aivi.app.lifecycle (AppCommand)

value newFileCommand:AppCommand = {
    label: "New File",
    description: "Create a new empty file",
    shortcut: None
}
```

### UndoState

```aivi
type UndoState = {
    canUndo: Bool,
    canRedo: Bool,
    depth: Int
}
```

Snapshot of the undo/redo history. Bind this to toolbar button sensitivity and menu items.

- `canUndo` — true when there is at least one undoable action
- `canRedo` — true when there is at least one redoable action
- `depth` — total number of recorded history entries

```aivi
use aivi.app.lifecycle (UndoState)

value emptyUndoState:UndoState = {
    canUndo: False,
    canRedo: False,
    depth: 0
}
```

### NotificationLevel

```aivi
type NotificationLevel =
  | NoteInfo
  | NoteWarning
  | NoteError
  | NoteSuccess
```

Severity of an in-app notification. Maps to libadwaita toast and banner styling.

### AppNotification

```
type AppNotification = {
    level: NotificationLevel,
    title: Text,
    body: Option Text
}
```

A structured in-app notification (toast, banner, or status bar message).

```aivi
use aivi.app.lifecycle (
    AppNotification
    NotificationLevel
)

value savedNotification:AppNotification = {
    level: NoteInfo,
    title: "File saved",
    body: None
}
```

## Example — tracking app lifecycle in a signal

```aivi
use aivi.app.lifecycle (AppLifecycle)

signal lifecycle:AppLifecycle = source Starting

fun isRunning:Bool state:AppLifecycle => state
  ||> Running -> True
  ||> _       -> False
```

## Example — undo button binding

```aivi
use aivi.app.lifecycle (UndoState)

fun undoButtonSensitive:Bool undoState:UndoState =>
    undoState.canUndo
```
