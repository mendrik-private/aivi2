---
apply: always
---

# AGENTS.md

You are implementing **AIVI** in **Rust**: a purely functional, reactive, GTK/libadwaita-first language. Optimize for correctness, explicit invariants, strong abstractions, deterministic behavior, and production-quality implementation.

## Source of truth

- Follow the language spec over preference.
- If the spec is ambiguous: identify the ambiguity, list plausible interpretations, choose the narrowest coherent one, implement so later refinement is cheap, and document the decision.
- Do not silently invent material semantics.

## Assume

- purely functional surface model,
- strict closed types,
- no null / undefined,
- no if/else or loops in surface syntax,
- expression-first design,
- pipe algebra as primary control flow,
- first-class signals and source-backed reactivity,
- higher-kinded abstractions in the core,
- typed external decoding,
- native compilation,
- lowering through **HIR -> typed core -> closed typed lambda IR -> backend IR**,
- Cranelift for AOT and JIT,
- runtime scheduler, signal engine, GC, source watchers, and GTK bridge,
- GTK main thread must never block on background work.

## Before coding

Identify:
- semantic invariants,
- ownership/lifetime invariants,
- threading and scheduler invariants,
- stack-safety invariants,
- IR invariants,
- diagnostic invariants.

Prefer:
- principled models over special cases,
- typed structure over stringly protocols,
- explicit costs over hidden costs,
- deterministic scheduling over opportunistic behavior,
- message passing over shared mutable state,
- reusable abstractions over copy-paste,
- root-cause fixes over patches.

Make illegal states unrepresentable where practical.

## Layering

Use the correct layer:

1. parser / CST
2. name resolution / HIR
3. type + kind checking
4. typed core desugaring
5. closure/lambda lowering
6. monomorphization and/or dictionary passing
7. runtime-aware/backend IR
8. Cranelift codegen
9. runtime / scheduler / GTK bridge
10. tooling / diagnostics / formatting

Do not solve type problems in the parser, runtime semantics in ad-hoc AST rewrites, or GTK concerns in the pure core unless the spec requires it.

## Rust rules

- Encode invariants in types.
- Use explicit enums, typed IDs, and clear ownership boundaries.
- Minimize global mutable state.
- Keep `unsafe` tiny, audited, and justified by an explicit invariant.
- Make `Send`/`Sync` boundaries explicit.
- Avoid `Rc<RefCell<_>>` as architecture unless it is the best constrained local tradeoff.
- Justify new crates by invariant, runtime cost, compile time, binary size, and maintenance risk.

Prefer arenas, interners, slot maps, immutable sharing, bounded queues, and explicit worklists where they improve clarity and predictability.

## IR and semantics

Each IR must define:
- ownership model,
- identity strategy,
- span/source mapping,
- validation rules,
- debug/pretty-print form,
- test fixtures.

Be rigorous about:
- closed ADTs and records,
- constructor arity,
- exhaustiveness,
- kind checking,
- HKTs,
- partial application of type constructors,
- monomorphization vs dictionary-passing boundaries,
- lawful core abstractions,
- signal vs non-signal separation,
- purity boundaries.

Keep inference local and predictable. Prefer explicit, actionable diagnostics over cleverness.

## Runtime, concurrency, stack safety

Never assume recursion is safe. Prevent stack overflow by design using tail-position analysis, trampolines, loops, or explicit worklists where depth may be unbounded. Avoid recursive evaluators or walkers that fail on adversarial input.

Runtime rules:
- GTK widget creation, mutation, and event dispatch stay on the GTK main thread.
- I/O, decoding, file watching, networking, D-Bus round-trips, and heavy computation run on workers.
- Workers publish immutable messages into scheduler-owned queues.
- Workers never mutate UI-owned state directly.
- Signal propagation must be batched, topologically ordered, glitch-free, and transactional per scheduler tick.
- Design out deadlocks, starvation, leaks, races, spin loops, and teardown bugs early.

## Memory and FFI

Assume ordinary language values may move. Do not rely on stable addresses for them.

At FFI and UI boundaries:
- use stable handles, pinning, or copied representations only where required,
- keep ownership transfer explicit,
- keep pinning narrow,
- preserve abstractions so the allocator/collector can evolve without semantic churn.

## GTK / GNOME boundary

Target the real Linux desktop: GTK4/libadwaita, GLib main-context integration, GObject ownership semantics, non-blocking UI behavior, D-Bus, filesystem watching, process/OS integration, and correct startup/disposal/shutdown.

Keep pure language logic pure. Cross the UI boundary through controlled, testable effect layers.

## Testing

Use the right mix of unit, snapshot, parser round-trip, type-check expectation, property, fuzz, scheduler stress, diagnostic regression, GTK integration, leak/drop/ownership, stack-depth, and malformed-input tests.

Every bug fix should state:
- which invariant failed,
- which test now locks it down,
- whether a missing abstraction caused it.

## How to work

For non-trivial work:
1. identify subsystem and invariants,
2. state the architecture decision before patching,
3. implement the full change across affected layers,
4. add or update tests,
5. report what changed, what was validated, and what remains.

Fix nearby in-scope issues. Remove obsolete code, comments, branches, and unused helpers.

## Done means

Do not stop at a superficially working patch.

Done requires:
- requested behavior works end to end,
- affected paths and call sites are updated consistently,
- implied edge cases are handled,
- validation is proportionate,
- no stubs, TODOs, fake behavior, brittle special cases, or incomplete refactors remain unless explicitly requested,
- result is production-worthy and architecture-aligned.

If blocked, state the exact blocker and mark the work incomplete. Never present partial work as finished.
