# aivi.gnome.tray

GNOME-first tray bridge vocabulary.

This module defines the nominal handle type used with the builtin `tray` capability family. The
current tray surface is intentionally small:

- `trayHandle.ownName` lowers to `dbus.ownName`
- `trayHandle.actions` lowers to a service-side `dbus.method` binding for `Action`

Menu rendering stays in the GNOME Shell extension/backend. AIVI owns the bridge and the app-side
signals.

## Import

```aivi
use aivi.gnome.tray (
    TraySource
    TrayActionCall
    BusNameState
    defaultPath
    defaultInterface
    actionMember
)
```

## Overview

| Name | Description |
| --- | --- |
| `TraySource` | Nominal handle annotation for `@source tray ...` |
| `TrayActionCall` | Inbound tray action record with text `body` payload |
| `BusNameState` | Re-exported D-Bus name ownership state |
| `defaultPath` | Default D-Bus object path for tray action dispatch |
| `defaultInterface` | Default D-Bus interface for tray action dispatch |
| `actionMember` | Default method name used by tray extensions |

## Example

```aivi
use aivi.gnome.tray (BusNameState, TrayActionCall, TraySource)

@source tray "io.mailfox.Tray"
signal tray : TraySource

signal trayName : Signal BusNameState = tray.ownName
signal trayActions : Signal TrayActionCall = tray.actions
```

`tray.actions` listens for `Action` method calls on the default tray bridge endpoint. A GNOME Shell
extension can call that method with an action id while the AIVI app reacts to the published
`TrayActionCall`; the action id lands in `body`.

For GNOME-first apps, `aivi run` and built launchers also look for companion backend assets in
`tray/gnome-shell-extension` (or embedded `apps/tray/gnome-shell-extension`) and render/install the
extension host config automatically before launch.
