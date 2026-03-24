# Error Handling

AIVI has no exceptions. Errors are values.

This is not a limitation — it is a design. When errors are values, the type system enforces
that you handle them. Nothing can go wrong silently.

## Result E A: the error type

```text
// TODO: add a verified AIVI example here
```

A `Result E A` is either a successful value (`Ok A`) or an error (`Err E`).
Every operation that can fail returns `Result`.

## Matching on results

Use `\|\|>` to branch on `Ok` vs `Err`:

```text
// TODO: add a verified AIVI example here
```

The compiler ensures you handle both cases. You cannot accidentally ignore an error.

## Chaining operations that might fail

A common pattern is a sequence of operations where each step can fail.
Use `||>` to branch on `Ok` and `Err` at each step:

```text
// TODO: add a verified AIVI example here
```

## Propagating errors in signals

When a signal holds a `Result`, downstream signals can propagate the `Ok` value or branch
on the `Err`:

```text
// TODO: add a verified AIVI example here
```

## Showing errors in markup

```text
// TODO: add a verified AIVI example here
```

## The Option type for optional values

`Option A` handles absence (not failure):

```text
// TODO: add a verified AIVI example here
```

Use `Result` when an operation attempted and failed.
Use `Option` when a value is simply optional.

## Never throw

There is no `throw` in AIVI. Functions that encounter error conditions return `Err msg`.
Callers handle it explicitly.

This means:
- Reading a source file: returns `Result Text`.
- Parsing a number: returns `Result Int`.
- HTTP requests: return `Result Response`.
- Looking up a key in a map: returns `Option Value`.

The return type tells you whether the operation can fail before you even read the documentation.

## Recovering from errors

To fall back to a default value when a result is an error:

```text
// TODO: add a verified AIVI example here
```

Or inline in a pipe:

```text
// TODO: add a verified AIVI example here
```

## Counting valid items in a list

When validating a list of items, derive a count of the valid values with a named predicate:

```text
// TODO: add a verified AIVI example here
```

This gives you a stable summary signal or value you can render directly. For detailed error
reporting, branch on each item individually with `||>` in the calling code.

## Summary

- AIVI has no exceptions. Errors are `Result E A = Ok A | Err E`.
- Use `||>` to branch on `Ok` vs `Err`. The compiler enforces exhaustiveness.
- `Option A = Some A | None` for optional values.
- Chain results with `||>` arms that produce new `Result` values.
- `withDefault` recovers a fallback when a result is an error.
- Return type signatures communicate failure potential before reading docs.
