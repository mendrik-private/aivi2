# aivi.timer

Marker types for timer-backed signals.

`aivi.timer` is intentionally tiny. Its job is to give clear names to the values produced by
timer sources, so your signal declarations read like documentation.

## Import

```aivi
use aivi.timer (
    TimerTick
    TimerReady
    TimerMode
    Repeating
    OneShot
)
```

## Overview

| Type | Meaning |
| --- | --- |
| `TimerTick` | The payload published by repeating timer signals |
| `TimerReady` | The payload published by one-shot timer signals |
| `TimerMode` | A small enum for your own timer-related state |

## `TimerTick`

```aivi
type TimerTick = Unit
```

Use this when a signal comes from `@source timer.every`.

## `TimerReady`

```aivi
type TimerReady = Unit
```

Use this when a signal comes from `@source timer.after`.

`TimerTick` and `TimerReady` are both `Unit` under the hood. The different names are there to
make intent obvious at the call site.

## `TimerMode`

```aivi
type TimerMode =
  | Repeating
  | OneShot
```

This is useful when your own application state needs to say whether something should repeat
or fire only once.

```aivi
use aivi.timer (
    TimerMode
    Repeating
    OneShot
)

type TimerMode -> Text
func describeMode = mode => mode
 ||> Repeating -> "repeat"
 ||> OneShot   -> "run once"
```

## Example — timer source signals

```aivi
use aivi.timer (
    TimerTick
    TimerReady
)

@source timer.every 120 with {
    immediate: True
}
signal tick : Signal TimerTick

@source timer.after 1000
signal ready : Signal TimerReady
```

These examples use the currently exercised integer-millisecond form. See the source guide for
option details such as `immediate` and for the broader timer source rules.
