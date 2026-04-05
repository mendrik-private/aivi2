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

For external systems, prefer provider capabilities defined through `@source`. The related stdlib
modules now mainly carry shared data vocabulary and handle-marker types such as `FsSource`,
`HttpSource`, `EnvSource`, and `LogSource`.

After [`aivi.prelude`](/stdlib/prelude), the modules most people reach for first are
[`aivi.option`](/stdlib/option), [`aivi.result`](/stdlib/result), [`aivi.list`](/stdlib/list),
[`aivi.text`](/stdlib/text), and [`aivi.math`](/stdlib/math).

## At a glance

| Module | Description | Key exports |
| --- | --- | --- |
| [aivi.prelude](prelude.md) | Convenience re-exports and built-in types | `Option`, `Result`, `Signal`, `Eq`, `Ord`, `Functor` |
| [aivi.option](option.md) | Values that may be missing | `map`, `withDefault`, `andThen`, `fromResult` |
| [aivi.result](result.md) | Success-or-error values | `map`, `mapErr`, `andThen`, `onOk`, `onErr` |
| [aivi.validation](validation.md) | Accumulating validation for independent inputs | `zipValidation`, `andThen`, `onValid`, `onInvalid` |
| [aivi.core.either](either.md) | Disjoint union holding one of two alternatives | `Left`, `Right`, `mapLeft`, `mapRight` |
| [aivi.list](list.md) | Purely functional list operations | `map`, `filter`, `fold`, `append`, `flatten` |
| [aivi.nonEmpty](nonEmpty.md) | Non-empty list guaranteed at the type level | `head`, `last`, `singleton`, `cons`, `fromList` |
| [aivi.pair](pair.md) | Two-element tuples | `first`, `second`, `mapFirst`, `mapSecond` |
| [aivi.matrix](matrix.md) | Rectangular 2D collections | `init`, `fromRows`, `width`, `height`, `rows` |
| [aivi.core.dict](dict.md) | Association map keyed by any `Eq` type | `entries`, `merge`, `combine` |
| [aivi.core.set](set.md) | Unordered set for any `Eq` type | `singleton`, `member`, `insert`, `union` |
| [aivi.core.range](range.md) | Inclusive integer range `[start, end]` | `start`, `end`, `toList`, `contains` |
| [aivi.core.fn](fn.md) | Higher-order function combinators | `compose`, `const`, `flip`, `combine` |
| [aivi.order](order.md) | Comparison utilities parameterised by a comparator | `compare`, `minOf`, `maxOf`, `descending` |
| [aivi.bool](bool.md) | Boolean helpers | `and`, `or`, `not`, `all`, `any` |
| [aivi.defaults](defaults.md) | Default values for common types | `defaultText`, `defaultInt`, `defaultBool` |
| [aivi.math](math.md) | Integer arithmetic utilities | `abs`, `clamp`, `min`, `max`, `gcd` |
| [aivi.core.float](float.md) | IEEE 754 double-precision helpers | `floor`, `ceil`, `round`, `sqrt`, `pi` |
| [aivi.bigint](bigint.md) | Arbitrary-size integers | `parse`, `plus`, `times`, `dividedBy` |
| [aivi.text](text.md) | Text manipulation | `length`, `contains`, `trim`, `split`, `toUpper` |
| [aivi.regex](regex.md) | Regular-expression matching and replacement | `matches`, `hasMatch`, `replaceFirst`, `allMatches` |
| [aivi.core.bytes](bytes.md) | Byte sequence operations | `fromText`, `toText`, `slice`, `append` |
| [aivi.duration](duration.md) | Typed time spans | `ms`, `sec`, `min`, `hr`, `millis` |
| [aivi.time](time.md) | Clock, timestamp, and formatting helpers | `nowMs`, `monotonicMs`, `format`, `parse` |
| [aivi.timer](timer.md) | Marker types for timer-backed signals | `immediate` |
| [aivi.random](random.md) | Randomness vocabulary and `RandomSource` | `randomInt`, `randomFloat`, `randomBytes` |
| [aivi.fs](fs.md) | Filesystem vocabulary and `FsSource` | `readText`, `writeText`, `deleteFile` |
| [aivi.path](path.md) | Lexical path manipulation | `join`, `basename`, `dirname`, `extension` |
| [aivi.env](env.md) | Environment vocabulary and `EnvSource` | `get`, `getAll`, `EnvSource` |
| [aivi.stdio](stdio.md) | Standard I/O vocabulary and `StdioSource` | `StdioSource`, `StdinLine`, `stdout` |
| [aivi.log](log.md) | Logging vocabulary and `LogSource` | `levelToText`, `kv`, `LogSource` |
| [aivi.process](process.md) | Process vocabulary and `ProcessSource` | `command`, `args`, `workingDir`, `env` |
| [aivi.url](url.md) | Typed URLs with explicit parsing | `parse`, `unwrap`, `scheme`, `host`, `path` |
| [aivi.http](http.md) | HTTP vocabulary and `HttpSource` | `HttpSource`, `Request`, `Response` |
| [aivi.auth](auth.md) | OAuth 2.0 / PKCE sign-in records | `OAuthConfig`, `OAuthToken`, `SignInState` |
| [aivi.db](db.md) | Database vocabulary and `DbSource` | `query`, `commit`, `DbSource` |
| [aivi.imap](imap.md) | Mailbox and folder types for IMAP integrations | `FolderSummary`, `MailEvent`, `lastSyncedAt` |
| [aivi.smtp](smtp.md) | Outgoing mail configuration and messages | `from`, `to`, `subject`, `bodyText`, `SmtpConfig` |
| [aivi.app](app.md) | Application framework types | `AppLifecycle`, `AppActionResult`, `AppCommand` |
| [aivi.app.lifecycle](lifecycle.md) | Lifecycle state, commands, undo, notifications | `label`, `shortcut`, `canUndo`, `NotificationLevel` |
| [aivi.desktop.xdg](xdg.md) | XDG error vocabulary | `dataHome`, `configHome`, `cacheHome` |
| [aivi.portal](portal.md) | Desktop portal results | `openFile`, `openUri`, `screenshot` |
| [aivi.dbus](dbus.md) | D-Bus vocabulary and `DbusSource` | `destination`, `path`, `interface`, `member` |
| [aivi.gnome.settings](settings.md) | GSettings schema, key, and value types | `make`, `parse`, `GSettingsSource` |
| [aivi.gnome.onlineAccounts](onlineAccounts.md) | Desktop account and token records | `id`, `token`, `tokenType`, `expiresAt` |
| [aivi.gnome.notifications](notifications.md) | Desktop notification payloads | `summary`, `body`, `icon`, `actions` |
| [aivi.clipboard](clipboard.md) | Clipboard content types and watcher shapes | `ClipboardContent`, `ClipboardSource` |
| [aivi.color](color.md) | Packed UI color domain | `argb`, `red`, `green`, `blue`, `withAlpha` |
| [aivi.image](image.md) | Image data, metadata, and load errors | `format`, `size`, `bytes`, `hasAlpha` |
| [aivi.gresource](gresource.md) | Bundled GResource paths and load errors | `readText`, `readBytes` |
| [aivi.i18n](i18n.md) | Internationalisation marker helpers | `tr`, `trn` |

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
- [`aivi.matrix`](/stdlib/matrix) — rectangular two-dimensional collections.
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

### Time, randomness, and scheduling

- [`aivi.duration`](/stdlib/duration) — typed time spans such as `5sec`.
- [`aivi.time`](/stdlib/time) — clocks, timestamps, and time formatting helpers.
- [`aivi.timer`](/stdlib/timer) — marker types for timer-backed signals.
- [`aivi.random`](/stdlib/random) — randomness vocabulary plus `RandomSource`.

### Files, environment, and processes

- [`aivi.fs`](/stdlib/fs) — filesystem vocabulary plus `FsSource`.
- [`aivi.path`](/stdlib/path) — checked path values.
- [`aivi.env`](/stdlib/env) — environment vocabulary plus `EnvSource`.
- [`aivi.stdio`](/stdlib/stdio) — stdio vocabulary plus `StdioSource`.
- [`aivi.log`](/stdlib/log) — logging vocabulary plus `LogSource`.
- [`aivi.process`](/stdlib/process) — process vocabulary plus future capability shapes.

### Network and services

Some modules in this group are full helpers, and some are shared data shapes for integrations.
The linked pages spell out which functions exist today.

- [`aivi.url`](/stdlib/url) — typed URLs and helpers for their parts.
- [`aivi.http`](/stdlib/http) — HTTP vocabulary plus `HttpSource`.
- [`aivi.auth`](/stdlib/auth) — OAuth / PKCE sign-in records and state types.
- [`aivi.db`](/stdlib/db) — database vocabulary plus `DbSource`.
- [`aivi.imap`](/stdlib/imap) — mailbox types for current integrations and future source capabilities.
- [`aivi.smtp`](/stdlib/smtp) — outgoing mail settings, messages, and errors.

### Desktop, UI, and GNOME

Many pages in this group describe handle vocabularies, watcher/source shapes, partial runtime
surfaces, or shared desktop data types rather than a full feature API. They are still the right
place to look when wiring a Linux desktop app together.

- [`aivi.app`](/stdlib/app) — application framework types.
- [`aivi.app.lifecycle`](/stdlib/lifecycle) — lifecycle state, commands, undo state, and in-app notifications.
- [`aivi.desktop.xdg`](/stdlib/xdg) — XDG error vocabulary; actual directories come from `PathSource`.
- [`aivi.portal`](/stdlib/portal) — desktop portal results for file picking, opening URIs, and screenshots.
- [`aivi.dbus`](/stdlib/dbus) — D-Bus vocabulary plus `DbusSource`.
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
