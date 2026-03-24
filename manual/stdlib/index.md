# Standard Library

The AIVI standard library catalog is being built out alongside the language implementation.
This page will be the full reference once modules stabilize.

## Available modules

| Module | Exported names |
|---|---|
| `aivi.list` | `length` `head` `tail` `last` `zip` `any` `all` `count` `find` `findMap` `partition` `isEmpty` `nonEmpty` `Partition` |
| `aivi.option` | `isSome` `isNone` `getOrElse` `orElse` `flatMap` `flatten` `toList` `toResult` |
| `aivi.text` | `join` `concat` `surround` `isEmpty` `nonEmpty` |
| `aivi.defaults` | `Option` (enables record-field defaults for `Option` fields) |

Built-in (no import needed): `Option` `Some` `None` `Result` `Ok` `Err` `True` `False` `reduce` `append` `head` `tail`

## Importing modules

```aivi
use aivi.list (
    length
    head
    tail
    any
    isEmpty
    nonEmpty
)

use aivi.text (join)
```

`use` brings specific names into scope.

```aivi
use aivi.list (
    length
    isEmpty
    nonEmpty
)

val nums:List Int = [
    1,
    2,
    3,
    4,
    5
]

val total:Int =
    nums
     |> length

val empty:Bool =
    nums
     |> isEmpty
```

## Current status

The core language and all basic types (`Option`, `Result`, `List`, `Bool`, `Int`, `Text`) are
implemented. The `aivi.list`, `aivi.option`, and `aivi.text` modules are available now.
Network, filesystem, and additional GTK-specific modules are under active development.

Check the [GitHub repository](https://github.com/mendrik/aivi2) for the latest status.
