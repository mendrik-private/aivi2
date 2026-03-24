# AIVI Language Specification

## Draft v0.5 — implementation-facing

> Status: normative working draft. Milestones 1–8 (surface through backend) are substantially complete. Sections §26–§28 cover the CLI, LSP, and pre-stdlib implementation gaps.

---

## 1. Vision

AIVI is a purely functional, reactive, GTK/libadwaita-first programming language for building native Linux desktop applications.

Its defining shape is:

- pure user code by default
- strict closed types
- no `null` / `undefined`
- no `if` / `else`
- no loops in the surface language
- expression-first control flow
- pipe algebra as a first-class surface
- first-class signals and source-backed reactivity
- higher-kinded abstractions in the core
- native compilation through Rust and Cranelift
- a runtime that integrates scheduler, signal propagation, GC, sources, and GTK

AIVI is not a thin syntax layer over Rust or GTK. It is a coherent language with a pure semantic core and an explicit runtime boundary.

---

## 2. Design goals and non-goals

### 2.1 Primary goals

- Make GTK4/libadwaita application development on GNOME Linux the flagship use case.
- Preserve a pure, explicit, analyzable user model.
- Make reactivity part of the language, not an afterthought library.
- Compile to native code.
- Keep correctness legible through closed types, explicit boundaries, and strong diagnostics.

### 2.2 Non-goals for v1

The initial implementation does **not** optimize for:

- unrestricted systems programming
- implicit mutation-oriented UI models
- open-world structural typing
- type-level metaprogramming beyond narrow HKT support
- general-purpose dynamic graph monads for signals

These are non-goals for v1 because they weaken the main design: a pure, typed, reactive language for native desktop software.

---

## 3. Implementation invariants

This section is normative for the implementation architecture.

### 3.1 Semantic invariants

- Ordinary user functions are pure.
- `Signal` values denote time-varying values whose dependencies are known after elaboration.
- `Task E A` denotes a one-shot effectful computation description; it is not an immediate effect.
- Closed records reject undeclared fields.
- Closed sum types have a finite known constructor set.
- Pattern matching on sums is exhaustiveness-checked.

### 3.2 Ownership invariants

- Ordinary AIVI values are runtime-managed and may move.
- Stable addresses are not guaranteed for ordinary values.
- Stable foreign-facing identity is provided through runtime handles, pinned wrappers, or copied boundary values.
- GTK widgets and foreign runtime objects are never exposed as ordinary moving AIVI values.

### 3.3 Threading invariants

- GTK widget creation, mutation, and event dispatch are confined to the GTK main thread.
- Workers never mutate UI-owned state directly.
- Cross-thread communication is message-based and immutable from the user model.
- Scheduler ticks are single-owner operations from the runtime's point of view.

### 3.4 Stack-safety invariants

- No implementation pass may rely on unbounded Rust recursion over user-controlled depth.
- Tail recursion in lowered runtime code must be compiled in a stack-safe form.
- Signal propagation, pattern compilation, decode walking, and tree traversals must use explicit worklists or bounded recursion strategies where input depth is unbounded.
- The implementation must include deep-input torture tests.

### 3.5 IR invariants

Each IR boundary must define:

- node ownership model
- identity strategy
- source span strategy
- validation rules and entry points
- pretty-print/debug output
- losslessness expectations when the layer claims source fidelity

### 3.6 Error-reporting invariants

- Diagnostics are attached to source spans and preserve the user's surface constructs where possible.
- Desugaring must not erase the ability to point at the original cause.
- Ambiguity is surfaced explicitly rather than guessed silently.

---

## 4. Compiler pipeline

The implementation pipeline is:

1. **Lexer / parser**
2. **CST**
3. **HIR**
4. **Typed core**
5. **Closed typed lambda IR**
6. **Backend IR**
7. **Cranelift code generation**
8. **Runtime integration**

The repository keeps the implementation-facing companion contract for these layers in
`docs/ir-boundary-contracts.md`. The RFC freezes the minimum semantics each boundary must
preserve.

### 4.1 CST

The CST is source-oriented and lossless enough for formatting and diagnostics.

Boundary contract:

- ownership: `aivi_syntax::ParsedModule` owns both the lossless token buffer and the structural
  CST module
- identity: top-level items are source-addressed by `TokenRange` into the token buffer rather than
  synthetic arena ids; nested nodes are structural within their parent item
- source spans: user-addressable CST nodes carry `SourceSpan`; top-level items additionally retain
  `TokenRange` so tooling can map back into trivia-preserving source
- validation entry points: `aivi_syntax::lex_module` establishes token/trivia invariants and
  `aivi_syntax::parse_module` establishes CST shape plus recoverable syntax diagnostics
- losslessness: comments, whitespace, and other trivia remain in the token buffer even when the
  structured tree does not lower them into dedicated CST nodes

### 4.2 HIR

HIR is the first module-owned arena IR.

Boundary contract:

- ownership: one `aivi_hir::Module` owns arenas for items, expressions, patterns, decorators,
  bindings, markup nodes, control nodes, and type nodes
- identity: opaque arena ids such as `ItemId`, `ExprId`, `PatternId`, `DecoratorId`,
  `MarkupNodeId`, and `ControlNodeId`
- source spans: every user-facing name, item header, expression, pattern, markup node, and control
  node carries the source span that diagnostics must report
- validation entry points: `aivi_hir::lower_module` / `lower_module_with_resolver`,
  `aivi_hir::validate_module`, and `aivi_hir::typecheck_module`

HIR responsibilities:

- names resolved
- imports resolved
- decorators attached
- markup nodes represented explicitly
- pipe clusters represented explicitly
- surface sugar preserved where useful for diagnostics
- source metadata and source-lifecycle/decode/fanout/recurrence elaboration reports made explicit
- body-less annotated `sig` declarations preserved as first-class input signals rather than erased

### 4.3 Typed core

Typed core is the first post-HIR layer that owns fully typed runtime-facing nodes rather than
resolved surface syntax.

Boundary contract:

- ownership: one `aivi_core::Module` owns typed arenas for items, expressions, pipes, stages,
  sources, and decode programs
- identity: opaque ids such as `ItemId`, `ExprId`, `PipeId`, `StageId`, `SourceId`,
  `DecodeProgramId`, and `DecodeStepId`
- source spans: expressions, patterns, stages, items, source nodes, and decode nodes preserve
  source spans; origin handles back into HIR stay attached where later layers need them
- validation entry points: `aivi_core::lower_module`, `aivi_core::lower_runtime_module`, and
  `aivi_core::validate_module`

Typed core responsibilities:

- all names resolved
- kinds checked
- class constraints attached
- `&|>` normalized into applicative spines
- pattern matching normalized
- record default elision elaborated
- markup control nodes typed
- signal dependency graph extracted
- blocked or not-yet-proven ordinary expression slices kept explicit rather than guessed into core

### 4.4 Closed typed lambda IR

The typed lambda layer keeps closure structure explicit without collapsing directly into backend
layout or ABI choices.

Boundary contract:

- ownership: one `aivi_lambda::Module` owns closure and capture arenas while embedding the
  validated typed-core module it wraps
- identity: explicit `ClosureId` and `CaptureId` plus carried-through core ids for items, pipes,
  stages, sources, and decode programs
- source spans: closure, item, pipe, and stage nodes preserve source spans from typed core / HIR
- validation entry points: `aivi_lambda::lower_module` and `aivi_lambda::validate_module`

Responsibilities:

- explicit closures
- explicit environments
- explicit runtime nodes for sources/tasks/signals where needed
- dictionary passing or monomorphization decisions applied
- no remaining surface sugar

### 4.5 Backend IR and codegen

Backend IR is the first layer that owns ABI/layout/runtime call contracts outright.

Boundary contract:

- ownership: one backend `Program` owns items, pipelines, kernels, layouts, sources, and decode
  plans
- identity: backend-owned ids such as `PipelineId`, `KernelId`, `KernelExprId`, `LayoutId`,
  `SourceId`, `DecodePlanId`, `DecodeStepId`, `EnvSlotId`, and `InlineSubjectId`, plus origin
  links back into earlier IRs
- source spans: item, pipeline, stage, source, and kernel origins preserve source spans; backend
  expressions keep source spans for diagnostics and debug dumps
- validation entry points: `aivi_backend::lower_module`, `aivi_backend::validate_program`, and
  `aivi_backend::compile_program`

Responsibilities:

- layout decisions
- concrete calling conventions
- Cranelift lowering
- AOT and JIT support

---

## 5. Top-level forms

Canonical top-level declarations:

```aivi
type Bool = True | False

class Eq A
    (==) : A -> A -> Bool

val answer = 42

fun add:Int #x:Int #y:Int =>
    x + y

sig counter = 0

use aivi.network (
    http
    socket
)
```

The core top-level forms are:

- `type`
- `class`
- `instance`
- `val`
- `fun`
- `sig`
- `use`
- `export`
- decorators via `@name`

A module may export exactly one process entry point named `main`.

Comment syntax in v1:

- `//` starts a line comment and runs to end of line
- `/* ... */` is a block comment (may span multiple lines)
- `/** ... **/` is a doc comment (may span multiple lines)
- all three forms are trivia in the lossless token stream; they do not create ordinary expression or
  item nodes in the CST
- the lexical distinction between `//`, `/* */`, and `/** **/` is stable; declaration attachment
  and doc extraction remain tooling-owned work above the syntax layer

### 5.1 Import rules

Name lookup is intentionally simple in v1:

- local names and a small explicit import set work
- no wildcard imports in v1
- no module-qualified call syntax such as `List.map` in v1
- built-in names keep priority where needed

#### Import aliases

`use module (member as localName)` is the disambiguation escape hatch when two imports would otherwise provide the same local name:

```aivi
use aivi.network (http)
use my.client (fetch as clientFetch)
```

The original member name still drives compiler-known metadata. The alias changes only the local binding name.

#### Name resolution for terms

The compiler generally prefers unqualified term use and resolves the right binding from local name plus already-known context. When several candidates remain after contextual filtering, the compiler reports an `ambiguous-domain-member` or similar diagnostic and requires explicit disambiguation through an import alias.

---

## 6. Type system

## 6.1 Kinds

AIVI includes a small explicit kind system.

Base kind:

- `Type`

Constructor kinds:

- `Type -> Type`
- `Type -> Type -> Type`
- right-associative arrow kinds

Examples:

- `Int : Type`
- `Text : Type`
- `Option : Type -> Type`
- `Signal : Type -> Type`
- `Result : Type -> Type -> Type`
- `Task : Type -> Type -> Type`

Partial application of named type constructors is supported.

Valid examples:

- `Option`
- `List`
- `Signal`
- `Result HttpError`
- `Task FsError`

Invalid examples:

- passing `Result` where a unary constructor is required
- passing `List Int` where a constructor is required

Full type-level lambdas are deferred.

## 6.2 Core primitive and standard types

Minimum practical v1 set:

- `Int`
- `Float`
- `Decimal`
- `BigInt`
- `Bool`
- `Text`
- `Unit`
- `Bytes`
- `List A`
- `Map K V`
- `Set A`
- `Option A`
- `Result E A`
- `Validation E A`
- `Signal A`
- `Task E A`

### 6.2.1 Numeric literal surface in v1

The implemented v1 literal surface is intentionally narrower than the full set of numeric types
listed above.

Accepted surface forms:

- unsuffixed integer literals are ASCII decimal digits only: `0`, `42`, `9000`
- built-in float literals are ASCII decimal digits, one `.`, and ASCII decimal digits:
  `0.5`, `3.14`
- built-in decimal literals are ASCII decimal digits with a trailing `d`, optionally with one
  fractional `.<digits>` part before the suffix: `19d`, `19.25d`
- built-in BigInt literals are ASCII decimal digits with a trailing `n`: `123n`
- a compact `digits + identifier` form is parsed as a domain literal suffix candidate when it does
  not match one of the built-in non-`Int` literal forms: `250ms`, `0xFF`
- spacing is semantic: `250ms` is one suffixed literal candidate, while `250 ms` is ordinary
  application
- leading zeroes do not introduce octal or any other alternate base; `007` is decimal
- exact one-letter `d` / `n` compact suffixes are reserved for the built-in `Decimal` / `BigInt`
  literal families; longer suffix spellings remain in the domain-suffix surface

Not part of the v1 literal grammar:

- sign-prefixed numeric literals
- `_` separators inside numeric tokens
- built-in hex, binary, or octal integer forms
- exponent notation

A compact suffix form is only well-typed when exactly one domain literal suffix in scope claims
that suffix name and accepts the base integer family. Otherwise it is rejected during later
validation as an unresolved or ambiguous suffix literal.

### 6.2.2 Executable numeric literal slice

The current executable backend/runtime slice intentionally stops short of a general numeric tower.

- `Int` literals execute as by-value `i64`.
- `Float` literals execute as finite IEEE-754 `f64` values and keep the backend's native by-value
  scalar ABI.
- `Decimal` literals execute as exact decimal runtime values, but backend layout marks them
  by-reference and Cranelift materializes them only as immutable literal cells with
  `mantissa:i128 (little-endian) + scale:u32 (little-endian)`.
- `BigInt` literals execute as exact arbitrary-precision integer runtime values, but backend layout
  marks them by-reference and Cranelift materializes them only as immutable literal cells with
  `sign:u8 + 7 bytes padding + byte_len:u64 (little-endian) + magnitude bytes (little-endian)`.
- `Decimal` and `BigInt` literal cells are introduction-only in the current Cranelift slice. This
  is an explicit layout/runtime boundary, not an implicit promise of full decimal/bignum arithmetic
  in backend codegen yet.
- Non-`Int` arithmetic and ordered comparison remain deferred in the executable backend slice even
  though the parser, HIR, and literal execution path recognize these builtin literal families.
- Diagnostics must preserve the user's raw numeric spelling for all literal families.

## 6.3 Closed types

Closed types mean:

- no `null` inhabitants unless represented explicitly in an ADT
- records are closed by default
- sums are closed by default
- missing or extra decoded fields are errors by default
- exhaustiveness checking is available for closed sums

## 6.4 Product types and data constructors

Constructor-headed product declarations are the default product form.

```aivi
type Vec2 = Vec2 Int Int
type Date = Date Year Month Day
```

### 6.4.1 Term-level constructor semantics

Every non-record ADT constructor is an ordinary curried value constructor.

```aivi
type Result E A = Err E | Ok A

val ok  = Ok
val one = Ok 1
```

Under-application is legal. Exact application constructs the value. Over-application is a type error.

This applies to both unary and multi-argument constructors.

### 6.4.2 Record construction

Records are built with record literals, not implicit curried record constructors.

```aivi
type User = { name: Text, age: Int }

val u:User = { name: "Ada", age: 36 }
```

### 6.4.3 Opaque and branded types

Opaque or branded types are recommended for domain-safe wrappers such as `Year`, `Month`, `Path`, `Url`, `Color`, and `Duration`. Public unary constructors are appropriate only when constructor application is intentionally part of the surface API.

## 6.5 Sum types

Canonical sum syntax:

```aivi
type Bool = True | False

type Option A =
  | None
  | Some A
```

Nested constructor patterns are allowed. Exhaustiveness is required for sum matches unless a wildcard is present.

## 6.6 Records, tuples, and lists

Value forms:

```aivi
(1, 2)
{ name: "Ada", age: 36 }
[1, 2, 3]
```

- tuples are positional products
- records are named products
- lists are homogeneous sequences

## 6.7 Maps and sets

Collection literal forms:

```aivi
Map { "x": 1, "y": 2 }
Set [1, 2, 4]
```

Rules:

- plain `{ ... }` is always a record
- plain `[ ... ]` is always a list
- duplicate record fields are a compile-time error
- duplicate map keys are a compile-time error
- duplicate set entries are allowed but may be warned and deduplicated

---

## 7. Core abstraction model

AIVI includes a small class/instance abstraction mechanism, lowered by dictionary passing or intrinsics.

Conceptually:

```aivi
class Functor F
    map : (A -> B) -> F A -> F B

class Functor F => Applicative F
    pure  : A -> F A
    apply : F (A -> B) -> F A -> F B

class Applicative F => Monad F
    bind : F A -> (A -> F B) -> F B
```

### 7.1 Resolution rules

- instance resolution is coherent
- overlapping instances are not allowed in v1
- orphan instances are **fully disallowed** in v1 to keep behavior consistent and easy to reason about
- instance search is compile-time only

### 7.2 Core instances

Recommended v1 instances:

- `Option` implements `Functor`, `Applicative`, `Monad`
- `Result E` implements `Functor`, `Applicative`, `Monad`
- `List` implements `Functor`, `Applicative`, `Monad`
- `Task E` implements `Functor`, `Applicative`, `Monad`
- `Signal` implements `Functor`, `Applicative`
- `Validation E` implements `Functor`, `Applicative`
- `Eq` is compiler-provided for the structural cases in §7.3

### 7.2.1 `Foldable.reduce`

`Foldable.reduce` is the current compiler-provided reduction surface for builtin collection/error
carriers:

- `List A` folds left-to-right in source order
- `Option A` folds zero or one payloads: `None` returns the seed unchanged, `Some x` applies the
  step once
- `Result E A` folds over the success payload only: `Err _` returns the seed unchanged, `Ok x`
  applies the step once
- `Validation E A` folds over the valid payload only: `Invalid _` returns the seed unchanged,
  `Valid x` applies the step once

This surface is intentionally narrow: it preserves the applicative meaning of `Validation` and
does not imply any `Foldable Task` or `Foldable Signal` instance in v1.

### 7.3 Equality

AIVI includes a first-order equality class:

```aivi
class Eq A
    (==) : A -> A -> Bool
```

`Eq` uses the ordinary class/instance resolution rules in §7.1. In the initial implementation, the compiler provides the required `Eq` instances; user-authored `Eq` instances are deferred until the wider instance system is implemented end to end.

Compiler-derived `Eq` instances are required for:

- primitive scalars: `Int`, `Float`, `Decimal`, `BigInt`, `Bool`, `Text`, `Unit`
- tuples whose element types are `Eq`
- closed records whose field types are `Eq`
- closed sums whose constructor payload types are all `Eq`
- `List A` and `Option A` when `A` is `Eq`
- `Result E A` and `Validation E A` when both `E` and `A` are `Eq`

Constructor-headed product declarations such as `type Vec2 = Vec2 Int Int` participate through the closed-sum rule.

Derived equality is structural and type-directed:

- tuple equality is position-by-position
- record equality is fieldwise over the declared closed field set
- sum equality compares constructor tags first, then constructor payloads
- list equality is length- and order-sensitive
- primitive scalar equality is same-type only; it is not coercive or approximate

`Eq` is not compiler-derived in v1 for `Bytes`, `Map`, `Set`, `Signal`, `Task`, function values, GTK/foreign handles, or other runtime-managed boundary types whose equality semantics have not yet been specified.

### 7.4 Non-instances

`Signal` is **not** a `Monad` in v1.

Rationale:

- monadic signals tend to imply dynamic dependency rewiring
- that complicates graph extraction, scheduling, teardown, and diagnostics
- AIVI wants a static, explicit, topologically scheduled signal graph

`Validation E` is **not** a `Monad` in v1 because the intended accumulation semantics are applicative rather than dependent short-circuiting.

### 7.5 Laws

The standard semantic laws are normative for lawful instances:

- `Eq`: reflexivity, symmetry, transitivity
- `Functor`: identity, composition
- `Applicative`: identity, homomorphism, interchange, composition
- `Monad`: left identity, right identity, associativity

The compiler is not required to prove these laws.

---

## 8. Validation

`Validation E A` is a standard-library ADT for independent error accumulation.

```aivi
type Validation E A =
  | Invalid (NonEmptyList E)
  | Valid A
```

Unlike `Result E A`, the applicative instance for `Validation E` accumulates independent errors instead of short-circuiting on the first failure.

### 8.1 Applicative semantics

For `Validation E`, applicative combination behaves as follows:

- `pure x` yields `Valid x`
- applying `Valid f` to `Valid x` yields `Valid (f x)`
- applying `Invalid e` to `Valid _` yields `Invalid e`
- applying `Valid _` to `Invalid e` yields `Invalid e`
- applying `Invalid e1` to `Invalid e2` yields `Invalid (e1 ++ e2)`

Here `++` is concatenation of the underlying `NonEmptyList E`.

### 8.2 Intent

`Validation` is the canonical carrier for form validation under `&|>` because the inputs are independent and all failures should be reported together.

Example:

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

If all validators succeed, the result is `Valid (UserDraft ...)`.
If one or more validators fail, all reported errors are accumulated into one `Invalid` value in source order.

`Validation E` is intentionally applicative-only in v1. Dependent validation that requires earlier successful values to choose later checks should use `Result`, `Task`, or explicit pattern matching instead.

---

## 9. Defaults and record omission

Defaulting is explicit and scoped. It does not make records open.

### 9.1 Default class

AIVI includes a small defaulting class:

```aivi
class Default A
    default : A
```

### 9.2 `aivi.defaults`

The module `aivi.defaults` exports instance bundles. The required first bundle is:

```aivi
use aivi.defaults (Option)
```

which brings the following idea into scope:

```aivi
Default (Option A)
default = None
```

### 9.3 Record literal elision

When an expected closed record type is known, omitted fields are filled only if a `Default` instance is in scope for every omitted field type.

Example:

```aivi
type User = {
    name: Text,
    nickname: Option Text,
    email: Option Text
}

use aivi.defaults (Option)

val user:User = { name: "Ada" }
```

This elaborates to:

```aivi
val user:User = {
    name: "Ada",
    nickname: None,
    email: None
}
```

### 9.4 Record shorthand

When an expected closed record type is known, a field whose label and in-scope value name coincide may be written in shorthand form.

Example:

```aivi
val snake = initialSnake
val food = initialFood
val status = Running
val score = 0

val game:Game = {
    snake,
    food,
    status,
    score
}
```

This elaborates to:

```aivi
val game:Game = {
    snake: snake,
    food: food,
    status: status,
    score: score
}
```

The same shorthand is allowed in record patterns.

```aivi
game
 ||> { snake, food, status, score } => score
```

This elaborates to:

```aivi
game
 ||> { snake: snake, food: food, status: status, score: score } => score
```

Shorthand is legal only when:

- the expected record type is known
- the field name exists on that closed record type
- a local binding of the same name is in scope for record construction
- the shorthand is unambiguous in patterns

Shorthand does not introduce open records, punning across different field names, or implicit defaults.

### 9.5 Restrictions

Omission is legal only when:

- the expected record type is known
- each omitted field has a `Default` instance in scope

This feature does **not**:

- open records
- change pattern matching semantics
- weaken strict source decoding
- add runtime fallback guessing

---

## 10. Expression model and control flow

AIVI is expression-first.

### 10.1 No `if` / `else`

AIVI does not use `if` / `else`. Branching uses pattern matching or predicate-gated flow.

### 10.2 No loops

The surface language has no imperative loop constructs. Repetition is expressed through:

- recursion
- collection combinators
- source/retry/interval flows
- controlled recurrent pipe forms

### 10.3 Ambient subject

Within a pipe, there is a current ambient subject.

- `_` means the entire current subject
- `.field` projects from the current subject
- `.field.subfield` chains projection
- `.field` is illegal where no ambient subject exists

---

## 11. Pipe algebra

Pipe algebra is one of AIVI's defining surface features.

## 11.1 Operators

Core v1 operators:

- ` |>` transform
- `?|>` gate
- `||>` case split
- `*|>` map / fan-out
- `&|>` applicative cluster stage
- `@|>` recurrent flow start
- `<|@` recurrence step
- ` | ` tap
- `<|*` fan-out join

Ordinary expression precedence, from tighter to looser binding:

1. function application
2. binary `+` and `-`
3. binary `>`, `<`, `==`, `!=`
4. `and`
5. `or`

Operators at the same binary precedence associate left-to-right.

The current surface subset also supports prefix `not`; it applies to its following ordinary
expression before binary reassociation.

Pipe operators are **not** part of that binary table. A pipe spine starts from one ordinary
expression head, then consumes pipe stages left-to-right. Each stage payload is parsed as an
ordinary expression using the table above until the next pipe operator boundary.

Reactivity does **not** come from pipe operators. Reactivity comes from `sig` and `@source`. Pipe operators are flow combinators inside those reactive or ordinary expressions.

### 11.2 `|>` transform

Transforms the current subject into a new subject.

```aivi
order |> .status
```

### 11.3 `?|>` gate

Allows the current subject through only if the predicate holds.

```aivi
users ?|> .active
```

The gate body is typed against the current ambient subject and must produce `Bool`.

Signal semantics:

- for `Signal A`, updates whose predicate is `True` are forwarded
- updates whose predicate is `False` are suppressed
- the result type remains `Signal A`
- no synthetic negative update is emitted

Ordinary-value semantics:

- for an ordinary subject `A`, `?|>` lowers to `Option A`
- success yields `Some subject`
- failure yields `None`

Example:

```aivi
user
 ?|> .active
 T|> .email
 F|> "inactive"
```

This is the canonical expression-level replacement for keeping or dropping a value without introducing `if` / `else`.

Restrictions:

- the predicate must be pure
- `?|>` is not a general branch operator; use `||>` when the two paths compute unrelated shapes
- `?|>` does not inspect prior history or future updates; it is pointwise over the current subject

### 11.4 `||>` case split

Performs pattern matching over the current subject.

```aivi
status
 ||> Paid    => "paid"
 ||> Pending => "pending"
```

### 11.4.1 `T|>` and `F|>` truthy / falsy branching

`T|>` and `F|>` are shorthand predicate-gated branch operators for carriers with canonical positive and negative constructors.

They are surface sugar over `||>` and elaborate deterministically.

Boolean example:

```aivi
ready
 T|> start
 F|> wait
```

elaborates to:

```aivi
ready
 ||> True  => start
 ||> False => wait
```

`Option` example:

```aivi
maybeUser
 T|> greet _
 F|> showLogin
```

elaborates to:

```aivi
maybeUser
 ||> Some a => greet a
 ||> None   => showLogin
```

`Result` example:

```aivi
loaded
 T|> render _
 F|> showError _
```

elaborates to:

```aivi
loaded
 ||> Ok a  => render a
 ||> Err e => showError e
```

The canonical truthy / falsy constructor pairs in v1 are:

- `True` / `False`
- `Some _` / `None`
- `Ok _` / `Err _`
- `Valid _` / `Invalid _`

Rules:

- `T|>` and `F|>` may appear only as an adjacent pair within one pipe spine
- the subject type must have a known canonical truthy / falsy pair
- inside a `T|>` or `F|>` body, `_` is rebound to the matched payload when that constructor has exactly one payload
- zero-payload cases such as `True`, `False`, and `None` do not introduce a branch payload
- use `||>` when named binding, nested patterns, or more than two constructors are required
- user-defined truthy / falsy overloads are not supported in v1

### 11.5 `*|>` map / fan-out

Maps over each element of a collection.

```aivi
users
 *|> .email
```

Each element becomes the ambient subject within the fan-out body.

Typing and lowering rules:

- for `List A`, `*|>` maps `A -> B` to produce `List B`
- for `Signal (List A)`, fan-out is lifted pointwise to produce `Signal (List B)`
- the body is typed as if it were a normal pipe body with the element as ambient subject
- the outer collection is not implicitly ambient inside the body; capture it by name if needed

`*|>` is pure mapping only. It does not implicitly flatten nested collections, sequence `Task`s, or merge nested `Signal`s.

### 11.5.1 `<|*` fan-out join

Joins the collection produced by the immediately preceding `*|>` with an explicit reducer.

```aivi
users
 *|> .email
 <|* Text.join ", "
```

`xs *|> f <|* g` elaborates to `g (map f xs)`.

For `Signal (List A)`, the same rule is lifted pointwise over signal updates.

Restrictions:

- `<|*` is legal only immediately after a `*|>` segment
- the join function is explicit; there is no implicit flattening or collection-specific default join

### 11.6 `|` tap

Observes the subject without changing it.

```aivi
value
 |> compute
 |  debug
 |> finish
```

The tap body is evaluated with the current subject as ambient subject. Its result is ignored. The outgoing subject is exactly the incoming subject.

Conceptually, `x | f` behaves like `let _ = f x in x`.

`|` is intended for tracing, metrics, and named observers. It is not a hidden mutation or control-flow channel.

### 11.7 `@|>` and `<|@`

These mark explicit recurrent flows used for retry, polling, and stream-style pipelines.

`@|>` enters a recurrent region. Each subsequent `<|@` stage contributes to the per-iteration step function over the current loop state.

Conceptually, a recurrent spine denotes a scheduler-owned loop node rather than direct self-recursion. The current iteration value is the ambient subject within the recurrent region.

Normative v1 rules:

- recurrent pipes are legal only where the compiler can lower them to a built-in runtime node for `Task`, `Signal`, or `@source` helpers
- recurrence wakeups must be explicit: timer, backoff, source event, or provider-defined trigger
- each iteration is scheduled and stack-safe; recurrent pipes must not lower to unbounded direct recursion
- cancellation or owner teardown disposes the pending recurrence immediately
- if the compiler cannot determine a valid runtime lowering target, the recurrent pipe is rejected

---

## 12. Exact applicative surface semantics for `&|>`

This section is normative.

## 12.1 Intent

`&|>` is the surface operator for **applicative clustering**: combining independent effectful/reactive values under a shared `Applicative` and then applying a pure constructor or function.

It is intended for:

- form validation
- combining independent signals
- assembling values from independent `Option`, `Result`, `Validation`, or `Task` computations

It is **not**:

- monadic sequencing
- short-circuit imperative flow
- ad-hoc tuple syntax

## 12.2 Surface forms

A cluster may start either from an ordinary expression or from a leading cluster stage.

### Expression-headed cluster

```aivi
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

### Leading cluster form

A leading `&|>` is legal at the start of a pipe spine or multiline body.

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

This form is preferred when scanning a validation spine because every independent input aligns at the operator.

## 12.3 Grammar shape

Conceptually:

```text
ApplicativeCluster ::=
    ClusterHead ClusterTail+ Finalizer?
  | LeadingClusterHead ClusterTail+ Finalizer?

ClusterHead        ::= Expr
LeadingClusterHead ::= "&|>" Expr
ClusterTail        ::= "&|>" Expr
Finalizer          ::= " |>" Expr
```

`ApplicativeCluster` is a surface form only. It does not survive into backend-facing IR.

## 12.4 Typing rule

All cluster members must have the same outer applicative constructor `F`.

Examples of legal clusters:

- `Validation FormError A`
- `Signal A`
- `Option A`
- `Result HttpError A`
- `Task FsError A`

All cluster members in one cluster must be of shape `F Ai` for the same `F`.

## 12.5 Desugaring

A finished cluster:

```aivi
 &|> a
 &|> b
 &|> c
  |> f
```

desugars to:

```aivi
pure f
    |> apply a
    |> apply b
    |> apply c
```

which is equivalent to:

```aivi
apply (apply (apply (pure f) a) b) c
```

The leading form:

```aivi
&|> a
&|> b
&|> c
 |> f
```

desugars the same way.

## 12.6 End-of-cluster default

If a cluster reaches pipe end without an explicit finalizer, it finalizes to a tuple constructor of matching arity.

```aivi
&|> a
&|> b
```

desugars to:

```aivi
pure Tuple2
    |> apply a
    |> apply b
```

Implementations may represent these tuple constructors internally; the surface semantics are tuple formation.

## 12.7 Restrictions

Inside an unfinished applicative cluster:

- ambient-subject projections such as `.field` are illegal unless they occur inside a nested expression whose own subject is explicit
- `?|>` and `||>` are illegal until the cluster is finalized
- the finalizer must be a pure function or constructor from the user's perspective

These restrictions keep the operator law-abiding and make elaboration deterministic.

## 12.8 Examples

### Validation

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

### Signals

```aivi
sig fullName =
 &|> firstName
 &|> lastName
  |> joinName
```

### Result

```aivi
val loaded =
 &|> readConfig path
 &|> readSchema schemaPath
  |> buildRuntimeConfig
```

## 12.9 `Signal` interaction

For `Signal`, `&|>` builds a derived signal whose dependencies are the union of the member dependencies. The result observes the latest stable upstream values per scheduler tick.

That is applicative combination, not signal monadic binding.

---

## 13. Signals and scheduler semantics

`sig` introduces a reactive binding.

```aivi
sig x = 3
sig y = x + 5
```

A signal referenced inside a `sig` is read as its current value during evaluation of that `sig`. The enclosing `sig` becomes dependent on every signal referenced in its definition.

### 13.1 Rules

- `sig` is the reactive boundary
- `val` must not depend on signals
- pure helper functions used inside `sig` stay pure
- signal dependency extraction happens after elaboration
- dependency graphs are static after elaboration for ordinary derived signals

### 13.5 Input signals

An annotated body-less `sig` declaration is a first-class input signal — an externally publishable entry point for reactive inputs such as GTK events.

```aivi
sig clicked : Signal Unit
sig query   : Signal Text
```

These are not errors. They define runtime-owned slots that external code (event handlers, tests, FFI) may publish into. Their type annotation is mandatory. They participate in the signal dependency graph exactly like derived signals.

Input signals are the canonical mechanism for routing GTK event payloads into the language-level reactive graph. See §17.2 for event hookup rules.

### 13.2 Applicative meaning of `Signal`

`pure x` creates a constant signal.

`apply : Signal (A -> B) -> Signal A -> Signal B` creates a derived signal node with:

- dependency set equal to the union of the input dependencies
- latest-value semantics
- transactional visibility per scheduler tick
- glitch-free propagation

### 13.3 Scheduler guarantees

The runtime scheduler must provide:

- topological propagation order
- batched delivery per tick
- no mixed-time intermediate observations
- deterministic behavior for a fixed input event order

### 13.4 No `Monad Signal`

AIVI v1 does not expose `bind` for `Signal`.

Any feature that would imply dynamic dependency rewiring must be expressed through explicit source/runtime nodes rather than a general `Monad Signal`.

---

## 14. Sources and decoding

External inputs enter through `@source` on `sig`.

```aivi
@source http.get "/users"
sig users : Signal (Result HttpError (List User))
```

Source arguments and options are ordinary typed expressions. They may use interpolation and may depend on signals whose dependency sets are statically known.

Example:

```aivi
@source http.get "{baseUrl}/users" with {
    headers: authHeaders,
    decode: Strict
}
sig users : Signal (Result HttpError (List User))
```

If an argument or option depends on a signal, the source node becomes dependent on that signal. Changing that dependency reconfigures the source transactionally while keeping the graph shape static.

### 14.1 Source contract

A source is a runtime-owned producer that publishes typed values into the scheduler.

Sources may represent:

- HTTP
- file watching
- file reads
- sockets
- D-Bus
- timers
- process events
- mailboxes/channels

### 14.1.1 Recurrence decorators on non-source declarations

Plain repeating `sig` and `val` bodies prove their wakeup only through explicit `@recur.timer` or `@recur.backoff` decorators. Each takes exactly one positional witness expression.

```aivi
@recur.timer 1000ms
sig polled : Signal Status

@recur.backoff initialDelay
sig retried : Signal (Result FetchError Data)
```

Rules:
- `@recur.timer expr` and `@recur.backoff expr` are the only recurrence decorators for non-`@source` declarations
- neither accepts `with { ... }` options or duplicates
- they are not allowed on `@source` signals (source wakeups use the source contract)

### 14.1.2 Source declaration shape

The general surface form is:

```aivi
@source provider.variant arg1 arg2 with {
    option1: value1,
    option2: value2
}
sig name : Signal T
```

The `with { ... }` option record is optional.

Minimal form:

```aivi
@source timer.every 120
sig tick : Signal Unit
```

Optioned form:

```aivi
@source http.get "/users" with {
    decode: Strict,
    retry: 3x,
    timeout: 5s
}
sig users : Signal (Result HttpError (List User))
```

Rules:

- the provider and variant are resolved statically
- positional arguments are provider-defined and typed
- options are a closed record whose legal fields are provider-defined
- unknown options are a compile-time error
- duplicate options are a compile-time error
- argument and option expressions may be ordinary values or signal-derived expressions with statically known dependencies
- if a reactive argument or option changes, the runtime re-evaluates the source configuration using the latest stable upstream values for that scheduler tick
- if the change affects source identity such as URL, path, channel, or process arguments, the old runtime subscription is disposed and a new one is created

Reactive source configuration does not make sources dynamic in the type-theoretic sense. The provider kind and dependency graph remain statically known; only runtime configuration values change.

### 14.1.3 Recommended v1 source variants

The following provider variants are recommended for v1.

#### HTTP

```aivi
@source http.get "/users"
sig users : Signal (Result HttpError (List User))

@source http.post "/login" with {
    body: creds,
    headers: authHeaders,
    decode: Strict,
    timeout: 5s
}
sig login : Signal (Result HttpError Session)
```

Recommended HTTP options:

- `headers : Map Text Text`
- `query : Map Text Text`
- `body : A`
- `decode : DecodeMode`
- `timeout : Duration`
- `retry : Retry`
- `refreshOn : Signal B`
- `refreshEvery : Duration`
- `activeWhen : Signal Bool`

HTTP source semantics:

- a request source issues one request when subscribed unless the provider defines a different default
- `refreshOn` reissues the request whenever the trigger signal updates
- `refreshEvery` creates scheduler-owned polling using the latest stable source configuration
- `activeWhen` gates startup and refresh; when it becomes `False`, polling is suspended, the
  current request generation becomes inactive, and any later completion from that inactive
  generation must not publish
- when reactive URL, query, header, or body inputs change, the runtime creates a replacement
  request generation using the latest stable values
- if `refreshOn`, `refreshEvery`, or reactive reconfiguration fires while an earlier HTTP request
  is still in flight, the newest request supersedes the older one
- built-in HTTP providers request best-effort cancellation of the superseded request; regardless of
  cancellation success, stale completions from superseded generations are dropped
- v1 does not require a queue of pending HTTP refreshes; request issuance is latest-generation-wins

#### Timer

```aivi
@source timer.every 120
sig tick : Signal Unit

@source timer.after 1000
sig ready : Signal Unit
```

Recommended timer options:

- `immediate : Bool`
- `jitterMs : Int`
- `coalesce : Bool`
- `activeWhen : Signal Bool`

#### File watching and reading

```aivi
@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
sig fileEvents : Signal FsEvent

@source fs.read "/tmp/demo.txt" with {
    decode: Strict,
    reloadOn: fileEvents
}
sig fileText : Signal (Result FsError Text)
```

`fs.watch` publishes file-system events only. It does not implicitly read file contents.
`fs.read` publishes a snapshot of the current file contents and may be retriggered explicitly.

Recommended file-watch options:

- `events : List FsWatchEvent`
- `recursive : Bool`

Recommended file-read options:

- `decode : DecodeMode`
- `reloadOn : Signal A`
- `debounce : Duration`
- `readOnStart : Bool`

This split keeps change detection and snapshot loading explicit. A common pattern is to watch a path, debounce the resulting events, and use those events to trigger `fs.read`.

#### Socket / channel / mailbox

```aivi
@source socket.connect "ws://localhost:8080" with {
    decode: Strict
}
sig inbox : Signal (Result SocketError Message)

@source mailbox.subscribe "jobs"
sig jobs : Signal Job
```

Recommended socket and mailbox options:

- `decode : DecodeMode`
- `buffer : Int`
- `reconnect : Bool`
- `heartbeat : Duration`
- `activeWhen : Signal Bool`

#### Process events

```aivi
@source process.spawn "rg" ["TODO", "."]
sig grepEvents : Signal ProcessEvent
```

Recommended process options:

- `cwd : Path`
- `env : Map Text Text`
- `stdout : StreamMode`
- `stderr : StreamMode`
- `restartOn : Signal A`

#### GTK / window events

```aivi
@source window.keyDown with {
    repeat: False
}
sig keyDown : Signal Key
```

Recommended window-event options:

- `capture : Bool`
- `repeat : Bool`
- `focusOnly : Bool`

### 14.1.4 Decode and delivery modes

Recommended supporting enums:

```aivi
type DecodeMode =
  | Strict
  | Permissive

type StreamMode =
  | Ignore
  | Lines
  | Bytes
```

Semantics:

- `Strict` rejects unknown or missing required fields according to closed-type decoding rules
- `Permissive` may ignore extra fields but still requires required fields unless a decoder override says otherwise
- decode happens before scheduler publication
- delivery into the scheduler remains typed and transactional

### 14.2 Decoding

AIVI includes compiler-generated structural decoding by default.

Default decoding rules:

- closed records reject missing required fields
- extra fields are rejected in strict mode by default
- sum decoding is explicit
- decoder override remains possible where necessary
- domain-backed fields decode through the domain's explicit parser or constructor surface; they do not silently accept the raw carrier unless that surface says so

Record default elision for user-written literals does **not** weaken source decoding by default.

Decode failures are reported through the source's typed error channel. They do not escape as untyped runtime exceptions.

### 14.3 Cancellation and lifecycle

Source subscriptions must carry explicit runtime cancellation and disposal semantics. Every
`@source` site owns one stable runtime instance identity. When the owning graph or view is torn
down, that instance is disposed.

Additional lifecycle rules:

- reconfiguration caused by reactive source arguments or options replaces the superseded runtime
  resource before the replacement may publish
- stale work from a superseded, disposed, or inactive source generation is dropped and must never
  publish into the live graph
- built-in `activeWhen` gates suspend delivery without changing the static graph shape; while the
  gate is `False`, new trigger work is not started for that inactive generation
- request-like built-ins such as HTTP and `fs.read` additionally request best-effort in-flight
  cancellation when they are replaced, suspended, or disposed
- custom providers inherit the generic replacement and stale-publication rules, but option names
  such as `activeWhen` or `refreshOn` have built-in meaning only where the provider contract
  explicitly defines that wakeup surface

### 14.4 Custom provider declarations

Custom source providers are declared at the top level with a `provider` keyword:

```aivi
provider my.data.source
    wakeup: timer
    argument url: Url
    option timeout: Duration
    option retries: Int
```

Declaration rules:

- the provider name must be fully qualified with a `.`-separated path
- `wakeup` is mandatory and must be one of: `timer`, `backoff`, `sourceEvent`, `providerTrigger`
- `argument` members declare positional source arguments in order
- `option` members declare named source options
- argument and option types must be from the current closed proof surface: primitive types, same-module named types and domains, and those shapes under `List` or `Signal`
- richer types such as records, arrows, imported constructors, or `Option`/`Result` in provider schemas are rejected on the declaration
- duplicate declarations for the same qualified name are an error
- unqualified provider names are not allowed

Provider declarations match against `@source` use sites in the same module through same-module order-independent lookup. Custom providers reuse the standard reactive-reconfiguration and stale-publication model; additional `activeWhen` or trigger semantics require explicit `wakeup` metadata.

---

## 15. Effects and `Task`

## 15.1 Purity boundary

Ordinary `val` and `fun` definitions are pure.

Effects enter through:

- `Task`
- `sig` / `@source`
- GTK event boundaries
- runtime-owned scheduling and source integration

## 15.2 `Task E A`

`Task E A` is the only user-visible one-shot effect carrier.

`Task`:

- describes a one-shot effectful computation
- may fail with `E`
- may succeed with `A`
- is schedulable by the runtime
- is lawful as `Functor`, `Applicative`, and `Monad`

## 15.3 Event handler routing

The implemented GTK event surface is intentionally narrower than a future general callback
language.

In v1 live GTK routing:

- markup `on*={handler}` attributes are routing declarations, not arbitrary callback bodies
- `handler` must resolve to a directly publishable input signal declared as a body-less annotated
  `sig name : Signal T`
- the concrete GTK host must recognize the exact widget/event pair before the attribute is treated
  as live event routing
- the routed input signal payload type must match the concrete GTK event payload type
- discrete GTK events publish one payload into the scheduler input signal and force their own
  runtime tick

Broader internal normalization of arbitrary handler expressions into runtime-owned actions remains
future work. It is not part of the current implemented surface contract.

## 15.4 Inter-thread communication

Message-passing primitives may exist as runtime/library types:

```aivi
type Sender A
type Receiver A
type Mailbox A
```

with effects returning `Task` and receiving through `@source` integration.

---

## 16. Runtime architecture

## 16.1 Memory management

The target runtime is a mostly-moving generational collector with incremental scheduling plus narrow stable-handle support at foreign boundaries.

Language-visible guarantees:

- ordinary values may move
- stable addresses are not guaranteed
- GTK/GObject/FFI interactions use stable handles, pinned wrappers, or copied values

## 16.2 Threads

Recommended runtime shape:

- one GTK UI island on the main thread
- worker threads for I/O, decoding, and heavy computation
- immutable message passing from workers to scheduler-owned queues

## 16.3 Scheduler

The scheduler owns:

- signal propagation
- source event ingestion
- task completion publication
- cancellation/disposal
- tick boundaries

The scheduler must be designed so that it cannot:

- block the GTK main loop during heavy work
- deadlock on normal cross-thread publication
- recurse unboundedly during propagation
- leak torn-down subscriptions

---

## 17. GTK / libadwaita embedding

AIVI's primary UI target is GTK4/libadwaita on Linux.

The pure language core must remain pure. UI effects cross a controlled boundary through the GTK bridge.

## 17.1 View model

AIVI uses typed markup-like view syntax and lowers it to a stable widget/binding graph.

It does **not** use a virtual DOM.

### 17.1.1 Direct lowering rules

Each markup node compiles to:

- widget/control-node kind
- static property initializers
- dynamic property bindings
- signal/event handlers
- child-slot instructions
- teardown logic

Ordinary widget nodes are created once per node identity. Dynamic props update through direct setter calls. Event handlers connect directly to GTK signals.

There is no generic diff engine over a virtual tree.

## 17.2 Property and event binding

Example:

```aivi
<Label text={statusLabel order} visible={isVisible} />
```

If an expression is reactive, the compiler extracts a derived signal and the runtime:

- computes the initial value
- subscribes once
- calls the concrete GTK setter on change

### 17.2.1 Event hookups

Expression-valued markup attributes lower as live GTK event routes only when the widget schema
catalog declares that exact widget/event pair.

```aivi
sig clicked : Signal Unit

<Button label="Click me" onClick={clicked} />
```

Event hookup rules:

- the handler expression must name a directly publishable input signal (see §13.5 and §15.3)
- only direct input signals are legal in the current live GTK surface; arbitrary callback
  expressions are future work
- the input signal's payload type must match the GTK event's concrete payload type
- unsupported event names on a given widget type remain ordinary attributes and are rejected by
  run-surface validation rather than silently treated as live events
- GTK discrete events (such as button clicks) force their own runtime ticks; rapid repeated events are processed as separate transactions and not collapsed within one tick

Event props lower to direct GTK signal connections that publish into runtime input signals, not
user-visible callbacks.

### 17.2.2 Executable widget schema metadata

The live GTK host is driven by one compiled widget schema catalog shared by lowering, `aivi run`
validation, and concrete GTK hookup.

Each widget schema entry defines:

- the current markup lookup key (today: the final widget path segment)
- property descriptors: exact property name, semantic value shape, and GTK setter route
- event descriptors: exact event name, GTK signal route, and payload shape
- child-group descriptors: group name, container policy, and child-count bounds
- whether the widget is window-like for root validation/presentation

Current executable catalog:

- `Window` — properties `title`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events;
  child group `content` accepting at most one child; treated as a window root
- `Box` — properties `orientation`, `spacing`, `visible`, `sensitive`, `hexpand`, `vexpand`; no
  markup events; child group `children` with append-only sequence semantics
- `ScrolledWindow` — properties `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events;
  child group `content` accepting at most one child
- `Label` — properties `text`, `label`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup
  events; no child groups
- `Button` — properties `label`, `visible`, `sensitive`, `hexpand`, `vexpand`; event `onClick`
  publishing `Unit`; no child groups
- `Entry` — properties `text`, `placeholderText`, `editable`, `visible`, `sensitive`, `hexpand`,
  `vexpand`; event `onActivate` publishing `Unit`; no child groups
- `Switch` — properties `active`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup
  events; no child groups

Widgets outside this catalog are not part of the current live GTK surface. Expanding the catalog to
more GTK4/libadwaita widgets is separate follow-on work.

## 17.3 Control nodes

Control nodes are part of the view language and lower directly.

### 17.3.1 `<show>`

```aivi
<show when={isVisible}>
    <Label text="Ready" />
</show>
```

Semantics:

- `when` must be `Bool`
- when false, the subtree is absent
- when true, the subtree is present

Optional flag:

```aivi
<show when={isVisible} keepMounted={True}>
    ...
</show>
```

- `keepMounted = False` is the default
- if `False`, hide means full subtree teardown per §17.4: unmount widgets, disconnect event
  handlers, and dispose owned subscriptions; show means recreate the subtree from scratch
- if `True`, the subtree mounts once and hide/show becomes a visibility transition rather than an
  unmount/remount cycle
- while hidden under `keepMounted = True`, property bindings, signal subscriptions, source
  subscriptions, and event hookups remain installed
- concrete input delivery while hidden follows the host toolkit; for the current GTK host,
  invisible widgets do not receive pointer or keyboard events even though their handlers remain
  connected
- when visibility returns under `keepMounted = True`, the existing subtree becomes visible again
  without recreation

### 17.3.2 `<each>`

```aivi
<each of={items} as={item} key={item.id}>
    <Row item={item} />
</each>
```

Semantics:

- `of` must yield `List A`
- `as` binds the element within the body
- the body must produce valid child content for the parent slot
- `key` is required for reorderable/dynamic collections and strongly recommended in general

Runtime behavior:

- keyed child identity is maintained by key
- updates compute localized child edits
- existing child subtrees are reused by key where possible
- actual GTK child insertion/removal/reordering happens directly

This is localized child management, not virtual DOM diffing.

#### `<empty>`

`<each>` may optionally contain an `<empty>` branch rendered only when the list is empty.

```aivi
<each of={items} as={item} key={item.id}>
    <Row item={item} />
    <empty>
        <Label text="No items" />
    </empty>
</each>
```

### 17.3.3 `<match>`

Because the language has no `if` / `else`, markup supports direct pattern-based rendering.

```aivi
<match on={status}>
    <case pattern={Paid}>
        <Label text="Paid" />
    </case>
    <case pattern={Pending}>
        <Label text="Pending" />
    </case>
</match>
```

Rules:

- `on` is any expression
- cases use ordinary AIVI patterns
- exhaustiveness follows ordinary match rules
- lowering selects/deselects concrete subtrees directly

### 17.3.4 `<fragment>`

```aivi
<fragment>
    <Label text="A" />
    <Label text="B" />
</fragment>
```

Groups children without creating a wrapper widget.

### 17.3.5 `<with>`

Useful local naming is allowed in markup through a non-reactive binding node:

```aivi
<with value={formatUser user} as={label}>
    <Label text={label} />
</with>
```

`<with>` introduces a pure local binding for the subtree. It does not create an independent signal node.

## 17.4 Teardown and lifecycle

Tearing down a subtree must:

- disconnect event handlers
- dispose source subscriptions owned by that subtree
- release widget handles
- preserve correctness under repeated show/hide and list churn

GTK correctness is part of the language runtime contract, not a best-effort library concern.

---

## 18. Pattern matching and predicates

Pattern matching is the main branching form in both ordinary expressions and markup control nodes.

### 18.1 Rules

- sum matches must be exhaustive unless `_` is present
- boolean matches must cover `True` and `False` unless `_` is present
- record patterns may be field-subset patterns
- nested constructor patterns are allowed

### 18.2 Predicates

Predicates may use:

- ambient projections such as `.age > 18`
- `_` for the current subject
- `and`, `or`, `not`
- `==` / `!=` when an `Eq` instance is available for the operand type

Examples:

```aivi
users |> filter (.active and .age > 18)
xs    |> takeWhile (_ < 10)
```

`x == y` desugars to `(==) x y`. `x != y` desugars to `not (x == y)` and does not introduce a separate class member.

---

## 19. Strings and regex

### 19.1 Text

String concatenation is not a core language feature. Text composition uses interpolation.

```aivi
"{name} ({status})"
```

### 19.2 Regex

Regex is a first-class compiled type with literal syntax such as:

```aivi
rx"\d{4}-\d{2}-\d{2}"
```

Invalid regex literals are compile-time errors.

---

## 20. Domains

Domains are nominal value spaces defined over an existing carrier type.

They are used when a value should:

- have the runtime representation of some existing type
- remain distinct at the type level
- optionally support domain-specific literal suffixes
- optionally expose domain-specific operators and smart constructors
- reject accidental mixing with the raw carrier or with other domains over the same carrier

Typical examples include:

- `Duration over Int`
- `Url over Text`
- `Path over Text`
- `Color over Int`
- `NonEmpty A over List A`

A domain is not a type alias. A domain is not subtyping. A domain does not imply implicit casts.

### 20.1 Declaration form

Canonical syntax:

```aivi
domain Duration over Int
domain Url over Text
domain Path over Text
domain NonEmpty A over List A
domain ResourceId A over Text
```

General form:

```text
DomainDecl ::= "domain" TypeName TypeParam* "over" Type DomainBody?
```

Rules:

- `domain D over C` introduces a new nominal type `D`
- `domain D A over C A` introduces a new unary type constructor
- the domain's kind is determined by its parameters exactly as for ordinary type constructors
- the carrier type on the right of `over` may mention only the domain's declared type parameters
- full type-level lambdas remain out of scope for v1

Examples of kinds:

- `Duration : Type`
- `Url : Type`
- `NonEmpty : Type -> Type`
- `ResourceId : Type -> Type`

### 20.2 Core meaning

A declaration:

```aivi
domain D A1 ... An over C
```

defines a fresh nominal type constructor `D A1 ... An` with carrier `C`.

From the user's point of view:

- `D A1 ... An` is distinct from `C`
- `D A1 ... An` is distinct from every other domain, even if they share the same carrier
- carrier operations are not inherited implicitly
- domain values do not pattern-match as carrier values
- implicit conversion between domain and carrier does not exist

This means:

```aivi
domain Duration over Int
domain UserId over Int
```

do **not** allow `Duration` where `Int` is expected, and do **not** allow `UserId` where `Duration` is expected.

### 20.3 Relation to opaque and branded types

A domain is the canonical surface form for a branded or opaque wrapper whose representation is intentionally based on another type.

Conceptually, a domain behaves like an opaque nominal wrapper over its carrier, but with optional language support for:

- literals
- operators
- parsing and smart construction
- formatting hooks

In other words:

- use `type` for ordinary ADTs and records
- use `domain` for nominal value spaces over an existing representation

### 20.4 Construction and elimination

A domain may be introduced only through domain-owned constructors or smart constructors.

Recommended surface shape:

```aivi
domain Url over Text
    parse : Text -> Result UrlError Url
    value : Url -> Text
```

```aivi
domain Duration over Int
    millis     : Int -> Duration
    trySeconds : Int -> Result DurationError Duration
    value      : Duration -> Int
```

The exact names are domain-defined, but the semantics are fixed:

- construction is explicit
- unwrapping is explicit
- domain invariants may be enforced by smart constructors
- unsafe or unchecked construction should be library-internal or explicitly marked

For v1, domains should not expose implicit coercions, automatic unboxing, or pattern aliases over the carrier.

### 20.5 Literal suffixes

Domains may bind literal suffixes.

Example:

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    literal min : Int -> Duration
```

This enables:

```aivi
val a:Duration = 250ms
val b:Duration = 10sec
val c:Duration = 3min
```

Literal-suffix rules:

- suffix resolution is compile-time only
- a suffix maps to exactly one domain literal definition in scope
- the suffix function must accept the literal family's base type
- numeric suffixes do not imply cross-domain arithmetic
- unsuffixed literals remain ordinary numeric literals

Examples:

- `250ms : Duration`
- `250 : Int`
- `250ms + 3min` is legal only if `Duration` defines `+`
- `250ms + 3` is illegal unless an explicit constructor or operator admits it

If two imported domains define the same suffix, that is a compile-time ambiguity error.

### 20.6 Domain operators

Domains may define a restricted set of domain-local operators.

Example:

```aivi
domain Duration over Int
    literal ms : Int -> Duration
    (+)        : Duration -> Duration -> Duration
    (-)        : Duration -> Duration -> Duration
    (*)        : Duration -> Int -> Duration
    compare    : Duration -> Duration -> Ordering
```

Example:

```aivi
domain Path over Text
    (/) : Path -> Text -> Path
```

Operator rules:

- operator resolution is static
- operators are not inherited from the carrier automatically
- operators must be declared by the domain or provided by instances over the domain type
- domain operators must preserve explicitness and must not trigger hidden conversion to or from the carrier

### 20.7 Smart construction and invariants

Domains are the preferred place to attach invariants that are stronger than the carrier type.

Examples:

- `Url over Text` may require URL parsing
- `Path over Text` may normalize separators
- `Color over Int` may require packed ARGB layout
- `NonEmpty A over List A` may reject empty lists

Example:

```aivi
domain NonEmpty A over List A
    fromList : List A -> Option (NonEmpty A)
    head     : NonEmpty A -> A
    tail     : NonEmpty A -> List A
```

The carrier type alone does not imply the invariant. The domain does.

### 20.8 Parameterized domains

Domains may be parameterized in the same style as ordinary type constructors.

Example:

```aivi
domain ResourceId A over Text
domain NonEmpty A over List A
```

Typing rules:

- parameters are ordinary type parameters
- kinds follow the ordinary kind system
- the carrier may use those parameters
- partial application of parameterized domains is allowed when the resulting kind matches the expected constructor kind

This keeps domain syntax parallel to the rest of the type system without adding new kind machinery.

### 20.9 Equality and instances

A domain does not automatically inherit all instances of its carrier.

Recommended v1 rule:

- `Eq` may be compiler-derived for a domain if its carrier has `Eq` and the domain does not opt out

Example:

```aivi
domain Duration over Int
```

may derive `Eq`, but that does **not** make it interchangeable with `Int`.

For other classes, instances should be explicit unless the language later adopts a clear derive mechanism for domains.

### 20.10 Runtime representation

A domain is representation-backed by its carrier, but user code must not rely on that representation detail for semantics.

Implementation guidance:

- a domain may compile to the same runtime layout as its carrier
- the compiler may erase wrapper overhead where sound
- diagnostics and typing must still treat the domain as distinct

This preserves performance without weakening the surface model.

### 20.11 No implicit casts

Domains never participate in implicit cast chains.

Illegal examples:

```aivi
val x:Int = 250ms
val y:Duration = 250
val z:UserId = durationValue
```

Legal examples:

```aivi
val x:Duration = 250ms
val y:Int = Duration.value x
val z:Duration = Duration.millis 250
```

This rule is normative.

### 20.12 Diagnostics

Diagnostics for domains should be explicit and domain-aware.

Examples:

- when a carrier is used where a domain is expected:
  
  - `expected Duration but found Int`
  - suggestion: `use Duration.millis`, `Duration.parse`, or another domain constructor

- when a domain is used where a carrier is expected:
  
  - `expected Text but found Url`
  - suggestion: `use Url.value`

- when a suffix is ambiguous:
  
  - `literal suffix 'ms' is provided by multiple domains in scope`

- when an operator is missing:
  
  - `operator '+' is not defined for Duration and Int`

### 20.13 Recommended examples

#### Duration

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    literal min : Int -> Duration
    (+)         : Duration -> Duration -> Duration
    (-)         : Duration -> Duration -> Duration
    value       : Duration -> Int
```

#### Url

```aivi
domain Url over Text
    parse : Text -> Result UrlError Url
    value : Url -> Text
```

#### Path

```aivi
domain Path over Text
    parse : Text -> Result PathError Path
    (/)   : Path -> Text -> Path
    value : Path -> Text
```

#### NonEmpty

```aivi
domain NonEmpty A over List A
    fromList : List A -> Option (NonEmpty A)
    head     : NonEmpty A -> A
    tail     : NonEmpty A -> List A
```

### 20.14 Design boundary for v1

Domains in v1 are intentionally narrow.

They do:

- define nominal carrier-backed types
- support optional literals
- support optional domain operators
- support explicit smart constructors and explicit unwrapping
- compose with the existing kind system

They do **not**:

- introduce subtyping
- introduce implicit casts
- introduce open-ended type-level computation
- allow arbitrary carrier pattern matching through the domain
- replace ordinary ADTs or records

---

## 21. Diagnostics

AIVI favors explicitness over clever inference.

Diagnostics must:

- identify the failed invariant
- point at the user-visible cause
- avoid leaking backend IR details unless requested in debug output
- suggest the intended construct when the misuse is obvious

Examples:

- using a signal in `val` should suggest `sig`
- omitting a record field without a `Default` instance should name the missing field and missing instance
- mixing applicative constructors in one `&|>` cluster should report the first mismatch and the expected common outer constructor

---

## 22. Formatter

The formatter is part of the language contract.

### 22.1 Formatter goals

- canonical pipe alignment
- canonical arrow alignment in contiguous match arms
- stable formatting for records, markup, and clustered applicative spines

### 22.2 `&|>` formatting

The formatter should preserve and prefer the leading-cluster style when the spine is vertically scanned for independence.

Preferred example:

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

This is a first-class canonical style, not a tolerated edge case.

---

## 23. Testing and hardening

The implementation must include:

- unit tests
- parser golden tests
- formatter golden tests
- type-checker expectation tests
- property tests for lawful core instances where practical
- fuzzing for parser and decoder paths
- scheduler stress tests
- GTK subtree lifecycle tests
- stack-depth torture tests
- teardown/leak tests
- deterministic scheduling tests

Every bug fix should add a regression test that names the failed invariant.

---

## 24. Milestones

These milestones do **not** reduce scope. They partition implementation work.

Status legend: **COMPLETE** = fully implemented; **PARTIAL** = core slice implemented with known gaps; **PENDING** = not yet started.

### Milestone 1 — Surface and CST freeze — **COMPLETE**

- lexer ✓
- parser ✓
- CST (lossless for formatting and diagnostics) ✓
- formatter (canonical pipe, arrow, cluster alignment) ✓
- syntax for `type`, `class`, `instance`, `val`, `fun`, `sig`, `use`, `export`, markup, and pipe operators ✓
- line/block/doc comment lexing (`//`, `/* */`, `/** **/`) and trivia retention in the token stream ✓
- regex literal lexing and HIR validation ✓
- compact suffix literal lexing (`250ms`) ✓

### Milestone 2 — HIR and names — **COMPLETE**

- name resolution ✓
- import resolution ✓
- import alias (`use module (x as y)`) ✓
- decorator attachment (`@source`, `@recur.timer`, `@recur.backoff`) ✓
- explicit HIR nodes for applicative clusters and markup control nodes ✓
- domain declarations and suffix namespaces ✓
- `instance` blocks with same-module class resolution ✓
- provider declarations (`provider qualified.name`) ✓
- input signal declarations (body-less annotated `sig`) ✓

### Milestone 3 — Kinds and core typing — **COMPLETE**

- kind checking ✓
- class/instance resolution and evidence ✓
- constructor partial application ✓
- `Validation` ✓
- `Default` and record default elaboration ✓
- `Eq` compiler derivation ✓
- module-aware expression typechecker in `aivi-hir` ✓
- operator typechecking (`==`, `!=`, domain operators) ✓
- truthy/falsy branch handoff (`T|>`, `F|>`) ✓
- case exhaustiveness checks for known closed sums ✓
- bidirectional record/collection/projection shape checking ✓

### Milestone 4 — Pipe normalization — **COMPLETE**

- exact `&|>` normalization into applicative spines ✓
- recurrence node representation ✓
- recurrence scheduler-node handoff ✓
- gate (`?|>`) lowering plan ✓
- fan-out (`*|>` / `<|*`) typed handoff ✓
- source lifecycle handoff ✓
- diagnostics for illegal unfinished clusters ✓

### Milestone 5 — Reactive core and scheduler — **COMPLETE**

- signal graph extraction ✓
- topological scheduling with GLib main-context integration ✓
- transactional ticks with generation stamps ✓
- deterministic propagation with stale-publication rejection ✓
- cancellation/disposal and owner-liveness tracking ✓
- GLib cross-thread wakeup with reentry guard ✓

### Milestone 6 — Tasks and sources — **PARTIAL**

- `Task` typed IR and scheduler ports ✓
- `@source` runtime contract and instance lifecycle ✓
- decode integration (structural decoder, domain parse method resolution) ✓
- worker/UI publication boundary ✓
- timer sources (`timer.every`, `timer.after`) — fully working ✓
- HTTP sources — runtime contract wired, provider execution pending
- `fs.read`, `fs.watch` — contract wired, full runtime execution pending
- socket / mailbox / process / window-event sources — pending
- `Task` full worker execution — pending

### Milestone 7 — GTK bridge — **PARTIAL**

- widget plan IR ✓
- runtime assembly ✓
- GTK bridge graph and child-group lowering ✓
- executor with direct setter/event/child management ✓
- `<show>` (mount/unmount) ✓
- `keepMounted` on `<show>` ✓
- `<each>` with keys and localized child edits ✓
- `<empty>` branch ✓
- `<match>` ✓
- `<fragment>` ✓
- `<with>` ✓
- widget schema metadata for the current live widget surface ✓
- full widget property catalog — pending

### Milestone 8 — Backend and hardening — **PARTIAL**

- lambda IR with explicit closures and environments ✓
- backend IR with layouts, kernels, pipelines ✓
- Cranelift AOT codegen for scalars and item-body kernels ✓
- runtime startup linking (HIR → backend → scheduler) ✓
- inline helper pipe execution in item/source kernels ✓
- body-backed signal inline transform/tap/case/truthy-falsy execution against committed snapshots ✓
- general lambda/closure conversion for arbitrary bodies — pending
- scheduler-owned signal filter/fanout/recurrence pipeline execution — pending
- GC integration — pending
- performance pass plan frozen; implementation pending (see §28.9)
- fuzzing and stress infrastructure — pending

Milestone 8 performance work is scoped narrowly: post-typed-core lambda/backend/codegen/runtime
passes only. HIR, typechecking, and typed core remain the proof and diagnostic source-of-truth
layers. The normative pass order and benchmark gates are defined in §28.9.

---

## 25. Bottom-line implementation guidance

AIVI should be implemented as one coherent system:

- typed and lowered through explicit IR boundaries
- stack-safe by design
- scheduler-driven and deterministic
- pure in the language core
- explicit at all effect boundaries
- GTK-first without collapsing into callback-driven impurity
- direct-binding-oriented, not virtual-DOM-oriented

The implementation should prefer one correct algebraic model over many local patches. In particular:

- `&|>` must remain one applicative story across `Validation`, `Signal`, `Option`, `Result`, and `Task`
- record omission must remain explicit-default completion, not open records
- `Task` must remain the only user-visible one-shot effect carrier
- GTK markup must lower directly and predictably to widgets, setters, handlers, and child management
---

## 26. CLI reference

The `aivi` CLI provides the following subcommands.

### 26.1 `aivi check <path>`

Validates an AIVI source file through the full frontend pipeline:

```
aivi check src/main.aivi
```

Pipeline: source → CST → HIR → typed core → lambda → backend (no code emission).

Reports diagnostics with source locations. Exits 0 if there are no errors, 1 if there are errors, 2 on internal failure.

Invoking `aivi <path>` with no subcommand is equivalent to `aivi check <path>`.

### 26.2 `aivi compile <path> [-o <output>]`

Compiles an AIVI source file to a native object file:

```
aivi compile src/main.aivi -o build/main.o
aivi compile src/main.aivi --output build/main.o
```

Pipeline: source → CST → HIR → typed core → lambda → backend → Cranelift → object file.

If `-o` / `--output` is omitted, no output file is written but the pipeline is validated. Exits 0 on success, 1 on compilation errors.

**Current limitation**: object output is emitted but runtime startup, linking, and executable launch are not yet fully automated. The explicit per-stage failure boundary is reported rather than pretending a full executable is produced.

### 26.3 `aivi run <path> [--view <name>]`

Compiles and runs an AIVI module as a GTK application:

```
aivi run src/app.aivi
aivi run src/app.aivi --view mainWindow
```

View selection rules:

1. If `--view <name>` is given, the named top-level markup-valued `val` is used.
2. Otherwise, if there is exactly one top-level markup-valued `val` named `view`, that is used.
3. Otherwise, if there is a unique top-level markup-valued `val`, that is used.
4. If several candidates remain, `--view` is required.

The selected root must be a `Window`. The CLI does not auto-wrap arbitrary widgets into windows.

The run session integrates a GLib main loop, evaluates markup fragments against committed runtime signal snapshots, and re-evaluates the selected view after each meaningful scheduler tick. Live GTK updates are applied through the bridge executor.

Exits 0 on clean application close, 1 on startup/compilation error.

### 26.4 `aivi fmt [--stdin | --check] [<path>...]`

Formats AIVI source files:

```
aivi fmt src/app.aivi             # format to stdout
aivi fmt --stdin                  # read from stdin, write to stdout
aivi fmt --check src/a.aivi src/b.aivi   # verify formatting; exit 1 if any differ
```

The formatter is canonical: it produces a single deterministic output for any valid source. Formatting is part of the language contract (§22).

### 26.5 `aivi lex <path>`

Tokenizes an AIVI source file and prints the token stream:

```
aivi lex src/app.aivi
```

Useful for debugging lexer behavior or suffix literal resolution.

### 26.6 `aivi lsp`

Starts the AIVI Language Server on stdin/stdout using the Language Server Protocol:

```
aivi lsp
```

Editor integrations should launch this subprocess and communicate over stdio. See §27 for supported LSP capabilities.

---

## 27. Language server (LSP)

The AIVI language server (`aivi lsp`) provides editor integration through the Language Server Protocol. It is backed by the `aivi-query` incremental query database, which caches source, parse, HIR, diagnostic, symbol, and format results per revision.

### 27.1 Supported capabilities

| Capability | Status |
|---|---|
| Text document sync (full) | ✓ |
| Diagnostics (publish on open/change) | ✓ |
| Document formatting | ✓ |
| Document symbols | ✓ |
| Workspace symbols | ✓ |
| Hover documentation | ✓ |
| Go-to-definition | ✓ |
| Completion (triggered on `.`) | ✓ |
| Semantic tokens (full) | ✓ |

### 27.2 Architecture

The LSP server is read-only from the user model's point of view. All editor features go through the query database rather than invoking ad-hoc frontend passes. Incremental memoization is per file revision so rapid keystroke changes do not invalidate unrelated cached queries.

### 27.3 Current limitations

- multi-file workspace analysis is not yet implemented; each file is checked independently
- completion suggestions are basic; type-directed completion over expected record fields and constructor arguments is pending
- semantic token legend is defined but token-type coverage is partial

---

## 28. Pre-stdlib gaps and implementation status

This section lists known gaps that must be addressed or explicitly scoped before a standard library can be built on the language.

### 28.1 General expression typechecking

The module-aware expression typechecker in `aivi-hir` covers:

- top-level annotation checks
- record default and shorthand elaboration
- operator typechecking (`==`, `!=`, domain operators)
- class instance checking
- truthy/falsy branch handoff
- case exhaustiveness for known closed sums
- bidirectional record/collection/projection shape checking
- `Apply` and `Name` expressions with local expected-type propagation

**Gap**: General expression inference across arbitrary expression trees (function application chains, let-bindings, lambda expressions as values, higher-order combinators) is not yet complete. Many ordinary bodies remain "unlowered" in typed-core rather than being proven through the typechecker. This is the primary blocker for a general-purpose stdlib.

### 28.2 Task runtime execution

The `Task` typed IR and scheduler ports exist. Workers publish through typed scheduler-owned ports.

**Gap**: Full task worker execution — scheduling a `Task` value, running its body on a worker thread, and routing success/error results back into the scheduler — is not yet fully wired. User-authored `Task`-returning `fun` bodies cannot be executed at runtime until the general expression lowering gap (§28.1) is also closed.

### 28.3 Source provider coverage

Timer sources (`timer.every`, `timer.after`) are fully working. HTTP, `fs.read`, `fs.watch`,
socket, mailbox, process, and window-event sources have their runtime contracts wired but provider
execution is pending. For request-like sources, that wired contract already uses a latest-wins
policy: refresh or reconfiguration supersedes older work, requests best-effort cancellation where
the provider supports it, and drops stale completions before publication.

**Gap**: Any stdlib module that uses non-timer sources cannot be exercised end-to-end until those providers are implemented.

### 28.4 Lambda/closure conversion

Typed-lambda IR with explicit closures and environments exists. Backend item-body kernels cover same-module value/function bodies with explicit parameter contracts.

**Gap**: General lambda/closure conversion for arbitrary higher-order function values (closures captured by source or task workers, callbacks passed as arguments) is not yet complete. Body-backed signal inline `|>` / `|` / `||>` / `T|>` / `F|>` execution now works against committed snapshots, including same-module closed-sum case patterns, but scheduler-owned signal filter/fanout/recurrence pipelines remain blocked.

### 28.5 GTK widget coverage

The GTK executor, bridge graph, and host are implemented. `<show>` including `keepMounted`,
`<each>` (with keys), `<empty>`, `<match>`, `<fragment>`, and `<with>` work.

**Gap**:
- the executable widget schema catalog currently covers `Window`, `Box`, `ScrolledWindow`,
  `Label`, `Button`, `Entry`, and `Switch`; broader GTK4 and libadwaita
  widget/property/event coverage is the next expansion step
- libadwaita widget bindings beyond that basic GTK4 slice are not yet enumerated

### 28.6 Multi-file compilation and modules

The compiler currently treats each file as a standalone module. Import catalog metadata is carried as a closed Milestone-2 catalog.

**Gap**: Multi-file compilation, cross-module name resolution at the type/term level, and a real module system (including the `aivi.*` standard library namespace) do not yet exist. The stdlib itself requires this to be addressable.

### 28.7 Garbage collection

The runtime currently does not include the generational moving GC described in §16.1. Values are managed via Rust's ownership system in the current implementation.

**Gap**: The moving GC, stable-handle boundary, and GC-safe pointer tagging are required before the runtime matches the language-visible guarantees in §16.1.

### 28.8 Stack-depth guarantees

The implementation uses explicit worklists in the compiler and scheduler. Cranelift-emitted code does not yet implement tail-call optimization or stack-safe recursion for user-authored recursive functions.

**Gap**: User-authored recursive `fun` bodies that could overflow the native stack are not yet protected. Tail recursion must be compiled in a stack-safe form per §3.4.

### 28.9 Milestone 8 performance pass plan

Milestone 8's `performance passes` are defined narrowly. They are post-typing implementation
passes that improve compile-time or runtime cost without changing the proof surface, inventing new
type facts, or moving runtime semantics into earlier layers. Frontend/query/LSP performance work
is tracked separately from this milestone.

General constraints:

- no performance pass runs before successful HIR/type/kind checking and
  `aivi_core::validate_module`
- `aivi-lambda` passes may rewrite only closure/capture metadata and must re-run
  `aivi_lambda::validate_module`
- `aivi-backend` / codegen passes may rewrite only backend-owned kernels/layout/call-lowering data
  and must re-run `aivi_backend::validate_program`
- runtime/GTK passes must be differential-tested against the committed scheduler/bridge behavior on
  the same publication trace
- surviving optimized nodes keep the root source span/origin of the surface construct they came
  from; performance work must not erase diagnostic provenance

The first implementation wave is fixed in this order:

1. typed-lambda capture pruning and environment compaction
2. backend kernel simplification
3. direct self-tail recursion loop lowering in codegen
4. scheduler frontier dedup and per-tick hydration coalescing

#### Pass P1 — typed-lambda capture pruning and environment compaction

- **Owning layer**: `aivi-lambda`, after `aivi_lambda::lower_module` and before
  `aivi_backend::lower_module`
- **Transform**: compute the live free bindings of each closure body/runtime edge, remove unused
  captures, canonicalize zero-capture closures to empty environments, and keep surviving captures
  in lexical-binding order so backend env slots stay deterministic
- **Must preserve**:
  - explicit closure boundaries
  - `ClosureId` / `CaptureId` validity
  - capture binding ids, types, and source spans
  - item/pipe/stage identity carried from typed core
  - the existing lambda validation rules for free-binding coverage
- **Must not**:
  - inline closure bodies
  - merge distinct closures
  - reorder surviving captures by heuristic cost
  - invent broader closure-conversion rules than the validated lambda layer already proves
- **Benchmark / acceptance gate**: `closure-thin`
  - fixture shape: 200 closures; each scope exposes 16 candidate locals and each closure body reads
    exactly 2 free bindings; until the surface can express that corpus directly, the fixture may be
    built at the `aivi-lambda` layer
  - structural gate: total capture count after the pass is exactly `400`, and backend environment
    slots equal `parameters + live captures` only
  - timing gate: median lambda→backend lowering time in `--release`, measured over 30 runs on the
    same machine, improves by at least `5%` versus the same corpus with the pass disabled, while
    RSS does not regress by more than `10%`

#### Pass P2 — backend kernel simplification

- **Owning layer**: `aivi-backend::Program`, after `aivi_backend::lower_module` and before
  `aivi_backend::compile_program`
- **Transform**: eliminate subject/env copy chains, fold projections from closed tuple/record/sum
  constructor values, collapse aggregate literals whose children are already constants, and remove
  backend-only dead temporary nodes created by those rewrites
- **Must preserve**:
  - `aivi_backend::validate_program` success
  - item/stage/source boundary layouts and calling conventions
  - constructor identities and closed-sum arity
  - root source spans/origins for every surviving kernel root
  - runtime error timing by refusing to fold fallible operations
- **Must not**:
  - evaluate domain-member calls, class-member intrinsics, decode programs, source/task/signal
    runtime nodes, or GTK/runtime handles
  - fold arithmetic that can change observable failure behavior such as division or modulo by zero
  - perform cross-item inlining or rewrite scheduler-visible pipeline structure
- **Benchmark / acceptance gate**: `kernel-const`
  - fixture shape: 1,000 backend kernels built from total tuple/record/sum constructors,
    projections, and copy chains
  - structural gate: no remaining `projection(constant)` or `copy(copy(...))` nodes; every closed
    projection chain simplifies to a literal, constructor, env slot, or inline-subject read
  - timing gate: median `validate_program + compile_program` time or emitted object bytes improve
    by at least `5%` versus the pass-disabled baseline on the same corpus

#### Pass P3 — direct self-tail recursion loop lowering

- **Owning layer**: `aivi-backend::codegen` during Cranelift lowering
- **Transform**: direct self-tail recursion in proven item bodies and hidden callable items lowers
  to a loop/trampoline with slot reassignment instead of recursive native calls
- **Initial slice only**:
  - direct self recursion with statically known callee and arity
  - validated item bodies and hidden callable items already owned by the backend
  - mutual recursion, heap-allocated closures, and general higher-order worker callbacks stay out
    of scope until the broader closure-conversion gap (§28.4) is closed
- **Must preserve**:
  - result value and error behavior
  - parameter evaluation order
  - closure environment contents
  - source-span mapping for the root callable
  - zero scheduler interaction from ordinary recursive computation
- **Must not**:
  - depend on host tail-call support
  - allocate one environment frame per step
  - rewrite non-tail calls
- **Benchmark / acceptance gate**: `tail-loop-sum`
  - fixture shape: one direct self-tail-recursive accumulator with `1_000_000` tail steps,
    compiled through the backend/codegen path
  - correctness gate: the `--release` binary completes the full depth with no stack overflow
  - structural gate: the codegen dump for the hot tail path shows a loop/backedge and no recursive
    self-call
  - timing gate: zero per-step heap allocations after entry and median runtime no worse than `2x`
    the hand-written Rust loop in the same harness

#### Pass P4 — scheduler frontier dedup and per-tick hydration coalescing

- **Owning layer**: `aivi-runtime` scheduler and runtime-owned view-hydration scheduling;
  concrete GTK mutation still happens in `aivi-gtk`
- **Transform**: once a tick's ingress publication set is frozen, queue each reachable derived
  signal, source reconfiguration site, and dirty view owner at most once for that tick; repeated
  dirty marks set scheduler-owned bits/counters rather than pushing duplicate work items
- **Must preserve**:
  - topological propagation order
  - latest stable upstream values per tick
  - transactional visibility and no mixed-time observations
  - generation-stamp rejection and latest-wins source supersession
  - separate runtime ticks for discrete GTK events
  - GTK-main-thread-only widget mutation
- **Must not**:
  - coalesce publications across tick boundaries
  - drop distinct discrete GTK events
  - let workers mutate UI-owned state directly
  - recurse unboundedly during propagation
- **Benchmark / acceptance gate**: `scheduler-fanout-1024`
  - fixture shape: 1 input signal, 1,024 reachable derived nodes in a shared DAG, 64 view-owned
    sinks, and 10,000 ticks; correctness traces additionally include stale generations and rapid
    GTK click bursts
  - structural gate: for each tick, `eval_count <= dirty_derived_nodes`,
    `source_reconfig_count <= dirty_source_sites`, `hydration_count <= dirty_view_owners`, and
    `queue_pushes <= ingress_publications + dirty_derived_nodes + dirty_source_sites + dirty_view_owners`
  - correctness gate: committed snapshots and hydration decisions match the reference scheduler
    exactly on the recorded traces
  - timing gate: p95 tick latency in `--release` improves by at least `20%` on the synthetic DAG,
    otherwise the pass does not land

Deferred from the first wave: cross-item inlining, signal-graph fusion, aggregate ABI reshaping,
speculative GTK setter elision, and frontend/query caching work. Those remain out of scope until
the first-wave passes exist and the benchmark corpus shows where the real cost sits.

### 28.10 Summary: what must close before stdlib work begins

| Gap | Blocking for stdlib? |
|---|---|
| General expression typechecking (§28.1) | **Critical** — needed for all typed stdlib functions |
| Task runtime execution (§28.2) | **Critical** — needed for any effectful stdlib |
| Multi-file modules (§28.6) | **Critical** — stdlib must live in `aivi.*` modules |
| Lambda/closure conversion (§28.4) | **High** — needed for higher-order combinators |
| Source provider coverage (§28.3) | **Medium** — needed for network/file stdlib |
| GTK widget coverage (§28.5) | **Medium** — needed for UI component stdlib |
| GC (§28.7) | **Medium** — needed for runtime correctness guarantees |
| Stack-safety for recursion (§28.8) | **Medium** — needed before recursive stdlib functions are safe |
| Performance passes (§28.9) | No — not a semantic blocker for the first stdlib slice, but required before Milestone 8 can claim production compile/run budgets |
