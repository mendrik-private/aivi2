# Pipes & Operators

Pipes are the main way to express flow in AIVI. Instead of deeply nested calls, you write a left-to-right transformation pipeline.

## The basic pipe `|>`

`|>` sends the value on the left into the function on the right:

```aivi
type Int -> Int
func double = n =>
    n * 2

type Int -> Int
func addOne = n =>
    n + 1

value result = 5
  |> double
  |> addOne
```

That reads in execution order: start with `5`, then double it, then add one.

## Passing extra arguments

The piped value becomes the last argument:

```aivi
type Int -> Int -> Int
func multiply = factor n =>
    factor * n

value scaled = 5
  |> multiply 3
```

`multiply 3` produces a function waiting for the final argument, so the pipeline stays compact.

## Type-level record row pipes

The same `|>` surface is also available in type position for record row transforms.

In a type pipeline, the type on the left becomes the final argument to the transform on the right:

```aivi
type User = {
    id: Int,
    name: Text,
    createdAt: Text,
    isAdmin: Bool
}

type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }
```

That is equivalent to:

```aivi
type UserPublic = User |> Omit (isAdmin) |> Rename { createdAt: created_at }
```

Type-level pipes are currently limited to record row transforms such as `Pick`, `Omit`, `Optional`, `Required`, `Defaulted`, and `Rename`.

## Pattern matching with `||>`

`||>` is the branching pipe:

```aivi
type Status =
  | Draft
  | Published
  | Archived

type Status -> Text
func statusLabel = status => status
 ||> Draft     -> "draft"
 ||> Published -> "published"
 ||> Archived  -> "archived"

value currentLabel = statusLabel Published
```

Inside pipe stages, `.` names the ambient subject. `_` is only a discard pattern or discard
binder; it never means “the current subject”.

Today `||>` supports patterns only. Case-stage guard syntax is not implemented end to end yet,
so the current workaround is to match the shape first and then compute a `Bool` in the arm body
or a helper, branching with `T|>` / `F|>` if needed.

## Boolean branches with `T|>` and `F|>`

For `Bool`, the dedicated true/false pipes are shorter than a full match:

```aivi
type Bool -> Text
func availabilityLabel = ready => ready
 T|> "ready"
 F|> "waiting"

value shownAvailability = availabilityLabel True
```

## Filtering with `?|>`

`?|>` keeps a value only when a predicate holds.

For ordinary values, it returns `Option A`:

```aivi
type User = {
    active: Bool,
    age: Int,
    email: Text
}

value seed : User = {
    active: True,
    age: 32,
    email: "ada@example.com"
}

value activeAdult : (Option User) = seed
 ?|> .active and .age > 18
```

This is especially useful when a later step should only run for values that pass a gate.

For `Signal A`, the same operator keeps the `Signal A` carrier and filters out updates whose predicate fails.

## `when` is not a pipe

Reactive update clauses use a separate top-level form:

```aivi
signal left = 20
signal right = 22
signal total = 0
signal ready = True

when ready => total <- left + right
when tick _ => total <- left + right
```

This is different from `?|>` and the rest of pipe algebra:

- `when` does not live inside a pipe spine
- `when` targets an existing signal explicitly
- `when` can match a named signal emission with `when <signal> <pattern> => ...`
- the body has no ambient subject such as `.`
- `when` is for event-driven commits, while pipes are for left-to-right expression flow

If you can describe the logic as “take this current value and keep transforming it”, use pipes. If you mean “when this guard fires, commit a value into that signal”, use `when`.

## Previous-value pipe `~|>`

`~|>` pairs the current value with a previous one. The argument supplies the initial previous value:

```aivi
signal score = 10

signal previousScore = score
 ~|> 0
```

## Diff pipe `-|>`

`-|>` tracks a change relative to the previous value:

```aivi
signal score = 10

signal scoreDelta = score
 -|> 0
```

## Other accepted pipe forms

The current parser/compiler also accept these pipe-stage forms:

| Operator | Meaning |
| --- | --- |
| `\|` | Tap / observe while preserving the current subject |
| `*\|>` | Map / fan-out |
| `<\|*` | Fan-out join |
| `&\|>` | Applicative cluster stage |
| `!\|>` | Validation stage |
| `+\|>` | Stateful accumulation |
| `@\|>` | Explicit recurrence start |
| `<\|@` | Explicit recurrence step |

Some of these advanced stages still have narrower validation/runtime coverage than the core `|>`, `||>`, `?|>`, `T|>`, `F|>`, `~|>`, and `-|>` forms. In particular, `+|>` now lowers through the recurrence path for signal accumulation, while more exotic applicative/validation combinations still have the narrower executable slice documented in the RFC.

## One important rule: no nested pipes

Pipes must stay on the top-level expression spine. If you need a pipe inside another expression, pull it out into a named helper:

```aivi
type Text -> Text
func normalizeTitle = title => title
 ||> "Inbox" -> "priority"
 ||> _       -> title

type Text -> Text
func displayTitle = title =>
    normalizeTitle title
```

That keeps pipe flow explicit and matches the compiler's current nesting rule.

## Operators in this guide

| Operator | Meaning |
| --- | --- |
| `\|>` | Apply a function |
| `\|\|>` | Pattern match / case split |
| `T\|>` | Branch for `True` |
| `F\|>` | Branch for `False` |
| `?\|>` | Gate values; ordinary values become `Option`, signals stay `Signal` |
| `~\|>` | Carry previous value |
| `-\|>` | Compute a difference |
| `\|` | Tap / preserve the current subject |
| `*\|>` | Map / fan-out |
| `<\|*` | Fan-out join |
| `&\|>` | Applicative cluster stage |
| `!\|>` | Validation stage |
| `+\|>` | Stateful accumulation |
| `@\|>` | Explicit recurrence start |
| `<\|@` | Explicit recurrence step |
