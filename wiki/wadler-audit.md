# Wadler-style audit

Focused audit notes for the uniform-elegance cleanup backlog.

Status: backlog closed on 2026-04-13. This page now records the post-closeout state instead of the pre-fix drift that originally triggered the backlog.

## Executable class support documentation

- Canonical human-facing source: `manual/guide/typeclasses.md#canonical-builtin-executable-support`
- Canonical implementation source: `crates/aivi-core/src/class_support.rs`
- Policy: other docs should summarize or link to that section instead of copying support matrices

## Preserved invariants

- `Signal` remains applicative and non-monadic.
- `Validation E` remains applicative and non-monadic.
- `Task E` keeps its current builtin executable `Functor` / `Apply` / `Applicative` / `Chain` / `Monad` support.
- `Traversable` support and traverse-result applicative support stay distinct: `Signal` is allowed only on the result-applicative side, and `Task` is excluded there.
- Docs in this slice should describe `!=` as surface inequality using the same `Eq` evidence as `==`; do not teach separate `(!=)` instance bodies as the canonical user story.

## Verdict

AIVI is strongly Wadlerian in *intent*: the language is pure by default, effect boundaries are explicit, `Signal` and `Validation` are deliberately applicative rather than monadic, and the ambient class graph is recognizably algebraic. The main weakness is not the surface philosophy but the uneven realization: executable semantics depend heavily on builtin carrier tables and backend intrinsics, while the stdlib and prelude present a more concrete, datatype-specific API than the class hierarchy suggests. (`syntax.md:11-18`; `AIVI_RFC.md:15-27`; `manual/guide/typeclasses.md:40-69`; `crates/aivi-hir/src/lower/ambient.rs:15-123`; `crates/aivi-core/src/expr.rs:91-155`; `crates/aivi-backend/src/kernel.rs:25-45`)

## Where AIVI aligns well with Wadler

### Pure core and explicit effect boundaries

- The public language shape is explicitly pure, closed, and expression-first. `Option` replaces nullability, loops/`if` are absent from the surface, and `Signal`/`Validation` are explicitly non-monadic. (`syntax.md:11-18`)
- The RFC keeps the same story: pure user code by default, a pure semantic core, and `Task E A` as a one-shot effect description rather than an immediate effect. (`AIVI_RFC.md:15-27`; `AIVI_RFC.md:57-63`; `AIVI_RFC.md:2191-2210`)

### Applicative discipline is principled, not accidental

- The RFC describes `&|>` as the exact applicative clustering surface for independent `Option`, `Result`, `Validation`, `Signal`, and `Task` computations. (`AIVI_RFC.md:1519-1529`)
- `Signal` remains applicative because the runtime wants a static dependency graph, and `Validation` remains applicative because independent error accumulation should not collapse into monadic dependence. The guide now states this explicitly. (`manual/guide/typeclasses.md:61-69`)

### The class hierarchy is mathematically legible

- The ambient prelude defines `Functor`, `Apply`, `Applicative`, `Chain`, `Monad`, `Foldable`, `Traversable`, `Filterable`, `Bifunctor`, `Category`, `Profunctor`, and related classes in a coherent superclass graph. (`crates/aivi-hir/src/lower/ambient.rs:15-123`)
- The user-facing typeclass guide presents the same core ladder: `Functor -> Apply -> Applicative`, `Apply -> Chain -> Monad`, with `Traversable` above `Functor` and `Foldable`. (`manual/guide/typeclasses.md:40-49`)

### The kind system is intentionally narrow and predictable

- `aivi-typing` models only `Type` and right-associative kind arrows. That is a modest HKT story, but it is explicit and mechanically checkable rather than magical. (`crates/aivi-typing/src/kind.rs:3-11`; `crates/aivi-typing/src/kind.rs:18-32`)

## Where AIVI departs from Wadler-style elegance

### Algebraic structure is real, but much of it is builtin rather than uniform

- Core and backend IRs encode class support through builtin carrier tables rather than a fully uniform dictionary-passing model. `Functor`, `Applicative`, `Apply`, `Monad`, `Foldable`, `Traversable`, `Filterable`, and `Bifunctor` each have explicit carrier enums in the core and backend. (`crates/aivi-core/src/expr.rs:91-155`; `crates/aivi-backend/src/kernel.rs:25-45`; `crates/aivi-backend/src/kernel.rs:47-111`)
- The actual executable slice is broad: `Functor`/`Apply`/`Applicative` cover `List`, `Option`, `Result`, `Validation`, `Signal`, and `Task`; `Monad` covers `List`, `Option`, `Result`, and `Task`; `Foldable`/`Traversable` cover `List`, `Option`, `Result`, and `Validation`; `Filterable` covers `List` and `Option`; `Bifunctor` covers `Result` and `Validation`. (`crates/aivi-core/src/expr.rs:91-155`; `manual/guide/typeclasses.md:56-69`)
- The runtime evaluator confirms this is not just metadata: it implements `Task`-specific `map`, `apply`, `chain`, and `join`, including deferred task plans for non-pure cases. (`crates/aivi-backend/src/runtime/evaluator.rs:1846-1860`; `crates/aivi-backend/src/runtime/evaluator.rs:2048-2074`; `crates/aivi-backend/src/runtime/evaluator.rs:2180-2204`; `crates/aivi-backend/src/runtime/evaluator.rs:2288-2303`)

This is a pragmatic design, but it is less Wadler-clean than a single abstraction mechanism that treats builtin and user-authored instances the same way.

### User-authored higher-kinded abstraction exists, but only in a narrow slice

- The typeclass guide explicitly says unary higher-kinded instances and partially applied heads work end to end today, while indexed / multi-parameter evidence remains unfinished. (`manual/guide/typeclasses.md:106-123`)
- The typechecker tests prove that same-module higher-kinded instance member signatures and partially applied same-module instances are accepted. (`crates/aivi-hir/src/typecheck/tests.rs:1725-1785`)
- `aivi.matrix` is the clearest authored example: it supplies `Functor` and `Foldable` instances via ordinary AIVI code rather than a new builtin carrier. (`stdlib/aivi/matrix.aivi:100-118`)
- Typed-core lowering materializes authored instance members as synthetic hidden items named `instance#...::member#...`, and the core lowering tests explicitly assert that this hidden-item synthesis happens. (`crates/aivi-core/src/lower/module_lowerer.rs:2916-2975`; `crates/aivi-core/src/lower/tests.rs:1036-1082`)
- Backend tests show imported `map` may lower either through a builtin intrinsic or an ambient hidden callable such as `__aivi_option_map`, which makes the split visible even in tests. (`crates/aivi-backend/tests/foundations_parts/runtime_eval.rs:804-840`)

This is an interesting compromise, but it weakens the “one algebraic story everywhere” feel.

### The stdlib surface is now much closer to the algebraic story

- `Result` and `Validation` tell a principled story: `Result` provides sequential `flatMap`, while `Validation` keeps independent accumulation through `zipValidation` over `NonEmptyList` errors. (`stdlib/aivi/result.aivi:48-55`; `stdlib/aivi/validation.aivi:6-24`; `stdlib/aivi/validation.aivi:71-79`)
- `List` is implemented in a strongly fold-derived style: `length`, `head`, `map`, `filter`, `flatten`, `flatMap`, `any`, `all`, `find`, and related combinators are visibly built from `reduce`. (`stdlib/aivi/list.aivi:140-187`; `stdlib/aivi/list.aivi:216-282`)
- The prelude cleanup now makes the ambient story more class-polymorphic first: bare `join` is the generic `Monad.join`, text joining stays explicit on `aivi.text.join`, and legacy pair aliases no longer define the ambient surface. (`stdlib/aivi/prelude.aivi`; `stdlib/aivi/text.aivi`; `stdlib/aivi/pair.aivi`; `manual/stdlib/prelude.md`)
- `Validation` is now first-class in the prelude surface, with ambient `validationGetOrElse`, `validationMapErr`, `validationToResult`, `validationFromResult`, `validationToOption`, `validationMap`, `validationAndThen`, `zipValidation`, and `validationFold`. (`stdlib/aivi/prelude.aivi`; `manual/stdlib/prelude.md`; `manual/guide/typeclasses.md`)

The result is still pragmatic rather than maximally abstract, but the main ambient surface now teaches the algebraic model far more directly.

### The high-friction stdlib API mismatches were resolved

- `List.maximum`, `minimum`, `unique`, and `sort` now use ambient `Ord` / `Eq`, while the explicit comparator-taking variants are named `maximumBy`, `minimumBy`, `uniqueBy`, and `sortBy`. (`stdlib/aivi/list.aivi`; `manual/stdlib/list.md`)
- `List.contains` now means `Eq`-driven membership; predicate search stays on `any`. (`stdlib/aivi/list.aivi`; `manual/stdlib/list.md`; `wiki/equality-semantics.md`)
- `Matrix.filled` now follows the checked constructor policy and returns `Result MatrixError (Matrix A)` for invalid dimensions, matching `init` and `fromRows`. (`stdlib/aivi/matrix.aivi`; `manual/stdlib/matrix.md`; `wiki/demo-audit.md`)

## Documentation status after cleanup

### RFC, manual, and wiki now tell the same primary story

- The RFC header now reflects the current executable slice instead of claiming `Monad` / `Chain` and `&|>` typed-core lowering are still missing. (`AIVI_RFC.md`)
- The typeclass guide now uses the correct `Applicative G => ...` signature form, the builtin support table correctly marks `Task` as monadic, and the execution boundary between builtin carriers and authored instances is explicit. (`manual/guide/typeclasses.md`; `crates/aivi-core/src/class_support.rs`)
- The classes guide and law docs now teach `!=` as ordinary surface inequality reusing `Eq`, not as a second canonical instance member. (`manual/guide/classes.md`; `manual/guide/class-laws.md`)
- The wiki now records the prelude surface policy, canonical executable-support ownership, law coverage, list membership semantics, and checked matrix constructor policy. (`wiki/prelude-surface-policy.md`; `wiki/type-system.md`; `wiki/stdlib.md`; `wiki/equality-semantics.md`)

## Backlog outcomes

1. **Canonical executable class support source landed.** Human docs now point at the registry-backed table generated from `crates/aivi-core/src/class_support.rs`.
2. **Builtin-vs-authored execution boundary landed.** The manual and wiki now state the split explicitly instead of leaving it implicit in tests and backend code.
3. **Prelude surface was rebalanced.** `Validation` is coherent in prelude, bare `join` is generic, text join is explicit, and legacy pair aliases no longer dominate the ambient story.
4. **Misleading list and matrix APIs were corrected.** `contains` is membership, comparator-taking variants carry `...By` names, and `Matrix.filled` is checked instead of lossy.
5. **Law/docs/RFC/wiki drift was closed.** The class laws page, RFC header, typeclass guide, classes guide, and supporting wiki pages now describe the same executable slice.

## Bottom line

If the question is whether AIVI *thinks* like Wadler, the answer is yes. After the cleanup backlog, the implementation, stdlib surface, RFC, manual, and wiki are much closer to one coherent algebraic story. The main remaining caveat is architectural explicitness: builtin carriers and authored unary higher-kinded instances still use different executable paths, but that boundary is now deliberate, documented, and test-backed rather than accidental drift.
