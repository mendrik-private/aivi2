# AIVI Language Guide

The AIVI guide walks you from first steps through advanced topics: language fundamentals, reactive signals, GTK markup, and real application examples. Each page is self-contained; use the grouped table below to find what you need.

## Getting started

| Page | Description |
| --- | --- |
| [getting-started.md](getting-started.md) | What AIVI is and your first program |
| [your-first-app.md](your-first-app.md) | Building a counter GTK app step by step |
| [why-aivi.md](why-aivi.md) | Design principles and motivation |

## Language core

| Page | Description |
| --- | --- |
| [thinking-in-aivi.md](thinking-in-aivi.md) | Mental model for writing without loops or if/else |
| [values-and-functions.md](values-and-functions.md) | Values, functions, and type annotations |
| [types.md](types.md) | Type system: primitives, records, tagged unions, aliases |
| [pattern-matching.md](pattern-matching.md) | Case-split pipe `\|\|>` and exhaustive branching |
| [record-patterns.md](record-patterns.md) | Destructuring records by field name, dotted paths, projections |
| [predicates.md](predicates.md) | Inline filter expressions for selectors and collections |
| [pipes.md](pipes.md) | Pipe algebra — the primary control flow mechanism |
| [domains.md](domains.md) | Branded wrapper types with semantic names |
| [classes.md](classes.md) | Typeclass-style abstraction with `class` and `instance` |
| [typeclasses.md](typeclasses.md) | Higher-kinded hierarchy, built-in support matrix, HKT limits |

## Signals and reactivity

| Page | Description |
| --- | --- |
| [signals.md](signals.md) | Derived and input signals, the reactive dependency graph |
| [sources.md](sources.md) | Source providers — bridging external data into the graph |
| [source-catalog.md](source-catalog.md) | Built-in `@source` catalog and configuration reference |

## UI and markup

| Page | Description |
| --- | --- |
| [markup.md](markup.md) | AIVI markup syntax, widgets, conditionals, and iteration |
| [building-snake.md](building-snake.md) | Full working example: Snake game using every major feature |

## Reference

| Page | Description |
| --- | --- |
| [modules.md](modules.md) | Module system: `use`, `export`, and file structure |
| [surface-feature-matrix.md](surface-feature-matrix.md) | Implementation status matrix for all surface features |
