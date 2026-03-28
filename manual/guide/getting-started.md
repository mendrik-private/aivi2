# What is AIVI?

AIVI is a purely functional, reactive, statically typed language designed for building native Linux desktop applications with GTK4 and libadwaita.

## Core Philosophy

AIVI removes entire categories of bugs by design:

- **No mutation** — all values are immutable
- **No null or undefined** — missing data uses `Option`, failures use `Result`
- **No loops** — iteration is expressed through pipes and folds
- **No implicit side effects** — all I/O goes through declared `source` nodes
- **No `if`/`else`** — branching uses pattern matching and pipe operators

The result is code where data flow is always visible, behaviour is always deterministic, and the compiler catches a wide class of errors before your program runs.

## The Signal Graph

At the heart of AIVI is the **signal graph**. A signal is a time-varying value — think of it as a cell in a spreadsheet. When its inputs change, it recomputes automatically.

```aivi
signal greeting: Signal Text =
    nameInput
     |> formatGreeting
```

When `nameInput` emits a new value, `greeting` updates immediately. The runtime batches these updates, eliminates redundant recomputation, and ensures consistency across the entire graph.

## A Taste of the Language

Here is a self-contained counter application:

```aivi
@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown: Signal Key

fun increment:Int _:Key count:Int =>
    count + 1

signal count: Signal Int =
    keyDown
     |> scan 0 increment

value main =
    <Window title="Counter">
        <Box orientation={Vertical} spacing={8}>
            <Label text="Press any key to count" />
            <Label text="{count}" />
        </Box>
    </Window>

export main
```

This example demonstrates:
- A `@source` declaration that listens for key events
- A `fun` that increments a count (ignoring the key itself)
- A `signal` that accumulates state using `scan`
- Markup that binds the signal directly to a label

## Key Concepts

| Concept | What it is |
|---|---|
| `value` | A named constant — computed once, never changes |
| `fun` | A named pure function — same inputs always give same output |
| `signal` | A reactive node — re-evaluates when its inputs change |
| `source` | An effectful acquisition — HTTP, timers, keyboard, filesystem |
| `type` | A data shape — union type, record, or newtype |
| `domain` | A typed abstraction over a carrier type — like a typeclass for operators |
| `class` | A typeclass — defines a contract that types can implement |

## What Makes It Different

Most reactive frameworks bolt reactivity on top of an imperative language. In AIVI, reactivity is **the** execution model. There is no way to write code that accidentally bypasses it.

The language compiles to native binaries via Cranelift. There is no virtual machine, no garbage collection pauses on the hot path, and no runtime overhead from a JavaScript or JVM bridge.

## Next Steps

- [Values & Functions](/guide/values-and-functions) — the building blocks
- [Types](/guide/types) — how data shapes are declared
- [Pattern Matching](/guide/pattern-matching) — branching without `if`/`else`
- [Pipes & Operators](/guide/pipes) — composing data transformations
- [Signals](/guide/signals) — reactive state
- [Sources](/guide/sources) — declaring effects
- [Markup & UI](/guide/markup) — building GTK interfaces
