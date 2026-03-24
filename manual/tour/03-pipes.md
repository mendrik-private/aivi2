# Pipes

Pipes are the centrepiece of AIVI's surface syntax.
The idea is borrowed from Unix: data flows from left to right through a sequence of transformations.

## The transform pipe \|>

`\|>` takes the value on its left and passes it as the first argument to the function on its right.

```aivi
fun double:Int #n:Int =>
    n * 2

fun asText:Text #n:Int =>
    "{n}"

val result:Text =
    42
     |> double
     |> asText
```

This is equivalent to `toString (double 42)`. Pipes let you read computation top-to-bottom
instead of inside-out.

Compare these two forms of the same computation:

```aivi
fun clampLow:Int #lo:Int #value:Int =>
    value < lo
     T|> lo
     F|> value

fun clamp:Int #lo:Int #hi:Int #value:Int =>
    value > hi
     T|> hi
     F|> clampLow lo value

fun addBonus:Int #bonus:Int #score:Int =>
    score + bonus

fun formatScore:Text #score:Int =>
    "Score: {score}"

val rawScore:Int = 95

val formatted:Text =
    rawScore
     |> clamp 0 100
     |> addBonus 10
     |> formatScore
```

When you pass partial arguments before the piped value, `|>` inserts the left-hand value
as the **last** argument:

```aivi
fun clampLow:Int #lo:Int #value:Int =>
    value < lo
     T|> lo
     F|> value

fun clamp:Int #lo:Int #hi:Int #value:Int =>
    value > hi
     T|> hi
     F|> clampLow lo value

val rawScore:Int = 95

val clamped:Int =
    rawScore
     |> clamp 0 100
```

## Projection shorthand

A common pattern is projecting a field from a record:

```aivi
type User = {
    id: Int,
    username: Text,
    email: Text
}

val user:User = {
    id: 1,
    username: "ada",
    email: "ada@example.com"
}

fun getName:Text #user:User =>
    user
     |> .username
```

The `.field` syntax is a shorthand for `\r => r.field`.
It composes naturally in pipes:

```aivi
type Board = {
    width: Int,
    height: Int
}

fun formatWidth:Text #n:Int =>
    "Board width: {n}"

sig board : Signal Board = {
    width: 10,
    height: 10
}

sig boardTitle : Signal Text =
    board
     |> .width
     |> formatWidth
```

## Chaining pipes

Pipes chain arbitrarily. Each `\|>` is one step in the computation:

```aivi
type Game = { score: Int }

fun timesten:Int #n:Int =>
    n * 10

fun asScoreText:Text #n:Int =>
    "Score: {n} pts"

sig game : Signal Game = {
    score: 0
}

sig scoreLabel : Signal Text =
    game
     |> .score
     |> timesten
     |> asScoreText
```

## Why pipes instead of nested calls?

Consider a computation with five steps. With nested calls, you must read inside-out:

```text
step5 (step4 (step3 (step2 (step1 input))))
```

With pipes:

```aivi
fun step1:Int #n:Int =>
    n + 1

fun step2:Int #n:Int =>
    n * 2

fun step3:Int #n:Int =>
    n - 3

fun step4:Int #n:Int =>
    n * n

fun step5:Text #n:Int =>
    "result: {n}"

val input:Int = 5

val result:Text =
    input
     |> step1
     |> step2
     |> step3
     |> step4
     |> step5
```

The computation reads in execution order, top to bottom.
Each step is on its own line. Inserting, removing, or reordering steps is straightforward.

## The gate pipe ?\|>

`?\|>` passes the value only if a condition is true.
If the condition is false, the value is **suppressed** — nothing flows downstream.

```aivi
fun isNonEmpty:Bool #text:Text =>
    text != ""

sig rawInput : Signal Text = ""

sig validInput : Signal Text =
    rawInput
     ?|> isNonEmpty
```

`validInput` only has a value when `rawInput` is non-empty.
This is useful for validation: downstream signals only fire when the gate is open.

```aivi
type FormData = {
    name: Text,
    email: Text
}

fun hasName:Bool #form:FormData =>
    form.name != ""

fun hasEmail:Bool #form:FormData =>
    form.email != ""

sig formData : Signal FormData = {
    name: "",
    email: ""
}

sig submittable : Signal FormData =
    formData
     ?|> hasName
     ?|> hasEmail
```

## The truthy and falsy pipes T\|> and F\|>

`T\|>` and `F\|>` are conditional path selectors. Given a `Bool` on the left, they pass
a value (not the condition) depending on whether it is `True` or `False`:

```aivi
fun absolute:Int #n:Int =>
    n < 0
     T|> 0 - n
     F|> n
```

`T\|>` and `F\|>` are usually used in pairs. They are the AIVI alternative to `if`/`else`:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

fun oppositeOf:Direction #d:Direction =>
    d
     ||> Up    => Down
     ||> Down  => Up
     ||> Left  => Right
     ||> Right => Left

fun applyDirection:Direction #candidate:Direction #current:Direction =>
    oppositeOf candidate == current
     T|> current
     F|> candidate
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
