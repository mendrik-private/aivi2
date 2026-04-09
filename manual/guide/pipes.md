# Pipes & Operators

In most languages, you nest function calls: `addOne(double(5))`. The deeper the nesting, the harder it is to read. In AIVI, you write the same thing as a left-to-right pipeline:

```aivi
value result = 5
  |> double
  |> addOne
```

This is not just syntax sugar. Pipes are the **primary control flow** in AIVI. They replace `if`/`else` (with pattern matching pipes), loops (with collection combinators in pipes), and nested calls (with transform pipes). Understanding pipes is understanding how AIVI programs are structured.

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

## Choosing the pipe subject in a function header

When a helper takes extra arguments but the body should start from one chosen parameter, mark that
parameter with `!` and begin the body with `|>`:

```aivi
type Int -> Int -> Int
func add = left right =>
    left + right

type Int -> Int -> Int
func addFrom = amount value!
  |> add amount

value total = addFrom 2 40
```

`value!` means "push `value` into the ordinary single-subject pipe flow." The same header form also
works with patch-rooted continuations like `counter! amount` followed by `<| { ... }`, and with
projection selectors like `state { x.y.z! }` when the flow should start from a nested field.

## Remembering stage values with `#name`

`#name` is the pipe memo operator. Use it when you want the convenience of a local `let`
without leaving the pipe.

- Put `#name` right after the operator to name the stage input for that stage body.
- Put `#name` at the end of the stage to name the stage result for later stages.

```aivi
value total : Int = 20
  |> #before before + 1 #after
  |> after + before
```

Here `before` only exists while the first stage body runs. `after` is available to the rest of
the pipe.

Branching stages support the same pattern. When you want the merged branch result later, write the
same result memo on each arm:

```aivi
type StageChoice =
  | Ready Int
  | Missing

value resolvedScore : Int = Ready 2
 ||> #input Ready value -> value + 1 #resolved
 ||> Missing            -> 0 #resolved
  |> resolved
```

This is the pipe-native replacement for the local binding you might otherwise reach for in a
statement-oriented language. Pipe memos work across ordinary pipe stages, including transforms,
taps, gates, case splits, truthy/falsy pairs, fan-out and join, validation, temporal signal
stages, accumulation, and explicit recurrence. Applicative clusters (`&|>`) use separate
applicative-cluster semantics rather than this single-subject memo flow.

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

Type-level pipes are currently limited to record row transforms such as `Pick`, `Omit`, `Optional`, `Required`, `Defaulted`, and `Rename`.

## Pattern matching with `||>`

`||>` is the branching pipe:

```aivi
type Status =
  | Draft
  | Published
  | Archived

type Status -> Text
func statusLabel = .
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

## Truthy/falsy branches with `T|>` and `F|>`

`T|>` / `F|>` are shorthand for the built-in two-way carriers with a canonical
truthy/falsy split:

| Subject type | `T\|>` matches | `F\|>` matches |
| --- | --- | --- |
| `Bool` | `True` | `False` |
| `Option A` | `Some _` | `None` |
| `Result E A` | `Ok _` | `Err _` |
| `Validation E A` | `Valid _` | `Invalid _` |

The same shorthand also lifts through one outer `Signal (...)`, so `Signal Bool`
and `Signal (Option A)` work the same way pointwise. Inside a chosen branch,
`.` is rebound to the matched payload when that constructor has exactly one
payload. Use `||>` when you need named bindings, nested patterns, or a
non-canonical carrier.

```aivi
type Bool -> Text
func availabilityLabel = .
 T|> "ready"
 F|> "waiting"

value shownAvailability = availabilityLabel True
```

```aivi
type User = { name: Text }

type Option User -> Text
func userNameOrGuest = .
 T|> .name
 F|> "guest"

value shownUserName =
    userNameOrGuest (
        Some {
            name: "Ada"
        }
    )
```

```aivi
type Result Text Int -> Text
func loadStatus = .
 T|> "loaded"
 F|> .

value currentLoadStatus = loadStatus (Err "network down")
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

## Signal merge is not a pipe

Signal merge syntax lives alongside pipes but is **not** a pipe operator. Merging source signals
into a new signal uses `|` to list sources, then `||>` arms with `=>` to discriminate by source and
pattern. See the [Signals guide](/guide/signals#signal-merge-and-reactive-arms) for full details.

```aivi
type Key = Key Text

type Event = Tick | Turn Text

@source timer.every 120ms
signal tick : Signal Unit

@source window.keyDown
signal keyDown : Signal Key

signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowUp") => Turn "up"
  ||> _ => Tick
```

This is different from `?|>` and the rest of pipe algebra:

- merge bodies use `=>` (fat arrow), pipe case arms use `->` (thin arrow)
- merge targets the declaring signal, not a flowing subject
- the body has no ambient subject such as `.`
- merge is for event-driven composition, while pipes are for left-to-right expression flow

If you can describe the logic as "take this current value and keep transforming it", use pipes. If you mean "combine events from different sources into one signal", use signal merge.

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

## Delay pipe `delay|>`

`delay|>` re-emits the current signal payload once after a duration:

```aivi
signal click : Signal Text

signal delayedClick = click
 delay|> 120
```

The payload is preserved. If a new upstream event arrives before the delay fires, the newer event
replaces the pending one.

## Burst pipe `burst|>`

`burst|>` replays the current signal payload a fixed number of times on a scheduler-owned cadence:

```aivi
signal click : Signal Text

signal flashingClick = click
 burst|> 200 3
```

This emits three delayed replays of the same payload. The first replay happens after the first
interval, not immediately. As with `delay|>`, a newer upstream event replaces any in-flight burst.

## Tap `|`

The tap pipe observes the current subject without changing it. The tap expression runs (useful
for logging or side effects), but the subject flows through unchanged:

```aivi
type Text -> Text
func greet = name =>
    "Hello, {name}"

value result = "Ada"
  | log "processing"
  |> greet
```

The log call fires, but the subject remains `"Ada"` for the next stage.

## Map / fan-out `*|>`

`*|>` maps a function over each element of a collection:

```aivi
type User = {
    name: Text,
    email: Text
}

value users : List User = [
    {
        name: "Ada",
        email: "ada@example.com"
    },
    {
        name: "Grace",
        email: "grace@example.com"
    }
]

value emails : List Text = users
 *|> .email
```

This reads: *"for each user, extract the email."* The result is a `List Text`.

For `Signal (List A)`, fan-out lifts pointwise — each tick maps the function across the current
list:

```aivi
signal userList : Signal (List User) = [
    {
        name: "Ada",
        email: "ada@example.com"
    }
]

signal emailList : Signal (List Text) = userList
 *|> .email
```

`*|>` is pure mapping only. It does not flatten nested collections or sequence tasks.

## Fan-out join `<|*`

`<|*` reduces the collection produced by the immediately preceding `*|>`:

```aivi
type User = {
    name: Text,
    email: Text,
    score: Int
}

value users : List User = [
    {
        name: "Ada",
        email: "ada@example.com",
        score: 90
    },
    {
        name: "Grace",
        email: "grace@example.com",
        score: 85
    }
]

value totalScore : Int = users
 *|> .score
 <|* sum
```

The fan-out extracts scores, then the join sums them. `<|*` may only appear immediately after `*|>`.

## Applicative cluster `&|>`

`&|>` combines independent values under the same applicative constructor. All cluster members
must share the same outer wrapper — `Option`, `Result`, `Validation`, `Signal`, `Task`, or `List`.

A typical use is combining several validations:

```aivi
type ValidatedUser = {
    name: Text,
    email: Text,
    age: Int
}

type Text -> Validation (List Text) Text
func validateName = name =>

type Text -> Validation (List Text) Text
func validateEmail = email =>

type Text -> Validation (List Text) Int
func validateAge = ageText => ageText
  |> parseInt
 T|> .
 F|> Invalid ["Age must be a number"]

value draft = unit
 &|> validateName "Ada"
 &|> validateEmail "ada@example.com"
 &|> validateAge "30"
  |> ValidatedUser
```

If all three succeed, the finalizer (`ValidatedUser`) receives their unwrapped values. If any fail,
the errors accumulate (because `Validation` is applicative, not monadic).

When no explicit finalizer appears, the cluster defaults to a tuple.

## Validation `!|>`

`!|>` runs a validation function that must return `Result` or `Validation`. If the validation
fails, the error propagates:

```aivi
type Text -> Result Text Text
func nonEmpty = text =>

value checked : Result Text Text = "hello"
 !|> nonEmpty
```

## Accumulation `+|>`

`+|>` folds signal events into state over time. It takes a seed and a step function:

```aivi
type Event =
  | Increment
  | Decrement
  | Reset

type Event -> Int -> Int
func step = event count => event
 ||> Increment -> count + 1
 ||> Decrement -> count - 1
 ||> Reset     -> 0

signal event : Signal Event = keyDown
  ||> Key "ArrowUp" => Increment
  ||> Key "ArrowDown" => Decrement
  ||> _ => Increment

signal count = event
 +|> 0 step
```

This reads: *"start at 0; each time `event` fires, apply `step` to compute the next value."*
The accumulation is managed by the signal engine — there are no mutable variables.

## One important rule: no nested pipes

Pipes must stay on the top-level expression spine. If you need a pipe inside another expression, pull it out into a named helper:

```aivi
type Text -> Text
func normalizeTitle = .
 ||> "Inbox" -> "priority"
 ||> _       -> .

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
| `T\|>` | Branch for `True` / `Some` / `Ok` / `Valid` |
| `F\|>` | Branch for `False` / `None` / `Err` / `Invalid` |
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

---

**See also:** [Pattern Matching](pattern-matching.md) — the `\|\|>` case-split pipe in depth
