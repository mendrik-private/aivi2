# Type System

AIVI's type system is a strict, closed, purely functional type theory with higher-kinded types and structural type class derivation.

## Kinds

**Source**: `crates/aivi-typing/src/kind.rs`

AIVI has a proper kind system:

| Kind | Description |
|------|-------------|
| `*` | Ground (concrete) kind |
| `* → *` | Unary type constructor (e.g. `List`, `Maybe`) |
| `* → * → *` | Binary type constructor (e.g. `Dict`, `Either`) |

The `KindChecker` validates kind expressions at HIR time. `KindStore` maps `TypeConstructorId` → `Kind`. Kind parameters (`KindParameterId`) support polymorphic kinds.

## Type Expressions

**Source**: `crates/aivi-hir/src/hir.rs`, `crates/aivi-core/src/ty.rs`

AIVI types in HIR:
- Ground types: `Int`, `Float`, `Bool`, `Text`, `Bytes`, `BigInt`
- Named types: resolved by name to a `TypeId`
- Type applications: `List Int`, `Dict Text Bool`
- Type parameters: `A`, `B` — universally quantified in functions and instances
- Function types: `A -> B` (internal only; surface uses domain-annotated functions)
- Domain types: opaque wrappers with declared operators

## Type Checking

**Source**: `crates/aivi-hir/src/typecheck.rs`, `typecheck_context.rs`

Bidirectional type checking:
- **Check mode**: propagate an expected type downward into an expression
- **Infer mode**: synthesise a type for an expression and return it upward

`GateType::unify_type_params()` collects `TypeParameter → concrete` bindings by structural matching, used for polymorphic imported function calls.

Cross-module polymorphic imports use `ImportBindingMetadata::InstanceMember` and `check_expected_apply` / `apply_function` in `typecheck_context.rs`.

## Type Classes

**Source**: `crates/aivi-hir/src/hir.rs` — `ClassItem`, `ClassMember`, `InstanceItem`, `InstanceMember`

- `class Eq A { ... }` — declares a type class with member signatures
- `instance Eq Int { ... }` — provides a concrete implementation
- Constraint syntax: `Eq A => A -> A -> Bool`
- **Constraint separator is `=>` only** — `->` after a constraint is a parse error

### Eq Class

**Source**: `crates/aivi-typing/src/eq.rs`

`EqDeriver` structurally derives `Eq` instances for records, sum types, and domains. The `EqContext` tracks derivation progress; `EqDerivation` is the result.

**Important**: `==` at generic type requires an `Eq` constraint at the definition site. For polymorphic code, pass an explicit `eq` comparator function (see `stdlib/aivi/list.aivi` — `contains` takes an `eq` function parameter).

## Domains

**Source**: `crates/aivi-hir/src/hir.rs` — `DomainItem`, `DomainMember`; `crates/aivi-typing/src/kind.rs`

Domains are opaque newtypes with declared operator methods:

```aivi
domain Duration {
  (+): Duration -> Duration -> Duration
  (<): Duration -> Duration -> Bool
}
```

Domain layouts: in the backend, `is_named_domain_layout()` detects domain types. The `arguments` field in domain layouts is always empty — carrier type info is not stored there.

## Decode Derivation

**Source**: `crates/aivi-typing/src/decode.rs`, `crates/aivi-hir/src/decode_elaboration.rs`

`DecodePlanner` produces a `SourceDecodePlan` — a validated program for decoding external JSON/data into typed AIVI values. Plans are validated structurally; blocking errors are surfaced explicitly.

Supported strategies:
- Record field decode (required / optional)
- Sum variant decode (tag-based or structural)
- Domain decode (via a declared surface candidate)
- Primitive passthrough

## HKT Abstractions

AIVI's stdlib uses higher-kinded type classes for generic data abstractions:

| Class | Carrier kind |
|-------|-------------|
| `Functor` | `* → *` |
| `Applicative` | `* → *` |
| `Monad` | `* → *` |
| `Filterable` | `* → *` |
| `Foldable` | `* → *` |
| `Traversable` | `* → *` |
| `Bifunctor` | `* → * → *` |
| `Append` | `* → *` |

These correspond to the `Builtin*Carrier` types in `crates/aivi-core/src/expr.rs`.

## Constraints

Constraint syntax: `ClassName TypeParam => ReturnType`

- Multiple constraints: `Eq A, Ord A => ...`
- Constraint separator is `=>` (fat arrow). `->` after a constraint name is a parse error.
- Constraints are checked at definition site.

*See also: [compiler-pipeline.md](compiler-pipeline.md), [signal-model.md](signal-model.md)*
