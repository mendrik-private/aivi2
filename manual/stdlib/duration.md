# aivi.duration

Typed time spans.

`aivi.duration` gives you a `Duration` domain instead of passing around plain `Int` values.
That makes time-related code easier to read: `5sec` says more than `5000`.

A `Duration` is a domain over `Int`, so construction is explicit; the carrier is always accessible via `.carrier`.

## Import

```aivi
use aivi.duration (
    Duration
    DurationError
)
```

Because `aivi.duration` declares `hoist`, the literal suffixes (`ms`, `sec`, `min`, `hr`, `dy`) and constructor helpers (`millis`, `trySeconds`) are available project-wide in every AIVI file without any `use` statement. Import `Duration` explicitly when you need the type name in annotations, and `DurationError` when you need to handle construction failures.

## Overview

| Member | Type | Description |
| --- | --- | --- |
| `ms` | `Int -> Duration` | Literal suffix for milliseconds, as in `250ms` |
| `sec` | `Int -> Duration` | Literal suffix for seconds, as in `5sec` |
| `min` | `Int -> Duration` | Literal suffix for minutes, as in `2min` |
| `hr` | `Int -> Duration` | Literal suffix for hours, as in `1hr` |
| `dy` | `Int -> Duration` | Literal suffix for days, as in `7dy` |
| `millis` | `Int -> Duration` | Build a duration from a raw millisecond count |
| `trySeconds` | `Int -> Result DurationError Duration` | Smart constructor that can fail |
| `(+)` | `Duration -> Duration -> Duration` | Add two durations |
| `(-)` | `Duration -> Duration -> Duration` | Subtract one duration from another |
| `(*)` | `Duration -> Int -> Duration` | Multiply a duration by a whole number |
| `(<)` | `Duration -> Duration -> Bool` | Compare two durations |

## Literal suffixes

The shortest way to make a duration is with a suffix literal:

```aivi
use aivi.duration (Duration)

value debounce : Duration = 250ms
value retryDelay : Duration = 5sec
value sessionLength : Duration = 30min
value backupWindow : Duration = 1hr
value trialPeriod : Duration = 14dy
```

These values stay typed as `Duration`, so they are harder to confuse with unrelated `Int`
values elsewhere in your program.

## Constructors

### `millis`

```aivi
```

Build a duration from a raw millisecond count.

```aivi
use aivi.duration (Duration)

value shortDelay : Duration = millis 150
```

### `trySeconds`

```aivi
```

A safe constructor for whole seconds. Use this when you want construction to report a
`DurationError` instead of assuming the input is valid.

```aivi
use aivi.duration (
    Duration
    DurationError
)

value pollInterval : Result DurationError Duration = trySeconds 10
```

## `.carrier`

Access the raw `Int` carrier. In this module the direct constructor is `millis`, so this is
the millisecond count that backs the duration value.

```aivi
use aivi.duration (Duration)

value totalWait : Duration = 1min + 30sec
value totalWaitMs : Int = totalWait.carrier
```

## Operators

The `Duration` domain includes a small set of arithmetic and comparison operators.

```aivi
use aivi.duration (Duration)

value total : Duration = 45sec + 15sec
value remaining : Duration = total - 10sec
value doubled : Duration = 250ms * 2
value isShorter : Bool = 30sec < 1min
value rawMs : Int = doubled.carrier
```

## Error type

```aivi
type DurationError = Text
```

When a smart constructor fails, the module reports a plain text message.

## Example — readable scheduling values

```aivi
use aivi.duration (Duration)

value animationFrame : Duration = 16ms
value autosaveEvery : Duration = 30sec
value timeout : Duration = 2min
value timeoutMs : Int = timeout.carrier
```
