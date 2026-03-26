# AIVI

> Parser/runtime/stdlib-grounded reference for code generation.
>
> This document intentionally ignores `./manual`. It is derived from the shipped parser (`crates/aivi-syntax`), backend/runtime-visible behavior (`crates/aivi-runtime`, `crates/aivi-backend`), `stdlib/`, and the shipped fixture corpus.

## Read this before generating AIVI

- AIVI is not Haskell, Elm, PureScript, or F#.
- Do **not** invent `if/else`, `case/of`, `let/in`, `where`, `do`, `module`, wildcard imports, or HTML/virtual-DOM conventions.
- Use `Option`, `Result`, and `Validation`, not `Maybe`, `Either`, `Cmd`, or `Sub`.
- `Signal` is reactive and applicative-oriented. Do not assume general monadic signal chaining.
- The current-subject placeholder is `.`. `_` is a wildcard pattern, **not** the pipe subject placeholder.
- Control flow is pipe-first: `|>`, `?|>`, `||>`, `T|>`, `F|>`, `*|>`, `<|*`, `&|>`, `@|>`, `<|@`.
- Imports are explicit: `use module.path (name, other as alias)`.
- If a construct is not shown here or in the shipped fixtures, do not assume it exists.

## Core model

- `val` and `fun` are pure immutable definitions.
- `sig` introduces reactive values and source-backed signals.
- Surface control flow is expression-first and pipe-first.
- Closed types, closed records, exhaustive pattern matching, and typed effects are central.
- GTK/libadwaita markup is a first-class expression form.

## Comments and naming

Comments supported by the lexer/parser:

- `// line comment`
- `/* block comment */`
- `/** doc comment **/`

Examples and fixtures use:

- lowercase names for values/functions/signals/fields
- `UpperCamelCase` for types and constructors
- dot-qualified module paths such as `aivi.option`

## Top-level declarations

Every declaration is a top-level item. The shipped surface is:

| Form | Meaning |
| --- | --- |
| `use aivi.option (getOrElse, isSome as optionIsSome)` | explicit member imports only |
| `export name` | export one local binding/type |
| `export (A, B, C)` | export several local names |
| `type User = { name: Text }` | closed record type |
| `type Status = Loading \| Ready Text \| Failed Text` | closed sum type |
| `type Vec2 = Vec2 Int Int` | constructor/product-style type |
| `class Eq A` | type class header |
| `instance Eq Blob` | instance header |
| `domain Duration over Int` | nominal zero-cost wrapper over a carrier |
| `val answer[:Int] = expr` | pure immutable binding; annotation optional |
| `fun add[:Int] x[:Int] y[:Int] => expr` | pure function; return and parameter annotations are optional |
| `sig count[:Signal Int] = expr` | derived signal |
| `sig input : Signal Int` | bodyless input/source-backed signal |
| `provider custom.feed` | source provider contract |
| decorators such as `@source ...`, `@recur.timer ...`, `@recur.backoff ...` | attach to the next top-level item |

### Examples

```aivi
use aivi.option (
    getOrElse
    isSome as optionIsSome
)

export summary
export (Path, PathError)

type User = {
    name: Text,
    email: Option Text
}

type Screen =
  | Loading
  | Ready Text
  | Failed Text

val greeting: Text = "hello"
val inferred = 42

fun add:Int x:Int y:Int =>
    x + y

fun step value =>
    value

sig counter = 0
sig nextCounter = counter + 1
```

## Imports and exports

### Imports

Imports are explicit member imports from a qualified module path:

```aivi
use aivi.result (
    withDefault
    mapErr
    toOption as resultToOption
)
```

Notes:

- No wildcard imports are present in the shipped parser/fixtures.
- Aliases rename the imported member, not the module.
- Module paths are dot-qualified names such as `aivi.result` or `shared.logic`.

### Exports

Both of these forms are shipped:

```aivi
export Path
export (Option, Result, Validation)
```

Exports name local bindings/types/constructors by identifier. There is no separate `module` declaration surface.

## Types

The shipped type-expression forms are:

- type name: `Int`, `Text`, `User`
- type application: `Option Text`, `Result HttpError Text`, `List (Int, Text)`
- function type: `A -> B`
- tuple type: `(A, B, C)`
- record type: `{ name: Text, age: Int }`
- grouped type: `(Result E A)`

The CST includes:

- type names
- grouped types
- tuple types
- record types
- arrow types
- applied types

### Built-in/core types visible in shipped code

- primitives: `Int`, `Float`, `Decimal`, `BigInt`, `Bool`, `Text`, `Bytes`, `Unit`
- collections: `List A`, `Map K V`, `Set A`
- core algebraic carriers: `Option A`, `Result E A`, `Validation E A`
- effects/reactivity: `Signal A`, `Task E A`
- ordering: `Ordering = Less | Equal | Greater`

### Type declaration forms

```aivi
type User = {
    name: Text,
    email: Option Text
}

type Screen =
  | Loading
  | Ready Text
  | Failed Text

type Vec2 = Vec2 Int Int
type HttpTask A = (Task HttpError A)
```

## Literals

The parser/CST ships these literal forms:

- integer: `42`
- float: `3.14`
- decimal: `19.25d`
- big integer: `123n`
- suffixed integer literal: `5ms`, `3x`, `2sec`
- text with interpolation: `"Hello {name}"`
- regex literal: `rx"\d+"`
- unit: `()`
- tuple: `(1, "Ada")`
- list: `[1, 2, 3]`
- range: `1..10` and `[1..10]`
- record literal: `{ id: 1, title: "Alpha" }`
- record shorthand: `{ name, nickname }`
- map literal: `Map { "Authorization": "Bearer demo" }`
- set literal: `Set ["news", "featured"]`

### Record elision

Closed record construction supports omitted fields when default evidence is in scope.

The shipped pattern is:

```aivi
use aivi.defaults (Option)

type Profile = {
    name: Text,
    nickname: Option Text,
    bio: Option Text
}

val minimalProfile:Profile = {
    name: "Grace"
}
```

`aivi.defaults` currently exports `Option` default evidence, which is how omitted `Option` fields are filled in shipped fixtures.

## Expressions

The shipped expression forms visible in the CST/fixtures are:

- names: `answer`
- constructors/constants: `Some`, `None`, `Ok`, `Err`, `Valid`, `Invalid`
- grouped expressions: `(expr)`
- tuples, lists, maps, sets, records
- current subject placeholder: `.`
- ambient projection: `.email`, `.shipping.status`
- direct projection: `user.email`
- ranges: `1..10`
- function application by juxtaposition: `f x y`
- unary `not`
- infix binary operators
- pipe expressions
- markup expressions

### Function application

Application is ordinary juxtaposition:

```aivi
itemLabel item
min smaller 4 2
toResult "missing" someName
```

There is **no** shipped anonymous-lambda expression surface in the current CST. Use named `fun` helpers, constructors, projections, and pipe cases instead.

### Subject placeholder and projection

The parser uses `.` for the current subject:

```aivi
val current = .
val projection = .email

fun displayEmail:Text user:User =>
    .email
```

Important:

- `.` is the current-subject placeholder in expressions and pipes.
- `_` is a wildcard/discard pattern, not the subject placeholder.

### Operators and precedence

Shipped unary operator:

- `not`

Shipped binary operators:

- arithmetic: `+`, `-`, `*`, `/`, `%`
- comparison: `>`, `<`, `==`, `!=`
- boolean: `and`, `or`

Parser precedence is code-backed:

1. application
2. unary `not`
3. `* / %`
4. `+ -`
5. `> < == !=`
6. `and`
7. `or`
8. pipe stages outside ordinary binary precedence

## Pipes and control flow

Pipe algebra is the main control-flow surface.

| Operator | Meaning |
| --- | --- |
| ` \|>` | ordinary transform |
| `?\|>` | carrier-aware gate/filter |
| `\|\|>` | pattern-match arm |
| `T\|>` | canonical truthy/success/present branch |
| `F\|>` | canonical falsy/error/empty branch |
| `*\|>` | fan-out / map |
| `<\|*` | explicit fan-in / join |
| ` \| ` | tap/observe without replacing the subject |
| `&\|>` | applicative cluster |
| `@\|>` | recurrence start |
| `<\|@` | recurrence step |

### `|>` transform

```aivi
order
 |> .shipping
 |> .status
```

### `?|>` gate/filter

Shipped fixtures show carrier-aware gating:

```aivi
val maybeActive:Option User =
    seed
     ?|> .active

sig activeUsers:Signal User =
    sessions
      |> .user
     ?|> .active
```

Inside fan-out, `?|>` is also used as a filter step.

### `||>` pattern cases

```aivi
status
 ||> Paid          => "paid"
 ||> Pending       => "pending"
 ||> Failed reason => "failed {reason}"
```

### `T|>` and `F|>`

These are the shipped replacement for `if/else`-style branching over canonical carriers:

```aivi
ready
 T|> "start"
 F|> "wait"

maybeUser
 T|> .name
 F|> "guest"

loaded
 T|> .name
 F|> .message
```

Shipped fixtures cover `Bool`, `Option`, `Result`, `Validation`, and reactive uses.

### `*|>` and `<|*`

```aivi
val emails:List Text =
    users
     *|> .email

val joinedEmails:Text =
    users
     *|> .email
     <|* joinEmails
```

Shipped fixtures cover fan-out over `List` and `Signal (List A)`.

### `&|>` applicative clusters

```aivi
sig validatedUser =
 &|> nameText
 &|> emailText
 &|> ageValue
  |> UserDraft
```

Use clusters to combine independent reactive inputs applicatively.

### Recurrence: `@|>`, optional `?|>` guards, and `<|@`

Validated recurrence suffixes have this shape:

`seed ... @|> start ?|> guard ... <|@ step ...`

```aivi
type Cursor = {
    hasNext: Bool
}

fun keep:Cursor cursor:Cursor =>
    cursor

val initial:Cursor = {
    hasNext: True
}

@recur.timer 1s
sig cursor : Signal Cursor =
    initial
     @|> keep
     ?|> .hasNext
     <|@ keep
```

`@recur.timer ...` and `@recur.backoff ...` are the explicit non-source wakeup proofs used by recurrent `Signal` and `Task` declarations.

### Tap: `|`

```aivi
order
 |> .shipping
 | observeShipping
 |> .status
```

## Patterns

Shipped pattern forms in `||>` and markup `<case pattern={...}>` include:

- wildcard: `_`
- name binding: `name`
- integer/text literals
- grouped patterns
- tuple patterns
- list patterns with rest: `[first, second, ...rest]`
- record patterns: `{ name, nickname }`
- constructor/application patterns: `Some item`, `Ready title`, `User name _`

### Examples

```aivi
values
 ||> []                       => 0
 ||> [first]                  => first
 ||> [first, second, ...rest] => first + second + listLength rest

screen
 ||> Loading       => "loading"
 ||> Ready title   => title
 ||> Failed reason => reason

profile
 ||> { name, nickname } => name
```

The shipped fixture corpus includes exhaustiveness checks for both pipe matches and markup `<match>`.

## Reactivity, sources, and recurrence

### `val` versus `sig`

- `val` is pure and non-reactive.
- `sig` is reactive.
- The shipped invalid fixtures explicitly reject `val` depending on `sig`.

### Derived signals

```aivi
sig nextCounter = counter + 1
```

### Bodyless/input/source-backed signals

```aivi
@source http.get "/users"
sig users : Signal (Result HttpError (List User))
```

Important:

- `@source ...` decorates a bodyless `sig`.
- Do not attach `@source` to `val`, `fun`, or a body-backed `sig`.

### `scan`

Shipped recurrence fixtures use `scan` for stateful signal accumulation:

```aivi
fun step:Int tick:Unit current:Int =>
    current + 1

sig retried : Signal Int =
    tick
     |> scan 0 step
```

The shipped step shape is event first, current state second.

### Built-in source/provider forms confirmed by runtime and fixtures

These are the public built-in source forms that are clearly exercised in shipped code:

- `timer.every`
- `timer.after`
- `http.get`
- `http.post`
- `fs.read`
- `fs.watch`
- `process.spawn`
- `mailbox.subscribe`
- `socket.connect`
- `window.keyDown`

Common shipped source options:

- HTTP: `headers`, `decode`, `retry`, `timeout`, `body`
- timer: `immediate`, `coalesce`
- filesystem: `decode`, `reloadOn`, `events`
- process: `stdout`, `stderr`

Common shipped option/value types:

- `DecodeMode = Strict | Permissive`
- process stream modes `Ignore | Lines | Bytes`
- retry domain literals such as `3x`
- duration literals such as `5s`, `120ms`

### Provider contracts

The parser ships custom provider contracts:

```aivi
provider custom.feed
    argument path: Text
    option timeout: Duration
    option mode: Mode
    wakeup: providerTrigger
```

The parser/runtime surface for built-in providers is ahead of the custom-provider ecosystem. Documented syntax is real; custom runtime integration is narrower than the built-in provider family.

## Markup / GTK surface

Markup is a first-class expression form. It is GTK/libadwaita-oriented, not HTML.

### Widget-style nodes

```aivi
<Label text={header} />
<Button label="Click" />
<Window title="Inbox">
    ...
</Window>
```

Shipped fixtures and runtime code clearly exercise widget/control names such as:

- `Window`
- `Box`
- `Label`
- `Button`
- `Entry`
- `Switch`

### Attribute values

- plain text: `text="Hello"`
- expression: `text={header}`
- interpolated text: `text="Error {reason}"`

### Control nodes

```aivi
<fragment>
    <show when={True} keepMounted={True}>
        <with value={screen} as={currentScreen}>
            <match on={currentScreen}>
                <case pattern={Loading}>
                    <Label text="Loading..." />
                </case>
                <case pattern={Ready items}>
                    <each of={items} as={item} key={item.id}>
                        <Label text={item.title} />
                        <empty>
                            <Label text="No items" />
                        </empty>
                    </each>
                </case>
            </match>
        </with>
    </show>
</fragment>
```

Shipped control nodes:

- `<fragment>`
- `<show when={...}>`
- `<with value={...} as={...}>`
- `<match on={...}>`
- `<case pattern={...}>`
- `<each of={...} as={...} key={...}>`
- `<empty>`

Important limits backed by fixtures:

- `<each>` requires `key={...}`.
- `<match>` is checked for exhaustiveness.
- Child interpolation is **not** HTML-style free text; shipped code uses widget nodes and attribute expressions instead.
- Event attributes target input signals, not arbitrary callback expressions.

## Domains

Domains are nominal wrappers over a carrier type with explicit members.

### Syntax

```aivi
domain Duration over Int
    literal ms : Int -> Duration
    (+) : Duration -> Duration -> Duration
    value : Duration -> Int

domain Path over Text
    parse : Text -> Result PathError Path
    (/) : Path -> Text -> Path
    value : Path -> Text
```

Shipped domain member forms include:

- `literal` suffix constructors
- ordinary named members such as `parse`, `millis`, `value`
- operator members such as `(+)`, `(-)`, `(/)`

Current bundled domains:

- `Duration over Int`
- `Path over Text`
- `Url over Text`
- `Color over Int`
- `Retry over Int` (inside `aivi.http`)

Domains do not imply automatic carrier coercions. Use explicit domain members such as `value` or `parse`.

## Classes and instances

### Class declarations

```aivi
class Eq A
    (==) : A -> A -> Bool

class Display A
    display : A -> Text
```

Shipped class bodies are indented member signatures. Member names may be ordinary identifiers or parenthesized operators.

### Instance declarations

```aivi
type Blob = Blob Bytes

fun blobEquals: Bool left: Blob right: Blob =>
    True

instance Eq Blob
    (==) left right = blobEquals left right
```

Important:

- instance members use `=` definitions inside the instance body
- shipped fixtures focus on same-module instance declarations
- the CST carries context for constrained class/instance forms, but the fixture corpus centers on straightforward declarations

## Bundled stdlib batteries

Not every bundled module exports functions. Some modules are mostly:

- core constructors/classes
- helper functions
- domain declarations
- error/event types used by sources and tasks
- intrinsic surfaces injected by the compiler/runtime

### Root modules

| Module | What it gives you |
| --- | --- |
| `aivi` | core algebraic types and classes: `Ordering`, `List`, `Option`, `Result`, `Validation`, `Signal`, `Task`, constructors like `Some`/`None`/`Ok`/`Err`, classes like `Eq`, `Functor`, `Applicative`, `Foldable`, etc. |
| `aivi.prelude` | primitive type names plus convenience re-exports/wrappers such as `getOrElse`, `withDefault`, `length`, `head`, `min`, `max`, `minOf`, `join`, `concat`, `isSome`, `isErr` |

### Core helper modules

| Module | Shipped surface |
| --- | --- |
| `aivi.list` | `Partition`, `isEmpty`, `nonEmpty`, `length`, `head`, `tail`, `tailOrEmpty`, `last`, `zip`, `any`, `all`, `count`, `find`, `findMap`, `partition` |
| `aivi.option` | `isSome`, `isNone`, `getOrElse`, `orElse`, `flatMap`, `flatten`, `toList`, `toResult` |
| `aivi.result` | `isOk`, `isErr`, `mapErr`, `withDefault`, `orElse`, `flatMap`, `flatten`, `toOption`, `toList` |
| `aivi.text` | `isEmpty`, `nonEmpty`, `join`, `concat`, `surround` |
| `aivi.validation` | `Errors`, `isValid`, `isInvalid`, `getOrElse`, `mapErr`, `toResult`, `fromResult`, `toOption` |
| `aivi.nonEmpty` | `NonEmpty`, `NonEmptyList`, `singleton`, `cons`, `head`, `toList`, `fromNonEmpty` |
| `aivi.order` | `min`, `max`, `minOf` |
| `aivi.defaults` | default evidence for omitted `Option` record fields (`use aivi.defaults (Option)`) |

### Domain modules

| Module | Shipped surface |
| --- | --- |
| `aivi.duration` | `DurationError`, domain `Duration` with literals `ms`, `sec`, `min`, members `millis`, `trySeconds`, `value`, operators `(+)`, `(-)` |
| `aivi.path` | `PathError`, domain `Path` with `parse`, `(/)`, `value` |
| `aivi.url` | `UrlError`, domain `Url` with `parse`, `value` |
| `aivi.color` | domain `Color` with `argb`, `value` |
| `aivi.http` | `HttpError`, `HttpHeaders`, `HttpQuery`, `HttpResponse`, `HttpTask`, `DecodeMode = Strict | Permissive`, domain `Retry` with literal `x` |
| `aivi.timer` | `TimerTick`, `TimerReady` types used by `timer.every` / `timer.after` sources |

### Runtime/integration type modules

These modules are bundled batteries mostly because they define data shapes, errors, and task/source-facing types:

| Module | Shipped surface |
| --- | --- |
| `aivi.fs` | `FsError`, `FsEvent`; runtime also ships `fs.read`, `fs.watch`, plus intrinsic file-write tasks |
| `aivi.stdio` | intrinsic CLI tasks `stdoutWrite`, `stderrWrite` are compiler/runtime-injected |
| `aivi.random` | intrinsic random surface is compiler/runtime-injected (`randomInt`, `randomBytes`, `RandomError`) |
| `aivi.log` | `LogLevel`, `LogContext`, `LogEntry`, `LogError`, `LogTask`, `LogWrite` |
| `aivi.db` | `DbError`, `SortDir` |
| `aivi.dbus` | `DbusValue`, `DbusCall`, `DbusSignal`, `BusNameFlag`, `BusNameState`, `DbusError` |
| `aivi.auth` | `PkceConfig`, `PkceToken`, `PkceError` |
| `aivi.imap` | `SyncState`, `ImapError` |
| `aivi.smtp` | `Attachment`, `SmtpConfig`, `SmtpMessage`, `SmtpError` |
| `aivi.gnome.notifications` | `NotificationAction`, `Notification`, `NotificationError` |
| `aivi.gnome.onlineAccounts` | `GoaAccountId`, `GoaCapability`, `GoaProvider`, `GoaAccount`, `AccessToken`, `OAuthToken`, `GoaError` |

### Ambient/core operations that shipped code uses constantly

Some important names are not defined as ordinary functions in `stdlib/*.aivi`; they are part of the bundled core classes/prelude/runtime surface and appear in real shipped programs as plain names/operators.

Shipped examples use:

- equality/order: `==`, `!=`, `compare`
- semigroup/monoid: `append`, `empty`
- bifunctor/traversal/filtering: `bimap`, `traverse`, `filterMap`
- folds/accumulation: `reduce`, `scan`

Treat these as real AIVI surface names, not as Haskell-style method syntax.

### A few batteries-included examples

```aivi
use aivi.option (getOrElse)
use aivi.result (withDefault)
use aivi.text (join)

val chosenName: Text =
    getOrElse "guest" None

val countValue: Int =
    withDefault 0 (Ok 4)

val labels: Text =
    join ", " ["Ada", "Grace"]
```

```aivi
use aivi.duration (Duration)
use aivi.http (HttpError, HttpResponse, Strict, Retry)
use aivi.timer (TimerTick)

@source http.get "https://api.example.com/users" with {
    decode: Strict,
    retry: 3x,
    timeout: 5sec
}
sig users : Signal (HttpResponse Text)

@source timer.every 120 with {
    immediate: True,
    coalesce: True
}
sig tick : Signal TimerTick
```

## Current hard limits: do not invent these

These are the most important anti-hallucination constraints for code generation:

- no `if ... then ... else ...`
- no `case ... of ...`
- no `let ... in ...`
- no `where`
- no `do` notation
- no `module` declarations
- no wildcard imports
- no HTML text children as a general-purpose UI story
- no `_` current-subject placeholder; use `.`
- no evidence in the current CST for anonymous lambda expressions
- no assumption that `Signal` is a general monad
- no assumption that every bundled module exports helper functions; many modules are type/domain catalogs for the runtime boundary

## Source-of-truth files

When this document and the implementation disagree, trust these files:

- parser/CST: `crates/aivi-syntax/src/lex.rs`, `parse.rs`, `cst.rs`
- runtime/provider surface: `crates/aivi-runtime/src/providers.rs`, `source_decode.rs`, `startup.rs`
- backend/runtime-visible lowering: `crates/aivi-backend/src/*`, `crates/aivi-runtime/src/*`
- stdlib: `stdlib/aivi/**/*.aivi`, `stdlib/aivi.aivi`
- language examples and negative cases: `fixtures/frontend/**/*.aivi`
