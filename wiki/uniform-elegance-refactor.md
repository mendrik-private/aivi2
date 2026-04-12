# Uniform elegance refactor contract

This page is binding for the Wadler-driven cleanup backlog. Its job is to prevent the work from stopping at local improvements while leaving the underlying split architecture intact.

## Target state

AIVI should expose one coherent algebraic story across:

- language surface
- executable class evidence
- stdlib and prelude APIs
- documentation and laws

Builtin carriers, authored instances, and imported higher-kinded instances must feel like one system, not neighboring systems with different rules.

## Non-negotiable invariants

- Keep pure-by-default user code.
- Keep `Signal` applicative and non-monadic.
- Keep `Validation` applicative and non-monadic.
- Do not reduce current `Task` executable support.
- Keep kind checking explicit and predictable.
- Do not replace principled abstractions with undocumented runtime magic.

## Forbidden end states

Work is **not** done if any of these remain:

- builtin evidence and authored evidence still travel through visibly different architectural paths
- support matrices duplicated across core, backend, and docs without one canonical source of truth
- prelude still advertises algebraic classes while primarily steering users toward ad hoc wrapper helpers
- `Validation` remains second-class in the public surface
- list/order APIs still prefer comparator plumbing where lawful `Eq` / `Ord` use should be canonical
- misleading names like predicate-shaped `contains` remain uncorrected
- silent lossy behavior like current `Matrix.filled` survives without an intentional, explicitly named API boundary
- docs still disagree with implementation about executable support, signatures, or operator semantics
- law documentation still stops short of the classes the language advertises
- temporary shims, compatibility aliases, dual lowering paths, TODOs, or deferred cleanup notes are left behind as the final state

## Done criteria

Each backlog item must close all affected surfaces before it can move to done:

1. implementation path updated end to end
2. stdlib/prelude surface aligned where relevant
3. docs and examples updated
4. tests or regression coverage added or adjusted
5. obsolete branches, aliases, and competing paths removed

## Priority rule

Root-cause architecture work goes first. User-facing stdlib cleanup follows after the architecture can support it cleanly. Documentation and law cleanup must describe final reality, not intermediate compromise states. Final validation is mandatory.

## Review question

For every change, ask:

> Does this make AIVI more like one mathematically legible system, or does it merely hide current asymmetry better?

Only the first kind of change counts.
