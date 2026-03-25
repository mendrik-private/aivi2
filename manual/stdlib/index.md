# Standard Library

The AIVI standard library catalog is being built out alongside the language implementation.
This page will be the full reference once modules stabilize.

## Available modules

| Module | Exported names |
|---|---|
| `aivi.list` | `length` `head` `tail` `last` `zip` `any` `all` `count` `find` `findMap` `partition` `isEmpty` `nonEmpty` `Partition` |
| `aivi.option` | `isSome` `isNone` `getOrElse` `orElse` `flatMap` `flatten` `toList` `toResult` |
| `aivi.result` | `withDefault` `mapOk` `mapErr` `toOption` `fromOption` |
| `aivi.text` | `join` `concat` `surround` `isEmpty` `nonEmpty` |
| `aivi.defaults` | `Option` (enables record-field defaults for `Option` fields) |

Built-in (no import needed): `Option` `Some` `None` `Result` `Ok` `Err` `True` `False` `reduce` `append` `head` `tail`

## Importing modules

```aivi
// TODO: add a verified AIVI example here
```

`use` brings specific names into scope.

```aivi
// TODO: add a verified AIVI example here
```

## Current status

The core language and all basic types (`Option`, `Result`, `List`, `Bool`, `Int`, `Text`) are
implemented. The `aivi.list`, `aivi.option`, and `aivi.text` modules are available now.
Network, filesystem, and additional GTK-specific modules are under active development.

Check the [GitHub repository](https://github.com/mendrik/aivi2) for the latest status.
