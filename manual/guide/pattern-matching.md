# Pattern Matching

AIVI uses pattern matching for branching. There is no `if` / `else` statement layer on top of the language: values are inspected directly with the case-split pipe `||>`.

Every branch is an expression, and the compiler checks that your match is exhaustive.

## The case-split pipe `||>`

Use `||>` when you want to branch on a value:

```aivi
type Int -> Text
func describeNumber = n => n
 ||> 0 -> "zero"
 ||> 1 -> "one"
 ||> _ -> "many"

value sampleDescription = describeNumber 1
```

The `_` pattern matches anything. Arms are tried from top to bottom. Within pipe bodies, `.`
is the ambient subject; `_` is only a discard pattern or discard binding.

Today `||>` supports patterns only. Case-stage guard syntax is not implemented end to end yet, so
the current workaround is to match first and then compute a `Bool` in the arm body or in a named
helper.

## Matching custom types

Custom sum types are declared with `type` and matched by constructor name:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

type Direction -> Text
func directionLabel = direction => direction
 ||> Up    -> "up"
 ||> Down  -> "down"
 ||> Left  -> "left"
 ||> Right -> "right"

value currentDirection = directionLabel Left
```

If a constructor carries a payload, bind that payload in the pattern:

```aivi
type LoadState =
  | NotAsked
  | Loaded Text
  | Failed Text

type LoadState -> Text
func describeLoadState = state => state
 ||> NotAsked      -> "waiting"
 ||> Loaded name   -> "loaded {name}"
 ||> Failed reason -> "error {reason}"

value stateMessage = describeLoadState (Loaded "Ada")
```

## Wildcards

Use `_` when you only care about one or two cases and want to discard the unmatched payload:

```aivi
type Status =
  | Running
  | Paused
  | Stopped

type Status -> Bool
func isRunning = status => status
 ||> Running -> True
 ||> _       -> False

value runningNow = isRunning Running
```

## Condition-first branching

When the choice is really a boolean condition, calculate the condition first and then branch with `T|>` / `F|>`:

```aivi
type Int -> Text
func classifyNumber = n => n > 0
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

type Profile -> Text
func profileSummary = profile => profile
 ||> { name, score } -> "{name} scored {score}"

value summaryText =
    profileSummary {
        name: "Ada",
        score: 100
    }
```

Because case-stage guards are not implemented end to end yet, combine record destructuring with a
follow-up boolean check when you need an extra condition:

```aivi
type Profile = {
    name: Text,
    score: Int
}

type Profile -> Bool
func isTopScore = profile => profile
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
type Point =
  | Point Int Int

type Direction =
  | Up
  | Down
  | Left
  | Right

type (Point, Direction) -> Point
func step = move => move
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

Patterns can be nested as deeply as the value requires:

```aivi
type Inner = A | B

type Outer =
  | Outer Inner

type Outer -> Text
func describeOuter = outer => outer
 ||> Outer A -> "outer A"
 ||> Outer B -> "outer B"

value outerLabel = describeOuter (Outer A)
```

## Built-in sum types

`Option` and `Result` are ordinary tagged types, so matching them feels the same:

```aivi
type Option Text -> Text
func displayName = maybeName => maybeName
 ||> Some name -> name
 ||> None      -> "anonymous"

value shownName = displayName (Some "Ada")
```

```aivi
type Result Text Int -> Text
func handleResult = result => result
 ||> Ok value    -> "got {value}"
 ||> Err message -> "failed {message}"

value handledResult = handleResult (Ok 42)
```

## Boolean branches

When the subject is already `Bool`, `T|>` and `F|>` are shorter than a full match:

```aivi
type Bool -> Text
func statusLabel = active => active
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

| Operator | Meaning |
| --- | --- |
| `\|\|>` | Pattern match / case split |
| `T\|>` | Branch for `True` |
| `F\|>` | Branch for `False` |
