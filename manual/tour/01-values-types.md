# Values and Types

Every piece of data in AIVI has a type.
Types in AIVI are **closed**: the compiler knows every possible shape a value of that type can have.
There is no `any`, no `object`, no `null`.

## `val` — a named constant

```text
-- declare a constant 'answer' with value 42
-- declare a constant 'greeting' with text "Hello, world!"
-- declare a constant 'pi' with value 3.14159
```

`val` declares an immutable, named value. There are no variables in AIVI — once declared,
a value does not change. If something needs to change over time, that is a `sig` (covered in
[Chapter 05](/tour/05-signals)).

You can annotate the type explicitly:

```text
-- declare a constant 'answer' of type Int with value 42
-- declare a constant 'greeting' of type Text with value "Hello, world!"
```

The compiler infers types when they are omitted, but annotations are welcome for documentation.

## `type` — defining your own types

AIVI has two flavours of `type`: **sum types** and **product types**.

### Sum types (variants)

A sum type lists all possible values (variants). If you have used TypeScript unions or Rust enums,
this is the same idea — but exhaustive and closed.

```text
-- declare a sum type 'Direction' with variants: Up, Down, Left, Right
-- declare a sum type 'Status' with variants: Running, Paused, GameOver
-- declare a sum type 'Bool' with variants: True, False
```

Each variant is a constructor — a value of that type.
You can write `Up` or `GameOver` directly; they are ordinary values.

### Sum types with data

Variants can carry data:

```text
-- declare a parametric type 'Option A' with variants: Some (holding a value of type A) and None
-- declare a parametric type 'Result E A' with variants: Ok (holding A) and Err (holding E)
-- declare a sum type 'Shape' with variant Circle carrying a radius integer,
--   and variant Rectangle carrying width and height integers
```

`Option` and `Result` are **parametric types** — the type variables are filled in at use sites:

```text
-- declare 'found' as an Option Int holding the value 42
-- declare 'missing' as an Option Int with no value
-- declare 'success' as a Result Text Int holding the successful value 100
-- declare 'failure' as a Result Text Int holding the error text "not found"
```

The type variable is always lowercase; type names and constructors are uppercase.

### No null

AIVI has no `null`, `nil`, or `undefined`. The absence of a value is always explicit:

```text
-- Option A is a sum type: Some (holding A) or None
-- declare 'username' as Option Text with no value (not logged in)
-- declare 'username' as Option Text holding "ada" (logged in)
```

Because you cannot ignore the `None` case (the compiler enforces it), null pointer bugs are
impossible by construction.

### Product types (records)

A product type groups multiple named fields into one value:

```text
-- declare a product type 'Point' with integer fields x and y
-- declare a product type 'User' with integer field id, and text fields username and email
```

Create a record by listing its fields:

```text
-- declare 'origin' as a Point with x and y both set to 0
-- declare 'user' as a User with id 1, username "ada", and email "ada@example.com"
```

Access fields with dot projection:

```text
-- project 'username' field from user, binding result to 'name'
-- project 'x' field from origin, binding result to 'x'
```

Records are immutable — you cannot update a field in place. Instead, create a new record:

```text
-- create a new User 'movedUser' copying id and email from user, but with username "lovelace"
```

### Combining sum and product types

Real programs combine both. Here is a snapshot from the Snake demo:

```text
-- declare a type 'Vec2' as a single-variant type carrying two integers (x and y)
-- declare a sum type 'Status' with variants Running and GameOver
-- declare a product type 'Snake' with a Vec2 head, a Vec2 second segment, and an integer length
-- declare a product type 'Game' with a Snake, a Status, and an integer score
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
