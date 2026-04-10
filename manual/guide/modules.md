# Modules

Each `.aivi` file is a module. Modules import names with `use` and expose names with `export`.

## Importing with `use`

```aivi
use aivi.http (
    HttpError
    HttpResponse
)

use aivi.result (isOk)
```

Imported names become available to the rest of the file.

## Import aliases

Use `as` when you want a local name that differs from the exported one:

```aivi
use aivi.http (
    HttpError as FetchError
    HttpResponse as Response
)

type Response Text -> Bool
func isSuccess = resp =>
    isOk resp
```

## Exporting names

You can export one name:

```aivi
value greeting = "hello"

export greeting
```

Or several names together:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

type Direction -> Direction
func opposite = .
 ||> Up    -> Down
 ||> Down  -> Up
 ||> Left  -> Right
 ||> Right -> Left

value startDirection : Direction = Right

export (Direction, opposite, startDirection)
```

## A small complete module

```aivi
use aivi.text (
    trim
    toUpper
)

type Text -> Text -> Text
func joinLabels = left right =>
    "{left}/{right}"

value primaryLabel = "primary"
value fallbackLabel = "fallback"
value labelPair = joinLabels primaryLabel fallbackLabel

export labelPair
```

## Typical module layout

A practical order is:

1. `use`
2. `type` / `domain` / `class`
3. `func` and `value`
4. `signal`
5. `export`

That ordering is not required by the language, but it keeps modules easy to scan.

## Publishing names project-wide with `hoist`

`hoist` is a **self-declaration**: a module declares that its own exports should be lifted into the project-wide namespace. Every other `.aivi` file in the project can then use those names directly, without any `use` statement.

```aivi
// libs/types/ids.aivi
hoist

domain AccountId over Text
```

```aivi
// libs/types/mail.aivi
hoist

type Message = {}
```

Any file in the project can then use `AccountId`, `Message`, etc. without any `use` statement.
To show the contrast — a consuming file needs no imports at all:

```text
// apps/ui/view.aivi — no use or hoist needed here
type AccountId -> Text -> Widget
func accountLabel = id label => ...
```

### The stdlib prelude

Each AIVI standard library module declares its own `hoist`:

```aivi
// stdlib/aivi/list.aivi
hoist
```

This means `map`, `filter`, `length`, `getOrElse`, `isOk`, and the rest of those modules are available in every AIVI project without any `use` declaration.

### Kind filters

When you only want to publish specific kinds of exports:

```aivi
hoist (func, value)
```

Valid kind filters: `func`, `value`, `signal`, `type`, `domain`, `class`.

### Hiding specific names

Suppress individual names from the hoist:

```aivi
hoist hiding (head, tail)
```

Combine kind filters and hiding:

```aivi
hoist (func) hiding (foldr, foldl)
```

### Name disambiguation

When two hoisted modules export the same name (e.g. `map` from both `aivi.list` and `aivi.option`), the compiler picks the right one from type context:

```aivi
value numbers = [1, 2, 3]
value doubled
value maybeName = Some "Alice"
value greeting
```

If the type context is insufficient, the compiler reports an error and suggests using `hiding` to exclude the conflicting name from one of the hoisted modules.

### Priority order

```
local definitions > use imports > hoisted globals > ambient prelude
```

`use` always wins over `hoist` for the same name, so you can override a hoisted name locally for a specific file.

## Summary

| Form | Meaning |
| --- | --- |
| `use module (names)` | Import selected names into this file |
| `use module (name as localName)` | Import one name under a local alias |
| `export name` | Export one name to importers |
| `export (a, b, c)` | Export several names to importers |
| `hoist` | Publish this module's exports to the whole project |
| `hoist (func, value)` | Publish only selected kinds project-wide |
| `hoist hiding (a, b)` | Publish all except named items project-wide |
