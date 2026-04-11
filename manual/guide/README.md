# AIVI Language Guide

AIVI is a purely functional, reactive, GTK-native language. Everything fits together as a stack: pure functions and types at the foundation, pipe algebra as the primary control flow, domains for semantic safety, signals for reactivity, and GTK markup at the top. Sources wire the outside world into the signal graph.

This guide is organised as **five core stories** you can read in order, plus an integrations arc that shows how to connect an app to external systems efficiently.

---

## Getting started

| Page | Description |
| --- | --- |
| [why-aivi.md](why-aivi.md) | Design principles and motivation |
| [getting-started.md](getting-started.md) | What AIVI is and your first program |
| [your-first-app.md](your-first-app.md) | A working counter app, step by step |

---

## Story 1 — Functional programming

AIVI is expression-first. There are no statements, no mutation, no loops or `if`/`else` blocks. You define values and functions; the runtime handles everything else. This section builds the mental model before introducing pipes and signals.

| Page | Description |
| --- | --- |
| [thinking-in-aivi.md](thinking-in-aivi.md) | Replacing loops and `if`/`else` with expressions, cases, and collection combinators |
| [values-and-functions.md](values-and-functions.md) | Values, functions, currying, and type annotations |
| [types.md](types.md) | Primitives, records, tagged unions, and type aliases |

---

## Story 2 — Pipe algebra

Pipes are AIVI's primary control flow mechanism — not function nesting, not `if`/`else` trees. Every transformation, case split, filter, and projection is a pipe stage. Once you think in pipes, the whole language clicks.

| Page | Description |
| --- | --- |
| [pipes.md](pipes.md) | `\|>` transform, `\|\|>` case-split, `T\|>` / `F\|>` boolean branch, `?\\|>` filter, `+\\|>` accumulation, `*\\|>` fan-out, and more |
| [pattern-matching.md](pattern-matching.md) | Exhaustive case splitting with `\|\|>` — sum types, wildcards, guards |
| [record-patterns.md](record-patterns.md) | Destructuring records by field name, dotted paths, and projection expressions |
| [predicates.md](predicates.md) | Inline filter expressions inside selectors and collection traversals |

---

## Story 3 — Domains

A score and a player ID are both integers — but they are not the same thing. Domains give types a semantic name and their own operations. The compiler prevents mixing them up. This is the preferred abstraction for any value that carries meaning beyond its carrier type.

| Page | Description |
| --- | --- |
| [domains.md](domains.md) | Declaring domains, suffix constructors, named members, operators, generics, and the `.carrier` accessor |

---

## Story 4 — Signals & reactivity

A signal is a value that changes over time. AIVI's runtime tracks dependencies automatically, propagates changes in topological order, and never produces a glitch. Sources are the declared boundary where the outside world feeds typed data into the signal graph.

| Page | Description |
| --- | --- |
| [signals.md](signals.md) | Derived signals, input signals, the reactive graph, boolean gating, signal merge, and accumulation |
| [sources.md](sources.md) | Source providers — the typed external boundary, capability handles, and the decode contract |

---

## Story 5 — GTK & markup

AIVI markup is an expression that builds a GTK widget tree. It participates in ordinary AIVI binding rules: signals flow into widget properties, GTK events flow back into input signals. The result is a live, reactive UI with no imperative update logic.

| Page | Description |
| --- | --- |
| [markup.md](markup.md) | Widgets, signal bindings, `<show>`, `<with>`, `<match>`, `<each>`, and the full widget catalog |

---

## External integrations

How to connect your app to HTTP APIs, files, timers, databases, D-Bus services, and custom sources. The integration pattern is always the same: declare a source, annotate the target type, derive signals, present results in the UI. This section shows how to do that efficiently for the most common cases.

| Page | Description |
| --- | --- |
| [integrations.md](integrations.md) | Integration patterns: HTTP, filesystem, timers, database, D-Bus, custom providers |
| [source-catalog.md](source-catalog.md) | Built-in `@source` reference — every provider, option, and configuration detail |

---

## Abstractions

Classes and modules provide the extensibility layer. Read these after the five core stories.

| Page | Description |
| --- | --- |
| [classes.md](classes.md) | Typeclass-style abstraction with `class` and `instance` |
| [typeclasses.md](typeclasses.md) | Higher-kinded hierarchy, built-in support matrix, HKT limits |
| [modules.md](modules.md) | Module system: `use`, `export`, and file layout |

---

## Examples

| Page | Description |
| --- | --- |
| [building-snake.md](building-snake.md) | Complete Snake game — every major feature in one working program |

---

## Reference

| Page | Description |
| --- | --- |
| [surface-feature-matrix.md](surface-feature-matrix.md) | Implementation status matrix for all surface features |

---

> **Standard library** — The stdlib reference lives at [/stdlib](/stdlib/index).
> It is a sidecar: reach for it when you need to know what a specific module exports,
> not as the primary way to learn the language. The five stories above are the language;
> the stdlib is the toolbox.
