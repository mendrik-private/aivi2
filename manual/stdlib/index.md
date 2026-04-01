# Standard Library

The AIVI standard library provides foundational types, classes, and functions. All of the following are available in every AIVI program without any `use` declaration unless noted.

## Core Types

### `Ordering`

The result of a comparison:

```aivi
type Ordering =
  | Less
  | Equal
  | Greater
```

### `Option A`

A value that may or may not be present:

```aivi
type Option A = None | Some A
```

| Constructor | Meaning |
|---|---|
| `None` | No value |
| `Some value` | Has a value |

### `Result E A`

Either a success or a failure:

```aivi
type Result E A = Err E | Ok A
```

| Constructor | Meaning |
|---|---|
| `Err error` | Failed with error of type `E` |
| `Ok value` | Succeeded with value of type `A` |

### `Validation E A`

Like `Result`, but with an accumulation-oriented applicative slice for independent failures. The current executable accumulation path combines `Validation (NonEmptyList E)` values rather than accumulating arbitrary error payloads:

```aivi
type Validation E A =
  | Invalid E
  | Valid A
```

### `Signal A`

A reactive node that carries values of type `A` over time.

### `Task E A`

A deferred computation that either produces `A` or fails with `E`.

---

## `aivi.option` — Option Utilities

```aivi
use aivi.option (
    isSome
    isNone
    getOrElse
    orElse
    flatMap
    flatten
    toList
    toResult
)
```

| Function | Signature | Description |
|---|---|---|
| `isSome` | `Option A -> Bool` | `True` if `Some` |
| `isNone` | `Option A -> Bool` | `True` if `None` |
| `getOrElse` | `A -> Option A -> A` | Extract value or return fallback |
| `orElse` | `Option A -> Option A -> Option A` | Use fallback option if `None` |
| `flatMap` | `(A -> Option B) -> Option A -> Option B` | Chain `Option`-returning functions |
| `flatten` | `Option (Option A) -> Option A` | Remove one layer of nesting |
| `toList` | `Option A -> List A` | `[value]` or `[]` |
| `toResult` | `E -> Option A -> Result E A` | Convert to `Result` with given error |

Example:

```aivi
use aivi.option (getOrElse)

type Option Text -> Text
func displayName = opt =>
    getOrElse "Anonymous" opt
```

---

## `aivi.result` — Result Utilities

```aivi
use aivi.result (
    isOk
    isErr
    mapErr
    withDefault
    orElse
    flatMap
    flatten
    toOption
    toList
)
```

| Function | Signature | Description |
|---|---|---|
| `isOk` | `Result E A -> Bool` | `True` if `Ok` |
| `isErr` | `Result E A -> Bool` | `True` if `Err` |
| `mapErr` | `(E1 -> E2) -> Result E1 A -> Result E2 A` | Transform the error type |
| `withDefault` | `A -> Result E A -> A` | Extract value or return fallback |
| `orElse` | `Result E A -> Result E A -> Result E A` | Use fallback result if `Err` |
| `flatMap` | `(A -> Result E B) -> Result E A -> Result E B` | Chain `Result`-returning functions |
| `flatten` | `Result E (Result E A) -> Result E A` | Remove one layer of nesting |
| `toOption` | `Result E A -> Option A` | Discard error, return `Option` |
| `toList` | `Result E A -> List A` | `[value]` or `[]` |

Example:

```aivi
use aivi.result (withDefault)

type Result Text Int -> Int
func safeScore = result =>
    withDefault 0 result
```

---

## `aivi.list` — List Utilities

```aivi
use aivi.list (
    length
    head
    at
    tail
    tailOrEmpty
    last
    isEmpty
    nonEmpty
    replaceAt
    any
    all
    count
    find
    findMap
    zip
    partition
)
```

List operations use the built-in `reduce` and `append` functions (available everywhere).

### Built-In List Functions

| Function | Signature | Description |
|---|---|---|
| `append` | `List A -> List A -> List A` | Concatenate two lists |
| `reduce` | `(B -> A -> B) -> B -> List A -> B` | Fold a list into a single value |

### `aivi.list` Functions

| Function | Signature | Description |
|---|---|---|
| `length` | `List A -> Int` | Number of elements |
| `isEmpty` | `List A -> Bool` | `True` if the list has no elements |
| `nonEmpty` | `List A -> Bool` | `True` if the list has at least one element |
| `head` | `List A -> Option A` | First element, or `None` |
| `at` | `Int -> List A -> Option A` | Element at the given zero-based index, or `None` when out of range |
| `tail` | `List A -> Option (List A)` | All but first element, or `None` |
| `tailOrEmpty` | `List A -> List A` | All but first element, or `[]` |
| `last` | `List A -> Option A` | Last element, or `None` |
| `replaceAt` | `Int -> A -> List A -> List A` | Replace the element at the given zero-based index, leaving out-of-range lists unchanged |
| `any` | `(A -> Bool) -> List A -> Bool` | `True` if any element satisfies the predicate |
| `all` | `(A -> Bool) -> List A -> Bool` | `True` if all elements satisfy the predicate |
| `count` | `(A -> Bool) -> List A -> Int` | Number of elements satisfying the predicate |
| `find` | `(A -> Bool) -> List A -> Option A` | First element satisfying the predicate |
| `findMap` | `(A -> Option B) -> List A -> Option B` | First non-`None` result of applying a function |
| `zip` | `List A -> List B -> List (A, B)` | Pair up elements from two lists |
| `partition` | `(A -> Bool) -> List A -> Partition A` | Split into matched and unmatched |

`Partition A` is a record with fields `matched: List A` and `unmatched: List A`.

Example — sum a list:

```aivi
use aivi.list (sum)

type List Int -> Int
func sumList = numbers =>
    sum numbers
```

Example — collect names from a list of users:

```aivi
use aivi.list (find)

type User = {
    id: Int,
    name: Text
}

type Int -> User -> Bool
func hasId = id user =>
    user.id == id

type Int -> (List User) -> (Option User)
func findById = id users =>
    find (hasId id) users
```

---

## `aivi.text` — Text Utilities

```aivi
use aivi.text (
    isEmpty
    nonEmpty
    join
    surround
)
```

| Function | Signature | Description |
|---|---|---|
| `isEmpty` | `Text -> Bool` | `True` if the string is `""` |
| `nonEmpty` | `Text -> Bool` | `True` if the string is not `""` |
| `join` | `Text -> List Text -> Text` | Join parts with a separator (`""` concatenates) |
| `surround` | `Text -> Text -> Text -> Text` | Wrap with prefix and suffix |

Example:

```aivi
use aivi.text (join)

type List Text -> Text
func csvLine = fields =>
    join "," fields
```

---

## `aivi.path` — File Paths

```aivi
use aivi.path (
    Path
    PathError
)
```

```aivi
domain Path over Text
```

Example:

```aivi
use aivi.path (Path)

value configPath : Path = root "/etc" / "myapp" / "config.toml"
```

---

## `aivi.fs` — File System Events

```aivi
use aivi.fs (
    FsError
    FsEvent
    Created
    Changed
    Deleted
)
```

```aivi
type FsError =
  | NotFound Text
  | PermissionDenied Text
  | ReadFailed Text
  | WriteFailed Text
  | FsProtocolError Text

type FsEvent =
  | Created
  | Changed
  | Deleted
```

Use with `@source fs.watch`:

```aivi
@source fs.watch "/tmp/data.txt" with {
    events: [Created, Changed, Deleted]
}
signal fileEvents : Signal FsEvent
```

---

## `aivi.http` — HTTP Client

```aivi
use aivi.http (
    HttpError
    HttpHeaders
    HttpResponse
    DecodeMode
    Strict
    Permissive
)
```

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text
  | RequestFailure Text

type DecodeMode =
  | Strict
  | Permissive

domain Retry over Int
```

Use with `@source http.get`:

```aivi
use aivi.http (
    HttpError
    Strict
)

type User = {
    id: Int,
    name: Text
}

@source http.get "https://api.example.com/users" with {
    decode: Strict,
    retry: 2times,
    timeout: 10sec
}
signal users : Signal (Result HttpError (List User))
```

---

## `aivi.nonEmpty` — Non-Empty Lists

```aivi
use aivi.nonEmpty (
    NonEmptyList
    singleton
    cons
    head
    toList
)
```

| Function | Signature | Description |
|---|---|---|
| `singleton` | `A -> NonEmptyList A` | Create a one-element list |
| `cons` | `A -> NonEmptyList A -> NonEmptyList A` | Prepend an element |
| `head` | `NonEmptyList A -> A` | First element (always safe — no `Option`) |
| `toList` | `NonEmptyList A -> List A` | Convert to a regular list |

---

## `aivi.timer`

```aivi
use aivi.timer (
    TimerTick
    TimerReady
)
```

Used with `@source timer.every`. The signal type is `Signal Unit`.

---

## Typeclasses

The following ambient classes and builtin carrier paths are wired through the current compiler/runtime slice:

| Class | Key operation | Instances include |
|---|---|---|
| `Eq A` | `==`, `!=` | primitive scalars, `Ordering`, `Option`, `Result`, `Validation`, `List` |
| `Ord A` | `compare` | `Int`, `Text`, `Ordering` |
| `Default A` | `default` | same-module `Default` instances; `Option` omission via `use aivi.defaults (Option)`; `Text` / `Int` / `Bool` omission via `use aivi.defaults (defaultText, defaultInt, defaultBool)` |
| `Functor F` | `map` (via `*\|>`) | `Option`, `Result`, `List`, `Validation`, `Signal` |
| `Semigroup A` | `<>` | `Text`, `List` |
| `Monoid A` | `empty` | `Text`, `List` |
| `Foldable F` | `reduce` | `List`, `Option`, `Result`, `Validation` |
| `Filterable F` | `filterMap` | `List`, `Option` |
| `Apply F` | `apply` | `Option`, `Result`, `List`, `Validation`, `Signal` |
| `Applicative F` | `pure` | `Option`, `Result`, `List`, `Validation`, `Signal`, `Task` (executable applicative slice) |
| `Monad F` | `chain`, `join` | `List`, `Option`, `Result` |
| `Bifunctor F` | `bimap` | `Result`, `Validation` |
| `Traversable F` | `traverse` | `List`, `Option`, `Result`, `Validation` |

This table describes the current executable builtin slice. For the full higher-kinded hierarchy, support boundaries, and the current same-module-only limits for user-authored higher-kinded classes and instances, see [Typeclasses & Higher-Kinded Support](/guide/typeclasses).
