# aivi.gnome.notifications

Types for GNOME desktop notifications.

These are operating-system notifications: the kind that appear in the desktop shell. They
are separate from `AppNotification` in `aivi.app.lifecycle`, which describes messages shown
inside your own app window.

This module defines the payloads and result types used by desktop-notification APIs. It does
not itself send a notification.

## Import

```aivi
use aivi.gnome.notifications (
    NotificationAction
    Notification
    NotificationError
    NotificationResponse
)
```

## Overview

| Type | Description |
|------|-------------|
| `NotificationAction` | One clickable action button in a notification |
| `Notification` | Full desktop notification payload |
| `NotificationError` | Failure case when notification delivery fails |
| `NotificationResponse` | What the user did with the notification |

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
  | NotificationFailed Text
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
