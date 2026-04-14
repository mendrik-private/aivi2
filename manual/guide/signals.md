# Signals

In most languages, you manage changing state with mutable variables and manual update logic. When the state grows, you spend time tracking *who changed what, when, and why*.

AIVI replaces mutable state with **signals** — reactive values in a dependency graph. A signal declares what it depends on, and the runtime handles the updates. You describe the relationships; the runtime does the work.

```
value  →  computed once, never changes
signal →  recomputes when its dependencies change
```

Think of signals as cells in a spreadsheet. When you change one cell, every cell that references it recalculates automatically. You never manually propagate changes.

## Declaring a signal

```aivi
signal count = 21
```

This declares a reactive value named `count`.

## Deriving from another signal

Signals are often defined from earlier signals with pipes:

```aivi
type Int -> Int
func double = n =>
    n * 2

signal count = 21

signal doubledCount = count
  |> double
```

## Grouping derivations with `from`

When several reactive projections all hang off the same upstream signal, group
them with `from`:

```aivi
type State = {
    score: Int,
    ready: Bool
}

signal state : Signal State = {
    score: 0,
    ready: True
}

from state = {
    type Bool
    readyNow: .ready

    type Int -> Bool
    atLeast threshold: .score >= threshold
}

signal thresholdMet : Signal Bool = atLeast 10
```

- Plain entries like `readyNow` create ordinary derived `signal`s.
- Parameterized entries like `atLeast threshold` create ordinary top-level
  selector helpers whose *surface* type omits the final `Signal`.
- The preceding standalone `type` line attaches to the next `from` entry only.
- Every entry still reads reactively from the shared upstream source.

## Signal branching

Signals can use the same truthy/falsy shorthand as ordinary values. `Signal Bool`
branches on `True` / `False`, and `Signal (Option A)`, `Signal (Result E A)`,
and `Signal (Validation E A)` use the same canonical pairs pointwise:

```aivi
signal ready = True

signal statusText = ready
 T|> "ready"
 F|> "waiting"
```

## Filtering with `?|>`

On signals, `?|>` filters updates whose predicate fails while keeping the `Signal A` carrier:

```aivi
type User = {
    active: Bool,
    email: Text
}

type Session = { user: User }

value seed : User = {
    active: True,
    email: "ada@example.com"
}

signal sessions : Session = {
    user: seed
}

signal activeUsers : User = sessions
  |> .user
 ?|> .active
```

For ordinary non-signal values, the same operator returns `Option A`.

## Signal merge and reactive arms

When a signal's value is driven by events from one or more source signals, use **merge + pattern arms** syntax. The signal body lists source signals separated by `|`, then `||>` arms discriminate by source name and payload pattern.

### Single-source merge

```aivi
signal left = 20
signal right = 22
signal ready = True

signal total : Signal Int = ready
 T|> left + right
 F|> 0
```

### Multi-source merge

```aivi
type Event = Tick | Turn Text

type Key = Key Text

@source timer.every 120ms
signal tick : Signal Unit

@source window.keyDown
signal keyDown : Signal Key

signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowUp") => Turn "up"
  ||> _ => Tick
```

### Rules

- The merge expression (`sig1 | sig2`) lists the source signals that feed the declaring signal.
- Each source must name a previously declared local `signal`.
- Multi-source arms: `||> <source-name> <pattern> => <body>` — source name prefix required, must match a signal in the merge list.
- Single-source arms: `||> <pattern> => <body>` — no source name prefix needed.
- Default arm: `||> _ => <body>` — required; provides the initial value before any source fires and handles unmatched cases.
- Pattern binders introduced by an arm are only in scope for that arm body.
- Body type must match the declaring signal's payload type.
- Unlike a pipe, there is no ambient subject value inside the body.
- If no arm matches, the signal keeps its previous committed value.
- If multiple sources fire in one tick, later arm in source order wins.

Use signal merge when you want event-shaped reactive commits. Use pipes when you want to transform the current subject flowing through one expression spine.

Self-reference: the declaring signal cannot read itself from its own arm bodies.

### A practical example: form validation

Signal merge shines when several user inputs feed one "form state" signal:

```aivi
type Key = Key Text

type FormField =
  | Name Text
  | Email Text
  | Submit

@source window.keyDown
signal keyDown : Signal Key

signal nameInput : Signal Text = "Ada"
signal emailInput : Signal Text = "ada@example.com"
signal submitClick : Signal Unit

signal formEvent : Signal FormField = nameInput | emailInput | submitClick
  ||> nameInput name => Name name
  ||> emailInput email => Email email
  ||> submitClick _ => Submit
  ||> _ => Name ""
```

Each source feeds the same `FormField` signal. Downstream derivations can pattern-match
the field type to update the form UI or trigger validation.

## Previous and diff

The language has dedicated pipes for time-oriented signal transformations:

```aivi
signal score = 10

signal previousScore = score
 ~|> 0

signal scoreDelta = score
 -|> 0
```

## Delay and burst

Signals can also schedule future replays of an existing payload without introducing a new source:

```aivi
signal click : Signal Text

signal delayedClick = click
  |> delay 80ms

signal flashingClick = click
  |> burst 150ms 3times
```

- `|> delay d` publishes the upstream payload once after `d`.
- `|> burst d count` publishes the same payload `count` times, one replay per interval `d`.
- A newer upstream event replaces any pending delay or burst schedule.
- The first `|> burst` replay happens after the first interval.

## Shaping signal outputs

Signals can still produce richer values without leaving the ordinary expression model:

```aivi
type NamePair = {
    first: Text,
    last: Text
}

signal firstName = "Ada"
signal lastName = "Lovelace"

signal namePair = {
    first: firstName,
    last: lastName
}
```

## Signals versus values

| Form | Meaning |
| --- | --- |
| `value answer = 42` | Fixed expression |
| `signal count = 21` | Reactive graph node |

Use `value` when something does not participate in reactive recomputation. Use `signal` when it should.

## Putting it together

Here is a small reactive timer that demonstrates the full signal lifecycle — sources feeding a
merge, accumulation folding events into state, and derivations driving the UI:

```aivi
type Event = Tick | Reset

type Key = Key Text

@source timer.every 1sec
signal tick : Signal Unit

@source window.keyDown
signal keyDown : Signal Key

signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "Space") => Reset
  ||> _ => Tick

type Event -> Int -> Int
func step = event count => event
 ||> Tick  -> count + 1
 ||> Reset -> 0

signal elapsed = event
 +|> 0 step

signal label = elapsed
  |> formatInt

value main =
    <Window title="Timer">
        <Label text={label} />
    </Window>

export main
```

```
timer.every  ──→  tick signal
                       ↓
keyboard     ──→  keyDown signal
                       ↓
              signal merge routes to event
                       ↓
              +|> accumulates into elapsed
                       ↓
              |> formats into label
                       ↓
              <Label text={label} /> updates
```

Every arrow is a declared dependency. There are no hidden subscriptions, no manual wiring, and
no callbacks.

## Request-like source companions

Built-in request-like sources that produce `Signal (Result E A)` synthesize a companion surface on
the signal itself:

| Member | Type | Meaning |
| --- | --- | --- |
| `.run` | `Signal Unit` | Publish to refetch or restart the source |
| `.loading` | `Signal Bool` | `True` while the active request is still pending |
| `.success` | `Signal (Option A)` | Current successful payload when the latest result is `Ok` |
| `.error` | `Signal (Option E)` | Current error payload when the latest result is `Err` |

```aivi
type User = {
    id: Int,
    name: Text
}

@source http.get "https://api.example.com/users"
signal usersResult : Signal (Result HttpError (List User))

value main =
    <Window title="Users">
        <Box>
            <Button label="Refresh" onClick={usersResult.run} />
            <show when={usersResult.loading}>
                <Spinner />
            </show>
            <show when={usersResult.error}>
                <Label text="Failed to load" />
            </show>
            <match on={usersResult.success}>
                <case pattern={Some users}>
                    <each of={users} as={user} key={user.id}>
                        <Label text={user.name} />
                    </each>
                </case>
            </match>
        </Box>
    </Window>
```

`success` and `error` are ordinary `Option` carriers, so they slot directly into existing
truthy/falsy pipes, `<show when>`, and `<match>` without inventing a UI-only async language.

### When to keep using AsyncTracker

`aivi.async.AsyncTracker` is still the right tool when you want **stale-while-revalidate** or when
you are folding an arbitrary `Result` signal that is not one of the built-in request-like sources.
Unlike `.success`, `AsyncTracker.done` keeps the last successful value even after a later failure.

### Fire once when done

A common need is to fire a side-effect exactly once — log a metric, navigate away, cache the
result — when a signal first succeeds. AIVI's accumulation operator gives you this without special
syntax:

```aivi
// A Bool that becomes True on the first success and never resets
type Option Text -> Bool -> Bool
func trackFirstDone = newDone hasFired => hasFired
 T|> True
 F|> isSome newDone

@source http.get "https://api.example.com/users"
signal usersResult : Signal (Result HttpError Text)

signal firstLoadDone : Signal Bool = usersResult.success
 +|> False trackFirstDone
```

`firstLoadDone` is `False` until `usersResult.success` is first `Some`, then `True` forever. Gate any
follow-up source with `activeWhen: firstLoadDone` to fire it exactly once.

See [`aivi.async`](/stdlib/async) for the full `AsyncTracker` reference.

---

**See also:** [Sources](sources.md) — how external data enters the reactive graph · [Source Catalog](source-catalog.md) — built-in `@source` providers and configuration
