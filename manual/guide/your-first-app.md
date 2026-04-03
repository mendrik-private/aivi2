# Your First App

This tutorial walks you through building a small GTK application in AIVI, step by step. By the end, you will have a working app with reactive state, keyboard input, and a live UI.

We will build a **simple counter** — not because it is exciting, but because it introduces every major concept in a small space: values, functions, signals, sources, pattern matching, pipes, and markup.

## Step 1: A static window

Every AIVI application starts with a `value` that describes the UI:

```aivi
value main =
    <Window title="Counter">
        <Label text="Hello from AIVI" />
    </Window>

export main
```

This is a complete, runnable module. The markup is an ordinary AIVI expression — `<Window>` and `<Label>` are GTK widgets with type-checked attributes. `export main` makes this the entry point.

## Step 2: Naming a value

Let us extract the label text into a named value:

```aivi
value greeting = "Hello from AIVI"

value main =
    <Window title="Counter">
        <Label text={greeting} />
    </Window>

export main
```

Inside markup, `{...}` embeds any AIVI expression. Here it references the `greeting` value. This is still static — the text will not change at runtime.

## Step 3: Adding a function

Functions in AIVI are declared with `func`. Let us add one that formats a count:

```aivi
type Int -> Text
func formatCount = n =>
    "Count: {n}"

value main =
    <Window title="Counter">
        <Label text={formatCount 0} />
    </Window>

export main
```

The `type Int -> Text` line is the function's signature — it takes an `Int` and returns `Text`. The body is a single expression using text interpolation.

## Step 4: Making it reactive with signals

To make the count change over time, we need a **signal**. A signal is a value that participates in the reactive dependency graph:

```aivi
type Int -> Text
func formatCount = n =>
    "Count: {n}"

signal count = 0

signal label = count
  |> formatCount

value main =
    <Window title="Counter">
        <Label text={label} />
    </Window>

export main
```

`signal count = 0` declares a reactive value starting at 0. `signal label` is **derived** from `count` — whenever `count` changes, `label` recomputes automatically.

The `|>` pipe sends `count` into `formatCount`. It reads naturally: *"take count, then format it."*

## Step 5: Defining events

We need a way to express what can happen. In AIVI, we model events as a sum type:

```aivi
type Event =
  | Increment
  | Decrement
  | Reset
```

This declares three possible events. The type is closed — nothing else can be an `Event`. Pattern matching will force us to handle all three.

## Step 6: A pure step function

Now we write a function that takes an event and the current count, and produces the next count:

```aivi
type Event -> Int -> Int
func step = event count => event
  ||> Increment -> count + 1
  ||> Decrement -> count - 1
  ||> Reset     -> 0
```

This is **pure** — no mutation, no side effects. It takes the current state and an event, and returns the new state. The `||>` operator pattern-matches on the event, and the compiler checks that all three constructors are covered.

## Step 7: Connecting keyboard input

We need events to come from somewhere. AIVI uses **sources** to connect the reactive graph to the outside world:

```aivi
type Key =
  | Key Text

@source window.keyDown with {
    repeat: False
}
signal keyDown : Signal Key
```

`@source window.keyDown` declares that `keyDown` receives keyboard events from the GTK window. The `Key` type wraps the key name as text.

Now we route specific keys to events:

```aivi
signal event : Signal Event

when keyDown (Key "ArrowUp") => event <- Increment
when keyDown (Key "ArrowDown") => event <- Decrement
when keyDown (Key "Space") => event <- Reset
```

Each `when` clause watches a signal for a specific pattern. When a matching key arrives, it writes the corresponding event into the `event` signal.

## Step 8: Accumulating state

The `+|>` pipe folds events into state over time:

```aivi
signal count = event
  +|> 0 step
```

This reads: *"start count at 0, and each time event fires, apply `step` to get the next value."* The accumulation is managed by the signal system — there are no mutable variables.

## Step 9: The complete app

Here is the full program:

```aivi
type Event =
  | Increment
  | Decrement
  | Reset

type Key =
  | Key Text

type Int -> Text
func formatCount = n =>
    "Count: {n}"

type Event -> Int -> Int
func step = event count => event
  ||> Increment -> count + 1
  ||> Decrement -> count - 1
  ||> Reset     -> 0

@source window.keyDown with {
    repeat: False
}
signal keyDown : Signal Key

signal event : Signal Event

when keyDown (Key "ArrowUp") => event <- Increment
when keyDown (Key "ArrowDown") => event <- Decrement
when keyDown (Key "Space") => event <- Reset

signal count = event
  +|> 0 step

signal label = count
  |> formatCount

value main =
    <Window title="Counter">
        <Box orientation="vertical" spacing={8}>
            <Label text={label} />
            <Label text="↑ increment  ↓ decrement  space reset" />
        </Box>
    </Window>

export main
```

## What we covered

Let us trace the concepts this small app introduced:

| Concept | Where it appeared |
| --- | --- |
| **Values** | `formatCount 0`, string literals, the `main` UI tree |
| **Functions** | `formatCount`, `step` — pure, typed, reusable |
| **Types** | `Event` sum type, `Key` wrapper, function signatures |
| **Signals** | `count`, `label`, `event`, `keyDown` — the reactive graph |
| **Sources** | `@source window.keyDown` — external input boundary |
| **Pattern matching** | `||>` in `step`, pattern matching in `when` clauses |
| **Pipes** | `|>` to transform, `+|>` to accumulate |
| **Markup** | `<Window>`, `<Box>`, `<Label>` — type-checked GTK widgets |
| **Reactivity** | `label` recomputes when `count` changes, automatically |

## The data flow

```
Keyboard → @source window.keyDown → keyDown signal
                                          ↓
                              when clauses route to event signal
                                          ↓
                              +|> accumulates into count signal
                                          ↓
                              |> derives label signal
                                          ↓
                              <Label text={label} /> updates
```

Every arrow is a declared dependency. There are no hidden subscriptions, no manual wiring, and no callbacks to forget.

## Next steps

- [Building Snake](/guide/building-snake) — a real game that uses these same concepts at scale
- [Pipes & Operators](/guide/pipes) — the full pipe algebra
- [Signals](/guide/signals) — signals in depth
- [Sources](/guide/sources) — all the ways to connect to the outside world
- [Markup & UI](/guide/markup) — the complete widget system
