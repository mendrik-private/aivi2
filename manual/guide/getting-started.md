# What is AIVI?

AIVI is a purely functional, reactive, statically typed language for native Linux desktop software.

It is designed around a few strong rules:

- values are immutable
- missing data is modeled explicitly with `Option`
- failure is modeled explicitly with `Result`
- branching is expression-based
- reactivity is part of the language model, not layered on afterward

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

The important idea is that `doubledCount` is defined from `count`, not assigned later.

## Key concepts

| Concept | What it is |
| --- | --- |
| `value` | A named constant expression |
| `func` | A named pure function |
| `signal` | A reactive value in the graph |
| `@source ...` on `signal` | A source-backed signal fed from the outside world |
| `type` | An alias, record, or constructor-backed tagged type |
| `domain` | An operator-oriented abstraction over a carrier type |

## Why it feels different

Most reactive systems add dataflow on top of an imperative host language. AIVI makes dataflow the default shape of the program, so dependencies stay visible in the source.

That is why the language leans so heavily on named declarations and pipes: the graph is meant to be readable.

## Next steps

- [Values & Functions](/guide/values-and-functions)
- [Types](/guide/types)
- [Pattern Matching](/guide/pattern-matching)
- [Pipes & Operators](/guide/pipes)
- [Signals](/guide/signals)
- [Sources](/guide/sources)
- [Markup & UI](/guide/markup)
