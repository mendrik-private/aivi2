# Values and Types

Every piece of data in AIVI has a type.
Types in AIVI are **closed**: the compiler knows every possible shape a value of that type can have.
There is no `any`, no `object`, no `null`.

## `val` — a named constant

```text
// TODO: add a verified AIVI example here
```

`val` declares an immutable, named value. There are no variables in AIVI — once declared,
a value does not change. If something needs to change over time, that is a `sig` (covered in
[Chapter 05](/tour/05-signals)).

You can annotate the type explicitly:

```text
// TODO: add a verified AIVI example here
```

The compiler infers types when they are omitted, but annotations are welcome for documentation.

## `type` — defining your own types

AIVI has two flavours of `type`: **sum types** and **product types**.

### Sum types (variants)

A sum type lists all possible values (variants). If you have used TypeScript unions or Rust enums,
this is the same idea — but exhaustive and closed.

```text
// TODO: add a verified AIVI example here
```

Each variant is a constructor — a value of that type.
You can write `Up` or `GameOver` directly; they are ordinary values.

### Sum types with data

Variants can carry data:

```text
// TODO: add a verified AIVI example here
```

`Option` and `Result` are **parametric types** — the type variables are filled in at use sites:

```text
// TODO: add a verified AIVI example here
```

The type variable is always lowercase; type names and constructors are uppercase.

### No null

AIVI has no `null`, `nil`, or `undefined`. The absence of a value is always explicit:

```text
// TODO: add a verified AIVI example here
```

Because you cannot ignore the `None` case (the compiler enforces it), null pointer bugs are
impossible by construction.

### Product types

A product type groups multiple values into one shape. Use a constructor product for positional
data and a record when fields are naturally named:

```text
// TODO: add a verified AIVI example here
```

Create a value by calling the constructor or listing named fields:

```text
// TODO: add a verified AIVI example here
```

Access named fields with dot projection:

```text
// TODO: add a verified AIVI example here
```

`Point` is positional, so you usually unpack it with pattern matching (covered in
[Chapter 04](/tour/04-pattern-matching)). `User` is a record, so dot projection works directly.

Records are immutable — you cannot update a field in place. Instead, create a new record:

```text
// TODO: add a verified AIVI example here
```

### Combining sum and product types

Real programs combine both. Here is a snapshot from the Snake demo:

```text
// TODO: add a verified AIVI example here
```

`Vec2` is a sum type with one variant that carries two `Int` values.
`Snake` and `Game` are product types whose fields reference other types.

## Built-in types

| Type | Description | Example |
|---|---|---|
| `Int` | Signed integer | `42`, `-7` |
| `Float` | Floating-point | `3.14` |
| `Bool` | `True` or `False` | `True` |
| `Text` | UTF-8 string | `"hello"` |
| `List A` | Homogeneous list | `[1, 2, 3]` |

`Bool` is a regular sum type under the hood: `type Bool = True | False`.
The operators `and`, `or`, and `not` work on it.

## Summary

- `val` = named constant.
- `type X = A | B` = sum type (pick one variant).
- `type X = { field: T }` = product type (all fields together).
- Type variables are lowercase; type names and constructors are uppercase.
- No null — use `Option A`.

[Next: Functions →](/tour/02-functions)
