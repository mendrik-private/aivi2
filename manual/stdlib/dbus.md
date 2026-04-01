# aivi.dbus

Types for D-Bus calls, signals, and values.

D-Bus is the message bus GNOME apps and system services use to talk to each other. This module gives you the data shapes for that conversation. The current stdlib file defines types only; it does not send or receive bus traffic by itself.

## Import

```aivi
use aivi.dbus (
    DbusValue
    DbusString
    DbusInt
    DbusBool
    DbusList
    DbusStruct
    DbusVariant
    DbusCall
    DbusSignal
    BusNameFlag
    AllowReplacement
    ReplaceExisting
    DoNotQueue
    BusNameState
    Owned
    Queued
    Lost
    DbusError
    NameNotOwned
    ServiceUnknown
    NoReply
    AccessDenied
    InvalidArgs
    DbusProtocolError
    DbusCallResult
    DbusTask
    DbusMatchRule
)
```

## Overview

| Type | Purpose |
|------|---------|
| `DbusValue` | One value carried over the bus |
| `DbusCall` | A method call request |
| `DbusSignal` | A broadcast message from a service |
| `DbusMatchRule` | Optional filters for selecting signals |
| `BusNameFlag` | How to request ownership of a bus name |
| `BusNameState` | Current ownership state of a bus name |
| `DbusError` | Structured D-Bus failures |
| `DbusCallResult` | Result of a call returning D-Bus values |
| `DbusTask A` | Background D-Bus work returning `A` |

---

## `DbusValue`

```aivi
type DbusValue =
  | DbusString Text
  | DbusInt Int
  | DbusBool Bool
  | DbusList (List DbusValue)
  | DbusStruct (List DbusValue)
  | DbusVariant DbusValue
```

The current value model covers strings, integers, booleans, lists, structs, and variants.

- `DbusList` groups a list of bus values
- `DbusStruct` groups several fields into one ordered payload
- `DbusVariant` wraps a value when the protocol expects a runtime-typed value inside another value

```aivi
use aivi.dbus (
    DbusValue
    DbusString
    DbusBool
    DbusStruct
)

value loginHint : DbusValue =
    DbusStruct [
        DbusString "ada@example.com",
        DbusBool True
    ]
```

---

## `DbusCall`

```aivi
type DbusCall = {
    destination: Text,
    path: Text,
    interface: Text,
    member: Text,
    body: List DbusValue
}
```

A method call sent to a D-Bus service.

- `destination` — the bus name to talk to
- `path` — the object path on that service
- `interface` — the interface name that owns the method
- `member` — the method name
- `body` — the ordered argument list

```aivi
use aivi.dbus (DbusCall)

value pingCall : DbusCall = {
    destination: "org.freedesktop.DBus",
    path: "/org/freedesktop/DBus",
    interface: "org.freedesktop.DBus.Peer",
    member: "Ping",
    body: []
}
```

---

## `DbusSignal`

```aivi
type DbusSignal = {
    path: Text,
    interface: Text,
    member: Text,
    body: List DbusValue
}
```

A broadcast message emitted by a service. The shape is close to `DbusCall`, but there is no `destination` because signals are observed rather than directly addressed.

---

## `DbusMatchRule`

```aivi
type DbusMatchRule = {
    path: Option Text,
    interface: Option Text,
    member: Option Text
}
```

Optional filters for selecting only the signals you care about. Leave a field as `None` when you do not want to filter on it.

```aivi
use aivi.dbus (DbusMatchRule)

value syncSignals : DbusMatchRule = {
    path: Some "/org/gnome/AiviMail/Daemon",
    interface: Some "org.gnome.AiviMail.IDaemon",
    member: Some "SyncStateChanged"
}
```

---

## `BusNameFlag`

```aivi
type BusNameFlag =
  | AllowReplacement
  | ReplaceExisting
  | DoNotQueue
```

Flags that control how a service asks for a bus name.

- `AllowReplacement` — let another process replace this name later
- `ReplaceExisting` — take over an existing name if the current owner allows it
- `DoNotQueue` — fail instead of waiting in a queue

---

## `BusNameState`

```aivi
type BusNameState =
  | Owned
  | Queued
  | Lost
```

Current state after requesting a bus name.

- `Owned` — this process owns the name now
- `Queued` — the request is waiting
- `Lost` — the name was lost or could not be kept

---

## `DbusError`

```aivi
type DbusError =
  | NameNotOwned Text
  | ServiceUnknown Text
  | NoReply
  | AccessDenied Text
  | InvalidArgs Text
  | DbusProtocolError Text
```

Structured failure reasons for D-Bus work.

- `NameNotOwned Text` — a required bus name is not owned
- `ServiceUnknown Text` — the destination service could not be found
- `NoReply` — the service did not answer in time
- `AccessDenied Text` — the bus rejected the request
- `InvalidArgs Text` — the call arguments were rejected
- `DbusProtocolError Text` — another protocol-level failure occurred

```aivi
use aivi.dbus (
    DbusError
    NameNotOwned
    ServiceUnknown
    NoReply
    AccessDenied
    InvalidArgs
    DbusProtocolError
)

type DbusError -> Text
func describeBusError = error => error
 ||> NameNotOwned name     -> "name not owned: {name}"
 ||> ServiceUnknown name   -> "service not found: {name}"
 ||> NoReply               -> "service did not reply"
 ||> AccessDenied msg      -> "access denied: {msg}"
 ||> InvalidArgs msg       -> "invalid arguments: {msg}"
 ||> DbusProtocolError msg -> "D-Bus protocol error: {msg}"
```

---

## `DbusCallResult` and `DbusTask`

```aivi
type DbusCallResult = Result DbusError (List DbusValue)

type DbusTask A = Task DbusError A
```

Use `DbusCallResult` when you want the raw values returned by a method call. Use `DbusTask A` when a higher-level helper decodes those values into some application type `A`.
