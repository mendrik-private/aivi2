# Pipes

Pipes are the centrepiece of AIVI's surface syntax.
The idea is borrowed from Unix: data flows from left to right through a sequence of transformations.

## The transform pipe `|>`

`|>` takes the value on its left and passes it as the first argument to the function on its right.

```aivi
// TODO: add a verified AIVI example here
```

This is equivalent to `toString (double 42)`. Pipes let you read computation top-to-bottom
instead of inside-out.

Compare these two forms of the same computation:

```aivi
// TODO: add a verified AIVI example here
```

When you pass partial arguments before the piped value, `|>` inserts the left-hand value
as the **last** argument:

```aivi
// TODO: add a verified AIVI example here
```

## Projection shorthand

A common pattern is projecting a field from a record:

```aivi
// TODO: add a verified AIVI example here
```

The `.field` syntax is a shorthand for `#r => r.field`.
It composes naturally in pipes:

```aivi
// TODO: add a verified AIVI example here
```

## Pipe context memos

Any pipe step can introduce a named binding that is available in **every subsequent stage**
of the same pipe chain. There are two placements:

**Before the expression** — names the incoming subject:

```aivi
[1 .. 10]
 *|> i i % 2    // name the incoming element 'i', fan out with i % 2
 ||> 0 => "even {i}"
 ||> _ => "odd {i}"
```

`#i` captures the element as it enters the step. The expression `i % 2` then computes
the new value flowing forward. `i` remains accessible in all stages below.

**After the expression** — names the result:

```aivi
uid
 |> fetchUser result
 T|> { user: Some .body, status: result.status }
 F|> { user: None,       status: "failed" }
```

`#result` captures the output of `fetchUser`. The `T|>` and `F|>` branches can then
reference both the unwrapped value (via `.body` for the ambient projection) and
`result.status` from the memo.

### Ambient value and ambient projection

Within a pipe step, `_` is the **ambient value** — the value currently flowing through.
`.field` is **ambient projection** — shorthand for accessing a field on the ambient value.
You cannot write `_.field`; write `.field` instead.

## Chaining pipes

Pipes chain arbitrarily. Each `|>` is one step in the computation:

```aivi
// TODO: add a verified AIVI example here
```

## Why pipes instead of nested calls?

Consider a computation with five steps. With nested calls, you must read inside-out:

```aivi
step5 (step4 (step3 (step2 (step1 input))))
```

With pipes:

```aivi
// TODO: add a verified AIVI example here
```

The computation reads in execution order, top to bottom.
Each step is on its own line. Inserting, removing, or reordering steps is straightforward.

## The gate pipe `?|>`

`?|>` passes the value only if a condition is true.
If the condition is false, the value is **suppressed** — nothing flows downstream.
Inside a `*|>` fan-out, suppression means the item is **dropped from the result list**.

```aivi
// TODO: add a verified AIVI example here
```

`validInput` only has a value when `rawInput` is non-empty.
This is useful for validation: downstream signals only fire when the gate is open.

```aivi
// TODO: add a verified AIVI example here
```

## The truthy and falsy pipes `T|>` and `F|>`

`T|>` and `F|>` are conditional path selectors. Given a `Bool` on the left, they pass
a value (not the condition) depending on whether it is `True` or `False`:

```aivi
// TODO: add a verified AIVI example here
```

`T|>` and `F|>` are usually used in pairs. They are the AIVI alternative to `if`/`else`:

```aivi
// TODO: add a verified AIVI example here
```

If `isOpposite candidate current` is `True`, the result is `current`.
Otherwise it is `candidate`.

## `T|>` and `F|>` with ADTs

`T|>` and `F|>` also work with `Option` and `Result`. The `T|>` branch receives the **inner
value** unwrapped; the `F|>` branch receives the error or absence:

```aivi
opt
  T|> "Hello {_}"      // same as: opt ||> Some x => "Hello {x}"
  F|> "Goodbye"        // same as:     ||> None   => "Goodbye"
```

```aivi
res
  T|> doFun _          // same as: res ||> Ok  x => doFun x
  F|> log _            //          res ||> Err e => log e
```

The `_` placeholder stands in for the unwrapped value on that branch.
This is more compact than spelling out the full `||>` match when you only care about the
success or failure path.

On signals, `T|>` / `F|>` runs pointwise over the committed snapshot for the currently supported
carrier slice: `Signal Bool`, `Signal (Option A)`, `Signal (Result E A)`, and
`Signal (Validation E A)`. The branch result stays a signal. The gate pipe `?|>` on signals remains a
separate scheduler-owned pipeline slice.

## The map pipe `*|>`

`*|>` applies a function to every element of a `List`, producing a new `List`:

```aivi
// TODO: add a verified AIVI example here
```

It is the pipe equivalent of `map`. The `*` reads as "for each":

```aivi
// TODO: add a verified AIVI example here
```

A `?|>` inside a `*|>` fan-out **skips the item** — it acts as a filter rather than a
gate on a signal. Items for which the predicate is false are dropped from the result list:

```aivi
// TODO: add a verified AIVI example here
```

This combines map and filter in a single pipeline without a separate `filter` call.

## The fan-in pipe `<|*`

`<|*` closes a `*|>` fan-out by collecting the results back into a single value using a
reducer function. Together, `*|>` and `<|*` express a map–reduce in a straight pipeline:

```aivi
// TODO: add a verified AIVI example here
```

The reducer receives the accumulated value and each element in turn.

## The recur-start pipe `@|>`

`@|>` enters the recurrent loop. The seed value sits on its left; the driver (source or
cursor) sits on its right:

```aivi
// TODO: add a verified AIVI example here
```

The seed is the accumulated state before any events arrive. The driver wakes the loop on
each event.

## The recur-step pipe `<|@`

`<|@` is the step function of the recurrence. It receives the current accumulated state and
returns the next state:

```aivi
// TODO: add a verified AIVI example here
```

Guards (`?|>`) may appear between `@|>` and `<|@` to suppress a step entirely when a
condition is false:

```aivi
initial
 @|> cursor
 ?|> cursor.hasNext
 <|@ cursor.next
```

## The apply pipe `&|>`

`&|>` zips two signals together, applying a signal of functions to a signal of values
pointwise. It is the signal equivalent of `<*>` in applicative functors:

```aivi
// TODO: add a verified AIVI example here
```

Every time either signal updates, the result recomputes. This is how you combine multiple
independent signals into one derived value without explicit `zip` calls.

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
| `@\|>` | recur enter | "enter loop driven by" |
| `<\|@` | recur step | "on each wakeup, advance with" |
| `<\|*` | fan-in | "join the collection from *\|> with a reducer" |

:::

## Summary

- `|>` passes a value through a function, left-to-right.
- `.field` is shorthand for `#r => r.field`.
- Pipes chain: each `|>` is one step.
- `?|>` gates: suppresses the value when false — drops the item inside `*|>`.
- `T|>` and `F|>` select branches based on a `Bool`, `Option`, or `Result`; the inner value is unwrapped automatically.
- `#name` before an expression names the incoming subject; `#name` after names the result — both are available in all stages below.
- `_` is the ambient value; `.field` is ambient projection (not `_.field`).
- `*|>` maps a function over every element of a list.
- `<|*` collects a `*|>` fan-out back into a single value with a reducer.
- `seed @|> driver` enters the recurrence loop driven by a source or cursor.
- `<|@` is the step function; `?|>` between `@|>` and `<|@` skips the iteration when false.
- `&|>` zips a signal of functions with a signal of values pointwise.

[Next: Pattern Matching →](/tour/04-pattern-matching)
