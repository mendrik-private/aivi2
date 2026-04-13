# Wadler-style audit

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

### The stdlib is algebraic internally, but concrete at the public surface

- `Result` and `Validation` tell a principled story: `Result` provides sequential `flatMap`, while `Validation` keeps independent accumulation through `zipValidation` over `NonEmptyList` errors. (`stdlib/aivi/result.aivi:48-55`; `stdlib/aivi/validation.aivi:6-24`; `stdlib/aivi/validation.aivi:71-79`)
- `List` is implemented in a strongly fold-derived style: `length`, `head`, `map`, `filter`, `flatten`, `flatMap`, `any`, `all`, `find`, and related combinators are visibly built from `reduce`. (`stdlib/aivi/list.aivi:140-187`; `stdlib/aivi/list.aivi:216-282`)
- But the prelude foregrounds concrete helpers such as `mapOption`, `mapResult`, `flatMapOption`, `flatMapResult`, `foldOption`, and `foldResult`, even while exporting class names like `Functor`, `Applicative`, and `Monad`. (`stdlib/aivi/prelude.aivi:117-183`; `stdlib/aivi/prelude.aivi:385`)
- The prelude also exports the `Validation` type without re-exporting its main combinators, which makes the public abstraction story feel incomplete. (`stdlib/aivi/prelude.aivi:1-115`; `stdlib/aivi/prelude.aivi:385`; `stdlib/aivi/validation.aivi:81-92`)

The result is a library that often *implements* algebraically but does not always *present itself* algebraically.

### Some stdlib APIs sacrifice semantic clarity

- `List.maximum`, `minimum`, `unique`, and `sortBy` all require explicit comparators instead of using `Ord`/`Eq`-driven genericity when instances exist. (`stdlib/aivi/list.aivi:388-396`; `stdlib/aivi/list.aivi:465-519`)
- `List.contains` is just `any` under a new name, taking a predicate instead of testing membership. That is flexible, but the name suggests a more specific meaning than the implementation provides. (`stdlib/aivi/list.aivi:398-400`)
- `Matrix.filled` silently turns negative dimensions into `MkMatrix 0 0 []`, unlike `init`/`fromRows`, which use `Result` for invalid shapes. (`stdlib/aivi/matrix.aivi:120-128`; `stdlib/aivi/matrix.aivi:248-251`)

## Current documentation drift

### The RFC underclaims the executable class story

- The RFC header still lists `Monad`/`Chain` lowering and `&|>` typed-core lowering as known open gaps. (`AIVI_RFC.md:1-5`)
- The current implementation no longer matches that description, and the typeclass guide is internally split: its support table still shows `Task` as applicative-only, but the explanatory note immediately below says `Task` has builtin executable `Functor`, `Apply`, `Applicative`, `Chain`, and `Monad` support, which matches the runtime evaluator. (`manual/guide/typeclasses.md:56-69`; `crates/aivi-backend/src/runtime/evaluator.rs:1846-1860`; `crates/aivi-backend/src/runtime/evaluator.rs:2048-2074`; `crates/aivi-backend/src/runtime/evaluator.rs:2180-2204`; `crates/aivi-backend/src/runtime/evaluator.rs:2288-2303`)

### Some guide examples and signatures still disagree with the current semantic story

- The typeclass guide writes `Traversable.traverse` as `Applicative G -> ...`, while the language syntax and RFC use the constraint form `Applicative G => ...`. (`manual/guide/typeclasses.md:47`; `syntax.md:77-109`; `AIVI_RFC.md:734-737`)
- The classes guide still shows `instance Eq Calendar` defining `(!=)`, while the RFC says `!=` is sugar over `not (==)` and has no separate dictionary slot. (`manual/guide/classes.md:159-162`; `AIVI_RFC.md:845-847`)

## Recommendations

1. **Promote one executable source of truth for class support.** The typeclass guide is now closer to reality than the RFC header; the RFC should stop claiming `Monad`/`Chain` and `&|>` are missing if that is no longer true. (`AIVI_RFC.md:1-5`; `manual/guide/typeclasses.md:56-69`)
2. **Document the builtin-vs-authored instance split as an architectural boundary.** Right now it is discoverable only by reading the guide, tests, and backend. Make the “builtin carriers plus unary hidden-callable path” model explicit in one canonical place. (`manual/guide/typeclasses.md:106-123`; `crates/aivi-backend/tests/foundations_parts/runtime_eval.rs:804-840`)
3. **Decide whether the stdlib wants a class-polymorphic public face or a concrete-first one.** Either is defensible, but the current hybrid prelude weakens the elegance of the class hierarchy. (`stdlib/aivi/prelude.aivi:117-183`; `stdlib/aivi/prelude.aivi:385`)
4. **Extend the law/documentation story to match the advertised hierarchy.** `Functor`, `Applicative`, and `Monad` get the clearest law treatment; the other prominently named classes deserve equally explicit guidance. (`manual/guide/typeclasses.md:40-49`; `crates/aivi-hir/src/lower/ambient.rs:15-123`)
5. **Tighten a few misleading APIs.** `List.contains` and `Matrix.filled` are the clearest places where the current surface sacrifices precision for convenience. (`stdlib/aivi/list.aivi:398-400`; `stdlib/aivi/matrix.aivi:248-251`)

## Bottom line

If the question is whether AIVI *thinks* like Wadler, the answer is mostly yes. If the question is whether the current implementation and stdlib expose that thinking with the same uniform elegance, the answer is not yet: the design philosophy is ahead of the surface curation and documentation consistency.
