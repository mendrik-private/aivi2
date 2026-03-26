# Standard Library

The bundled standard library lives under `stdlib/aivi/`. Imports are explicit; there is no wildcard import surface.

## Root modules

- `aivi` exports core types and constructors such as `Option`, `Result`, `Validation`, `Signal`, `Task`, `Ordering`, `Some`, `None`, `Ok`, `Err`, `Valid`, and `Invalid`.
- `aivi.prelude` exposes common class names and helpers such as `Eq`, `Default`, `Ord`, `Semigroup`, `Monoid`, `Functor`, `Bifunctor`, `Traversable`, `Filterable`, `Applicative`, `Monad`, `Foldable`, `getOrElse`, `withDefault`, `length`, `head`, `min`, `max`, `minOf`, and `join`.

```aivi
use aivi (
    None
    Ok
    Option
    Result
    Some
)

use aivi.prelude (
    List
    Text
    head
    length
    withDefault
)

val maybeName: Option Text = Some "Ada"
val loaded: Result Text Int = Ok 2
val count: Int = withDefault 0 loaded

val firstName: Option Text =
    head [
        "Ada",
        "Grace"
    ]

val nameCount: Int =
    length [
        "Ada",
        "Grace"
    ]
```

## Core helper modules

- `aivi.option` - `isSome`, `isNone`, `getOrElse`, `orElse`, `flatMap`, `flatten`, `toList`, `toResult`
- `aivi.result` - `isOk`, `isErr`, `mapErr`, `withDefault`, `orElse`, `flatMap`, `flatten`, `toOption`, `toList`
- `aivi.validation` - `Errors`, `isValid`, `isInvalid`, `getOrElse`, `mapErr`, `toResult`, `fromResult`, `toOption`
- `aivi.list` - `Partition`, `isEmpty`, `nonEmpty`, `length`, `head`, `tail`, `tailOrEmpty`, `last`, `zip`, `any`, `all`, `count`, `find`, `findMap`, `partition`
- `aivi.text` - `isEmpty`, `nonEmpty`, `join`, `concat`, `surround`
- `aivi.order` - `min`, `max`, `minOf`
- `aivi.nonEmpty` - `NonEmpty`, `NonEmptyList`, `singleton`, `cons`, `head`, `toList`, `fromNonEmpty`

```aivi
use aivi (
    Err
    None
    Ok
    Option
    Result
)

use aivi.list (
    Partition
    partition
)

use aivi.option (getOrElse)

use aivi.result (flatMap)

use aivi.text (join)

fun keepPositive:(Result Text Int) value:Int =>
    value > 0
     T|> Ok value
     F|> Err "non-positive"

fun low:Bool value:Int =>
    value < 3

val missingName: Option Text = None
val guest: Text = getOrElse "guest" missingName
val parsed: (Result Text Int) = flatMap keepPositive (Ok 2)

val labels: Text =
    join ", " [
        "Ada",
        "Grace"
    ]

val split: (Partition Int) =
    partition low [
        1,
        3,
        2
    ]
```

## Domains and runtime-facing modules

Bundled domain modules include `aivi.duration`, `aivi.path`, `aivi.url`, `aivi.color`, and `aivi.http` for HTTP types plus the `Retry` domain.

Runtime-facing or integration-heavy modules include `aivi.http`, `aivi.fs`, `aivi.timer`, `aivi.random`, `aivi.stdio`, `aivi.db`, `aivi.dbus`, `aivi.auth`, `aivi.imap`, `aivi.smtp`, `aivi.log`, `aivi.gnome.notifications`, and `aivi.gnome.onlineAccounts`.

Treat `aivi.md` plus the files under `stdlib/aivi/` as the authoritative inventory.
