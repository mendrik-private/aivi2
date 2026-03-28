# Standard Library

The AIVI standard library provides foundational types, classes, and functions. All of the following are available in every AIVI program without any `use` declaration unless noted.

## Core Types

### `Ordering`

The result of a comparison:

```aivi
type Ordering = Less | Equal | Greater
```

### `Option A`

A value that may or may not be present:

```aivi
type Option A =
  | None
  | Some A
```

| Constructor | Meaning |
|---|---|
| `None` | No value |
| `Some value` | Has a value |

### `Result E A`

Either a success or a failure:

```aivi
type Result E A =
  | Err E
  | Ok A
```

| Constructor | Meaning |
|---|---|
| `Err error` | Failed with error of type `E` |
| `Ok value` | Succeeded with value of type `A` |

### `Validation E A`

Like `Result`, but accumulates multiple errors instead of stopping at the first:

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
use aivi.option (isSome, isNone, getOrElse, orElse, flatMap, flatten, toList, toResult)
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

fun displayName: Text opt: Option Text =>
    getOrElse "Anonymous" opt
```

---

## `aivi.result` — Result Utilities

```aivi
use aivi.result (isOk, isErr, mapErr, withDefault, orElse, flatMap, flatten, toOption, toList)
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

fun safeScore: Int result: Result Text Int =>
    withDefault 0 result
```

---

## `aivi.list` — List Utilities

```aivi
use aivi.list (length, head, tail, tailOrEmpty, last, isEmpty, nonEmpty, any, all, count, find, findMap, zip, partition)
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
| `tail` | `List A -> Option (List A)` | All but first element, or `None` |
| `tailOrEmpty` | `List A -> List A` | All but first element, or `[]` |
| `last` | `List A -> Option A` | Last element, or `None` |
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
fun sumList: Int numbers: List Int =>
    numbers
     |> reduce (\total n => total + n) 0
```

Example — collect names from a list of users:

```aivi
use aivi.list (find)

type User = { id: Int, name: Text }

fun findById: Option User id: Int users: List User =>
    find (.id == id) users
```

---

## `aivi.text` — Text Utilities

```aivi
use aivi.text (isEmpty, nonEmpty, join, concat, surround)
```

| Function | Signature | Description |
|---|---|---|
| `isEmpty` | `Text -> Bool` | `True` if the string is `""` |
| `nonEmpty` | `Text -> Bool` | `True` if the string is not `""` |
| `join` | `Text -> List Text -> Text` | Join parts with a separator |
| `concat` | `List Text -> Text` | Concatenate without separator |
| `surround` | `Text -> Text -> Text -> Text` | Wrap with prefix and suffix |

Example:

```aivi
use aivi.text (join)

fun csvLine: Text fields: List Text =>
    join "," fields
```

---

## `aivi.path` — File Paths

```aivi
use aivi.path (Path, PathError)
```

```aivi
domain Path over Text
    parse: Text -> Result PathError Path
    (/): Path -> Text -> Path
    unwrap: Path -> Text
```

Example:

```aivi
use aivi.path (Path)

value configPath: Path = root "/etc" / "myapp" / "config.toml"
```

---

## `aivi.fs` — File System Events

```aivi
use aivi.fs (FsError, FsEvent, Created, Changed, Deleted)
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
signal fileEvents: Signal FsEvent
```

---

## `aivi.http` — HTTP Client

```aivi
use aivi.http (HttpError, HttpHeaders, HttpResponse, DecodeMode, Strict, Permissive)
```

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text
  | RequestFailure Text

type DecodeMode = Strict | Permissive

domain Retry over Int
    literal rt: Int -> Retry
```

Use with `@source http.get`:

```aivi
use aivi.http (HttpError, Strict)

type User = { id: Int, name: Text }

@source http.get "https://api.example.com/users" with {
    decode: Strict,
    retry: 2rt,
    timeout: 10sec
}
signal users: Signal (Result HttpError (List User))
```

---

## `aivi.nonEmpty` — Non-Empty Lists

```aivi
use aivi.nonEmpty (NonEmptyList, singleton, cons, head, toList)
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
use aivi.timer (TimerTick, TimerReady)
```

Used with `@source timer.every`. The signal type is `Signal Unit`.

---

## Typeclasses

The following typeclasses are available everywhere:

| Class | Key operation | Instances include |
|---|---|---|
| `Eq A` | `==`, `!=` | `Int`, `Bool`, `Text`, `Option`, `Result` |
| `Ord A` | `compare` | `Int`, `Text`, `Ordering` |
| `Default A` | `default` | `Int`, `Bool`, `Text`, `List` |
| `Functor F` | `map` (via `*\|>`) | `Option`, `Result`, `List`, `Signal` |
| `Semigroup A` | `<>` | `Text`, `List` |
| `Monoid A` | `empty` | `Text`, `List` |
| `Foldable F` | `reduce` | `List` |
| `Filterable F` | `filter` | `List`, `Option` |
| `Applicative F` | `pure`, `ap` | `Option`, `Result`, `List` |
| `Monad F` | `bind` | `Option`, `Result`, `List` |
| `Bifunctor F` | `bimap` | `Result` |
| `Traversable F` | — | `List` |
