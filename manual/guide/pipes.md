# Pipes & Operators

Pipes are the main way to express flow in AIVI. Instead of deeply nested calls, you write a left-to-right transformation pipeline.

## The basic pipe `|>`

`|>` sends the value on the left into the function on the right:

```aivi
fun double: Int n:Int =>
    n * 2

fun addOne: Int n:Int =>
    n + 1

value result =
    5
     |> double
     |> addOne
```

That reads in execution order: start with `5`, then double it, then add one.

## Passing extra arguments

The piped value becomes the last argument:

```aivi
fun multiply: Int factor:Int n:Int =>
    factor * n

value scaled =
    5
     |> multiply 3
```

`multiply 3` produces a function waiting for the final argument, so the pipeline stays compact.

## Pattern matching with `||>`

`||>` is the branching pipe:

```aivi
data Status =
  | Draft
  | Published
  | Archived

fun statusLabel: Text status:Status =>
    status
     ||> Draft     -> "draft"
     ||> Published -> "published"
     ||> Archived  -> "archived"

value currentLabel = statusLabel Published
```

## Boolean branches with `T|>` and `F|>`

For `Bool`, the dedicated true/false pipes are shorter than a full match:

```aivi
fun availabilityLabel: Text ready:Bool =>
    ready
     T|> "ready"
     F|> "waiting"

value shownAvailability = availabilityLabel True
```

## Filtering with `?|>`

`?|>` keeps a value only when a predicate holds, returning `Option A`:

```aivi
type User = {
    active: Bool,
    age: Int,
    email: Text
}

value seed: User = {
    active: True,
    age: 32,
    email: "ada@example.com"
}

value activeAdult: (Option User) =
    seed
     ?|> (.active and .age > 18)
```

This is especially useful when a later step should only run for values that pass a gate.

## Previous-value pipe `~|>`

`~|>` pairs the current value with a previous one. The argument supplies the initial previous value:

```aivi
signal score = 10

signal previousScore =
    score
     ~|> 0
```

## Diff pipe `-|>`

`-|>` tracks a change relative to the previous value:

```aivi
signal score = 10

signal scoreDelta =
    score
     -|> 0
```

## One important rule: no nested pipes

Pipes must stay on the top-level expression spine. If you need a pipe inside another expression, pull it out into a named helper:

```aivi
fun normalizeTitle: Text title:Text =>
    title
     ||> "Inbox" -> "priority"
     ||> _       -> title

fun displayTitle: Text title:Text =>
    normalizeTitle title
```

That keeps pipe flow explicit and matches the compiler's current nesting rule.

## Operators in this guide

| Operator | Meaning |
| --- | --- |
| `|>` | Apply a function |
| `||>` | Pattern match / case split |
| `T|>` | Branch for `True` |
| `F|>` | Branch for `False` |
| `?|>` | Filter to `Option` |
| `~|>` | Carry previous value |
| `-|>` | Compute a difference |
