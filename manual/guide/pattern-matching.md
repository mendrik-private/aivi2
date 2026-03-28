# Pattern Matching

AIVI uses pattern matching for branching. There is no `if` / `else` statement layer on top of the language: values are inspected directly with the case-split pipe `||>`.

Every branch is an expression, and the compiler checks that your match is exhaustive.

## The case-split pipe `||>`

Use `||>` when you want to branch on a value:

```aivi
fun describeNumber: Text n:Int =>
    n
     ||> 0 -> "zero"
     ||> 1 -> "one"
     ||> _ -> "many"

value sampleDescription = describeNumber 1
```

The `_` pattern matches anything. Arms are tried from top to bottom.

## Matching custom data

Custom sum types are declared with `data` and matched by constructor name:

```aivi
data Direction =
  | Up
  | Down
  | Left
  | Right

fun directionLabel: Text direction:Direction =>
    direction
     ||> Up    -> "up"
     ||> Down  -> "down"
     ||> Left  -> "left"
     ||> Right -> "right"

value currentDirection = directionLabel Left
```

If a constructor carries data, bind that payload in the pattern:

```aivi
data LoadState =
  | NotAsked
  | Loaded Text
  | Failed Text

fun describeLoadState: Text state:LoadState =>
    state
     ||> NotAsked      -> "waiting"
     ||> Loaded name   -> "loaded {name}"
     ||> Failed reason -> "error {reason}"

value stateMessage = describeLoadState (Loaded "Ada")
```

## Wildcards

Use `_` when you only care about one or two cases:

```aivi
data Status =
  | Running
  | Paused
  | Stopped

fun isRunning: Bool status:Status =>
    status
     ||> Running -> True
     ||> _       -> False

value runningNow = isRunning Running
```

## Condition-first branching

When the choice is really a boolean condition, calculate the condition first and then branch with `T|>` / `F|>`:

```aivi
fun classifyNumber: Text n:Int =>
    n > 0
     T|> "positive"
     F|> "not positive"

value numberClass = classifyNumber 12
```

That keeps the branch expression explicit without introducing a separate statement form.

## Record patterns

Records can be destructured directly in a match arm:

```aivi
type Profile = {
    name: Text,
    score: Int
}

fun profileSummary: Text profile:Profile =>
    profile
     ||> { name, score } -> "{name} scored {score}"

value summaryText =
    profileSummary {
        name: "Ada",
        score: 100
    }
```

You can combine record destructuring with a follow-up boolean check:

```aivi
type Profile = {
    name: Text,
    score: Int
}

fun isTopScore: Bool profile:Profile =>
    profile
     ||> { score } -> score >= 100

value topScore =
    isTopScore {
        name: "Grace",
        score: 120
    }
```

## Tuple patterns

Tuples let you match several values at once:

```aivi
data Point =
  | Point Int Int

data Direction =
  | Up
  | Down
  | Left
  | Right

fun step: Point move:(Point, Direction) =>
    move
     ||> (Point x y, Up)    -> Point x (y - 1)
     ||> (Point x y, Down)  -> Point x (y + 1)
     ||> (Point x y, Left)  -> Point (x - 1) y
     ||> (Point x y, Right) -> Point (x + 1) y

value movedPoint =
    step (
        Point 4 9,
        Up
    )
```

## Nested patterns

Patterns can be nested as deeply as the data requires:

```aivi
data Inner = A | B

data Outer =
  | Outer Inner

fun describeOuter: Text outer:Outer =>
    outer
     ||> Outer A -> "outer A"
     ||> Outer B -> "outer B"

value outerLabel = describeOuter (Outer A)
```

## Built-in sum types

`Option` and `Result` are ordinary data types, so matching them feels the same:

```aivi
fun displayName: Text maybeName:(Option Text) =>
    maybeName
     ||> Some name -> name
     ||> None      -> "anonymous"

value shownName = displayName (Some "Ada")
```

```aivi
fun handleResult: Text result:(Result Text Int) =>
    result
     ||> Ok value    -> "got {value}"
     ||> Err message -> "failed {message}"

value handledResult = handleResult (Ok 42)
```

## Boolean branches

When the subject is already `Bool`, `T|>` and `F|>` are shorter than a full match:

```aivi
fun statusLabel: Text active:Bool =>
    active
     T|> "active"
     F|> "inactive"

value currentStatus = statusLabel True
```

## Exhaustiveness

AIVI checks that every constructor is covered. In practice that means:

- list every constructor explicitly, or
- finish with `_` when you want a catch-all branch.

That guarantee is one of the reasons pattern matching is the normal way to branch in AIVI.

## Summary

| Pattern | Meaning |
| --- | --- |
| `Constructor` | Match one exact constructor |
| `Constructor name` | Match a constructor and bind its payload |
| `{ field, other }` | Destructure selected record fields |
| `(a, b)` | Match a tuple |
| `_` | Match anything without binding |
| `pattern if cond` | Match only when the extra condition holds |

| Operator | Meaning |
| --- | --- |
| `||>` | Pattern match / case split |
| `T|>` | Branch for `True` |
| `F|>` | Branch for `False` |
