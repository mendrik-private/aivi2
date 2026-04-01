# Standard Library

The AIVI standard library is the set of modules that ships with the language. Some names, such
as `Option`, `Result`, `Signal`, and `Task`, are built in. Most other tools live in named modules
that you import with `use`.

Use this page as a map:

- start with the modules in **Start here** if you are new to AIVI
- use the grouped lists below when you already know the job you need to do
- treat each linked page as the source of truth for what the current stdlib actually exposes

Several modules are intentionally small today, and some are shared type vocabularies rather than
full runtime clients. Their pages call that out up front so you do not have to guess.

After [`aivi.prelude`](/stdlib/prelude), the modules most people reach for first are
[`aivi.option`](/stdlib/option), [`aivi.result`](/stdlib/result), [`aivi.list`](/stdlib/list),
[`aivi.text`](/stdlib/text), and [`aivi.math`](/stdlib/math).

## Built-in types you will see often

| Type | Meaning | More |
|---|---|---|
| `Ordering` | The result of comparing two values: `Less`, `Equal`, or `Greater`. | [`aivi.order`](/stdlib/order) |
| `Option A` | A value that may be missing. | [`aivi.option`](/stdlib/option) |
| `Result E A` | A success value (`Ok`) or a failure value (`Err`). | [`aivi.result`](/stdlib/result) |
| `Validation E A` | Like `Result`, but useful when checking several independent inputs. | [`aivi.validation`](/stdlib/validation) |
| `Signal A` | A reactive value that changes over time. | [Signals guide](/guide/signals) |
| `Task E A` | Runtime work that may fail with `E` or succeed with `A`. | Used across I/O-oriented stdlib modules |

## Browse modules by area

### Start here

- [`aivi.prelude`](/stdlib/prelude) — a good first stop for everyday imports.
- [`aivi.defaults`](/stdlib/defaults) — named empty/default values for common built-in types.

### Core values and collections

- [`aivi.bool`](/stdlib/bool) — boolean helpers.
- [`aivi.option`](/stdlib/option) — values that may be missing.
- [`aivi.result`](/stdlib/result) — success-or-error values.
- [`aivi.validation`](/stdlib/validation) — validation values for user input and other checks.
- [`aivi.core.either`](/stdlib/either) — values that can hold one of two branches.
- [`aivi.list`](/stdlib/list) — list helpers.
- [`aivi.nonEmpty`](/stdlib/nonEmpty) — lists that always contain at least one item.
- [`aivi.pair`](/stdlib/pair) — two values grouped together.
- [`aivi.order`](/stdlib/order) — comparison results and ordering helpers.
- [`aivi.core.dict`](/stdlib/dict) — key/value dictionaries.
- [`aivi.core.set`](/stdlib/set) — collections of unique values.
- [`aivi.core.range`](/stdlib/range) — numeric ranges.
- [`aivi.core.fn`](/stdlib/fn) — small function and pipeline helpers.

### Numbers, text, and data

- [`aivi.math`](/stdlib/math) — everyday arithmetic helpers.
- [`aivi.core.float`](/stdlib/float) — floating-point numbers.
- [`aivi.bigint`](/stdlib/bigint) — integers that can grow past the normal `Int` range.
- [`aivi.text`](/stdlib/text) — text helpers.
- [`aivi.regex`](/stdlib/regex) — regular-expression matching and replacement.
- [`aivi.core.bytes`](/stdlib/bytes) — byte buffers.
- [`aivi.data.json`](/stdlib/json) — JSON values and helpers.

### Time, randomness, and scheduling

- [`aivi.duration`](/stdlib/duration) — typed time spans such as `5sec`.
- [`aivi.time`](/stdlib/time) — clocks, timestamps, and time formatting helpers.
- [`aivi.timer`](/stdlib/timer) — marker types for timer-backed signals.
- [`aivi.random`](/stdlib/random) — random bytes and random integers.

### Files, environment, and processes

- [`aivi.fs`](/stdlib/fs) — filesystem events and watcher-related types.
- [`aivi.path`](/stdlib/path) — checked path values.
- [`aivi.env`](/stdlib/env) — environment-variable lookups.
- [`aivi.stdio`](/stdlib/stdio) — write to standard output and standard error.
- [`aivi.log`](/stdlib/log) — runtime logging helpers.
- [`aivi.process`](/stdlib/process) — process-related records, results, and errors.

### Network and services

Some modules in this group are full helpers, and some are shared data shapes for integrations.
The linked pages spell out which functions exist today.

- [`aivi.url`](/stdlib/url) — typed URLs and helpers for their parts.
- [`aivi.http`](/stdlib/http) — HTTP request helpers.
- [`aivi.auth`](/stdlib/auth) — OAuth / PKCE sign-in records and state types.
- [`aivi.db`](/stdlib/db) — database connections, statements, paging, and errors.
- [`aivi.imap`](/stdlib/imap) — mailbox folders, sync state, and mail events.
- [`aivi.smtp`](/stdlib/smtp) — outgoing mail settings, messages, and errors.

### Desktop, UI, and GNOME

Many pages in this group describe task aliases, watcher/source shapes, or shared desktop data
types rather than a full feature API. They are still the right place to look when wiring a Linux
desktop app together.

- [`aivi.app`](/stdlib/app) — application framework types.
- [`aivi.app.lifecycle`](/stdlib/lifecycle) — lifecycle state, commands, undo state, and in-app notifications.
- [`aivi.desktop.xdg`](/stdlib/xdg) — standard Linux app directories.
- [`aivi.portal`](/stdlib/portal) — desktop portal results for file picking, opening URIs, and screenshots.
- [`aivi.dbus`](/stdlib/dbus) — D-Bus values, calls, signals, and match rules.
- [`aivi.gnome.settings`](/stdlib/settings) — GSettings schema, key, and value types.
- [`aivi.gnome.onlineAccounts`](/stdlib/onlineAccounts) — desktop account and token records.
- [`aivi.gnome.notifications`](/stdlib/notifications) — desktop notification payloads and responses.
- [`aivi.clipboard`](/stdlib/clipboard) — clipboard content types and watcher shapes.
- [`aivi.color`](/stdlib/color) — packed UI colors and channel helpers.
- [`aivi.image`](/stdlib/image) — image data, metadata, and load errors.
- [`aivi.gresource`](/stdlib/gresource) — bundled resource paths and load errors.
- [`aivi.i18n`](/stdlib/i18n) — translation marker helpers.

## Common interfaces (typeclasses)

If you have not used typeclasses before, think of them as shared capabilities that different
types can implement. The table below describes the current built-in support in the executable
language/runtime slice.

| Interface | What it gives you | Built-in support includes |
|---|---|---|
| `Eq A` | Equality via `==` and `!=` | primitive scalars, `Ordering`, `Option`, `Result`, `Validation`, `List` |
| `Ord A` | Ordering via `compare` | `Int`, `Text`, `Ordering` |
| `Default A` | A fallback value via `default` | same-module `Default` instances; `Option` omission via `use aivi.defaults (Option)`; `Text` / `Int` / `Bool` omission via `use aivi.defaults (defaultText, defaultInt, defaultBool)` |
| `Functor F` | Mapping over a wrapped value | `Option`, `Result`, `List`, `Validation`, `Signal` |
| `Semigroup A` | Combining two values with `<>` | `Text`, `List` |
| `Monoid A` | An identity value via `empty` | `Text`, `List` |
| `Foldable F` | Reducing a structure to one value | `List`, `Option`, `Result`, `Validation` |
| `Filterable F` | Keeping some values while dropping others | `List`, `Option` |
| `Apply F` | Applying wrapped functions to wrapped values | `Option`, `Result`, `List`, `Validation`, `Signal` |
| `Applicative F` | Lifting plain values into a context with `pure` | `Option`, `Result`, `List`, `Validation`, `Signal`, `Task` |
| `Monad F` | Chaining context-producing steps | `List`, `Option`, `Result` |
| `Bifunctor F` | Mapping both sides of a two-parameter type | `Result`, `Validation` |
| `Traversable F` | Walking a structure while building effects | `List`, `Option`, `Result`, `Validation` |

This table describes the current executable built-in slice. For the full higher-kinded hierarchy,
support boundaries, and the current same-module-only limits for user-authored higher-kinded
classes and instances, see [Typeclasses & Higher-Kinded Support](/guide/typeclasses).
