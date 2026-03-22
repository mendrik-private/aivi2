# Choices made for the initial implementation wave

This file is a plain-language summary of the current implementation choices, with one short explanation per section.

1. **Validation notation:** Keep the behavior the same, but describe it using AIVI's own terms instead of Haskell terms.

2. **`?|>` gate behavior:** This operator means "keep it if the check passes." For changing or live values, only passing updates continue and failures do not create fake opposite updates.

3. **Reactive text inside `@source`:** If a source string includes changing values, those values count as dependencies. When they change, the source is rebuilt using the new text.

4. **HTTP refreshes:** HTTP sources refresh only for explicit reasons, such as changing reactive inputs or timers or retries written in code. There are no hidden refreshes tied to focus, windows, or other app lifecycle events.

5. **Watching files vs reading files:** Watching a file only reports that something changed. Reading and decoding the file must still be done as a separate step.

6. **Orphan instances:** These are fully disallowed for now to keep behavior consistent and easy to reason about.

7. **Milestone order:** Build the system in milestone order and keep each layer responsible for its own job. Later runtime concerns should not leak back into earlier compiler stages.

8. **Advanced type features:** Start with a smaller, predictable set of advanced type features. More powerful type-level features can be added later as a separate design.

9. **`<each>` keys:** Every `<each>` must have a key. This keeps repeated UI items stable when they move or change.

10. **Decoder overrides:** Only the built-in decoding path exists for now. Custom decoder hooks are delayed until the base behavior is fully defined.

11. **Internal data layout:** Use stable internal IDs and iterative walking instead of deep recursion. This makes the compiler safer and more reliable on large inputs.

12. **Equality support:** Equality is provided automatically only for data shapes that are clearly and safely comparable. Functions, live values, tasks, and outside handles are excluded.

13. **Early internal program model:** The early compiler model uses stable IDs for items, bindings, and expressions, and it keeps source locations attached. That makes later processing and error reporting more reliable.

14. **Record shorthand:** Record shorthand stays in shorthand form during early processing. This preserves better errors and avoids pretending the compiler already knows the full record shape.

15. **Applicative clusters:** These special pipe clusters stay grouped together instead of being flattened too early. That keeps their original meaning and diagnostics intact.

16. **Special markup tags:** Tags like `show`, `each`, and `match` are treated as real control features, not normal markup. This makes later UI handling much simpler.

17. **Decorators:** Only `@source`, `@recur.timer`, and `@recur.backoff` are real decorators right now. `@source` stays signal-only, while the two `@recur.*` forms are closed non-source recurrence-wakeup witnesses on `val`, `fun`, and non-`@source` `sig` declarations. Other decorator-like syntax is rejected instead of being carried around as unknown metadata.

18. **Name lookup and imports:** Name lookup stays intentionally simple: local names and a small set of imports work, but there are no aliases, wildcards, or module-qualified names yet. Built-in names also keep priority where needed.

19. **Recent spec updates:** Recent spec updates change what later stages must do, but they do not force a redesign of the current early compiler shape. `domain` will be added as its own feature instead of being squeezed into older type handling.

20. **`domain` declarations:** A `domain` declaration is treated as its own real language feature. You can declare domain suffixes now, but using them directly in expressions is handled later.

21. **Equality for domains:** Domains can automatically get equality when their underlying value can be compared. Even so, they still remain their own named types.

22. **Suffix literals like `250ms`:** Compact forms like `250ms` are treated as special suffix literals, while spaced forms like `250 ms` keep their normal meaning. Only integer-based suffixes are supported for now.

23. **Where suffixes are resolved:** Literal suffixes are resolved only against declarations in the current module for now. No match is an error, and multiple matches are treated as ambiguous.

24. **Newer pipe operator rules:** Only the obvious shape and ordering rules for the newer pipe operators are enforced at this stage. Deeper behavior that depends on typing or runtime rules is left for later.

25. **Interpolated text structure:** Interpolated text is stored as alternating plain text and expression holes instead of one opaque string. That makes dependency tracking and error reporting clearer.

26. **Source dependency tracking:** Source-backed signals record which local signals they depend on. Imported references are not guessed to be signals yet.

27. **General signal dependency tracking:** All signals, not just source-backed ones, now carry an explicit list of local signal dependencies. This gives later scheduling work one consistent dependency story.

28. **Early `@source` errors:** The compiler now catches obviously malformed `@source` declarations early, such as missing variants or duplicate options. More detailed provider-specific checks still come later.

29. **Built-in source options:** Built-in sources now have a known list of allowed option names using clearer names like `timeout` and `refreshEvery`. The compiler checks the option names now, but not yet whether each value has the perfect type.

30. **Unfinished applicative clusters:** These shapes are no longer rejected too early by the parser. They are accepted first and then flagged later in the more appropriate validation step.

31. **Type-shape checking foundation:** The project now has a reusable foundation for checking whether advanced type constructors are being used in the right shape. This is groundwork for later type-checking.

32. **Using that checking in early validation:** That new checking is now used in places where the compiler already has enough trustworthy information. Same-module types are checked directly, and imported type constructors participate too only when the closed Milestone 2 import catalog carries explicit constructor-kind metadata. Imports without that metadata still stay skipped instead of being guessed.

33. **Repeating-flow syntax rules:** Repeating-flow syntax is limited to one narrow, clearly structured trailing form for now. Mixed or messy shapes are rejected.

34. **Internal view of applicative clusters:** The compiler keeps these clusters in their user-facing form, but also records a clean internal recipe for what they mean. Later stages can use that recipe without re-guessing it.

35. **Internal view of repeating-flow tails:** Repeating-flow syntax stays visible in the early internal model, but the compiler also exposes a clean extracted view of the repeating tail. Later stages can use that directly instead of rebuilding it by hand.

36. **Catalog of source option shapes:** There is now a central catalog describing the expected shape of built-in source options. That gives later checking a single source of truth.

37. **Gate behavior checks:** Gate behavior is checked using only type facts the compiler can already prove today. Obvious mistakes are rejected, while uncertain cases are left open instead of over-restricted.

38. **Where repeating flows are allowed:** Repeating flows are allowed only where the compiler can already prove they target something supported, such as a signal or task declaration. Everything else is rejected instead of guessed.

39. **Required trigger for repeating flows:** Every repeating flow must have a clear trigger the compiler can already recognize, like a built-in timer or event source, reactive custom-source input, or an explicit non-source `@recur.timer` / `@recur.backoff` witness. Cases without a provable trigger are rejected for now.

40. **Resolving source option types:** Source option schemas are now matched to real program types where possible. The compiler still stops short of fully type-checking the option values themselves.

41. **Lowering plan for gates:** The compiler now produces a clear lower-level plan for how gate stages should behave later. If it cannot prove enough today, it records the blocker instead of making something up.

42. **Runtime handoff for signal filters:** Signal-based filters now lower into a simple typed filter description that future runtime code can use. Only clearly safe expression forms are included for now.

43. **Local source option value checks:** Source option values are now checked only in cases the current resolved HIR can really prove: same-module annotations, suffix literals, same-module constructors, list elements built from those, and reactive `Signal` payloads used as ordinary source configuration values. Imported bindings and other harder expressions still wait for fuller expression typing.

44. **Custom `@source` recurrence wakeups:** Custom providers now get the largest honest wakeup slice the current compiler can prove. Reactive source inputs count as an explicit source-event wakeup for any provider, because RFC reconfiguration on upstream signal changes is provider-independent. Non-reactive custom providers still need future provider-contract metadata, so the compiler now carries an explicit custom wakeup hook in resolved source metadata instead of guessing that built-in option names like `retry` or `refreshOn` mean the same thing for custom providers.

45. **Non-source recurrence wakeup witnesses:** Plain repeating `Signal` and `Task` bodies now prove timer/backoff wakeups only through compiler-known `@recur.timer expr` and `@recur.backoff expr` decorators. Those decorators each take exactly one positional witness expression, reject `with { ... }` options or duplicates, and are not allowed on `@source` signals so the source-backed proof story stays separate.

46. **Fan-out carrier handoff:** `*|>` and an immediate `<|*` now use one focused typed handoff. `*|>` is only proven on `List A` or `Signal (List A)`, its body sees `A` as the ambient subject, and the result preserves ordinary-vs-`Signal` carrier shape without flattening nested collections or signals. `<|*` stays grouped with that map segment and is typed as a normal pipe body over the mapped collection, with `Signal` joins lifted pointwise instead of inventing scheduler/runtime nodes early.

47. **Bidirectional source constructors:** Source option constructor expressions are now checked against the already-resolved expected source-contract type. Same-module constructors may be used nullary or fully applied, and constructor field types are instantiated from the expected type arguments only when those field annotations lower back into the current closed source-option type surface. Imported bindings and contract-parameter-driven holes still stay blocked instead of being guessed.

48. **Built-in source recurrence metadata:** Built-in `@source` contracts now carry recurrence-specific metadata in the same typed contract layer as option legality. HIR wakeup validation reads retry/polling/trigger slots and intrinsic timer/event wakeups from that contract metadata instead of hard-coding provider semantics in multiple places, so future custom-provider declarations can plug into one contract-shaped handoff when the language grows a real provider declaration surface.

49. **Source contract parameter holes:** Source option checking now keeps provider-local `A` / `B` holes explicit in its internal expected-type patterns instead of erasing them to “anything.” That lets the compiler keep proving known outer structure such as `Signal ...` and same-module constructor field substitutions honestly, while a bare hole by itself still stays unproven until later work adds real provider-level parameter binding.

50. **Typed source-provider identity:** Before provider declarations existed, resolved HIR preserved each `@source` provider as missing, built-in, custom, or invalid-shape and carried custom-provider contract facts through one explicit hook. That kept later declaration/resolution work local and prevented custom metadata from being attached to built-in providers by accident.

51. **Imported source option bindings:** Imported source option values are checked only when the current Milestone 2 import catalog carries an explicit closed value type that resolved HIR can lower directly, such as `Text`, `List ...`, or `Signal ...`. Imports without that metadata still stay unproven instead of being guessed from names or module files the compiler does not yet model.

52. **Local source contract parameter bindings:** Resolved-HIR source option validation now carries a small provider-local binding environment for `A` / `B` across one `@source ... with { ... }` record. Bindings commit only from fully proven option expressions, pending options are retried to a fixed point so later proofs can unlock earlier constructor checks, and those bindings substitute back only through the current closed `GateType` proof surface. Generic constructor roots and other bare-parameter expressions that still lack honest local type evidence remain blocked until fuller ordinary expression typing exists.

53. **Narrow provider contract resolution:** Because the RFC still lacks a full custom-provider declaration chapter, the compiler keeps the smallest coherent declaration-and-resolution surface: a top-level `provider qualified.name` item with an optional indented member body, plus same-module order-independent lookup onto matching custom `@source` use sites. Only `wakeup: timer | backoff | sourceEvent | providerTrigger` lowers today; built-in provider keys, unqualified names, unknown fields, and duplicate `wakeup` members diagnose immediately, while missing or duplicate declarations do not invent extra provider-existence errors or arbitrary custom metadata.

54. **Generic bare source constructor roots:** A bare source-contract parameter like `A` may now bind from a same-module generic constructor root only when the current resolved-HIR layer can prove every constructor field from local evidence: already typed arguments, reactive payload peeling, or concrete field expectations checked through the existing source-option checker. Generic roots whose arguments still lack direct type evidence, or whose field annotations leave the current closed proof surface, remain unproven instead of inventing broader inference.

55. **Canonical recurrence wakeup proof:** For the current explicit-wakeup slice, recurrence planning records one deterministic explicit wakeup witness even when several proofs are available. Built-in sources keep a stable proof order—intrinsic provider wakeups first, then polling, retry, source-trigger options, and finally reactive inputs—while custom providers prefer declared provider-contract wakeups over reactive-input fallback. This keeps validation and later scheduler-node lowering deterministic without pretending the compiler already models combined wakeup graphs.

56. **Custom provider schema surface:** Custom `provider qualified.name` declarations now stay intentionally line-oriented: `wakeup: ...` plus repeated `argument name: Type` and `option name: Type` members. Those schema annotations are checked only through the same honest closed proof surface the compiler already has for source configuration values—primitive types, same-module named types/domains, and those shapes under `List` or `Signal`. Richer forms such as records, arrows, imported constructors, or `Option`/`Result` are rejected on the declaration instead of being guessed.

56. **Recurrence scheduler-node handoff:** The compiler now lowers each validated recurrence suffix into one typed scheduler-node report that keeps the `@|>` start stage, the ordered `<|@` step stages, the canonical target/wakeup plan, and any non-source wakeup witness separate instead of collapsing them into one opaque loop function. This is the narrowest honest handoff because the RFC distinguishes start from step, while later runtime/backend layers can consume that handoff without asking the frontend to guess more.

57. **Source lifecycle handoff:** Source-backed signals now carry one explicit lifecycle handoff. Same-module signal dependencies are split into reactive reconfiguration inputs, explicit trigger-signal slots, and built-in `activeWhen` gates; every `@source` site gets a stable instance ID plus mandatory stale-publication suppression on replaced or disposed work; and only compiler-known request-like built-ins (`http.*`, `fs.read`) are marked for best-effort in-flight cancellation. Custom providers still reuse the generic reconfiguration/stale model, but they do not gain invented `activeWhen` or trigger semantics until provider contracts grow that surface explicitly.

58. **Pipe/source umbrella boundary:** The RFC §11 / §14 frontend umbrella is considered complete once the compiler carries honest gate, fanout, recurrence, provider-contract, and source-lifecycle handoffs into resolved HIR and typing. Fuller ordinary expression typing for harder source option values remains separate follow-on work and should not keep that umbrella blocked.

59. **Recurrence runtime-lowering scope:** `pipe-recurrence-runtime-lowering` is complete once the compiler reaches the last honest pre-runtime handoff: closed target/wakeup proof in resolved HIR plus `aivi-hir::elaborate_recurrences` and `aivi-hir::elaborate_source_lifecycles`. Real typed-core/backend/runtime consumption stays separate follow-on work because those layers do not exist in this workspace yet.

60. **Bare source-root actual typing:** Source option root checking now has its own closed actual-type fallback instead of relying only on ordinary expression inference. It can recursively prove same-module constructor roots, unannotated local `val` bodies, tuple/record/list literals, and `Some` roots directly, while locally expected container shapes can also validate `None` / `Ok` / `Err` / `Valid` / `Invalid` once sibling bindings or concrete field annotations fix the missing type arguments.

61. **Context-free source builtin holes:** Provider-local source-option bindings may now carry a narrow partial actual-type proof for bare `None` / `Ok` / `Err` / `Valid` / `Invalid` roots. Those partial proofs keep only the built-in container shape plus anonymous wildcard leaves, refine when later local evidence arrives, and do not widen into general ordinary-expression inference.

62. **Regex literal validation layer:** Regex literal well-formedness now belongs to HIR validation instead of lexing. The compiler currently uses the Rust `regex-syntax` grammar only to accept or reject `rx"..."` literals at compile time, which keeps the validation slice explicit without pretending runtime lowering semantics already exist.

63. **Truthy/falsy branch handoff:** Resolved HIR now gives `T|>` / `F|>` one deterministic ordinary-carrier handoff: only builtin `Bool`, `Option`, `Result`, and `Validation` subjects elaborate today, each pair chooses the RFC’s canonical builtin constructors directly, one-payload branches type their body against that payload as the ambient pipe subject, zero-payload branches do not invent an ambient payload, and branch result mismatches are rejected only when the current local proof surface can really see both branch types. Signal-lifted branching, user-defined truthy/falsy carriers, and bare `_` spellings that still depend on the separate ambient-subject gap remain later work instead of being guessed here.

64. **Focused case exhaustiveness checks:** Resolved HIR now exhaustiveness-checks `||>` and markup `<match>` only when the current local proof surface can already know the scrutinee type honestly: ordinary `Bool`, `Option`, `Result`, `Validation`, and same-module closed sums reached through annotations, function parameters, and typed markup bindings. Missing constructors diagnose by name, `_` and named binding patterns count as explicit catch-alls, and imported sums, signal-lifted case splits, and harder constructor-built scrutinee inference remain later work instead of being guessed.

65. **Compiler-generated domain decode surfaces:** The structural decoder handoff now resolves domain-backed fields only through the narrowest deterministic same-module surface the current compiler can prove: a method named `parse` wins when its annotation has shape `Carrier -> Result E Domain`, otherwise exactly one method with shape `Carrier -> Domain` or `Carrier -> Result E Domain` is accepted. Operators, literal members, and ambiguous multiple constructor-like methods stay blocked instead of guessing runtime decode semantics.
