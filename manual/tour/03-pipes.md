# Pipes

Pipes are the centrepiece of AIVI's surface syntax.
The idea is borrowed from Unix: data flows from left to right through a sequence of transformations.

## The transform pipe \|>

`\|>` takes the value on its left and passes it as the first argument to the function on its right.

```text
// start with the value 42
// pass it through 'double'
// then pass the result through 'toString', binding the final value to 'result'
```

This is equivalent to `toString (double 42)`. Pipes let you read computation top-to-bottom
instead of inside-out.

Compare these two forms of the same computation:

```text
// nested style: clamp rawScore to 0–100, add bonus of 10, then format the score (reads inside-out)
// pipe style: start with rawScore, clamp to 0–100, add a bonus of 10, then format the score (reads top-to-bottom)
```

When you pass partial arguments before the piped value, `\|>` inserts the left-hand value
as the **last** argument:

```text
// clamp is a function taking lower bound, upper bound, and value
// pipe rawScore into clamp with bounds 0 and 100
// the piped value is inserted as the final argument to clamp
```

## Projection shorthand

A common pattern is projecting a field from a record:

```text
// project the 'username' field from user, binding the result to 'name'
```

The `.field` syntax is a shorthand for `\r => r.field`.
It composes naturally in pipes:

```text
// derive 'boardTitle' from the board signal
// extract its width field
// format it as the text "Board width: W"
```

## Chaining pipes

Pipes chain arbitrarily. Each `\|>` is one step in the computation:

```text
// derive 'scoreLabel' from the game signal
// extract the score field
// multiply the score by 10
// format it as "Score: N pts"
```

## Why pipes instead of nested calls?

Consider a computation with five steps. With nested calls:

```text
// apply five transformation steps to input in sequence, reading from innermost to outermost
```

You must read from the inside out, matching parentheses as you go.

With pipes:

```text
// start with input
// pass through step1, then step2, then step3, then step4, then step5 in sequence
// bind the final value to 'result'
```

The computation reads in execution order, top to bottom.
Each step is on its own line. Inserting, removing, or reordering steps is straightforward.

## The gate pipe ?\|>

`?\|>` passes the value only if a condition is true.
If the condition is false, the value is **suppressed** — nothing flows downstream.

```text
// declare a predicate 'isNonEmpty' that returns True when text is not empty
// derive 'validInput' from rawInput, suppressing the value when rawInput is empty
// validInput only carries a value when rawInput passes the isNonEmpty gate
```

`validInput` only has a value when `rawInput` is non-empty.
This is useful for validation: downstream signals only fire when the gate is open.

```text
// declare a predicate 'hasName' that checks the form has a non-empty name field
// declare a predicate 'hasEmail' that checks the form has a non-empty email field
// derive 'submittable' from formData, gating on both hasName and hasEmail
// submittable only has a value when both name and email are non-empty
```

## The truthy and falsy pipes T\|> and F\|>

`T\|>` and `F\|>` are conditional path selectors. Given a `Bool` on the left, they pass
a value (not the condition) depending on whether it is `True` or `False`:

```text
// declare a function 'absolute' taking an integer n
// if n is less than 0, return n negated
// otherwise return n unchanged
```

`T\|>` and `F\|>` are usually used in pairs. They are the AIVI alternative to `if`/`else`:

```text
// declare a function 'applyDirection' taking a current and a candidate direction
// if the candidate is opposite to the current direction, keep the current direction
// otherwise use the candidate direction
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
