# Modules

AIVI organises code into modules. Each file is a module. You can import names from other modules and choose which names your module exposes.

## Importing with `use`

The `use` declaration imports names from another module:

```aivi
use aivi.defaults (Option)
```

This imports `Option` from the `aivi.defaults` module. Imported names are available throughout the file.

You can import multiple names in a single `use`:

```aivi
use aivi.network (
    http
    socket
)
```

## Exporting with `export`

The `export` declaration controls what is visible to other modules:

```aivi
export (statusLabel, main)
```

Only the listed names are accessible from outside the module. Names not listed are private.

You can also export a single name:

```aivi
export main
```

## What Can Be Exported

Any top-level declaration can be exported: values, functions, signals, types, domains, and classes:

```aivi
type Status = Idle | Busy

fun statusLabel: Text status: Status =>
    status
     ||> Idle -> "Idle"
     ||> Busy -> "Busy"

value main =
    <Window title="App">
        <Label text={statusLabel Idle} />
    </Window>

export (statusLabel, main)
```

## The Standard Library

The standard library is in the `aivi` namespace. Key modules:

| Module | Contents |
|---|---|
| `aivi.defaults` | `Option`, `Result`, common types |
| `aivi.network` | `http`, `socket` sources |

The standard library exports the following types and classes by default:

```
Ordering, List, Option, Result, Validation, Signal, Task
Less, Equal, Greater
Some, None, Ok, Err, Valid, Invalid
Eq, Default, Functor, Ord, Semigroup, Monoid
Bifunctor, Traversable, Filterable, Applicative, Monad, Foldable
```

## Module Structure

A typical AIVI module is structured as:

1. `use` imports at the top
2. `type` declarations
3. `value` and `fun` declarations
4. `signal` declarations
5. `value main` (the root markup expression, if this is an entry point)
6. `export` at the bottom

```aivi
use aivi.defaults (Option)

type Direction = Up | Down | Left | Right

fun opposite: Direction d: Direction =>
    d
     ||> Up    -> Down
     ||> Down  -> Up
     ||> Left  -> Right
     ||> Right -> Left

value startDirection: Direction = Right

export (Direction, opposite, startDirection)
```

## Summary

| Form | Purpose |
|---|---|
| `use module (names)` | Import specific names from a module |
| `export (names)` | Make names visible to other modules |
| `export name` | Export a single name |
