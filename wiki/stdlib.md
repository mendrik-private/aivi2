# Standard Library

Overview of the AIVI standard library modules. All modules live in `stdlib/aivi/`. Documentation for each module lives in `manual/stdlib/`.

## Core Abstractions

| Module | Description |
|--------|-------------|
| `prelude` | Auto-imported: `Option`, `Result`, common functions, `Eq`/`Ord` basics |
| `bool` | Boolean operations |
| `option` | `Option A` type — `Some A \| None` |
| `result` | `Result A E` type — `Ok A \| Err E` |
| `either` | `Either A B` — left/right union |
| `pair` | `Pair A B` — product type with preferred `first` / `second` / `mapFirst` / `mapSecond` names and compatibility aliases `fst` / `snd` / `mapFst` / `mapSnd` |
| `validation` | Accumulating validation type |

## Collections

| Module | Description |
|--------|-------------|
| `list` | `List A` — immutable linked list; `Functor`, `Foldable`, `Filterable`, `Traversable`, plus `indexed` / `mapWithIndex` / `reduceWithIndex` / `filterMap` helpers |
| `nonEmpty` | `NonEmpty A` — list guaranteed non-empty |
| `dict` | `Dict K V` — key-value map |
| `matrix` | `Matrix A` — 2D grid with `MatrixIndex`, indexed traversal helpers, and user-authored `Functor` / `Foldable` instances |

**Note**: Generic `==` requires an `Eq` constraint at the definition site. For polymorphic list operations (e.g. `contains`), pass an explicit `eq` comparator function. See `stdlib/aivi/list.aivi`.

## Text & Numbers

| Module | Description |
|--------|-------------|
| `text` | `Text` — UTF-8 string operations |
| `bigint` | `BigInt` — arbitrary-precision integers |
| `arithmetic` | Compiler-backed named integer arithmetic intrinsics (`add`, `sub`, `mul`, `div`, `mod`, `neg`) |
| `bits` | Compiler-backed named bitwise intrinsics (`and`, `or`, `xor`, `not`, shifts) |
| `math` | Mathematical functions |
| `regex` | Regular expressions |
| `random` | Pseudo-random number generation |
| `data/json` | Structural `Json` vocabulary plus task-backed text helpers like `validate`, `get`, `pretty` |

## Time & Duration

| Module | Description |
|--------|-------------|
| `time` | `Time` — instant in time |
| `date` | `Date` — calendar date |
| `duration` | `Duration` — time interval domain type |
| `timer` | Timer source (fires periodically) |

## I/O & System

| Module | Description |
|--------|-------------|
| `fs` | File system access (read, write, watch) |
| `http` | HTTP client (GET, POST, etc.) |
| `env` | Environment variables |
| `path` | File path manipulation |
| `process` | External process execution |
| `stdio` | Standard input/output |
| `log` | Application logging |

## UI & Desktop

| Module | Description |
|--------|-------------|
| `app` | Application lifecycle (`stdlib/aivi/app/`) |
| `color` | `Color` type and operations |
| `image` | Image loading and display |
| `clipboard` | Clipboard read/write |
| `portal` | XDG portal integration (file chooser, etc.) |
| `desktop` | Desktop integration helpers |
| `gnome` | GNOME-specific APIs (`stdlib/aivi/gnome/`) |
| `gresource` | GResource bundle access |
| `i18n` | Internationalisation / gettext |

## Data & Services

| Module | Description |
|--------|-------------|
| `db` | SQLite database access |
| `dbus` | D-Bus method calls and signals |
| `api` | Shared auth/error vocabulary for `@source api` and generated OpenAPI handles |
| `imap` | IMAP email client |
| `smtp` | SMTP email sending |
| `url` | URL parsing and construction |
| `auth` | OAuth / authentication flows |
| `defaults` | GSettings/defaults persistence |

## Utilities

| Module | Description |
|--------|-------------|
| `fn` | Higher-order function utilities |
| `order` | `Ord` class and ordering helpers |
| `bytes` | `Bytes` — raw byte sequences |

## Subdirectories

| Path | Description |
|------|-------------|
| `stdlib/aivi/app/` | App-level constructs |
| `stdlib/aivi/core/` | Core language primitives |
| `stdlib/aivi/data/` | Data type utilities |
| `stdlib/aivi/desktop/` | Desktop/GNOME integration |
| `stdlib/aivi/gnome/` | GNOME-specific modules |

## Prelude Auto-Import

`prelude.aivi` is implicitly imported into every AIVI module. It re-exports:
- `Option`, `Result`, `Either`, `Pair`, `NonEmpty`
- `List` operations
- `Eq`, `Ord`, `Functor`, `Foldable`, common operators
- `contains` — takes an explicit `eq` comparator: `contains eq list value`
- pair helpers `first`, `second`, `mapFirst`, `mapSecond` plus compatibility aliases

Recent ergonomics additions:

- `aivi.option`: `fold`, `mapOr`, `isSomeAnd`
- `aivi.list`: `indexed`, `mapWithIndex`, `reduceWithIndex`, `filterMap`
- `aivi.matrix`: `coord`, `coords`, `entries`, `positionsWhere`, `count`, `modifyAt`, `replaceMany`
- `aivi.pair`: preferred `first`, `second`, `mapFirst`, `mapSecond` aliases over older `fst`, `snd`, `mapFst`, `mapSnd`
- low-level compiler-backed modules now documented in the manual: `aivi.api`, `aivi.arithmetic`, `aivi.bits`, `aivi.data.json`
- `aivi.prelude`: ambient `foldOption`, `mapOr`, `isSomeAnd`, `indexed`, `mapWithIndex`, `reduceWithIndex`, `count`, `findMap`, `first`, `second`, `mapFirst`, `mapSecond`

*See also: [indexed-collections.md](indexed-collections.md), [type-system.md](type-system.md), [signal-model.md](signal-model.md)*
