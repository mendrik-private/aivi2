# aivi.time

Clock, timestamp, and millisecond helpers.

`aivi.time` mixes two kinds of tools:

- task-based functions that ask the runtime for clock or timestamp work
- pure helpers for doing ordinary millisecond math in your own code

Unlike [`aivi.duration`](/stdlib/duration), this module does **not** define a time domain today.
`EpochMs` is still just an `Int` alias.

## Import

```aivi
use aivi.time (
    EpochMs
    nowMs
    monotonicMs
    format
    parse
    isoPattern
    datePattern
    timePattern
    formatIso
    formatDate
    formatTime
    parseIso
    msPerSecond
    msPerMinute
    msPerHour
    msPerDay
    toSeconds
    toMinutes
    toHours
    toDays
    fromSeconds
    fromMinutes
    fromHours
    fromDays
    elapsed
)
```

## Runtime clock functions

| Value | Type | Description |
| --- | --- | --- |
| `nowMs` | `Task Text Int` | Current wall-clock time in milliseconds since the Unix epoch |
| `monotonicMs` | `Task Text Int` | Monotonic milliseconds since the runtime started |
| `format` | `Int -> Text -> Task Text Text` | Format a timestamp using a pattern |
| `parse` | `Text -> Text -> Task Text Int` | Parse text into a timestamp |

### `nowMs`

```aivi
// <unparseable item>
```

Use this when you need a real-world timestamp for storage, logging, or comparing with other
epoch-based values.

### `monotonicMs`

```aivi
// <unparseable item>
```

Use this when you want a steady clock for measuring elapsed time inside the running program.
It is a better fit for timing than `nowMs`, because it is not tied to the wall clock.

```aivi
use aivi.time (
    nowMs
    monotonicMs
)

value savedAt : Task Text Int = nowMs
value stopwatchNow : Task Text Int = monotonicMs
```

## Current runtime note for `format` and `parse`

The API surface is already present, but the current runtime behavior is intentionally small:

- `format` ignores the pattern argument and returns the millisecond number as plain text
- `parse` ignores the pattern argument and only accepts text that is already an integer
  millisecond value

That means `formatIso`, `formatDate`, `formatTime`, and `parseIso` currently share the same
fallback behavior.

## Current limits

- timestamps are raw epoch-millisecond `Int` values, not a dedicated domain
- `format` and `parse` are still partial runtime stubs
- there is no richer calendar/date-time domain surface yet

### `format`

```aivi
// <unparseable item>
```

The surface API takes an epoch millisecond value and a pattern string.

```aivi
use aivi.time (
    format
    isoPattern
)

value shown : Task Text Text = format 1735689600000 isoPattern
```

Today this returns `"1735689600000"`, not a human-readable ISO timestamp yet.

### `parse`

```aivi
// <unparseable item>
```

The surface API takes text plus a pattern string and returns epoch milliseconds.

```aivi
use aivi.time (
    parse
    isoPattern
)

value parsed : Task Text Int = parse "1735689600000" isoPattern
```

Today this succeeds for decimal millisecond text and fails for ordinary date strings such as
`"2025-01-01T00:00:00"`.

## Pattern constants and wrappers

| Value | Type | Description |
| --- | --- | --- |
| `isoPattern` | `Text` | Named pattern string for ISO-like timestamps |
| `datePattern` | `Text` | Named pattern string for dates |
| `timePattern` | `Text` | Named pattern string for times |
| `formatIso` | `Int -> Task Text Text` | `format ms isoPattern` |
| `formatDate` | `Int -> Task Text Text` | `format ms datePattern` |
| `formatTime` | `Int -> Task Text Text` | `format ms timePattern` |
| `parseIso` | `Text -> Task Text Int` | `parse text isoPattern` |

These names make intent clearer even before the full formatter/parser behavior lands.

## Pure millisecond helpers

| Value | Type | Description |
| --- | --- | --- |
| `EpochMs` | `Int` | Type alias for epoch milliseconds |
| `msPerSecond` | `Int` | `1000` |
| `msPerMinute` | `Int` | `60000` |
| `msPerHour` | `Int` | `3600000` |
| `msPerDay` | `Int` | `86400000` |
| `toSeconds` | `Int -> Int` | Convert milliseconds to whole seconds |
| `toMinutes` | `Int -> Int` | Convert milliseconds to whole minutes |
| `toHours` | `Int -> Int` | Convert milliseconds to whole hours |
| `toDays` | `Int -> Int` | Convert milliseconds to whole days |
| `fromSeconds` | `Int -> Int` | Convert seconds to milliseconds |
| `fromMinutes` | `Int -> Int` | Convert minutes to milliseconds |
| `fromHours` | `Int -> Int` | Convert hours to milliseconds |
| `fromDays` | `Int -> Int` | Convert days to milliseconds |
| `elapsed` | `Int -> Int -> Int` | Subtract `start` from `finish` |

```aivi
use aivi.time (
    fromSeconds
    fromMinutes
    elapsed
    toSeconds
)

value timeoutMs : Int = fromSeconds 30
value cacheTtlMs : Int = fromMinutes 5
value requestTimeMs : Int = elapsed 1200 1875
value requestTimeSeconds : Int = toSeconds requestTimeMs
```

## Example — wall clock plus steady clock

```aivi
use aivi.time (
    nowMs
    monotonicMs
    elapsed
)

value createdAt : Task Text Int = nowMs
value timerStart : Int = 1000
value timerNow : Int = 1450
value timerElapsed : Int = elapsed timerStart timerNow
value steadySnapshot : Task Text Int = monotonicMs
```
