# Choice Gaps Feature Backlog

This backlog converts the `choices_made.md` alignment audit into follow-up feature work.

`AIVI_RFC.md` remains the semantic source of truth. `AGENTS.md` defines how to execute the work:

- follow the spec over preference
- when the spec is ambiguous, choose the narrowest coherent slice and document it
- keep each concern in the correct layer
- encode invariants in types
- preserve span/source mapping and user-facing diagnostics
- do not present pre-runtime handoffs as end-to-end completion
- add proportionate tests for every behavior change

Aligned choices are omitted here. This file only tracks gaps: direct RFC conflicts, intentionally narrower implementation slices, and milestone/documentation claims that should be tightened.

## Priority 0 — resolve direct conflicts and misleading completion claims

- [ ] `choice-gap-recurrence-surface`
  - Choices covered: 17, 45
  - Subsystem: parser / CST, name resolution / HIR, typed core, diagnostics
  - Gap: `choices_made.md` currently treats `@recur.timer` and `@recur.backoff` as the active recurrence surface, while the RFC models recurrence through `@|>` and `<|@`. The current wording also risks leaking recurrence into `val` / `fun`, which conflicts with purity boundaries.
  - AGENTS-guided invariants:
    - `val` and `fun` stay pure
    - recurrence is legal only where lowering is defined
    - wakeups remain explicit and deterministic
    - parser shape, HIR shape, and typed semantics stay separated
  - Work:
    - decide whether to migrate to the RFC recurrence surface or explicitly amend the RFC
    - remove or isolate unsupported `@recur.*` semantics from pure declarations
    - define one canonical recurrence entry shape before further lowering work
  - Exit criteria:
    - one recurrence surface is the documented source of truth
    - legality checks reject unsupported recurrence sites
    - downstream layers do not need to guess recurrence meaning
  - Validation:
    - parser round-trip tests
    - HIR snapshot tests
    - type-check expectation tests
    - diagnostic regressions for illegal recurrence placement

- [ ] `choice-gap-milestone-boundaries`
  - Choices covered: 19, 58, 59
  - Subsystem: documentation, milestone planning, layer handoff definitions
  - Gap: several choices describe HIR or pre-runtime handoffs as if they complete the full pipe/source umbrella or runtime lowering story. That does not match the RFC pipeline or `AGENTS.md`'s definition of done.
  - AGENTS-guided invariants:
    - HIR, typed core, lambda IR, backend IR, and runtime remain distinct milestones
    - partial handoffs are not reported as end-to-end completion
    - documentation stays architecture-aligned
  - Work:
    - rename or restate milestone boundaries so they stop at honest layer handoffs
    - separate "frontend handoff complete" from "runtime lowering complete"
    - update any nearby milestone notes that over-claim completion
  - Exit criteria:
    - milestone language matches the compiler pipeline
    - no pre-runtime artifact is labeled as completed runtime work
  - Validation:
    - documentation consistency review across planning files
    - spot-check milestone names against the RFC pipeline

## Priority 1 — bring narrower implementation slices up to RFC parity

- [ ] `choice-gap-pipe-operator-parity`
  - Choices covered: 24, 33, 37, 39, 41, 42, 55, 56b
  - Depends on: `choice-gap-recurrence-surface`
  - Subsystem: HIR validation, typed core elaboration, runtime-aware / backend IR handoff
  - Gap: pipe and recurrence handling is currently conservative and partially staged. Shape checks exist, but the full RFC story for gate legality, purity, explicit wakeups, deterministic recurrence proofs, and scheduler-node handoff still needs to be made canonical.
  - AGENTS-guided invariants:
    - gates are pure and `Bool`-typed
    - recurrence wakeups are explicit
    - scheduling handoff is deterministic
    - surface sugar stays visible until the correct lowering layer
  - Work:
    - finish RFC-level legality checks beyond early shape/order validation
    - define the canonical gate handoff and recurrence proof order
    - keep `@|>` start and `<|@` step stages distinct in lower IR
  - Exit criteria:
    - operator validation is complete for the supported RFC slice
    - gate and recurrence lowering no longer depend on ad hoc blockers
  - Validation:
    - elaboration snapshot tests
    - type-check expectation tests
    - lowering tests for gate and recurrence handoffs
    - scheduler stress tests once runtime consumers exist

- [ ] `choice-gap-domain-and-suffix-parity`
  - Choices covered: 20, 22, 23
  - Subsystem: parser / CST, name resolution / HIR, type checking, diagnostics
  - Gap: domains are treated as a real feature, but suffix resolution is still current-module-only and literal support is narrower than the RFC-facing intent.
  - AGENTS-guided invariants:
    - domains stay nominal
    - suffix resolution is explicit, scoped, and ambiguity-safe
    - no hidden coercions are introduced
  - Work:
    - extend suffix lookup from current-module-only to full in-scope resolution
    - decide whether non-integer literal families belong in the current implementation wave
    - complete any direct domain expression forms implied by the RFC
  - Exit criteria:
    - imported suffix definitions can participate in scoped resolution
    - ambiguity and missing-suffix errors remain explicit
    - domain surface behavior is documented consistently
  - Validation:
    - cross-module suffix resolution tests
    - ambiguity diagnostics
    - domain expression and equality tests

- [ ] `choice-gap-import-and-dependency-resolution`
  - Choices covered: 18, 26, 27, 32, 51
  - Subsystem: imports, name resolution / HIR, type checking, dependency extraction
  - Gap: current validation is strongest for same-module facts. Imported types, imported values, and imported signals still fall outside several proof paths even when the compiler could carry closed metadata for them.
  - AGENTS-guided invariants:
    - no guessed signal-ness or type facts
    - imported evidence is explicit and typed
    - ambiguity remains surfaced, not silently prioritized
  - Work:
    - define the next import-surface expansion clearly
    - enrich the import catalog with closed value types and signal metadata where available
    - let imported evidence participate in dependency extraction and early validation when proof is honest
  - Exit criteria:
    - imported closed facts are first-class inputs to validation
    - local-only restrictions are removed where unnecessary
  - Validation:
    - cross-module name resolution tests
    - imported signal dependency tests
    - imported source-option binding tests

- [ ] `choice-gap-source-option-typing`
  - Choices covered: 29, 40, 43, 47, 49, 52, 54, 60, 61
  - Depends on: `choice-gap-import-and-dependency-resolution`, `choice-gap-custom-provider-contracts`
  - Subsystem: type checking, source contract typing, diagnostics
  - Gap: source option checking currently proves a narrow local subset and carries several partial actual-type fallbacks. That is a good conservative start, but it does not yet cover the broader typed-expression surface implied by the RFC.
  - AGENTS-guided invariants:
    - source options remain typed ordinary expressions
    - provider contracts stay closed and explicit
    - inference remains local and predictable
    - blockers are recorded instead of guessed away
  - Work:
    - unify the current proof fragments into one reusable option-typing engine
    - broaden support for imported bindings, constructors, containers, and provider-local parameter substitutions
    - keep unsupported cases explicit instead of widening to implicit inference
  - Exit criteria:
    - source option typing covers the intended RFC slice without hidden heuristics
    - duplicate local proof logic is removed
  - Validation:
    - type-check expectation tests for local and imported options
    - constructor and container-shape tests
    - diagnostics for unresolved or ambiguous option proofs

- [ ] `choice-gap-custom-provider-contracts`
  - Choices covered: 48, 50, 53, 56a
  - Subsystem: RFC surface design, parser / CST, HIR, source contract typing
  - Gap: the compiler now carries explicit custom-provider hooks, but the RFC still lacks a proper custom-provider declaration chapter. The implementation surface exists only as a conservative placeholder.
  - AGENTS-guided invariants:
    - provider identity is explicit
    - wakeup, argument, and option metadata stay typed
    - built-in provider semantics do not leak into custom providers
  - Work:
    - define the minimal custom-provider declaration surface in the RFC
    - align parser and HIR representation to that surface
    - make provider contract validation reuse the same typed contract machinery as built-ins
  - Exit criteria:
    - custom providers have one documented declaration surface
    - provider contract metadata is validated through shared typed machinery
  - Validation:
    - parser tests for provider declarations
    - HIR snapshots
    - validation and diagnostic tests for bad provider schemas

## Priority 2 — policy decisions and narrower surface slices

- [ ] `choice-gap-orphan-instance-policy`
  - Choices covered: 6
  - Subsystem: RFC policy, instance validation, diagnostics
  - Gap: the current choice fully bans orphan instances, while the RFC wording still leaves room for either a ban or a tightly restricted form.
  - AGENTS-guided invariants:
    - instance coherence remains easy to reason about
    - lookup never depends on hidden global search
  - Work:
    - choose one v1 policy
    - encode the policy uniformly in docs and compiler validation
  - Exit criteria:
    - orphan-instance behavior is no longer ambiguous between docs and implementation
  - Validation:
    - instance resolution tests
    - diagnostics for illegal orphan forms

- [ ] `choice-gap-each-key-policy`
  - Choices covered: 9
  - Subsystem: markup validation, HIR, typed core, diagnostics
  - Gap: the implementation choice requires a key for every `<each>`, while the RFC requires keys for reorderable or dynamic collections and only strongly recommends them otherwise.
  - AGENTS-guided invariants:
    - repeated UI items keep stable identity
    - the compiler does not over-reject static safe cases without reason
  - Work:
    - decide whether to keep the stricter policy and amend the RFC, or relax the compiler to the RFC rule
    - align diagnostics and examples either way
  - Exit criteria:
    - `<each>` key requirements are consistent across docs and compiler behavior
  - Validation:
    - markup validation tests for static, dynamic, and reorderable collections
    - diagnostic regression tests

- [ ] `choice-gap-decoder-override-surface`
  - Choices covered: 10
  - Subsystem: typed external decoding, source contracts, diagnostics
  - Gap: only the built-in decode path is implemented today, but the RFC still leaves room for decoder override hooks where necessary.
  - AGENTS-guided invariants:
    - external decoding stays typed
    - override behavior does not become a stringly escape hatch
    - purity and effect boundaries stay explicit
  - Work:
    - either design a minimal decoder override surface or explicitly codify its deferral as a non-goal for the current wave
    - keep the default built-in decode path as the baseline behavior
  - Exit criteria:
    - decoder override status is explicit in both docs and implementation
  - Validation:
    - decoding expectation tests
    - diagnostics for unsupported override forms

## Suggested dependency order

1. `choice-gap-recurrence-surface`
2. `choice-gap-milestone-boundaries`
3. `choice-gap-domain-and-suffix-parity`
4. `choice-gap-import-and-dependency-resolution`
5. `choice-gap-custom-provider-contracts`
6. `choice-gap-source-option-typing`
7. `choice-gap-pipe-operator-parity`
8. `choice-gap-orphan-instance-policy`
9. `choice-gap-each-key-policy`
10. `choice-gap-decoder-override-surface`

## Housekeeping notes

- `choices_made.md` currently contains two entries numbered `56`; renumber that file when it is next updated.
- When any gap is resolved, update `choices_made.md` so it clearly distinguishes:
  - RFC-aligned behavior
  - temporary narrower implementation slices
  - resolved conflicts
