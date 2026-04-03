# What is AIVI?

AIVI is a purely functional, reactive, statically typed language designed for one purpose: building native Linux desktop applications that are correct by construction.

If you have used React, Elm, or SwiftUI, some ideas will feel familiar — reactive state, declarative UI, immutable data. But AIVI is not a framework inside another language. It is the language itself.

## What you need to know up front

AIVI does not have `if`/`else`, `for` loops, mutable variables, or null. Instead:

| Familiar concept | AIVI equivalent |
| --- | --- |
| `if`/`else` | Pattern matching with `\|\|>`, `T\|>`, `F\|>` |
| `for` loops | Collection combinators: `map`, `filter`, `reduce` |
| Mutable variables | Signals — reactive values in a dependency graph |
| Null | `Option` — explicit presence or absence |
| Callbacks / event handlers | Sources and `when` clauses |

If this sounds unusual, [Thinking in AIVI](/guide/thinking-in-aivi) explains the mental model shift in detail.

## A tiny first example

This is a complete, valid AIVI module:

```aivi
type Text -> Text
func formatGreeting = name =>
    "Hello, {name}!"

value greeting = formatGreeting "Ada"
```

It already shows the two most common top-level forms:

- `func` for a named pure function
- `value` for a named constant

## A first signal

Signals represent values that participate in the reactive graph:

```aivi
type Int -> Int
func double = n =>
    n * 2

signal count = 21

signal doubledCount = count
  |> double
```

The important idea is that `doubledCount` is **defined from** `count`, not assigned later. When `count` changes, `doubledCount` recomputes automatically. You declare the relationship once; the runtime handles the rest.

## A first UI

AIVI markup is an ordinary expression that describes GTK widgets:

```aivi
signal count = 42

signal label = count
  |> "Count: {.}"

value main =
    <Window title="My App">
        <Label text={label} />
    </Window>

export main
```

The `<Label>` attribute `text={label}` binds to the `label` signal. When `count` changes, the label updates. No manual subscriptions, no `setState`, no render loop.

## Key concepts

| Concept | What it is | Learn more |
| --- | --- | --- |
| `value` | A named constant expression | [Values & Functions](/guide/values-and-functions) |
| `func` | A named pure function | [Values & Functions](/guide/values-and-functions) |
| `signal` | A reactive value in the dependency graph | [Signals](/guide/signals) |
| `@source` | An external data feed (timer, keyboard, HTTP, etc.) | [Sources](/guide/sources) |
| `type` | An alias, record, or tagged union | [Types](/guide/types) |
| `domain` | A branded type with its own operators | [Domains](/guide/domains) |
| `\|>` | Pipe — sends a value into a function | [Pipes & Operators](/guide/pipes) |
| `\|\|>` | Pattern match — branches on a value | [Pattern Matching](/guide/pattern-matching) |

## Why it feels different

Most reactive systems add dataflow on top of an imperative host language. AIVI makes dataflow the **default shape** of the program, so dependencies stay visible in the source.

That is why the language leans so heavily on named declarations and pipes: the reactive graph is meant to be readable. You should be able to look at any signal and trace where its value comes from, without searching for mutation sites or callback registrations.

## Where to go next

Choose your path:

- **I want to understand the philosophy first** → [Why AIVI?](/guide/why-aivi)
- **I want to learn how to think without loops and if/else** → [Thinking in AIVI](/guide/thinking-in-aivi)
- **I want to build something immediately** → [Your First App](/guide/your-first-app)
- **I want to see a real program** → [Building Snake](/guide/building-snake)
- **I want the language reference** → [Values & Functions](/guide/values-and-functions)
