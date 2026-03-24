# Pipes

Pipes are the centrepiece of AIVI's surface syntax.
The idea is borrowed from Unix: data flows from left to right through a sequence of transformations.

## The transform pipe \|>

`\|>` takes the value on its left and passes it as the first argument to the function on its right.

```text
// TODO: add a verified AIVI example here
```

This is equivalent to `toString (double 42)`. Pipes let you read computation top-to-bottom
instead of inside-out.

Compare these two forms of the same computation:

```text
// TODO: add a verified AIVI example here
```

When you pass partial arguments before the piped value, `|>` inserts the left-hand value
as the **last** argument:

```text
// TODO: add a verified AIVI example here
```

## Projection shorthand

A common pattern is projecting a field from a record:

```text
// TODO: add a verified AIVI example here
```

The `.field` syntax is a shorthand for `\r => r.field`.
It composes naturally in pipes:

```text
// TODO: add a verified AIVI example here
```

## Chaining pipes

Pipes chain arbitrarily. Each `\|>` is one step in the computation:

```text
// TODO: add a verified AIVI example here
```

## Why pipes instead of nested calls?

Consider a computation with five steps. With nested calls, you must read inside-out:

```text
step5 (step4 (step3 (step2 (step1 input))))
```

With pipes:

```text
// TODO: add a verified AIVI example here
```

The computation reads in execution order, top to bottom.
Each step is on its own line. Inserting, removing, or reordering steps is straightforward.

## The gate pipe ?\|>

`?\|>` passes the value only if a condition is true.
If the condition is false, the value is **suppressed** — nothing flows downstream.

```text
// TODO: add a verified AIVI example here
```

`validInput` only has a value when `rawInput` is non-empty.
This is useful for validation: downstream signals only fire when the gate is open.

```text
// TODO: add a verified AIVI example here
```

## The truthy and falsy pipes T\|> and F\|>

`T\|>` and `F\|>` are conditional path selectors. Given a `Bool` on the left, they pass
a value (not the condition) depending on whether it is `True` or `False`:

```text
// TODO: add a verified AIVI example here
```

`T\|>` and `F\|>` are usually used in pairs. They are the AIVI alternative to `if`/`else`:

```text
// TODO: add a verified AIVI example here
```

If `isOpposite candidate current` is `True`, the result is `current`.
Otherwise it is `candidate`.

On signals, `T|>` / `F|>` runs pointwise over the committed snapshot for the currently supported
carrier slice: `Signal Bool`, `Signal (Option A)`, `Signal (Result E A)`, and
`Signal (Validation E A)`. The branch result stays a signal. Signal-filter `?|>` remains a
separate scheduler-owned pipeline slice.

## Operator quick reference

::: details Pipe operator quick reference

| Operator | Name | Reads as |
|---|---|---|
| `\|>` | transform | "then apply" |
| `?\|>` | gate | "only if" |
| `\|\|>` | case | "match against" — see next chapter |
| `*\|>` | map | "for each item in list, apply" |
| `&\|>` | apply | "zip-apply across signals" |
| `T\|>` | truthy branch | "if true, use" |
| `F\|>` | falsy branch | "if false, use" |
| `@\|>` | recur start | "starting from, fold over time" |
| `<\|@` | recur step | "on each event, update with" |
| `<\|*` | fan-in | "join the collection from *\|> with a reducer" |

:::

## Summary

- `\|>` passes a value through a function, left-to-right.
- `.field` is shorthand for `\r => r.field`.
- Pipes chain: each `\|>` is one step.
- `?\|>` gates: value passes only when the predicate is true.
- `T\|>` and `F\|>` select branches based on a `Bool`.

[Next: Pattern Matching →](/tour/04-pattern-matching)
