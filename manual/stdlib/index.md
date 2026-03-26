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
| `aivi.fs` | `FsError` `FsEvent` `writeText` `writeBytes` `createDirAll` `deleteFile` |
| `aivi.path` | `Path` `PathError` |
| `aivi.stdio` | `stdoutWrite` `stderrWrite` |
| `aivi.defaults` | `Option` (enables record-field defaults for `Option` fields) |

Built-in (no import needed): `Option` `Some` `None` `Result` `Ok` `Err` `True` `False` `reduce` `append` `head` `tail`

## Importing modules

```aivi
use aivi.stdio (
  stdoutWrite
)

val main : Task Text Unit =
  stdoutWrite "hello from AIVI"
```

`use` brings specific names into scope.

```aivi
use aivi.fs (
  writeText
)

val save : Task Text Unit =
  writeText "/tmp/demo.txt" "saved"
```

## Current status

The core language and all basic types (`Option`, `Result`, `List`, `Bool`, `Int`, `Text`) are
implemented. The `aivi.list`, `aivi.option`, and `aivi.text` modules are available now, and
headless CLI programs can use `aivi execute` together with the current `aivi.stdio` and
`aivi.fs` task intrinsics plus the host-context sources documented in the tour.

Check the [GitHub repository](https://github.com/mendrik/aivi2) for the latest status.
