# Pattern Matching

Pattern matching is AIVI's mechanism for inspecting the **shape** of a value and branching
based on what you find. It is more powerful than a `switch` statement because it matches
structure, not just equality.

## The match pipe \|\|>

`\|\|>` is the case pipe. It takes a value on the left and a pattern `=>` body arm on the right.
Multiple arms are written as successive `\|\|>` lines:

```text
// TODO: add a verified AIVI example here
```

Each arm is: `\|\|> pattern => expression`.
The value (`direction`) is matched top-to-bottom against each pattern.
The body of the first matching arm is evaluated and returned.

## Matching on constructors

The most common use is matching on sum type variants:

```text
// TODO: add a verified AIVI example here
```

The same constructor syntax works for your own same-module sum types and for builtin carriers like
`Some` / `None`, including data-carrying variants.

## Exhaustiveness

Pattern matching in AIVI is **exhaustive**: the compiler rejects any match that does not
cover all variants. If you add a new variant to a sum type, every match on that type becomes
a compile error until you handle the new case.

This is the key advantage over `switch` statements: you cannot accidentally forget a case.

```text
type Color = Red | Green | Blue

// Compile error: Blue is not covered
fun colorName:Text #color:Color =>
    color
     ||> Red   => "red"
     ||> Green => "green"
```

## Wildcard patterns

When you want a catch-all, use `_`:

```text
// TODO: add a verified AIVI example here
```

`_` matches anything and does not bind the value.

## Matching on literal values

You can match on integer and text literals directly:

```text
// TODO: add a verified AIVI example here
```

## Destructuring product types (records)

You can destructure a record in a pattern arm, binding its fields to names:

```text
// TODO: add a verified AIVI example here
```

Record patterns work similarly:

```text
// TODO: add a verified AIVI example here
```

Here `{ score }` matches any `Game` record and binds the `score` field.

## Matching on data-carrying constructors

When a variant carries data, the pattern binds the inner values:

```text
// TODO: add a verified AIVI example here
```

`Some value` binds the wrapped `A` to the name `value` in the body.

## Nested patterns

Patterns can be nested. In the snake game, the step logic matches on a record extracted
from a record:

```text
// TODO: add a verified AIVI example here
```

The record pattern `{ snake, food, score }` binds three fields of `Game` simultaneously,
without needing intermediate `let` bindings.

## \|\|> vs T\|>/F\|>

Use `\|\|>` when matching on a general sum type or literal. Use `T\|>` / `F\|>` when the
value is already a `Bool` and you want a two-branch conditional:

```text
// TODO: add a verified AIVI example here
```

```text
// TODO: add a verified AIVI example here
```

## Summary

- `\|\|>` is the match pipe. Each arm is `\|\|> pattern => body`.
- Matching is exhaustive — every variant must be covered.
- `_` is the wildcard that matches anything.
- Patterns can destructure records and data-carrying constructors.
- Patterns can be nested arbitrarily.

[Next: Signals →](/tour/05-signals)
