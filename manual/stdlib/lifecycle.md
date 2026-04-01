# aivi.app.lifecycle

Shared types for describing what a desktop app is doing: starting up, running commands,
showing in-app notifications, and shutting down again.

This module is a vocabulary module. It does not perform I/O by itself. Use these types in
your own signals, records, and messages so the rest of your app can talk about lifecycle
events clearly.

## Import

```aivi
use aivi.app.lifecycle (
    AppLifecycle
    AppActionResult
    AppCommand
    UndoState
    NotificationLevel
    AppNotification
    AppEvent
)
```

## Overview

| Type | Description |
|------|-------------|
| `AppLifecycle` | Which phase the app is in right now |
| `AppActionResult A` | Result of an action that may succeed, fail, or be cancelled |
| `AppCommand` | A user-visible command, suitable for menus and command palettes |
| `UndoState` | Whether undo/redo is possible, and how much history exists |
| `NotificationLevel` | Severity for an in-app message |
| `AppNotification` | Structured in-app notification data |
| `AppEvent` | One event value wrapping lifecycle, command, or notification changes |

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

Use `AppLifecycle` when you want a simple, explicit answer to “what phase is the app in?”

| State | Meaning |
|-------|---------|
| `Starting` | The app is opening and still setting itself up |
| `Running` | The app is ready for normal use |
| `Suspended` | The app is temporarily in the background |
| `Stopping` | Shutdown has started |
| `Stopped` | The app has finished stopping |

```aivi
use aivi.app.lifecycle (
    AppLifecycle
    Starting
    Running
    Suspended
    Stopping
    Stopped
)

type AppLifecycle -> Text
func lifecycleLabel = state => state
 ||> Starting  -> "Starting…"
 ||> Running   -> "Ready"
 ||> Suspended -> "In background"
 ||> Stopping  -> "Closing…"
 ||> Stopped   -> "Closed"
```

### AppActionResult

```aivi
type AppActionResult A =
  | ActionOk A
  | ActionFailed Text
  | ActionCancelled
```

Use this when an action can end in three different ways:

- it worked and produced a value (`ActionOk`)
- it failed with a message (`ActionFailed`)
- the user backed out on purpose (`ActionCancelled`)

That last case is useful for things like file pickers or confirmation dialogs, where
cancellation is normal and should not be treated like a crash.

```aivi
use aivi.app.lifecycle (
    AppActionResult
    ActionOk
    ActionFailed
    ActionCancelled
)

type AppActionResult Text -> Text
func saveResultMessage = result => result
 ||> ActionOk path      -> "Saved to {path}"
 ||> ActionFailed error -> "Save failed: {error}"
 ||> ActionCancelled    -> "Save cancelled"
```

### AppCommand

```aivi
type AppCommand = {
    label: Text,
    description: Text,
    shortcut: Option Text
}
```

`AppCommand` describes one user-facing command. It is the label and metadata you would show
in a menu, toolbar, or command palette.

- `label` — short name shown to the user
- `description` — a longer explanation
- `shortcut` — optional keyboard shortcut such as `"Ctrl+S"`

```aivi
use aivi.app.lifecycle (AppCommand)

value saveCommand : AppCommand = {
    label: "Save",
    description: "Write the current document to disk",
    shortcut: Some "Ctrl+S"
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

This is a snapshot of your undo history. It is especially handy for enabling or disabling
buttons.

- `canUndo` — `True` when an undo step is available
- `canRedo` — `True` when a redo step is available
- `depth` — total number of recorded history entries

```aivi
use aivi.app.lifecycle (UndoState)

type UndoState -> Bool
func undoButtonEnabled = state =>
    state.canUndo
```

### NotificationLevel

```aivi
type NotificationLevel =
  | NoteInfo
  | NoteWarning
  | NoteError
  | NoteSuccess
```

This is the severity of an in-app message such as a toast, banner, or status message.

- `NoteInfo` — ordinary information
- `NoteWarning` — something needs attention
- `NoteError` — something failed
- `NoteSuccess` — an action completed successfully

### AppNotification

```aivi
type AppNotification = {
    level: NotificationLevel,
    title: Text,
    body: Option Text
}
```

This record holds the content of an in-app notification.

- `level` — how serious the message is
- `title` — short headline
- `body` — optional extra detail

```aivi
use aivi.app.lifecycle (
    AppNotification
    NoteSuccess
)

value savedNotice : AppNotification = {
    level: NoteSuccess,
    title: "Saved",
    body: Some "Your changes are on disk."
}
```

### AppEvent

```aivi
type AppEvent =
  | LifecycleChanged AppLifecycle
  | CommandRequested AppCommand
  | NotificationIssued AppNotification
```

`AppEvent` lets you put different app-related events into one stream or one queue. Instead
of carrying lifecycle changes, commands, and notifications separately, you can wrap them in
a single type.

```aivi
use aivi.app.lifecycle (
    AppCommand
    AppEvent
    CommandRequested
)

value openCommand : AppCommand = {
    label: "Open",
    description: "Choose a file to open",
    shortcut: Some "Ctrl+O"
}

value openRequested : AppEvent =
    CommandRequested openCommand
```

## Example — route app events to text

```aivi
use aivi.app.lifecycle (
    AppEvent
    LifecycleChanged
    CommandRequested
    NotificationIssued
)

type AppEvent -> Text
func eventLabel = event => event
 ||> LifecycleChanged state -> "Lifecycle changed"
 ||> CommandRequested cmd   -> "Command: {cmd.label}"
 ||> NotificationIssued n   -> "Notification: {n.title}"
```
