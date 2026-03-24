# Domains

A domain is a **named refinement** of an existing type. It gives a base type (`Int`, `Text`,
`List A`) a distinct identity, its own operations, and optional literal syntax — without
wrapping it in a constructor or paying any runtime cost.

## Declaring a domain

```text
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    literal min : Int -> Duration
    (+) : Duration -> Duration -> Duration
    (-) : Duration -> Duration -> Duration
    value : Duration -> Int
```

- `domain Name over BaseType` — declares the domain.
- `literal name : Int -> Name` — enables the suffix literal syntax `5ms`, `10sec`, `2min`.
- Operator and function lines declare the operations available on the domain type.
- `value : Duration -> Int` — the conventional escape hatch back to the base type.

The domain type is **opaque**: you cannot pass a raw `Int` where a `Duration` is expected,
and you cannot mix `Duration` with `Path` even though both are over `Int`.

## Using domain literals

Once a `literal` is declared, you write the suffix directly after the number:

```text
val timeout = 5sec
val delay   = 120ms
val cycle   = 2min
```

The compiler resolves the suffix to the correct `literal` function. If two domains in
scope share a suffix the compiler reports an ambiguity error.

## Domain operations

Declared operations work like ordinary functions but are scoped to the domain:

```text
val total = 2sec + 500ms    // uses (+) from Duration
val path  = root / "home" / "user"  // uses (/) from Path
```

Operators must be declared in the domain body; they are not inherited from the base type.

## Parametric domains

Domains can have type parameters:

```text
domain NonEmpty A over List A
    fromList : List A -> Option (NonEmpty A)
    head     : NonEmpty A -> A
    tail     : NonEmpty A -> List A
```

`NonEmpty A` is a `List A` that is statically guaranteed to have at least one element.
`fromList` returns `None` if the list is empty, so the guarantee is enforced at the
boundary.

## Path — a domain over Text

```text
domain Path over Text
    parse : Text -> Result PathError Path
    (/)   : Path -> Text -> Path
    value : Path -> Text
```

`Path` wraps `Text` to give file paths a distinct type and a composable `/` operator:

```text
val config = parse "/etc" / "app" / "config.toml"
```

`parse` returns `Result PathError Path`, so invalid paths are handled explicitly.

## Domains in the standard library

| Domain | Over | Purpose |
|---|---|---|
| `Duration` | `Int` | Time intervals with `ms`, `sec`, `min` literals |
| `Path` | `Text` | File-system paths with `/` composition |
| `Retry` | `Int` | Retry counts with `x` literal (e.g. `3x`) |
| `NonEmpty A` | `List A` | Non-empty lists with a safe `head` |

Import domains from `aivi.duration`, `aivi.path`, `aivi.http` (for `Retry`), or
`aivi.nonEmpty`.

## Why not a `type` wrapper?

A `type` wrapper (e.g. `type Duration = Duration Int`) works but requires pattern
matching to unwrap and adds a constructor at runtime. A domain is zero-cost and gives
you operator and literal syntax without boilerplate.

## Summary

- `domain Name over BaseType` creates an opaque refinement of a base type.
- `literal name : Int -> Name` enables suffix literal syntax (`5ms`, `3x`).
- Declared operators and functions are the only operations on the domain type.
- Domains can have type parameters: `domain NonEmpty A over List A`.
- The `value` function is the conventional way to unwrap back to the base type.

That completes the Language Tour. Next: [The AIVI Way →](/aivi-way/)
