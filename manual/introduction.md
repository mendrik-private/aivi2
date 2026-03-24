# Introduction

## What is AIVI?

AIVI is a purely functional, reactive programming language for building native Linux desktop applications.
It compiles to native code via Cranelift and integrates directly with GTK4 and libadwaita —
the toolkit behind GNOME apps like Files, Settings, and Builder.

AIVI is not a binding layer over GTK, and it is not a framework running on top of another language.
It is a coherent, standalone language with its own syntax, type system, compiler, and runtime.
The GTK integration is baked into the language's execution model.

## Who is AIVI for?

AIVI is for programmers who want to build native Linux desktop software and who are tired of
the usual trade-offs:

- **Too much ceremony.** GTK in C or Vala demands lifecycle management, signal connections, and
  mutation everywhere. Small apps grow large fast.
- **Wrong abstractions.** Electron or Flutter bring their own rendering stack, bloat, and
  non-native aesthetics.
- **Surprising reactivity.** React and similar libraries require manual dependency lists
  (`useEffect`, `useMemo`) that are easy to get wrong.

You do not need a background in functional programming. If you have written TypeScript, Python,
or Rust, you already know enough to read AIVI.

## Why does AIVI exist?

The core insight is this: **a desktop UI is a function from a stream of events to a stream of
widget states**. You do not need mutation, callbacks, or an event loop in your *language* if the
*runtime* handles all of that for you.

AIVI makes this trade: you write pure, stateless transformations; the runtime owns all state changes,
scheduling, and GTK widget lifecycle.

The result is code that is easy to reason about locally (each function depends only on its inputs),
easy to test (pure functions need no mocks), and impossible to crash with a null pointer or
use-after-free.

## Core mental model

> **Signals are values that change over time. Functions transform them.**

A `sig` in AIVI is like a cell in a spreadsheet. When its inputs change, it recomputes
automatically. You do not call a setter. You do not subscribe to an event. You declare a
dependency, and the runtime ensures the value is always current.

```text
// TODO: add a verified AIVI example here
```

When either `firstName` or `lastName` changes, `fullName` recomputes. That is it.

This is the "Excel formula" mental model: **declare what a value is, not how to update it**.

## Compared to other approaches

| Approach | Event handling | State | Null safety |
|---|---|---|---|
| GTK/C | Manual signal connections | Mutable fields | Unchecked pointers |
| React/JSX | `useEffect` hooks | `useState` + reconciler | Optional chaining |
| Elm | `update` message dispatch | Single model record | No null (Maybe) |
| **AIVI** | Declared source bindings | Signal dependency graph | No null (`Option A`) |

AIVI is closest to Elm in spirit — a pure, message-driven model — but with native GTK rendering
and a pipe-oriented surface syntax rather than an ML-style record syntax.

## Key language properties

- **Pure by default.** `fun` declarations are pure functions. Effects happen only through
  `sig` with `@source` decorators.
- **Closed types.** Every `type` is a closed sum or product. The compiler knows all variants;
  pattern matches are exhaustive.
- **No `null`.** The absence of a value is represented by `type Option A = Some A | None`,
  which the compiler forces you to handle.
- **No `if`/`else`, no loops.** Control flow is pattern matching (`||>`) and recursion.
  This sounds restrictive; in practice it is liberating.
- **Pipe algebra.** Data flows left-to-right through a family of pipe operators. `|>` transforms,
  `?|>` gates, `||>` matches, `*|>` maps over lists, `@|>...<|@` folds over time.
- **Native compilation.** AIVI compiles to native binaries via Cranelift. No JVM, no V8, no
  interpreted runtime.

## Hello, world

```text
// TODO: add a verified AIVI example here
```

That is a complete AIVI application. One `val`, one `export`, two GTK widgets.

Next: [the Language Tour →](/tour/)
