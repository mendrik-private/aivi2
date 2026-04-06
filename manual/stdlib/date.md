# aivi.date

Calendar dates, times, and day-of-week types.

`aivi.date` provides structured date and time types with named positional fields,
day-of-week enumeration, date comparison helpers, ISO 8601 formatting, and a
`DateDelta` domain for day-level arithmetic.

## Import

```aivi
use aivi.date (
    Year
    Month
    Day
    Hour
    Minute
    Second
    Date
    TimeOfDay
    DateTime
    ZonedDateTime
    DayOfWeek
    DateDelta
    Monday
    Tuesday
    Wednesday
    Thursday
    Friday
    Saturday
    Sunday
    getYear
    getMonth
    getDay
    getHour
    getMinute
    getSecond
    getDate
    getTime
    getDateTime
    getZone
    isLeapYear
    daysInFeb
    daysInMonth
    dateToIso
    timeToIso
    dateTimeToIso
    zonedToIso
    dateEq
    dateLt
    dayOfWeekName
    dayOfWeekShort
    dayOfWeekIndex
    toDateTime
    toZoned
    midnight
    epoch
)
```

## Types

### Wrapper aliases

| Name | Definition | Description |
| --- | --- | --- |
| `Year` | `Int` | Calendar year |
| `Month` | `Int` | Month number (1–12) |
| `Day` | `Int` | Day of month (1–31) |
| `Hour` | `Int` | Hour (0–23) |
| `Minute` | `Int` | Minute (0–59) |
| `Second` | `Int` | Second (0–59) |

### Product types

```aivi
type Date =
  Date year:Year month:Month day:Day

type TimeOfDay =
  TimeOfDay hour:Hour minute:Minute second:Second

type DateTime =
  DateTime date:Date time:TimeOfDay

type ZonedDateTime =
  ZonedDateTime dateTime:DateTime zone:Text
```

Construction is positional:

```aivi
value today = Date 2024 6 15
value noon = TimeOfDay 12 0 0
value now = DateTime today noon
value utcNow = ZonedDateTime now "+00:00"
```

### DayOfWeek

```aivi
type DayOfWeek =
  | Monday
  | Tuesday
  | Wednesday
  | Thursday
  | Friday
  | Saturday
  | Sunday
```

## DateDelta domain

`DateDelta` wraps an `Int` representing a number of days.

```aivi
domain DateDelta over Int
```

| Literal | Example | Description |
| --- | --- | --- |
| `dy` | `7dy` | Days |
| `wk` | `2wk` | Weeks |

| Member | Type | Description |
| --- | --- | --- |
| `days` | `Int → DateDelta` | Wrap a day count |
| `(+)` | `DateDelta → DateDelta → DateDelta` | Add deltas |
| `(-)` | `DateDelta → DateDelta → DateDelta` | Subtract deltas |
| `(*)` | `DateDelta → Int → DateDelta` | Scale a delta |
| `(<)` | `DateDelta → DateDelta → Bool` | Compare deltas |

## Accessors

| Function | Type | Description |
| --- | --- | --- |
| `getYear` | `Date → Year` | Extract the year |
| `getMonth` | `Date → Month` | Extract the month |
| `getDay` | `Date → Day` | Extract the day |
| `getHour` | `TimeOfDay → Hour` | Extract the hour |
| `getMinute` | `TimeOfDay → Minute` | Extract the minute |
| `getSecond` | `TimeOfDay → Second` | Extract the second |
| `getDate` | `DateTime → Date` | Extract the date part |
| `getTime` | `DateTime → TimeOfDay` | Extract the time part |
| `getDateTime` | `ZonedDateTime → DateTime` | Extract the date-time |
| `getZone` | `ZonedDateTime → Text` | Extract the timezone |

## Calendar helpers

| Function | Type | Description |
| --- | --- | --- |
| `isLeapYear` | `Year → Bool` | Gregorian leap year test |
| `daysInFeb` | `Year → Day` | 28 or 29 depending on leap year |
| `daysInMonth` | `Month → Year → Day` | Number of days in a given month |

```aivi
// <unparseable item>
```

## Formatting

| Function | Type | Description |
| --- | --- | --- |
| `dateToIso` | `Date → Text` | `"2024-06-15"` |
| `timeToIso` | `TimeOfDay → Text` | `"14:30:00"` |
| `dateTimeToIso` | `DateTime → Text` | `"2024-06-15T14:30:00"` |
| `zonedToIso` | `ZonedDateTime → Text` | `"2024-06-15T14:30:00+00:00"` |

```aivi
// <unparseable item>
```

## Comparison

| Function | Type | Description |
| --- | --- | --- |
| `dateEq` | `Date → Date → Bool` | Structural equality |
| `dateLt` | `Date → Date → Bool` | Chronological less-than |

## Constructors

| Function | Type | Description |
| --- | --- | --- |
| `toDateTime` | `Date → TimeOfDay → DateTime` | Combine date and time |
| `toZoned` | `DateTime → Text → ZonedDateTime` | Attach timezone |

## Constants

| Value | Type | Description |
| --- | --- | --- |
| `midnight` | `TimeOfDay` | `TimeOfDay 0 0 0` |
| `epoch` | `Date` | `Date 1970 1 1` |

## DayOfWeek helpers

| Function | Type | Description |
| --- | --- | --- |
| `dayOfWeekName` | `DayOfWeek → Text` | Full name: `"Monday"` |
| `dayOfWeekShort` | `DayOfWeek → Text` | Short name: `"Mon"` |
| `dayOfWeekIndex` | `DayOfWeek → Int` | ISO index: Monday=1 … Sunday=7 |

## Example

```aivi
use aivi.date (
    Date
    DateTime
    TimeOfDay
    ZonedDateTime
    dateToIso
    dateTimeToIso
    isLeapYear
    daysInMonth
    midnight
    toDateTime
    toZoned
)

value birthday = Date 1990 3 14
value label = dateToIso birthday
value feb = daysInMonth 2 2024
value meeting = toDateTime (Date 2024 12 25) midnight
value utcMeeting = toZoned meeting "+00:00"
```

## Notes

- All types are purely structural — no runtime intrinsics are needed for construction, accessors,
  or formatting.
- `DateDelta` is a domain over `Int`. Its operator implementations are provided by the runtime,
  following the same pattern as `aivi.duration.Duration`.
- Month and day values are not range-checked at the type level. `Date 2024 13 32` is syntactically
  valid but semantically meaningless.
