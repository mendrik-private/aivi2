# aivi.gnome.notifications

Types for GNOME desktop notifications.

These are operating-system notifications: the kind that appear in the desktop shell. They
are separate from `AppNotification` in `aivi.app.lifecycle`, which describes messages shown
inside your own app window.

This module defines the payloads, handle annotation, task alias, and response types used by the
built-in desktop notification capability:

```aivi
@source notifications "io.mailfox"
signal notifications : NotificationSource
```

## Import

```aivi
use aivi.gnome.notifications (
    NotificationAction
    Notification
    NotificationSource
    NotificationTask
    NotificationError
    NotificationResponse
    NotificationEvent
)
```

## Overview

| Type | Description |
|------|-------------|
| `NotificationAction` | One clickable action button in a notification |
| `Notification` | Full desktop notification payload |
| `NotificationSource` | Nominal handle annotation for `@source notifications ...` |
| `NotificationTask A` | Generic desktop-notification task alias |
| `NotificationError` | Failure case when notification delivery fails |
| `NotificationResponse` | What the user did with the notification |
| `NotificationEvent` | Response paired with notification id |

## Types

### NotificationAction

```aivi
type NotificationAction = {
    label: Text,
    id: Text
}
```

One action button attached to a notification.

- `label` — text shown to the user
- `id` — stable identifier you can match on later if the action is triggered

```aivi
use aivi.gnome.notifications (NotificationAction)

value archiveAction : NotificationAction = {
    label: "Archive",
    id: "archive"
}
```

### Notification

```aivi
type Notification = {
    summary: Text,
    body: Option Text,
    icon: Option Text,
    actions: List NotificationAction
}
```

Full notification payload.

- `summary` — short headline
- `body` — optional longer message
- `icon` — optional icon name or other backend-specific text identifier
- `actions` — zero or more clickable actions

### NotificationSource

```aivi
type NotificationSource = Unit
```

Nominal handle annotation used with `@source notifications "app.name"`.

### NotificationTask

```aivi
type NotificationTask A = (Task NotificationError A)
```

Alias used by `notifications.send` and `notifications.close`.

```aivi
use aivi.gnome.notifications (
    Notification
    NotificationAction
)

value archiveAction : NotificationAction = {
    label: "Archive",
    id: "archive"
}

value newMail : Notification = {
    summary: "New mail",
    body: Some "You have 3 new messages",
    icon: Some "mail-unread",
    actions: [archiveAction]
}
```

### NotificationError

```aivi
type NotificationError =
  NotificationFailed Text
```

Notification delivery failed, with a backend-provided message.

### NotificationResponse

```aivi
type NotificationResponse =
  | ActionTriggered Text
  | Dismissed
```

Result from the desktop after a notification is shown.

- `ActionTriggered id` — the user clicked an action with the given `id`
- `Dismissed` — the notification was closed without an action

```aivi
use aivi.gnome.notifications (
    NotificationResponse
    ActionTriggered
    Dismissed
)

type NotificationResponse -> Text
func responseLabel = response => response
 ||> ActionTriggered actionId -> "Clicked: {actionId}"
 ||> Dismissed                -> "Dismissed"
```

### NotificationEvent

```aivi
type NotificationEvent = {
    id: Int,
    response: NotificationResponse
}
```

Published by `notifications.events` so apps can correlate responses with previously shown
desktop notifications.

## Capability surface

```aivi
use aivi.gnome.notifications (
    Notification
    NotificationEvent
    NotificationSource
)

@source notifications "io.mailfox" with {
    bus: "session"
}
signal notifications : NotificationSource

signal events : Signal NotificationEvent = notifications.events

value showMail : Task NotificationError Int =
    notifications.send {
        summary: "New mail"
        body: Some "Alex replied about dinner"
        icon: Some "mail-unread"
        actions: [{ label: "Open", id: "open" }, { label: "Mark read", id: "read" }]
    }

value closeMail : Task NotificationError Unit =
    notifications.close 42
```
