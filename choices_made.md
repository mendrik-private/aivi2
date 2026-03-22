# Choices made for the initial implementation wave

This log records narrow, reviewable implementation choices for ambiguous or staged parts of `AIVI_RFC.md` during the first Rust implementation wave. Each choice intentionally picks the smallest coherent interpretation that preserves the RFC's intent, keeps later refinement cheap, and avoids silently broadening semantics.

1. **Area:** Validation semantics notation (`RFC §8`)
   - **Ambiguity / decision point:** The RFC asks whether implementation-facing material should keep Haskell-style applicative notation for `Validation`.
   - **Chosen interpretation:** Keep the `Validation` semantics exactly as specified, but express implementation notes, tests, and diagnostics in AIVI terms (`pure`, `apply`, `Valid`, `Invalid`) rather than Haskell-specific notation.
   - **Rationale:** This preserves the applicative meaning while avoiding a second surface language in compiler-facing materials.
   - **Future refinement:** If cross-language comparison becomes useful, add a separate law appendix rather than mixing Haskell notation into core implementation artifacts.

2. **Area:** Ordinary `?|>` gate semantics (`RFC §11.3`)
   - **Ambiguity / decision point:** “For ordinary value flow, it lowers through the chosen flow carrier” is underspecified.
   - **Chosen interpretation:** In the initial wave, ordinary non-`Signal` `?|>` is accepted only for `List`-typed flows, where it lowers to list filtering. For `Signal`, it keeps the RFC’s update-filtering meaning. Other ordinary uses are rejected until a lawful carrier story is specified.
   - **Rationale:** This matches the filtering intuition, avoids inventing implicit carriers for bare values, and keeps elaboration deterministic. Truthy/falsy carrier branching still uses `T|>`, `F|>`, or `||>`.
   - **Future refinement:** Extend only after the RFC names additional concrete gate carriers or defines a dedicated filterable abstraction.

3. **Area:** Reactive text interpolation in `@source` arguments (`RFC §14`)
   - **Ambiguity / decision point:** The RFC notes that text interpolation with signals must work in source arguments for environment-specific URLs and similar cases.
   - **Chosen interpretation:** Text-typed source argument expressions, including interpolation, may reference signals. Those signal references become static dependencies of the source node, and a dependency change cancels and recreates the source subscription with freshly rendered text.
   - **Rationale:** This satisfies the note without introducing signal monads or hidden dependency rewiring; the dependency set remains statically extractable after elaboration.
   - **Future refinement:** Broaden to other reactive argument shapes only after provider lifecycle semantics are specified end to end.

4. **Area:** HTTP refresh scheduling (`RFC §14.1.2`, HTTP)
   - **Ambiguity / decision point:** The RFC asks how refreshes should be scheduled, mentioning both signal dependencies and broader lifecycle hooks.
   - **Chosen interpretation:** The initial wave supports only two refresh triggers: source recreation when statically known reactive text arguments change, and explicit polling/retry composition through timer/recurrence constructs (`timer.*`, `@|>`, `<|@`). There are no implicit database, focus, or window-lifecycle refresh hooks in the bootstrap implementation.
   - **Rationale:** This keeps refresh behavior explicit, scheduler-owned, and stack-safe, while staying compatible with static dependency extraction.
   - **Future refinement:** Add explicit lifecycle-trigger options only after their runtime contract is specified precisely.

5. **Area:** File watching versus file loading (`RFC §14.1.2`, file watching)
   - **Ambiguity / decision point:** The RFC asks how a watched file should be loaded on change.
   - **Chosen interpretation:** `fs.watch` remains an event-only source that yields `Signal FsEvent`. Loading and decoding file contents on change is explicit composition via a separate task or source helper triggered by those events. The listed `decode` option is treated as reserved for a future content-emitting file variant, not as behavior of raw `fs.watch`.
   - **Rationale:** This keeps watch events, file I/O, and decoding as separate effects, which preserves a clean source contract and avoids hiding extra work behind a watcher.
   - **Future refinement:** Introduce a distinct `fs.read*` or `fs.watchFile*` variant once content-emitting semantics are specified.

6. **Area:** Orphan-instance policy (`RFC §7.1`)
   - **Ambiguity / decision point:** The RFC says orphan instances are “disallowed or tightly restricted” in v1.
   - **Chosen interpretation:** The initial implementation disallows orphan instances entirely.
   - **Rationale:** This is the narrowest coherent choice, maximizes coherence, and avoids adopting an exception policy that would be hard to retract.
   - **Future refinement:** Revisit only if a concrete orphan policy can be stated without weakening compile-time coherence.

7. **Area:** Milestone sequencing and architecture gates (`RFC §24`, `AGENTS.md`)
   - **Ambiguity / decision point:** Staged delivery could blur architectural boundaries if later runtime concerns leak backward into earlier passes.
   - **Chosen interpretation:** Work proceeds in RFC milestone order, and each milestone must validate its own IR/contracts before the next milestone becomes implementation-critical. This is sequencing only, not a scope reduction.
   - **Rationale:** It keeps parser, HIR, typing, runtime, and GTK concerns in their proper layers and prevents backend/runtime shortcuts from redefining earlier semantics.
   - **Future refinement:** Internal task breakdown may change, but the layer order is expected to remain stable.

8. **Area:** Bootstrap kind-system feature cut (`RFC §6.1`)
   - **Ambiguity / decision point:** The implementation needs a concrete starting point for higher-kinded support.
   - **Chosen interpretation:** The bootstrap type system supports only the explicit v1 kind set plus named constructor partial application. Full type-level lambdas remain deferred and are not represented in the initial parser, HIR, or typed-core structures.
   - **Rationale:** This matches the RFC’s local, predictable inference goals and keeps kind checking small, explicit, and reviewable.
   - **Future refinement:** Add type-level lambdas only as a separately designed extension with explicit syntax, typing, and diagnostics.

9. **Area:** `<each>` key policy in the GTK bridge (`RFC §17.3.2`)
   - **Ambiguity / decision point:** The RFC requires keys for reorderable/dynamic collections and strongly recommends them otherwise, but proving which collections are “safe enough” to omit keys is itself non-trivial.
   - **Chosen interpretation:** The initial wave requires `key={...}` on every `<each>` node.
   - **Rationale:** This preserves child identity deterministically, simplifies keyed GTK child reuse, and avoids heuristic classification of collection dynamics in the first bridge.
   - **Future refinement:** Relax only if static analysis can soundly identify safe unkeyed cases.

10. **Area:** Decoder overrides (`RFC §14.2`)
    - **Ambiguity / decision point:** The RFC leaves custom decoder overrides possible but does not yet define the full user-facing override mechanism.
    - **Chosen interpretation:** The initial wave implements compiler-generated structural decoding only. Custom decoder overrides are deferred.
    - **Rationale:** This keeps source ingestion closed, typed, and testable while the baseline source/runtime contract is still being established.
    - **Future refinement:** Add override hooks only with explicit typing rules, span-preserving diagnostics, and clear interaction with `Strict`/`Permissive` modes.

11. **Area:** IR ownership and traversal shape (`RFC §3.4–§3.5`, `AGENTS.md`)
    - **Ambiguity / decision point:** The bootstrap Rust implementation needs a concrete internal representation strategy that respects stack-safety and source mapping requirements.
    - **Chosen interpretation:** Each IR uses arena-owned nodes addressed by typed IDs, with explicit validation passes at each boundary. Traversals over user-controlled depth must use worklists or other iterative strategies rather than unbounded Rust recursion.
    - **Rationale:** This encodes invariants in types, preserves stable cross-reference identity without relying on object addresses, and aligns with the stack-safety rules for parser, decoder, scheduler, and tree-walking paths.
    - **Future refinement:** Internal storage details may change later, but recursive ownership trees are not the baseline.

12. **Area:** Structural equality bootstrap (`RFC §7`, `§18.2`)
    - **Ambiguity / decision point:** The RFC has a general class/instance mechanism but no concrete equality class, no derivation boundary, and no explicit v1 stance on user-authored `Eq`.
    - **Chosen interpretation:** Add `class Eq A` with `(==) : A -> A -> Bool`. In the initial wave, `Eq` participates in ordinary coherent compile-time instance resolution, but the compiler supplies `Eq` dictionaries only for the explicitly structural cases: primitive scalars (`Int`, `Float`, `Decimal`, `BigInt`, `Bool`, `Text`, `Unit`), tuples, closed records, closed sums whose payloads are all `Eq`, `List A`/`Option A` when `A` is `Eq`, and `Result E A`/`Validation E A` when both parameters are `Eq`. Constructor-headed product declarations are covered by the closed-sum rule. Scalar equality is same-type only; there is no coercive or approximate comparison. `!=` is treated as surface sugar for `not (x == y)` rather than a second class member.
    - **Rationale:** This gives AIVI a type-directed structural equality story without inventing open-world equality for runtime/foreign values or requiring manual instance authoring before class resolution, dictionary passing, and diagnostics are implemented end to end.
    - **Future refinement:** Revisit user-authored `Eq` instances and additional built-ins such as `Bytes`, `Map`, and `Set` only after their laws and runtime semantics are specified precisely. `Signal`, `Task`, function values, and GTK/foreign handles remain outside v1 structural equality.

13. **Area:** Milestone 2 HIR ownership and symbol identity (`RFC §3.2`, `§3.5`, `§4.2`, `§24`, `AGENTS.md`)
    - **Ambiguity / decision point:** The RFC defines what HIR must know, but not the concrete ownership boundary for resolved symbols, binders, imports, and markup/control nodes.
    - **Chosen interpretation:** The Milestone 2 HIR is a module-owned arena graph with typed IDs for top-level items, local binders, import entries, expression/pattern nodes, and markup nodes. Resolution results point to those IDs, not to CST nodes, raw strings, or pointer identity. Every HIR node and resolution edge keeps the source span needed to report the original surface site.
    - **Rationale:** This keeps identity stable across later lowering, avoids lifetime coupling back to the CST, and satisfies the RFC’s explicit IR-boundary and diagnostic invariants.
    - **Future refinement:** Cross-module/package identity can later layer module/package IDs on top of the local typed-ID scheme without changing the Milestone 2 ownership model.

14. **Area:** Record shorthand preservation across HIR (`RFC §4.2`, `§9.4`, `§21`, `§24`)
    - **Ambiguity / decision point:** Record shorthand needs name information, but its field-validity check depends on an expected closed record type that HIR does not yet know.
    - **Chosen interpretation:** HIR preserves record-construction and record-pattern shorthand as distinct node forms instead of eagerly expanding them to explicit `label: value` pairs. In record expressions, the value side of shorthand resolves to a same-named term binding or import entry in Milestone 2; if none exists, HIR reports an unresolved-name error at that field. In record patterns, shorthand is always a binder pattern named by the field label; field-existence and ambiguity checks wait for the later expected-record-type check.
    - **Rationale:** This keeps `RFC §9.4` surface sugar available for diagnostics, avoids pretending HIR already knows the expected closed record type, and gives name resolution a coherent story for the value side without broadening pattern semantics.
    - **Future refinement:** Once typing supplies the expected closed record, later phases may elaborate shorthand to explicit fields while retaining the original shorthand span for diagnostics.

15. **Area:** Applicative cluster preservation at HIR (`RFC §4.2`, `§12`, `§21`, `§24`, `§25`)
    - **Ambiguity / decision point:** The CST represents `&|>` as pipe stages, while the RFC requires HIR to represent pipe clusters explicitly and typed core to normalize them later.
    - **Chosen interpretation:** Milestone 2 lifts every contiguous `&|>` region into a dedicated HIR applicative-cluster node with an ordered non-empty member list, an optional explicit finalizer, and a flag recording whether the user wrote the leading-cluster or expression-headed form. An expression-headed cluster contributes its pipe head as the first cluster member. HIR does not insert implicit tuple finalizers and does not lower clusters to `pure`/`apply` spines.
    - **Rationale:** This preserves the exact `RFC §12` surface boundary, keeps cluster-specific diagnostics attached to user-visible spans, and leaves one coherent applicative story for later typing and normalization.
    - **Future refinement:** Later passes may cache arity and resolved outer applicative-constructor information, but HIR remains the last IR that preserves the original cluster form.

16. **Area:** Markup control-node representation at HIR (`RFC §4.2`, `§17.3`, `§21`, `§24`, `§25`)
    - **Ambiguity / decision point:** The Milestone 1 CST treats all markup tags uniformly, but Milestone 2 requires explicit HIR nodes for the RFC-defined control forms.
    - **Chosen interpretation:** HIR splits ordinary markup elements from the current closed control-node set: `Show`, `Each`, `Match`, `Fragment`, and `With`, with `Empty` represented only as an optional branch inside `Each` and `Case` represented only inside `Match`. Their structural fields are lifted out of stringly attributes/children into typed HIR slots (`when`, `keepMounted`, `of`, `as` binder, `key`, `on`, `pattern`, `value`, bodies). Misplaced `<empty>` or `<case>` nodes are HIR errors rather than generic markup nodes.
    - **Rationale:** This matches `RFC §17.3`, keeps later typing and GTK lowering direct, and avoids carrying control-node meaning as ad-hoc tag-name checks past HIR.
    - **Future refinement:** Ordinary element nodes can later gain widget-resolution metadata, but the Milestone 2 control-node enum should stay closed to the currently parsed surface forms.

17. **Area:** Decorator resolution scope in Milestone 2 (`RFC §4.2`, `§24`, current parser surface`)
    - **Ambiguity / decision point:** The current parser accepts dotted decorator heads and a structured `@source` payload, but the language does not yet have decorator declarations, aliases, or decorator imports.
    - **Chosen interpretation:** Milestone 2 resolves decorators in a dedicated decorator namespace that is separate from term, type, and import scopes. `@source` is the only decorator with compiler-known semantics at this stage; its provider path is preserved exactly as a dotted provider key rather than resolved through ordinary `use` lookup. All other decorators remain opaque attached metadata keyed by their written qualified name, and `use` does not change decorator-head lookup in Milestone 2.
    - **Rationale:** This attaches decorators coherently without inventing a macro system or overloading ordinary name resolution with semantics the surface language has not declared.
    - **Future refinement:** If AIVI later adds declared decorators, aliases, or decorator imports, that can extend the decorator namespace without changing current attachment behavior.

18. **Area:** Import and lexical name-resolution scope in Milestone 2 (`RFC §4.2`, `§9.4`, `§17.3`, `§24`)
    - **Ambiguity / decision point:** The current surface has only unqualified identifier references plus `use module (imports)` with no alias, wildcard, or qualified ordinary-name syntax, but imported names are used by different later subsystems (`aivi.defaults`, `aivi.network`, and similar bundles).
    - **Chosen interpretation:** Milestone 2 resolves ordinary lexical names through nested term scopes (function parameters, pipe/case binders, record-pattern shorthand binders, `<each as={...}>`, and `<with as={...}>`) over module top-level term bindings and constructors. `use` declarations are recorded separately as file-level import entries keyed by module path and imported final segment; Milestone 2 does not eagerly merge them into term/type/decorator tables or invent module-qualified ordinary references. When later phases need an imported capability or symbol, they must pick from those explicit import entries and diagnose ambiguity instead of assuming a hidden aliasing rule.
    - **Rationale:** This matches the parser surface the workspace actually has, keeps HIR name resolution deterministic for truly lexical names, and supports current imports like `use aivi.defaults (Option)` and `use aivi.network (http)` without pretending the module system is more expressive than it is.
    - **Future refinement:** Alias syntax, wildcard imports, qualified ordinary references, and richer export/re-export behavior should only arrive as explicit surface features with their own resolution rules.
