# Choices made for the initial implementation wave

This log records narrow, reviewable implementation choices for ambiguous or staged parts of `AIVI_RFC.md` during the first Rust implementation wave. Each choice intentionally picks the smallest coherent interpretation that preserves the RFC's intent, keeps later refinement cheap, and avoids silently broadening semantics.

1. **Area:** Validation semantics notation (`RFC §8`)
   - **Ambiguity / decision point:** The RFC asks whether implementation-facing material should keep Haskell-style applicative notation for `Validation`.
   - **Chosen interpretation:** Keep the `Validation` semantics exactly as specified, but express implementation notes, tests, and diagnostics in AIVI terms (`pure`, `apply`, `Valid`, `Invalid`) rather than Haskell-specific notation.
   - **Rationale:** This preserves the applicative meaning while avoiding a second surface language in compiler-facing materials.
   - **Future refinement:** If cross-language comparison becomes useful, add a separate law appendix rather than mixing Haskell notation into core implementation artifacts.

2. **Area:** Ordinary `?|>` gate semantics (`RFC §11.3`)
   - **Ambiguity / decision point:** “For ordinary value flow, it lowers through the chosen flow carrier” is underspecified.
   - **Chosen interpretation:** Follow the updated RFC directly: for an ordinary subject `A`, `?|>` lowers to `Option A`, yielding `Some subject` when the predicate is `True` and `None` when it is `False`. For `Signal A`, `?|>` forwards only updates whose predicate is `True`, suppresses `False` updates, keeps result type `Signal A`, and emits no synthetic negative update.
   - **Rationale:** This now has concrete spec text, gives ordinary expressions a pointwise “keep or drop” form without introducing `if` / `else`, and keeps `Signal` behavior glitch-free and scheduler-friendly.
   - **Future refinement:** Additional carriers should only be added through explicit RFC text rather than inferred from library conventions.

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
    - **Chosen interpretation:** Milestone 2 lifts every contiguous `&|>` region into a dedicated HIR applicative-cluster node with an ordered non-empty member list, an explicit-vs-implicit finalizer slot, and a flag recording whether the user wrote the leading-cluster or expression-headed form. An expression-headed cluster contributes its pipe head as the first cluster member. When a cluster reaches pipe end without an explicit finalizer, HIR records the RFC-required implicit tuple finalizer rather than leaving the finalizer absent.
    - **Rationale:** This preserves the exact `RFC §12` surface boundary, keeps cluster-specific diagnostics attached to user-visible spans, and leaves one coherent applicative story for later typing and normalization.
    - **Future refinement:** Later passes may cache arity and resolved outer applicative-constructor information, but HIR remains the last IR that preserves the original cluster form.

16. **Area:** Markup control-node representation at HIR (`RFC §4.2`, `§17.3`, `§21`, `§24`, `§25`)
    - **Ambiguity / decision point:** The Milestone 1 CST treats all markup tags uniformly, but Milestone 2 requires explicit HIR nodes for the RFC-defined control forms.
    - **Chosen interpretation:** HIR splits ordinary markup elements from the current closed control-node set: `Show`, `Each`, `Match`, `Fragment`, and `With`, with `Empty` represented only as an optional branch inside `Each` and `Case` represented only inside `Match`. Their structural fields are lifted out of stringly attributes/children into typed HIR slots (`when`, `keepMounted`, `of`, `as` binder, `key`, `on`, `pattern`, `value`, bodies). Misplaced `<empty>` or `<case>` nodes are HIR errors rather than generic markup nodes.
    - **Rationale:** This matches `RFC §17.3`, keeps later typing and GTK lowering direct, and avoids carrying control-node meaning as ad-hoc tag-name checks past HIR.
    - **Future refinement:** Ordinary element nodes can later gain widget-resolution metadata, but the Milestone 2 control-node enum should stay closed to the currently parsed surface forms.

17. **Area:** Decorator resolution scope in Milestone 2 (`RFC §4.2`, `§24`, current parser surface`)
    - **Ambiguity / decision point:** The current parser accepts dotted decorator heads and a structured `@source` payload, but the language does not yet have decorator declarations, aliases, or decorator imports.
    - **Chosen interpretation:** Milestone 2 uses a closed compiler-known decorator registry with `@source` as the only accepted decorator. Its provider path is preserved exactly as a dotted provider key rather than resolved through ordinary `use` lookup, while its argument and `with { ... }` option expressions lower and resolve like ordinary term expressions. Any non-`@source` decorator is a Milestone 2 lowering error rather than opaque metadata.
    - **Rationale:** This matches the current fixture corpus and RFC surface without inventing a macro/decorator language that the spec has not declared, while still keeping source payload expressions in the ordinary HIR graph.
    - **Future refinement:** If AIVI later adds declared decorators, aliases, or decorator imports, that should extend this closed registry into a real decorator namespace intentionally rather than by preserving unknown annotations today.

18. **Area:** Import and lexical name-resolution scope in Milestone 2 (`RFC §4.2`, `§9.4`, `§17.3`, `§24`)
    - **Ambiguity / decision point:** The current surface has only unqualified identifier references plus `use module (imports)` with no alias, wildcard, or qualified ordinary-name syntax, but imported names are used by different later subsystems (`aivi.defaults`, `aivi.network`, and similar bundles).
    - **Chosen interpretation:** Milestone 2 resolves ordinary lexical names through nested term scopes (function parameters, pipe/case binders, record-pattern shorthand binders, `<each as={...}>`, and `<with as={...}>`) over module top-level term bindings, constructors, builtin terms/types, and explicit ordinary member imports. The current ordinary import catalog is intentionally narrow: imports like `use aivi.network (http)` populate ordinary lexical lookup, while bundle imports such as `use aivi.defaults (Option)` are recorded but do not shadow builtin lexical resolution of `Option`. Milestone 2 still does not invent aliasing, wildcard imports, or module-qualified ordinary references.
    - **Rationale:** This keeps lexical lookup deterministic for the current syntax surface, supports the fixture/module corpus honestly, and avoids fake-import modeling of builtins.
    - **Future refinement:** Alias syntax, wildcard imports, qualified ordinary references, and richer export/re-export behavior should only arrive as explicit surface features with their own resolution rules.

19. **Area:** Post-update RFC staging for domains, pipes, and sources (`RFC §8`, `§11.3`-`§11.7`, `§14`, `§20`)
    - **Ambiguity / decision point:** The RFC was sharpened mid-implementation: `Validation` notation was normalized, `?|>` / `T|>` / `F|>` / `*|>` / `<|*` / tap / recurrent-flow semantics were made explicit, source refresh and file-read-on-change behavior were tightened, and `domain ... over ...` gained a much richer nominal-data model.
    - **Chosen interpretation:** These updates change semantic obligations, but not the current Milestone 2 HIR shape. The active HIR pass preserves explicit pipe/control/source structure so later typing/runtime phases can implement the clarified behavior directly. `domain` is not retrofitted into existing `type` lowering; it remains a future dedicated top-level form to implement in a follow-up parser/HIR pass once the Milestone 2 lowering boundary is stable.
    - **Rationale:** This keeps layer boundaries honest: semantic/runtime clarifications belong to later phases, while nominal `domain` declarations deserve their own syntax/HIR node rather than ad-hoc alias/newtype reuse.
    - **Future refinement:** Add `domain` parser + HIR support in the planned follow-up, then implement the newly clarified pipe/source/runtime rules in typing, lowering, and scheduler/runtime layers against the already explicit HIR nodes.
 
20. **Area:** Domain declaration surface and HIR shape (`RFC §20`)
    - **Ambiguity / decision point:** The RFC gives domains their own nominal declaration form, carrier type, literal declarations, and operator/method members, but the existing frontend only had `type` / `class`-shaped top-level items.
    - **Chosen interpretation:** Implement `domain` as a dedicated top-level CST/HIR item. `domain` is a real top-level keyword, while `over` and `literal` are contextual parser keywords used only within domain declarations. The first implementation slice covers declaration parsing/formatting, HIR lowering, namespace participation, exportability, carrier resolution, member-signature preservation, and carrier self-reference rejection. Literal-suffix declarations are supported on the declaration side, but literal-suffix expression use sites such as `250ms` remain a later dedicated follow-up.
    - **Rationale:** This matches the RFC’s nominal model without pretending domains are aliases or repurposed classes, keeps future elaboration cheap, and avoids dragging literal-resolution semantics into the current HIR wave prematurely.
    - **Future refinement:** Add literal-suffix expression parsing/elaboration and fuller domain semantics (construction/elimination helpers, additional diagnostics, and later codegen/runtime behavior) on top of the now-stable domain item boundary.

21. **Area:** Compiler-derived `Eq` for domains (`RFC §20.9`, `§7.3`)
    - **Ambiguity / decision point:** The RFC recommends that domains may derive `Eq` from their carrier, but that does not imply domains are interchangeable with carriers, and no opt-out surface exists yet.
    - **Chosen interpretation:** Extend the focused `aivi-typing` `Eq` planner with an explicit nominal domain type node. A domain derives `Eq` exactly when its carrier derivation succeeds under the current context, and the resulting proof records a distinct domain wrapper step around the carrier proof. Because there is no surface opt-out mechanism yet, the current implementation treats domains as derivable whenever their carriers are derivable.
    - **Rationale:** This preserves nominal identity in the derivation model, keeps the current equality work aligned with the RFC’s domain guidance, and avoids smuggling domains through generic external references or alias-like collapse.
    - **Future refinement:** If/when a domain opt-out or explicit derive mechanism is added to the surface language, thread that flag into the domain `Eq` planner as a deliberate extension rather than by changing the nominal proof shape.

22. **Area:** Literal-suffix expression parsing and HIR lowering (`RFC §20.5`)
    - **Ambiguity / decision point:** The RFC allows forms such as `250ms`, but the existing surface grammar already treats adjacency as ordinary application (`f x`) and does not distinguish compact suffix syntax from spaced application.
    - **Chosen interpretation:** Parse immediate-adjacency integer-plus-identifier forms such as `250ms` as a dedicated suffixed-integer literal node in CST and HIR. Spaced forms such as `250 ms` remain ordinary application. The current implementation covers integer-family suffixes only, because that is the only literal family materially exercised by the RFC examples and current frontend.
    - **Rationale:** This adds the RFC’s domain-literal surface without weakening ordinary application syntax or inventing hidden whitespace rules. Using an explicit HIR node also keeps compile-time suffix resolution honest instead of pretending a domain literal is an ordinary top-level term.
    - **Future refinement:** Extend the same explicit-literal approach to other literal families only after their surface syntax and typing rules are specified, and broaden suffix scope beyond same-module declarations when the module/import model grows.

23. **Area:** Current scope of compile-time literal-suffix resolution (`RFC §20.5`)
    - **Ambiguity / decision point:** The RFC discusses suffix ambiguity “in scope,” including imported domains, but the current implementation still has a deliberately narrow ordinary module/import model and no user-module import graph.
    - **Chosen interpretation:** Compile-time literal suffix resolution currently ranges over visible domain literal declarations in the current module/HIR namespace. A unique suffix resolves to its owning domain literal declaration; multiple matching declarations are a compile-time ambiguity at the use site; no match is an unknown-suffix error.
    - **Rationale:** This fully implements the local compile-time behavior the current frontend can represent without inventing a broader import/export system than the repository currently has.
    - **Future refinement:** When user-module imports and re-exports are implemented, extend the same namespace model so imported domain literal declarations participate in suffix scope explicitly.

24. **Area:** Structural legality checks for sharpened pipe operators (`RFC §11.4.1`, `§11.5.1`)
    - **Ambiguity / decision point:** Some of the updated pipe rules are purely structural (`T|>` / `F|>` adjacency, `<|*` placement) while others are carrier/runtime dependent. The current codebase has HIR/lowering but not the later typing/runtime layers.
    - **Chosen interpretation:** Enforce the purely structural rules at HIR lowering time now: a run of truthy/falsy shorthand stages must be exactly one adjacent `T|>` / `F|>` pair, and `<|*` is legal only immediately after `*|>`. Carrier-specific behavior for `?|>`, `*|>`, recurrence, and source lifecycle remains deferred to later typing/runtime layers.
    - **Rationale:** This closes a real RFC gap in the existing frontend without smuggling type checking or scheduler behavior into Milestone 2.
    - **Future refinement:** Add typed elaboration/planning for ordinary vs `Signal` carriers and runtime-specific recurrence/source checks once those later layers exist.

25. **Area:** Structural text interpolation in syntax and HIR (`RFC §19.1`, `§14`)
    - **Ambiguity / decision point:** The frontend previously preserved string literals only as raw text plus a boolean interpolation flag, which was not enough for source dependency extraction, and the RFC does not define string-pattern destructuring semantics.
    - **Chosen interpretation:** Parse text literals eagerly into explicit alternating text fragments and `{ ... }` expression holes in CST and HIR. Raw text fragments preserve the literal interior spelling between holes, while interpolation holes parse as ordinary expressions with their own spans and canonical formatter output. Interpolated text remains legal in expression and markup/source-text positions, but interpolated text literals in pattern position are rejected as an explicit compile-time error.
    - **Rationale:** This provides the structural representation required by RFC text composition and source reactivity without introducing stringly heuristics, while the pattern-side restriction chooses the narrowest coherent behavior until the RFC gives explicit destructuring semantics.
    - **Future refinement:** If AIVI later adds string-pattern matching or richer interpolation escapes, extend the segment model intentionally rather than by weakening the current explicit-hole representation.

26. **Area:** Source reactivity metadata extraction (`RFC §13.1`, `§14`)
    - **Ambiguity / decision point:** The RFC requires statically known source dependency sets, but the current Milestone 2 HIR has only local module resolution and import bindings do not encode the imported item kind.
    - **Chosen interpretation:** After name resolution, every `@source`-backed `sig` receives `SourceMetadata` containing the resolved provider key, the sorted same-module set of directly referenced `sig` items reachable through source arguments/options (including text interpolation holes and nested expression structure), and an `is_reactive` flag derived from whether that dependency set is non-empty. Imported references are not treated as signal dependencies in the current implementation because Milestone 2 cannot yet prove that an import names a signal item.
    - **Rationale:** This matches the RFC’s “statically known dependencies” requirement as far as the current IR can represent honestly, keeps extraction structural rather than string-based, and avoids inventing cross-module signal knowledge that the current import model does not preserve.
    - **Future refinement:** Once imports resolve to richer item metadata or a typed inter-module graph exists, extend the same metadata pass so imported signal references participate in dependency extraction explicitly.

27. **Area:** General signal dependency metadata in HIR (`RFC §13.1`, `§14`)
    - **Ambiguity / decision point:** The RFC’s dependency discussion is source-driven, but ordinary derived `sig` items also need a stable structural dependency set if later scheduler/runtime layers are to stay deterministic.
    - **Chosen interpretation:** Every `sig`, not only `@source`-backed signals, now carries a sorted, duplicate-free same-module `signal_dependencies` list computed after name resolution from ordinary expression structure. For `@source` signals this list is the whole signal-facing dependency set, while `SourceMetadata.signal_dependencies` remains the source-config subset.
    - **Rationale:** This keeps dependency extraction uniform across signal forms, avoids a special-case metadata path for sources, and gives later layers one coherent place to read resolved signal-to-signal structure.
    - **Future refinement:** Broaden the same pass to imported signals only when the import model can prove signal item kinds without guessing.

28. **Area:** Structural `@source` diagnostics in HIR lowering (`RFC §14.1.1`)
    - **Ambiguity / decision point:** Some source-declaration errors are knowable from surface structure alone, while others depend on later typing or runtime provider contracts.
    - **Chosen interpretation:** Milestone 2 lowering now rejects structurally invalid `@source` forms immediately: missing provider variants, underspecified provider paths such as `http` without a variant, non-record `with` payloads, and duplicate option labels. These remain lowering diagnostics only; they do not change the resolved HIR shape for otherwise valid structure.
    - **Rationale:** This captures real user mistakes at the earliest honest layer without pretending HIR already knows provider schemas, option value types, or runtime lifecycle semantics.
    - **Future refinement:** Add richer provider-aware diagnostics only where the RFC gives a closed static contract that can be enforced without leaking typed/runtime behavior into Milestone 2.

29. **Area:** Built-in source option schemas and domain-valued source options (`RFC §14.1.2`, `§20.5`)
    - **Ambiguity / decision point:** The RFC lists recommended v1 provider options, while newer examples prefer domain-shaped quantities such as `5s` and `3x` over raw millisecond/count integers.
    - **Chosen interpretation:** Milestone 2 now uses a provider-keyed structural option registry for the compiler-known built-in source variants and validates option *names* against that registry. For quantity-like options, the registry follows the domain-oriented vocabulary (`timeout`, `refreshEvery`, `jitter`, `debounce`, `heartbeat`) rather than `*Ms` spellings. Option *values* are still just ordinary expressions at this layer, so domain literal forms such as `5s` and `3x` are accepted and resolved normally, but their expected types are not enforced until later typing.
    - **Rationale:** This lets the frontend reject misspelled source options now, aligns examples with the RFC’s domain-literal direction, and avoids fake type checking in HIR lowering.
    - **Future refinement:** When typed source schemas exist, validate source option expression types against provider contracts explicitly instead of relying only on option-name legality.

30. **Area:** Unfinished applicative-cluster diagnostics (`RFC §12.7`, `§24`)
    - **Ambiguity / decision point:** The surface fixture corpus already contained unfinished `&|>` clusters such as a cluster followed by `?|>`, but the parser was still rejecting them as syntax even though the RFC positions this as a later pipe-normalization legality rule.
    - **Chosen interpretation:** Parsing now accepts these pipe shapes structurally and leaves the legality check to HIR lowering. If a contiguous `&|>` region does not end in an explicit cluster finalizer and more pipe stages follow, lowering reports `illegal-unfinished-cluster` and still preserves structurally valid HIR.
    - **Rationale:** This moves the diagnostic to the correct layer, keeps the CST faithful to the authored surface, and matches the RFC’s Milestone 4 framing without prematurely normalizing clusters in the parser.
    - **Future refinement:** Extend the same Milestone 4 legality pass with the remaining cluster restrictions and exact normalization once typed applicative constructors are available.

31. **Area:** Milestone 3 kind-checking foundation (`RFC §6.1`, `§20.1`, `§20.8`)
    - **Ambiguity / decision point:** Milestone 3 requires kind checking and named constructor partial application, but the current codebase had only a focused structural `Eq` planner and no reusable kind model.
    - **Chosen interpretation:** Add a dedicated `aivi-typing::kind` module that models the v1 kind language (`Type` plus right-associative arrows), named type constructors, type parameters, structural type expressions, iterative kind inference, and explicit expected-kind checks. Parameterized domains use the same constructor-kind model as ordinary named constructors.
    - **Rationale:** This creates the missing type-side foundation without smuggling kind logic into the `Eq` planner, keeps stack-safety explicit via iterative inference, and gives later HIR/typed-core integration a principled API for constructor partial-application checks.
    - **Future refinement:** Thread this kind model into resolved HIR type expressions, then layer class/instance resolution and typed-core elaboration on top of the same constructor-kind discipline.

32. **Area:** HIR-integrated constructor kind validation (`RFC §6.1`, `§24`)
    - **Ambiguity / decision point:** Once the standalone kind model existed, the next question was how much of it should be enforced directly in resolved HIR without pretending full typing or typed-core elaboration already exists.
    - **Chosen interpretation:** `aivi-hir` now uses the `aivi-typing::kind` machinery only in `RequireResolvedNames` mode and only for root type positions that already semantically expect a concrete type or a specific constructor arity: alias bodies, variant fields, annotations, class/domain member types, domain carriers, and instance heads. Kinds are derived from builtins plus the parameter counts of resolved `type`, `class`, and `domain` items. Imported type references are intentionally skipped in this first slice because the current import model does not yet preserve constructor-kind metadata.
    - **Rationale:** This catches real over-/under-application errors at the earliest honest typed boundary, keeps the structural validator mode unchanged, and avoids inventing cross-module kind facts the current HIR cannot justify.
    - **Future refinement:** Extend the same pass to imported type constructors once import bindings carry kind metadata, then build class/instance resolution and typed-core elaboration on top of these resolved HIR kind checks.
