# Why AIVI?

Every language is a bet on what matters most. AIVI bets that **clarity, correctness, and reactivity** should not be afterthoughts — they should be the foundation.

## The problem AIVI solves

Building a desktop application today usually means picking an imperative language, bolting on a UI toolkit, and then layering reactivity on top with a library. The result works, but the seams show:

- State lives in mutable variables. You debug by asking *"who changed this, and when?"*
- Reactivity is a library concern. Dependencies between values are discovered at runtime, not proven at compile time.
- Null sneaks in everywhere. You handle it with discipline, not with the type system.
- Control flow is a mix of `if`/`else`, loops, callbacks, and async machinery. The actual data flow is buried.

AIVI takes a different path. It starts from the premise that a desktop application is a **reactive computation graph** — a set of values that depend on each other and on the outside world — and it builds the language around that idea.

## What makes AIVI different

### Everything is an expression

There are no statements in AIVI. No `if`/`else` blocks, no `for` loops, no `while`, no `return`. Every piece of code produces a value. Branching is done through pattern matching. Repetition is done through collection combinators like `map`, `filter`, and `reduce`.

This is not a limitation — it is a simplification. When everything is an expression, you can always ask *"what does this evaluate to?"* and get a clear answer.

```aivi
type Int -> Text
func classify = score => score >= 50
  T|> "pass"
  F|> "fail"
```

### Values do not change

Once a value is bound, it stays that way. There is no reassignment, no mutation, no `let` versus `const` distinction. If you need a modified version of something, you create a new value:

```aivi
type User = { name: Text, score: Int }

value alice : User = { name: "Alice", score: 10 }
value promoted : User = alice <| { score: alice.score + 5 }
```

`alice` still has a score of 10. `promoted` is a new value with a score of 15. There is no ambiguity about what `alice` means at any point in the program.

### Signals are reactive values, not callbacks

In most frameworks, reactivity means registering callbacks or subscribing to event streams. In AIVI, a **signal** is just a value that participates in a dependency graph:

```aivi
signal count = 0

signal doubled = count
  |> multiply 2

signal label = doubled
  |> "The count is {.}"
```

When `count` changes, `doubled` and `label` recompute automatically. You do not subscribe, unsubscribe, or manage lifecycles. The runtime knows the graph and does the minimal work.

### The outside world enters through sources

Pure functions cannot talk to the network, read files, or respond to keyboard input. AIVI models all of these as **sources** — typed, declared entry points into the reactive graph:

```aivi
@source timer.every 1000ms
signal tick : Signal Unit

@source window.keyDown
signal keys : Signal Key
```

Sources are the boundary between your pure code and the messy outside world. Inside that boundary, everything is deterministic. Outside, the runtime handles the chaos.

### Types prevent mistakes, not just document them

AIVI's type system is closed by default. There are no implicit conversions, no `null`, no `undefined`. Missing values use `Option`. Failures use `Result`. Custom types use exhaustive pattern matching:

```aivi
type LoadState =
  | Loading
  | Ready Text
  | Failed Text

type LoadState -> Text
func describe = state => state
  ||> Loading      -> "Loading..."
  ||> Ready data   -> "Got: {data}"
  ||> Failed error -> "Error: {error}"
```

If you add a new constructor to `LoadState`, the compiler tells you every place that needs updating. You cannot forget a case.

### Domains add meaning to raw types

A duration is not just an integer. A URL is not just text. AIVI lets you create **domains** that wrap a carrier type with semantic meaning:

```aivi
domain Duration over Int

value timeout : Duration = 5sec
value delay : Duration = 250ms
```

You cannot accidentally pass an `Int` where a `Duration` is expected. The compiler catches it. The domain also gives you a place to define operators and conversions that make sense for that concept.

### GTK is a first-class citizen

AIVI does not wrap GTK through a foreign-function interface that you have to fight. Widgets are part of the language surface, written in a familiar markup syntax:

```aivi
value view =
    <Window title="Hello">
        <Box orientation="vertical">
            <Label text="Welcome to AIVI" />
            <Button label="Click me" />
        </Box>
    </Window>
```

Attributes are type-checked. Signal-driven attributes update automatically. The markup is an ordinary AIVI expression, not a separate template language.

## Who is AIVI for?

AIVI is for developers who:

- Want to build native Linux desktop applications without Electron
- Value correctness and are willing to learn a different way of thinking
- Are curious about functional programming but want something practical, not academic
- Like the idea of a language where the compiler catches entire categories of bugs
- Want reactivity built into the language, not bolted on as a library

## Who is AIVI not for?

AIVI is honest about its scope:

- It targets **Linux desktops** with GTK4/libadwaita. It is not a web framework.
- It is **purely functional**. If you need mutable state as a core programming model, AIVI will feel restrictive.
- It is a **young language**. Some features are still being implemented. The [surface feature matrix](/guide/surface-feature-matrix) documents exactly what works today.
- It does not try to be everything. It tries to be the right tool for reactive, type-safe desktop applications.

## Next steps

Ready to see the language in action?

- [What is AIVI?](/guide/getting-started) — A quick tour of the language
- [Thinking in AIVI](/guide/thinking-in-aivi) — How to approach problems without loops or if/else
- [Your First App](/guide/your-first-app) — Build a working GTK application step by step
