# Equality Semantics

AIVI v1 has **compiler-derived structural `Eq`** for many concrete closed types. This is broader than some user-facing docs imply.

## Concrete closed types

**Sources**: `crates/aivi-hir/src/typecheck/checker.rs`, `crates/aivi-typing/src/eq.rs`

For direct `==` / `!=` on a concrete value, the type checker accepts:

- primitive types except `Bytes`
- tuples whose elements have `Eq`
- closed records whose fields have `Eq`
- closed sum types whose payloads have `Eq`
- domains whose carrier has `Eq`
- `List`, `Option`, `Result`, `Validation` when contained types have `Eq`

This is why code like `Coord 1 2 == Coord 1 2` checks successfully for a user-defined closed sum such as:

```aivi
type Coord = Coord Int Int
```

The implementation lives in `require_eq_with_scope()` and `require_compiler_derived_eq_with_scope()` in `crates/aivi-hir/src/typecheck/checker.rs:3340-3495`. The lower-level structural deriver in `crates/aivi-typing/src/eq.rs:523-731` mirrors this model and explicitly rejects open records, open sums, recursive derivations, and `Bytes`.

## Why demos still define `coordEq` / `cellEq`

**Sources**: `demos/reversi.aivi`, `demos/snake.aivi`, `stdlib/aivi/list.aivi`, `stdlib/aivi/prelude.aivi`

Some stdlib helpers are comparator-passing APIs rather than class-constrained APIs. Example:

- `stdlib/aivi/list.aivi:401-403` defines `contains : (A -> A -> Bool) -> A -> List A -> Bool`
- `stdlib/aivi/prelude.aivi:249-251` re-exports that shape as ambient `contains`

So demos wrap `==` into a named comparator:

```aivi
type Cell -> Cell -> Bool
func cellEq = a b =>
    a == b
```

That helper exists so code can **pass equality as first-class function value** into `listContains` / `contains`. It is not proof that `Cell` or `Coord` lack structural equality.

In a few places, demos also call `coordEq cell target` where plain `cell == target` would also work. That is best read as style or reuse, not a language restriction.

## Where `Eq` still needs help

**Sources**: `manual/guide/classes.md`, `crates/aivi-hir/src/typecheck/checker.rs`

For **open type parameters**, `==` still needs explicit evidence:

```aivi
type Eq K => K -> K -> Bool
func matchesKey = key candidate =>
    key == candidate
```

Without that constraint, the checker reports that the open type parameter has no compiler-derived `Eq` instance. This is separate from concrete-type structural equality.

## Ordering is a different story

`Eq` support for a concrete closed type does **not** by itself imply that ordinary `<`, `>`, `<=`,
or `>=` will work for that type.

Those operators lower through `Ord.compare`, not through structural `Eq` and not through domain body
members named `(<)` or `(>)`. The ambient prelude defines the ordering operators in terms of
`compare`, and the checker requires the shared operand type to satisfy `Ord`.

Practical consequences today:

- Imported `Date` values support `==` because `Date` is a concrete closed constructor-backed type and
  compiler-derived `Eq` accepts that shape.
- Imported `Date` values now also support infix ordering because `aivi.date` ships explicit `Eq` /
  `Ord` instances and first-order imported instance evidence is accepted during typechecking.
- Turning `Date` into a domain would still not be the key requirement for infix ordering. Ordinary
  `<` goes through `Ord.compare` either way.
- The existing `Duration` / `DateDelta` domain docs still overstate domain-operator support a bit:
  authored domain members such as `type (<) : Duration -> Duration -> Bool` are not sufficient for
  ordinary infix `<` under the current checker/lowering path without a matching `Ord` instance.

## Current exclusions

`Eq` is not compiler-derived in v1 for:

- `Bytes`
- functions / arrows
- `Signal`
- `Task`
- `Map`
- `Set`

Imported opaque types are accepted optimistically at use sites; their defining module is expected to validate equality there.

## Sources

- `crates/aivi-hir/src/typecheck/checker.rs`
- `crates/aivi-hir/src/lower/ambient.rs`
- `crates/aivi-hir/src/general_expr_elaboration.rs`
- `crates/aivi-cli/tests/check.rs`
- `stdlib/aivi/date.aivi`
- `stdlib/aivi/duration.aivi`
- `manual/aivi-snippet-todo.json`
