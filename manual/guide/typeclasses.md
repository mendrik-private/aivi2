# Typeclasses & Higher-Kinded Support

This page is the current manual source of truth for AIVI's higher-kinded class hierarchy and support boundaries.
It describes the **executable** compiler/runtime slice that exists today, not just surface syntax or checker acceptance.
For class declaration and instance syntax, see [Classes](/guide/classes).

## Current hierarchy

The ambient prelude includes a broader class graph, but the main higher-kinded slice currently centers on these relationships:

```text
Functor
├─ Apply
│  ├─ Applicative
│  └─ Chain
│     └─ Monad
├─ Filterable
└─ Traversable

Foldable
└─ Traversable

Bifunctor
```

`Monad` depends on both `Applicative` and `Chain`; `Chain` itself depends on `Apply`.

| Class | Direct superclasses | Primary member |
| --- | --- | --- |
| `Functor F` | — | `map : (A -> B) -> F A -> F B` |
| `Apply F` | `Functor F` | `apply : F (A -> B) -> F A -> F B` |
| `Applicative F` | `Apply F` | `pure : A -> F A` |
| `Monad M` | `Applicative M`, `Chain M` | `join : M (M A) -> M A` |
| `Foldable F` | — | `reduce : (B -> A -> B) -> B -> F A -> B` |
| `Traversable T` | `Functor T`, `Foldable T` | `traverse : Applicative G -> (A -> G B) -> T A -> G (T B)` |
| `Filterable F` | `Functor F` | `filterMap : (A -> Option B) -> F A -> F B` |
| `Bifunctor F` | — | `bimap : (A -> C) -> (B -> D) -> F A B -> F C D` |

## Builtin executable support

In this section, **executable support** means the current compiler lowers class-member use to dedicated builtin intrinsics in `aivi-core`.
If a carrier is not listed here for a class, that class is **not** runtime-backed for that carrier today, even if parser, HIR, or checker support exists for related syntax.

| Builtin carrier | Functor | Apply | Applicative | Monad | Foldable | Traversable | Filterable | Bifunctor |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `List` | yes | yes | yes | yes | yes | yes | yes | — |
| `Option` | yes | yes | yes | yes | yes | yes | yes | — |
| `Result E` | yes | yes | yes | yes | yes | yes | — | yes |
| `Validation E` | yes | yes | yes | — | yes | yes | — | yes |
| `Signal` | yes | yes | yes | — | — | — | — | — |
| `Task E` | — | — | yes | — | — | — | — | — |

- The `Monad` column means builtin executable lowering for `chain` and `join`.
- `Task E` currently has builtin executable `Applicative` support only. Broader checker-level `Functor`, `Apply`, `Chain`, and `Monad` matching is still not runtime-backed.
- `Signal` is intentionally **not** a `Monad`: executable signals keep a static dependency graph.
- `Validation E` is intentionally **not** a `Monad`: its supported accumulation semantics are applicative rather than dependent short-circuiting.
- There is no builtin executable `Foldable` or `Traversable` support for `Signal` or `Task` in the current slice.

## User-authored higher-kinded classes and instances

### Supported end to end today

- Same-module class declarations, including `with` superclasses and `require` constraints
- Same-module unary `instance` blocks for higher-kinded heads such as `instance Applicative Option`
- Same-module partially applied heads such as `instance Functor (Result Text)`
- Same-module references to those instance members, which lower to hidden callable items

### Not end to end today

- Imported user-authored instances are still deferred
- Imported polymorphic class-member execution is still deferred
- Declaring a new higher-kinded class or instance does **not** create new builtin runtime support for arbitrary carriers

In practice, user-authored higher-kinded classes and instances are trustworthy today for the current same-module checking and lowering slice, but they are not yet a general cross-module executable evidence system.

## Related pages

- [Classes](/guide/classes) for syntax and local examples
- [Pipes & Operators](/guide/pipes) for `*|>` and applicative clustering with `&|>`
- [aivi.prelude](/stdlib/prelude) for the ambient types and class names
