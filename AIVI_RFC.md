# AIVI Language Specification

## Draft v1.0 — implementation-facing resolved pass

> Status: normative working draft with implementation choices merged. Milestones 1–8 (surface through backend) are substantially complete. Sections §26–§28 cover the CLI, LSP, and pre-stdlib implementation gaps.

---

## 1. Vision

AIVI is a purely functional, reactive, GTK/libadwaita-first programming language for building native Linux desktop applications.

Defining properties:

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
- runtime integrating scheduler, signal propagation, GC, sources, and GTK

AIVI is not a thin syntax layer over Rust or GTK. It has a pure semantic core and an explicit runtime boundary.

---

## 2. Design goals and non-goals

### 2.1 Primary goals

- GTK4/libadwaita application development on GNOME Linux as the flagship use case
- pure, explicit, analyzable user model
- reactivity is a primitive of the language, not a library
- native JIT/AOT compilation
- correctness legible through closed types, explicit boundaries, and strong diagnostics

### 2.2 Non-goals

V1 excludes:

- unrestricted systems programming
- implicit mutation-oriented UI models
- open-world structural typing
- type-level metaprogramming beyond narrow HKT support
- general-purpose dynamic graph monads for signals

---

## 3. Implementation invariants

### 3.1 Semantic invariants

- Ordinary user functions are pure.
- `Signal` values denote time-varying values whose dependencies are known after elaboration.
- `Task E A` denotes a one-shot effectful computation description; not an immediate effect.
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

1. **Lexer / parser**
2. **CST**
3. **HIR**
4. **Typed core**
5. **Closed typed lambda IR**
6. **Backend IR**
7. **Cranelift code generation**
8. **Runtime integration**

The implementation-facing companion boundary contracts live in `docs/ir-boundary-contracts.md`.

### 4.1 CST

Source-oriented and lossless enough for formatting and diagnostics.

Boundary contract:

- ownership: `aivi_syntax::ParsedModule` owns both the lossless token buffer and the structural CST module
- identity: top-level items are source-addressed by `TokenRange` into the token buffer; nested nodes are structural within their parent item
- source spans: user-addressable CST nodes carry `SourceSpan`; top-level items additionally retain `TokenRange` for trivia-preserving source mapping
- validation entry points: `aivi_syntax::lex_module` establishes token/trivia invariants; `aivi_syntax::parse_module` establishes CST shape plus recoverable syntax diagnostics
- losslessness: comments, whitespace, and other trivia remain in the token buffer even when not lowered into dedicated CST nodes

### 4.2 HIR

First module-owned arena IR.

Boundary contract:

- ownership: one `aivi_hir::Module` owns arenas for items, expressions, patterns, decorators, bindings, markup nodes, control nodes, and type nodes
- identity: opaque arena ids — `ItemId`, `ExprId`, `PatternId`, `DecoratorId`, `MarkupNodeId`, `ControlNodeId`
- source spans: every user-facing name, item header, expression, pattern, markup node, and control node carries the source span that diagnostics must report
- validation entry points: `aivi_hir::lower_module` / `lower_module_with_resolver`, `aivi_hir::validate_module`, `aivi_hir::typecheck_module`

HIR responsibilities:

- names resolved
- imports resolved
- decorators attached
- markup nodes represented explicitly
- pipe clusters represented explicitly
- surface sugar preserved where useful for diagnostics
- source metadata and source-lifecycle/decode/fanout/recurrence elaboration reports made explicit
- body-less annotated `signal` declarations preserved as first-class input signals

### 4.3 Typed core

First post-HIR layer owning fully typed runtime-facing nodes.

Boundary contract:

- ownership: one `aivi_core::Module` owns typed arenas for items, expressions, pipes, stages, sources, and decode programs
- identity: opaque ids — `ItemId`, `ExprId`, `PipeId`, `StageId`, `SourceId`, `DecodeProgramId`, `DecodeStepId`
- source spans: expressions, patterns, stages, items, source nodes, and decode nodes preserve source spans; origin handles back into HIR stay attached where later layers need them
- validation entry points: `aivi_core::lower_module`, `aivi_core::lower_runtime_module`, `aivi_core::validate_module`

Typed core responsibilities:

- all names resolved
- kinds checked
- class constraints attached
- `&|>` normalized into applicative spines
- pattern matching normalized
- record default elision elaborated
- markup control nodes typed
- signal dependency graph extracted
- blocked or not-yet-proven ordinary expression slices kept explicit

### 4.4 Closed typed lambda IR

Keeps closure structure explicit without collapsing into backend layout or ABI choices.

Boundary contract:

- ownership: one `aivi_lambda::Module` owns closure and capture arenas while embedding the validated typed-core module
- identity: explicit `ClosureId` and `CaptureId` plus carried-through core ids for items, pipes, stages, sources, and decode programs
- source spans: closure, item, pipe, and stage nodes preserve source spans from typed core / HIR
- validation entry points: `aivi_lambda::lower_module` and `aivi_lambda::validate_module`

Responsibilities:

- explicit closures
- explicit environments
- explicit runtime nodes for sources/tasks/signals where needed
- dictionary passing or monomorphization decisions applied
- no remaining surface sugar

### 4.5 Backend IR and codegen

First layer owning ABI/layout/runtime call contracts.

Boundary contract:

- ownership: one backend `Program` owns items, pipelines, kernels, layouts, sources, and decode plans
- identity: `PipelineId`, `KernelId`, `KernelExprId`, `LayoutId`, `SourceId`, `DecodePlanId`, `DecodeStepId`, `EnvSlotId`, `InlineSubjectId`, plus origin links back into earlier IRs
- source spans: item, pipeline, stage, source, and kernel origins preserve source spans; backend expressions keep source spans for diagnostics and debug dumps
- validation entry points: `aivi_backend::lower_module`, `aivi_backend::validate_program`, `aivi_backend::compile_program`

Responsibilities:

- layout decisions
- concrete calling conventions
- Cranelift lowering
- AOT and JIT support

---

## 5. Top-level forms

```aivi
data Bool = True | False

class Eq A
    (==) : A -> A -> Bool

value answer = 42

value add:Int x:Int y:Int =>
    x + y

signal counter = 0

use aivi.network (
    http
    socket
)
```

Top-level forms:

- `type`
- `data`
- `class`
- `instance`
- `value`
- `signal`
- `source`
- `result`
- `view`
- `adapter`
- `use`
- `export`
- `provider`
- decorators via `@name`

### 5.0.1 Unified `value` declaration

`value` is the single keyword for all pure top-level bindings, replacing the former separate `val` and `fun` keywords.

No-params form (constant binding), uses `=`:

```aivi
value answer = 42
value greeting = "hello"
```

Params form (function binding), uses `=>`:

```aivi
value add:Int x:Int y:Int =>
    x + y

value greet:Text name:Text =>
    "Hello, {name}"
```

`value` is a **contextual keyword**: it is also a valid identifier and parameter name. The following is valid AIVI — the parameter is named `value`:

```aivi
value absolute:Int value:Int =>
    value < 0
     T|> 0 - value
     F|> value
```

The parser disambiguates by position: `value` at the start of a top-level form is a keyword; `value` as a subsequent token after the function name is a parameter.

### 5.0.2 `data` — ADT declarations

`data` declares algebraic data types (sum types and constructor-headed product types):

```aivi
data Bool = True | False

data Option A =
  | None
  | Some A

data Result E A = Err E | Ok A
```

Plain `type` remains for record type synonyms and other type declarations.

### 5.0.3 `signal` — reactive signal declarations

`signal` declares reactive nodes in the dependency graph:

```aivi
signal counter = 0
signal query : Signal Text
signal fullName = firstName ++ " " ++ lastName
```

Body-less annotated forms declare **input signals** — externally publishable entry points:

```aivi
signal clicked : Signal Unit
signal query   : Signal Text
```

### 5.0.4 `source` — source-backed signal declarations

`source` declares a signal backed by a runtime source provider:

```aivi
source users : Signal (Result HttpError (List User)) = http.get "/users"
source tick  : Signal Unit = timer.every 120
source keyDown : Signal Key = window.keyDown
```

The general form is `source name : Signal T = provider.variant args`. The provider and variant are resolved statically. This replaces the former `@source` decorator on a body-less `signal`.

### 5.0.5 `result` — graph-assembly records

`result` defines a named graph-assembly record whose fields are reactive nodes:

```aivi
result appState
    users   = fetchedUsers
    visible = users ?|> .active
    count   = visible *|> .id |> List.length
```

Fields form a dependency graph; execution order is dependency-driven, not declaration order.

### 5.0.6 `view` — markup view declarations

`view` declares a named GTK/libadwaita markup fragment:

```aivi
view mainWindow
    <Window title="My App">
        <Box orientation="vertical">
            <Label text={greeting} />
        </Box>
    </Window>
```

`view` declarations are the canonical entry points for `aivi run --view mainWindow`.

### 5.0.7 `adapter` — adapter declarations

`adapter` declares a named adapter, bridging external data shapes or protocols into the reactive graph.

---

A module may export at most one `main` binding. `main` is the conventional standalone-process entry for future packaging; the current `aivi run` surface does not privilege it over the static view-selection rules in §26.3. A top-level markup-valued `value` named `view` is the preferred unqualified preview entry when no explicit `--view` is given.

Comment syntax:

- `//` — line comment, runs to end of line
- `/* ... */` — block comment
- `/** ... **/` — doc comment
- all three are trivia in the lossless token stream; they do not create expression or item nodes in the CST
- the lexical distinction between `//`, `/* */`, and `/** **/` is stable; declaration attachment and doc extraction are tooling-owned work above the syntax layer

### 5.1 Import rules

- local names and a small explicit import set
- no wildcard imports
- no arbitrary value-level module qualification for imported module members
- compiler-known type/domain/class namespaces may expose qualified member lookup such as `Duration.value`, `Text.join`, or `Foldable.reduce`
- built-in names keep priority where needed
- callable domain members and class members participate in ordinary term lookup when in scope

#### Import aliases

`use module (member as localName)` is the disambiguation escape hatch when two imports would share a local name:

```aivi
use aivi.network (http)
use my.client (fetch as clientFetch)
```

The original member name drives compiler-known metadata. The alias changes only the local binding name.

#### Name resolution for terms

The compiler resolves from local name plus known context. Multiple candidates after contextual filtering produce an ambiguity diagnostic requiring explicit disambiguation through an import alias.

Qualified member syntax such as `Duration.value` is not a general module-qualification escape hatch; it is a compiler-known lookup surface for the owning type/domain/class namespace only.

---

## 6. Type system

## 6.1 Kinds

Base kind: `Type`

Constructor kinds: `Type -> Type`, `Type -> Type -> Type`, right-associative arrow kinds.

Examples:

- `Int : Type`
- `Text : Type`
- `Option : Type -> Type`
- `Signal : Type -> Type`
- `Result : Type -> Type -> Type`
- `Task : Type -> Type -> Type`

Partial application of named type constructors is supported.

Valid: `Option`, `List`, `Signal`, `Result HttpError`, `Task FsError`

Invalid: passing `Result` where a unary constructor is required; passing `List Int` where a constructor is required.

Full type-level lambdas are deferred.

## 6.2 Core primitive and standard types

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

### 6.2.1 Numeric literal surface

Accepted surface forms:

- unsuffixed integer literals: ASCII decimal digits only — `0`, `42`, `9000`
- float literals: ASCII decimal digits, one `.`, ASCII decimal digits — `0.5`, `3.14`
- decimal literals: ASCII decimal digits with trailing `d`, optionally with one fractional part — `19d`, `19.25d`
- BigInt literals: ASCII decimal digits with trailing `n` — `123n`
- compact `digits + suffix` is a domain literal suffix candidate only when the suffix is at least two ASCII letters and does not match a built-in non-`Int` literal form: `250ms`, `10sec`, `3min`
- spacing is semantic: `250ms` is one suffixed literal candidate; `250 ms` is ordinary application
- leading zeroes do not introduce octal or any other alternate base; `007` is decimal
- exact one-letter alphabetic compact suffixes are reserved for built-in numeric literal families and future core numeric extensions
- `d` and `n` are allocated to `Decimal` / `BigInt`; user-defined suffixes must use two or more letters

Not part of the literal grammar:

- sign-prefixed numeric literals
- `_` separators inside numeric tokens
- built-in hex, binary, or octal integer forms
- exponent notation

A compact suffix form is well-typed only when exactly one current-module domain literal suffix claims that suffix and accepts the base integer family. Otherwise the literal is rejected as unresolved or ambiguous.

### 6.2.2 Executable numeric literal slice

- `Int` literals execute as by-value `i64`.
- `Float` literals execute as finite IEEE-754 `f64`, by-value scalar ABI.
- `Decimal` literals execute as exact decimal runtime values; backend layout marks them by-reference; Cranelift materializes only immutable literal cells with `mantissa:i128 (little-endian) + scale:u32 (little-endian)`.
- `BigInt` literals execute as exact arbitrary-precision integer runtime values; backend layout marks them by-reference; Cranelift materializes only immutable literal cells with `sign:u8 + 7 bytes padding + byte_len:u64 (little-endian) + magnitude bytes (little-endian)`.
- `Decimal` and `BigInt` literal cells are introduction-only in the current Cranelift slice.
- Non-`Int` arithmetic and ordered comparison remain deferred in the executable backend slice even though parser, HIR, and literal execution recognize these literal families.
- Diagnostics must preserve the user's raw numeric spelling for all literal families.

## 6.3 Closed types

- no `null` inhabitants unless represented explicitly in an ADT
- records are closed by default
- sums are closed by default
- missing or extra decoded fields are errors by default
- exhaustiveness checking is available for closed sums

## 6.4 Product types and data constructors

```aivi
type Vec2 = Vec2 Int Int
type Date = Date Year Month Day
```

### 6.4.1 Term-level constructor semantics

Every non-record ADT constructor is an ordinary curried value constructor.

```aivi
type Result E A = Err E | Ok A

value ok  = Ok
value one = Ok 1
```

Under-application is legal. Exact application constructs the value. Over-application is a type error. Applies to both unary and multi-argument constructors.

### 6.4.2 Record construction

Records are built with record literals, not implicit curried constructors.

```aivi
type User = { name: Text, age: Int }

value u:User = { name: "Ada", age: 36 }
```

### 6.4.3 Opaque and branded types

Opaque or branded types are recommended for domain-safe wrappers such as `Year`, `Month`, `Path`, `Url`, `Color`, and `Duration`. Public unary constructors are appropriate only when constructor application is intentionally part of the surface API.

## 6.5 Sum types

```aivi
type Bool = True | False

type Option A =
  | None
  | Some A
```

Nested constructor patterns are allowed. Exhaustiveness is required for sum matches unless a wildcard is present.

## 6.6 Records, tuples, and lists

```aivi
(1, 2)
{ name: "Ada", age: 36 }
[1, 2, 3]
```

- tuples: positional products
- records: named products
- lists: homogeneous sequences

## 6.7 Maps and sets

```aivi
Map { "x": 1, "y": 2 }
Set [1, 2, 4]
```

- plain `{ ... }` is always a record
- plain `[ ... ]` is always a list
- duplicate record fields are a compile-time error
- duplicate map keys are a compile-time error
- duplicate set entries are allowed but may be warned and deduplicated

---

## 7. Core abstraction model

Core typeclasses are compiler-owned ambient prelude items injected into every checked module; local declarations may shadow them.

Constraint syntax shared across class heads, members, functions, and instance heads:

```aivi
class Functor F
    map : (A -> B) -> F A -> F B

class Functor F => Applicative F
    pure  : A -> F A
    apply : F (A -> B) -> F A -> F B

class (Eq A, Show A) => Example A
    render : A -> Text
```

`Constraint => ...` attaches a single constraint; `(C1, C2) => ...` attaches multiple.

### 7.1 Resolution rules

- instance resolution is coherent
- overlapping instances are not allowed
- orphan instances are **fully disallowed**
- instance search is compile-time only
- user-authored instance lookup is currently same-module only; imported user instances remain deferred
- unary `instance` blocks with indented member bindings are the implemented surface; instance contexts are deferred
- instance bodies are checked directly against the class-member arrow types with explicit local parameter bindings

### 7.1.1 Overloaded term lookup

Class members are overloaded term candidates. Ambient-prelude and same-module class members enter term lookup; evidence selection is driven by concrete argument/result types that the checker can prove locally.

Constraints:

- evidence must be concrete enough for checked HIR to choose a member
- imported polymorphic class-member execution remains deferred
- unresolved or multiply valid candidates are diagnosed explicitly

### 7.1.2 Lowering strategy

Checked HIR records the chosen class member, subject binding, and evidence source explicitly.

Typed core lowers the builtin runtime-supported class-member surface to intrinsic references for:

- `map`
- `pure`
- `apply`
- `reduce`
- `append`
- `empty`
- `compare`
- structural equality

Same-module instance members lower as hidden callable items per `(instance, member)`. Overloaded references point to those hidden callables.

### 7.2 Core instances

- `Option` implements `Functor`, `Applicative`, `Monad`
- `Result E` implements `Functor`, `Applicative`, `Monad`
- `List` implements `Functor`, `Applicative`, `Monad`
- `Task E` implements `Functor`, `Applicative`, `Monad`
- `Signal` implements `Functor`, `Applicative`
- `Validation E` implements `Functor`, `Applicative`
- `Eq` is compiler-provided for the structural cases in §7.3
- `Default` participates through the same evidence machinery used for other constraints

### 7.2.1 `Foldable.reduce`

`Foldable.reduce` is the compiler-provided reduction surface for builtin collection/error carriers:

- `List A`: folds left-to-right in source order
- `Option A`: `None` returns the seed unchanged; `Some x` applies the step once
- `Result E A`: `Err _` returns the seed unchanged; `Ok x` applies the step once
- `Validation E A`: `Invalid _` returns the seed unchanged; `Valid x` applies the step once

No `Foldable Task` or `Foldable Signal` instance in v1.

### 7.3 Equality

```aivi
class Eq A
    (==) : A -> A -> Bool
```

`Eq` uses the ordinary class/instance resolution rules in §7.1. Compiler-derived and builtin evidence covers the executable surface; user-authored `Eq` instances beyond same-module explicit evidence remain deferred.

Compiler-derived `Eq` is required for:

- primitive scalars: `Int`, `Float`, `Decimal`, `BigInt`, `Bool`, `Text`, `Unit`
- tuples whose element types are `Eq`
- closed records whose field types are `Eq`
- closed sums whose constructor payload types are all `Eq`
- constructor-headed product declarations through the same closed-sum rule
- `List A` and `Option A` when `A` is `Eq`
- `Result E A` and `Validation E A` when both `E` and `A` are `Eq`
- domains whose underlying carrier supports `Eq`, preserving domain identity

Derived equality is structural and type-directed:

- tuple equality: position-by-position
- record equality: fieldwise over the declared closed field set
- sum equality: constructor tags first, then constructor payloads
- list equality: length- and order-sensitive
- primitive scalar equality: same-type only; not coercive or approximate

`Eq` is not compiler-derived for `Bytes`, `Map`, `Set`, `Signal`, `Task`, function values, GTK/foreign handles, or other runtime-managed boundary types whose equality semantics have not yet been specified.

### 7.4 Non-instances

`Signal` is **not** a `Monad`. Rationale: monadic signals imply dynamic dependency rewiring, complicating graph extraction, scheduling, teardown, and diagnostics. AIVI requires a static, explicit, topologically scheduled signal graph.

`Validation E` is **not** a `Monad`. The intended accumulation semantics are applicative rather than dependent short-circuiting.

### 7.5 Laws

Normative for lawful instances:

- `Eq`: reflexivity, symmetry, transitivity
- `Functor`: identity, composition
- `Applicative`: identity, homomorphism, interchange, composition
- `Monad`: left identity, right identity, associativity

The compiler is not required to prove these laws.

---

## 8. Validation

```aivi
type Validation E A =
  | Invalid (NonEmptyList E)
  | Valid A
```

Unlike `Result E A`, the applicative instance accumulates independent errors instead of short-circuiting on the first failure.

### 8.1 Applicative semantics

- `pure x` yields `Valid x`
- `Valid f` applied to `Valid x` yields `Valid (f x)`
- `Invalid e` applied to `Valid _` yields `Invalid e`
- `Valid _` applied to `Invalid e` yields `Invalid e`
- `Invalid e1` applied to `Invalid e2` yields `Invalid (e1 ++ e2)` where `++` is `NonEmptyList` concatenation

### 8.2 Intent

`Validation` is the canonical carrier for form validation under `&|>` because inputs are independent and all failures must be reported together.

```aivi
signal validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

All validators succeed → `Valid (UserDraft ...)`; one or more fail → all errors accumulated into one `Invalid` value in source order.

`Validation E` is applicative-only. Dependent validation requiring earlier successful values to choose later checks should use `Result`, `Task`, or explicit pattern matching.

---

## 9. Defaults and record omission

Defaulting is explicit and scoped. It does not make records open.

### 9.1 Default class

```aivi
class Default A
    default : A
```

### 9.2 `aivi.defaults`

`use aivi.defaults (Option)` brings into scope:

```aivi
Default (Option A)
default = None
```

### 9.3 Record literal elision

When an expected closed record type is known, omitted fields are filled only if a `Default` instance is in scope for every omitted field type.

```aivi
type User = {
    name: Text,
    nickname: Option Text,
    email: Option Text
}

use aivi.defaults (Option)

value user:User = { name: "Ada" }
```

Elaborates to:

```aivi
value user:User = {
    name: "Ada",
    nickname: None,
    email: None
}
```

### 9.4 Record shorthand

When an expected closed record type is known, a field whose label and in-scope value name coincide may be written in shorthand:

```aivi
value game:Game = {
    snake,
    food,
    status,
    score
}
```

Elaborates to `{ snake: snake, food: food, status: status, score: score }`.

Shorthand is allowed in record patterns:

```aivi
game
 ||> { snake, food, status, score } => score
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

This does **not**:

- open records
- change pattern matching semantics
- weaken strict source decoding
- add runtime fallback guessing

---

## 10. Expression model and control flow

### 10.1 No `if` / `else`

Branching uses pattern matching or predicate-gated flow.

### 10.2 No loops

Repetition is expressed through:

- recursion
- collection combinators
- source/retry/interval flows
- controlled recurrent pipe forms

### 10.3 Ambient subject

Within a pipe:

- `_` — entire current subject
- `.field` — projects from the current subject
- `.field.subfield` — chains projection
- `.field` is illegal where no ambient subject exists

---

## 11. Pipe algebra

## 11.1 Operators

Core operators:

- ` |>` transform
- `?|>` gate
- `||>` case split / pattern match
- `!|>` validate
- `~|>` previous state
- `+|>` accumulate state
- `-|>` diff
- `*|>` map / fan-out
- `&|>` applicative cluster stage
- `T|>` / `F|>` boolean branch
- `@|>` recurrent flow start
- `<|@` recurrence step
- ` | ` tap
- `<|*` fan-out join

Ordinary expression precedence (tighter to looser):

1. function application
2. binary `+` and `-`
3. binary `>`, `<`, `==`, `!=`
4. `and`
5. `or`

Operators at the same binary precedence associate left-to-right. Prefix `not` applies to its following ordinary expression before binary reassociation.

Pipe operators are **not** part of the binary table. A pipe spine starts from one ordinary expression head, then consumes pipe stages left-to-right. Each stage payload is parsed as an ordinary expression until the next pipe operator boundary.

Reactivity comes from `signal` and `source`, not from pipe operators. Pipe operators are flow combinators inside reactive or ordinary expressions.

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

Signal semantics: `True` → forwarded; `False` → suppressed; result type remains `Signal A`; no synthetic negative update is emitted.

Ordinary-value semantics: `A` → `Option A`; success yields `Some subject`; failure yields `None`.

```aivi
user
 ?|> .active
 T|> .email
 F|> "inactive"
```

Restrictions:

- the predicate must be pure
- `?|>` is not a general branch operator; use `||>` when the two paths compute unrelated shapes
- `?|>` does not inspect prior history or future updates; it is pointwise over the current subject

### 11.4 `||>` case split

Pattern matching over the current subject.

```aivi
status
 ||> Paid    => "paid"
 ||> Pending => "pending"
```

List patterns are structural and ordered. They match a left-to-right prefix and may bind the
remaining suffix as another list:

```aivi
xs
 ||> []                       => 0
 ||> [first]                  => first
 ||> [first, second, ...rest] => first + second + sum rest
```

Rules:

- `...rest` is list-only and must be the final segment in the pattern
- fixed positions bind at the element type; `...rest` binds at the full `List A` type
- the current rollout does not add dedicated list exhaustiveness reasoning; use `_` when an
  explicit catch-all is required

### 11.4.1 `T|>` and `F|>` truthy / falsy branching

Surface sugar over `||>`; elaborates deterministically.

Boolean:

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

`Option`:

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

`Result`:

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

Canonical truthy / falsy constructor pairs:

- `True` / `False`
- `Some _` / `None`
- `Ok _` / `Err _`
- `Valid _` / `Invalid _`

A single outer `Signal` lift is implemented: `Signal Bool`, `Signal (Option A)`, `Signal (Result E A)`, and `Signal (Validation E A)` apply the same carrier plan pointwise, then re-wrap as `Signal`.

Rules:

- `T|>` and `F|>` may appear only as an adjacent pair within one pipe spine
- the subject type must have a known canonical truthy / falsy pair
- `_` is rebound to the matched payload inside `T|>` or `F|>` when the constructor has exactly one payload
- zero-payload cases (`True`, `False`, `None`) do not introduce a branch payload
- non-canonical inner carriers under `Signal` are rejected
- use `||>` when named binding, nested patterns, or more than two constructors are required

### 11.5 `*|>` map / fan-out

Maps over each element of a collection.

```aivi
users
 *|> .email
```

Typing and lowering rules:

- for `List A`: maps `A -> B` to produce `List B`
- for `Signal (List A)`: fan-out is lifted pointwise to produce `Signal (List B)`
- the body is typed as if it were a normal pipe body with the element as ambient subject
- the outer collection is not implicitly ambient inside the body

`*|>` is pure mapping only. It does not implicitly flatten nested collections, sequence `Task`s, or merge nested `Signal`s.

### 11.5.1 `<|*` fan-out join

Joins the collection produced by the immediately preceding `*|>` with an explicit reducer.

```aivi
users
 *|> .email
 <|* Text.join ", "
```

`xs *|> f <|* g` elaborates to `g (map f xs)`. For `Signal (List A)`, lifted pointwise over signal updates.

Restrictions:

- `<|*` is legal only immediately after a `*|>` segment
- the join function is explicit; no implicit flattening or default join

### 11.6 `|` tap

Observes the subject without changing it.

```aivi
value
 |> compute
 |  debug
 |> finish
```

The tap body is evaluated with the current subject as ambient subject. Its result is ignored. The outgoing subject is exactly the incoming subject. `x | f` behaves as `let _ = f x in x`.

`|` is intended for tracing, metrics, and named observers. It is not a hidden mutation or control-flow channel.

### 11.7 `@|>` and `<|@`

Mark explicit recurrent flows for retry, polling, and stream-style pipelines.

`@|>` enters a recurrent region. Each `<|@` stage contributes to the per-iteration step function over the current loop state. A recurrent spine denotes a scheduler-owned loop node rather than direct self-recursion.

Normative rules:

- recurrent pipes are legal only where the compiler can lower them to a built-in runtime node for `Task` or an explicit scheduler-owned `Signal` recurrence
- recurrence wakeups must be explicit: timer, backoff, source event, or provider-defined trigger
- each iteration is scheduled and stack-safe; recurrent pipes must not lower to unbounded direct recursion
- cancellation or owner teardown disposes the pending recurrence immediately
- recurrent pipes with no valid runtime lowering target are rejected
- ordinary source-driven signal state accumulation does not use `@|>` / `<|@`; it uses `+|>`

### 11.8 `!|>` validate

Applies a validation function to the current subject. The validation function must return `Result E B` or `Validation E B`.

```aivi
formInput
 !|> validateEmail
 !|> validateNotEmpty
  |> submitEmail
```

`!|>` is for attaching validation stages to a pipe spine. Multiple `!|>` stages on the same carrier type accumulate errors when the carrier is `Validation`; they short-circuit on the first failure when the carrier is `Result`.

Rules:

- the validation function must have shape `A -> Result E B` or `A -> Validation E B`
- result type carries the validated output type `B`
- signal semantics: validation is applied pointwise per upstream emission

### 11.9 `~|>` previous state

Reads the previously committed value of the upstream signal. Useful for comparing before/after, detecting transitions, or combining current and prior state.

```aivi
temperature
 ~|> prevTemp
  |> delta prevTemp
```

Rules:

- `~|>` binds the previous committed value as a named parameter
- the first emission has no previous value; the stage is suppressed on the first tick (no synthetic prior is invented)
- `~|>` is read-only over the prior epoch; it does not create mutable state

### 11.10 `+|>` accumulate

Accumulates state across signal emissions. See §13.2.1 for the full normative description.

```aivi
signal counter : Signal Int =
    tick +|> 0 (state _ => state + 1)

signal history : Signal (List Text) =
    message +|> [] (acc msg => acc ++ [msg])
```

Form: `signal +|> seed (state input => next)`

- `seed` is the initial state value
- `state` is the previous accumulated value
- `input` is the current upstream emission
- `next` is the new state expression

Shorthand: `signal +|> prev + .` where `prev` is the previous state and `.` is the current value.

### 11.11 `-|>` diff

Computes the structural or semantic difference between the current and previous emission. Returns a diff value whose shape depends on the carrier type.

```aivi
items
 -|> ListDiff.changes
  |> applyChanges
```

Rules:

- the diff function receives `(previous: A, current: A) -> D` where `D` is the diff type
- the first emission produces a diff from an empty/zero baseline
- `-|>` is the primary operator for driving localized update pipelines

---

## 12. Exact applicative surface semantics for `&|>`

## 12.1 Intent

`&|>` is the surface operator for **applicative clustering**: combining independent effectful/reactive values under a shared `Applicative` and then applying a pure constructor or function.

Intended for:

- form validation
- combining independent signals
- assembling values from independent `Option`, `Result`, `Validation`, or `Task` computations

Not:

- monadic sequencing
- short-circuit imperative flow
- ad-hoc tuple syntax

## 12.2 Surface forms

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
signal validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

This form is preferred when scanning a validation spine for independence.

## 12.3 Grammar shape

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

All cluster members must have the same outer applicative constructor `F` — e.g. `Validation FormError A`, `Signal A`, `Option A`, `Result HttpError A`, `Task FsError A`. All members in one cluster must be of shape `F Ai` for the same `F`.

## 12.5 Desugaring

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

equivalent to `apply (apply (apply (pure f) a) b) c`. The leading form desugars the same way.

## 12.6 End-of-cluster default

A cluster reaching pipe end without an explicit finalizer finalizes to a tuple constructor of matching arity:

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

## 12.7 Restrictions

Inside an unfinished applicative cluster:

- ambient-subject projections such as `.field` are illegal unless inside a nested expression with an explicit subject
- `?|>` and `||>` are illegal until the cluster is finalized
- the finalizer must be a pure function or constructor

## 12.8 Examples

### Validation

```aivi
signal validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

### Signals

```aivi
signal fullName =
 &|> firstName
 &|> lastName
  |> joinName
```

### Result

```aivi
value loaded =
 &|> readConfig path
 &|> readSchema schemaPath
  |> buildRuntimeConfig
```

## 12.9 `Signal` interaction

For `Signal`, `&|>` builds a derived signal whose dependencies are the union of member dependencies, observing the latest stable upstream values per scheduler tick. This is applicative combination, not monadic binding.

---

## 13. Signals and scheduler semantics

```aivi
signal x = 3
signal y = x + 5
```

A signal referenced inside a `signal` is read as its current committed value during evaluation. The enclosing `signal` depends on every **locally provable** signal referenced in its definition.

### 13.1 Rules

- `signal` is the reactive boundary
- `value` must not depend on signals
- pure helper functions used inside `signal` stay pure
- signal dependency extraction happens after elaboration
- ordinary derived-signal dependency graphs are static after elaboration
- all signals carry explicit local dependency lists for scheduling and diagnostics
- source-backed signals record local signal dependencies only; imported references are not assumed publishable signals unless the compiler has explicit proof

### 13.2 Input signals

A body-less annotated `signal` declaration is a first-class input signal — an externally publishable entry point for reactive inputs such as GTK events, tests, and runtime-owned completions.

```aivi
signal clicked : Signal Unit
signal query   : Signal Text
```

Type annotation is mandatory. Input signals participate in the signal dependency graph exactly like derived signals; their publication port is owned by the runtime rather than user code.

Input signals are the canonical mechanism for routing GTK event payloads into the reactive graph and the publication target for task completions and other runtime-owned boundaries.

When a `signal` has no body, the source owns only the raw event stream; stateful accumulation over that stream is expressed separately using `+|>`.

### 13.2.1 Stateful signal accumulation with `+|>`

`+|>` is the canonical accumulate pipe for building stateful signals:

```aivi
signal direction : Signal Direction =
    keyDown +|> Right (direction key => updateDirection key direction)

signal game : Signal Game =
    tick +|> initialGame (state _ => stepOnTick state)
```

Normative rules:

- surface form: `signal +|> seed (state input => next)`
- `signal` must elaborate to a signal whose payload type is `A`
- `seed` has state type `S` and is the first committed value
- the lambda `(state input => next)` takes the previous state first and the current upstream value second
- each later update is scheduled only when the upstream signal publishes a new value in the current scheduler tick
- `+|>` is the intended way to accumulate timer, event, source, and completion signals into state

Shorthand form for simple arithmetic accumulation:

```aivi
signal counter : Signal Int =
    tick +|> 0 (prev + 1)
```

`prev` refers to the previous state; `.` refers to the current upstream value.

### 13.3 Applicative meaning of `Signal`

`pure x` creates a constant signal.

`apply : Signal (A -> B) -> Signal A -> Signal B` creates a derived signal with:

- dependency set equal to the union of input dependencies
- latest-value semantics
- transactional visibility per scheduler tick
- glitch-free propagation

Dynamic rewiring must be expressed through explicit runtime/source nodes, not through `bind`.

### 13.4 Scheduler guarantees

The runtime scheduler must provide:

- topological propagation order
- committed-snapshot evaluation per tick
- no mixed-time intermediate observations
- deterministic behavior for a fixed input event order
- generation-stamped publication so stale source/task results are rejected before propagation
- recursive owner disposal so torn-down subtrees deactivate their dependent runtime-owned nodes

The scheduler is driven from an owned GLib main context. Workers may publish results and request wakeups but do not mutate scheduler-owned state directly.

### 13.5 No `Monad Signal`

`bind` is not exposed for `Signal`. Any feature implying dynamic dependency rewiring must be expressed through explicit source/runtime nodes.

---

## 14. Sources and decoding

External inputs are declared with `source`, which combines the provider binding and type annotation into a single declaration:

```aivi
source users : Signal (Result HttpError (List User)) = http.get "/users"
```

Source arguments and options are ordinary typed expressions. They may use interpolation and may depend on signals with statically known dependency sets.

```aivi
source users : Signal (Result HttpError (List User)) =
    http.get "{baseUrl}/users" with {
        headers: authHeaders,
        decode: Strict
    }
```

Reactive values in source strings, positional arguments, and options are real dependencies. When committed values change, the runtime rebuilds or retriggers the source per the provider contract while keeping the static graph shape fixed.

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
- GTK/window events

The HIR surface preserves for every `source` declaration site:

- provider identity: missing / builtin / custom / invalid-shape
- positional arguments as runtime expressions
- options as runtime expressions
- lifecycle metadata
- decode program selection
- stable source instance identity

### 14.1.1 Recurrence decorators on non-source declarations

```aivi
@recur.timer 1000ms
signal polled : Signal Status

@recur.backoff initialDelay
signal retried : Signal (Result FetchError Data)
```

Rules:

- `@recur.timer expr` and `@recur.backoff expr` are the only recurrence decorators for non-`source` declarations
- neither accepts `with { ... }` options or duplicates
- not allowed on `source` declarations; source wakeups come from the source contract
- a recurrent pipe is legal only where the compiler can prove a built-in runtime lowering target
- recurrence lowering produces an explicit scheduler-node handoff; it is not collapsed into opaque self-recursion

### 14.1.2 Source declaration shape

```aivi
source name : Signal T = provider.variant arg1 arg2 with {
    option1: value1,
    option2: value2
}
```

The `with { ... }` option record is optional.

```aivi
source tick : Signal Unit = timer.every 120
```

```aivi
source users : Signal (Result HttpError (List User)) =
    http.get "/users" with {
        decode: Strict,
        retry: Retry.times 3,
        timeout: 5sec
    }
```

Rules:

- provider and variant are resolved statically
- `source` declarations are the primary form for provider-backed signals
- positional arguments are provider-defined and typed
- options are a closed record whose legal fields come from a central provider option catalog
- unknown options are a compile-time error
- duplicate options are a compile-time error
- value checking is staged: the compiler validates supported local closed shapes and records explicit blockers for unsupported or unproven forms
- argument and option expressions may be ordinary values or signal-derived expressions with statically known dependencies
- reactive changes are split into three lifecycle classes: reconfiguration inputs, trigger/refresh inputs, and `activeWhen` gating inputs
- reconfiguration input change: old runtime instance is superseded and a new one created with a fresh generation
- imported option bindings are checked only when the import catalog provides an explicit closed value surface

Reactive source configuration does not make sources dynamic in the type-theoretic sense. Provider kind and dependency graph remain statically known; only runtime configuration values change.

Stateful source handling is expressed by deriving from the raw source signal:

```aivi
source tick : Signal Unit = timer.every 120

signal counter : Signal Int =
    tick +|> 0 (state _ => state + 1)
```

### 14.1.3 Recommended source variants

#### HTTP

```aivi
source users : Signal (Result HttpError (List User)) = http.get "/users"

source login : Signal (Result HttpError Session) =
    http.post "/login" with {
        body: creds,
        headers: authHeaders,
        decode: Strict,
        timeout: 5sec
    }
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

- refresh is explicit only: reactive config changes, `refreshOn`, `refreshEvery`, retries, or provider-defined intrinsic wakeups
- no lifecycle-event refreshes hidden behind GTK visibility or mount/unmount
- `refreshOn` reissues the request whenever the trigger signal updates
- `refreshEvery` creates scheduler-owned polling using the latest stable source configuration
- `activeWhen` gates startup and refresh; `False` suspends polling and makes the current generation inactive
- reactive URL, query, header, or body changes create a replacement request generation from latest committed values
- newer request generations supersede older ones; stale completions from superseded generations are dropped
- built-in HTTP providers request best-effort cancellation of superseded or suspended requests

#### Timer

```aivi
source tick  : Signal Unit = timer.every 120
source ready : Signal Unit = timer.after 1000
```

Recommended timer options:

- `immediate : Bool`
- `jitterMs : Int`
- `coalesce : Bool`
- `activeWhen : Signal Bool`

Bare integer timer arguments mean milliseconds. Suffixed durations such as `250ms` are accepted when the duration domain surface is in place.

#### File watching and reading

```aivi
source fileEvents : Signal FsEvent =
    fs.watch "/home/user/demo.txt" with {
        events: [Created, Changed, Deleted]
    }

source fileText : Signal (Result FsError Text) =
    fs.read "/home/user/demo.txt" with {
        decode: Strict,
        reloadOn: fileEvents
    }
```

`fs.watch` publishes file-system change notifications only; it does **not** implicitly read file contents. `fs.read` performs snapshot loading and decode. This split is normative.

Recommended file-watch options: `events : List FsWatchEvent`, `recursive : Bool`

Recommended file-read options: `decode : DecodeMode`, `reloadOn : Signal A`, `debounce : Duration`, `readOnStart : Bool`, `activeWhen : Signal Bool`

Built-in file sources request best-effort cancellation when superseded, suspended, or torn down.

#### Socket / mailbox

```aivi
source inbox : Signal (Result SocketError Message) =
    socket.connect "tcp://localhost:8080" with {
        decode: Strict
    }

source jobs : Signal Text = mailbox.subscribe "jobs"
```

- `socket.connect` is a raw `tcp://` line-stream provider, not a general WebSocket surface
- `mailbox.subscribe` is a process-local text bus
- unsupported options raise explicit runtime errors at provider registration

#### Process events

```aivi
source grepEvents : Signal ProcessEvent = process.spawn "rg" ["TODO", "."]
```

Recommended process options: `cwd : Path`, `env : Map Text Text`, `stdout : StreamMode`, `stderr : StreamMode`, `restartOn : Signal A`

#### GTK / window events

```aivi
source keyDown : Signal Key = window.keyDown with { repeat: False }
```

Recommended window-event options: `capture : Bool`, `repeat : Bool`, `focusOnly : Bool`

`window.keyDown` is lowered through the focused window's key controller. This is a provider-owned host boundary, not a generic DOM-like event model.

#### D-Bus

```aivi
source busName   : Signal BusNameState = dbus.ownName "org.example.Mail"
source busEvents : Signal MailBusEvent = dbus.signal "org.example.Mail" "/org/example/Mail" "NewMessage"
source showWindow : Signal Unit         = dbus.method "org.example.Mail" "/org/example/Mail" "ShowWindow"
```

- `dbus.ownName`: tracks `Owned`, `Queued`, or `Lost`
- `dbus.signal`: subscribes to inbound remote signals
- `dbus.method`: fire-and-forget inbound method dispatch; Unit reply is placed on the D-Bus wire before the AIVI signal is published
- non-Unit reply-producing methods are deferred

### 14.1.4 Decode and delivery modes

```aivi
type DecodeMode =
  | Strict
  | Permissive

type StreamMode =
  | Ignore
  | Lines
  | Bytes
```

- `Strict`: rejects unknown or missing required fields per closed-type decoding rules
- `Permissive`: may ignore extra fields but still requires required fields unless the built-in decode surface says otherwise
- decode happens before scheduler publication
- delivery into the scheduler remains typed and transactional

### 14.2 Decoding

Default decoding rules:

- closed records reject missing required fields
- extra fields are rejected in strict mode by default
- sum decoding is explicit
- decoder overrides are limited to the built-in decode surface; general custom decode hooks remain deferred
- domain-backed fields decode through the domain's explicit parser or constructor surface; they do not silently accept the raw carrier unless that surface says so

Runtime decode wire shape:

- payload bytes are interpreted as UTF-8 text for providers that promise text transport
- plain `Text` targets accept the raw text unchanged
- structural targets decode from JSON
- closed sums, `Option`, `Result`, and `Validation` use canonical JSON shape `{ tag, payload }`
- JSON-backed scalar targets decode through explicit wire contracts:
  - `Float` from JSON numbers
  - `Decimal` from JSON strings carrying canonical decimal literals like `"19.25d"`
  - `BigInt` from JSON strings carrying canonical bigint literals like `"123n"`
  - `Bytes` from JSON arrays of integer octets like `[104, 105]`
- domain-surface direct transport still fails explicitly at provider registration

Domain decode resolution order:

1. a domain-owned `parse` method with shape `Carrier -> Result E Domain`
2. otherwise, a unique domain-owned `Carrier -> Domain` or `Carrier -> Result E Domain`
3. otherwise, decode is rejected as ambiguous or unsupported

Operator methods, literal methods, and multiply matching domain conversions are not decode surfaces.

Record default elision for user-written literals does **not** weaken source decoding. Decode failures flow through the source's typed error channel; they do not escape as untyped runtime exceptions.

Regex literals are validated in HIR validation, not delegated to source providers.

### 14.3 Cancellation and lifecycle

Every `source` declaration owns one stable runtime instance identity.

Lifecycle rules:

- lifecycle metadata distinguishes reactive reconfiguration, trigger, and `activeWhen` inputs
- reconfiguration caused by reactive source arguments or options replaces the superseded runtime resource transactionally from committed scheduler values
- stale work from a superseded, disposed, or inactive source generation is dropped and must never publish into the live graph
- `activeWhen` suspends delivery without changing the static graph shape
- request-like built-ins (HTTP, `fs.read`) request best-effort in-flight cancellation when replaced, suspended, or disposed
- built-in `SourceRuntimeSpec` values are validated against provider contracts at registration
- custom providers inherit the generic replacement and stale-publication rules; built-in option names have semantics only when the provider contract declares them

### 14.4 Custom provider declarations

```aivi
provider my.data.source
    wakeup: providerTrigger
    argument url: Url
    option timeout: Duration
    option retries: Int
```

Implemented declaration rules:

- the provider name is a qualified top-level name
- `wakeup:` may currently be `timer`, `backoff`, `sourceEvent`, or `providerTrigger`
- unknown declaration fields are immediate diagnostics
- argument and option declarations are restricted to primitive types, same-module types, `List`, and `Signal` compositions over those closed shapes
- richer schemas are rejected at declaration time
- reactive source inputs always count as `sourceEvent` wakeups for any provider; non-reactive custom wakeups must be declared explicitly

---

## 15. Effects and `Task`

## 15.1 Purity boundary

Ordinary `value` definitions are pure.

Effects enter through:

- `Task`
- `signal` / `source`
- GTK event boundaries
- runtime-owned scheduling and source integration

## 15.2 `Task E A`

`Task E A` is the only user-visible one-shot effect carrier.

- describes a one-shot effectful computation
- may fail with `E`
- may succeed with `A`
- is schedulable by the runtime
- lawful as `Functor`, `Applicative`, and `Monad`

Runtime execution uses linked task bindings plus scheduler-owned hidden completion inputs. A direct top-level task value lowers to a `TaskRuntimeSpec`; a worker thread evaluates the linked backend item body and publishes its result through a typed completion port back into the scheduler.

Recurrent `@|> ... <|@` tasks are outside the current executable slice and remain explicit runtime blockers.

## 15.3 Event handler routing

In v1 live GTK routing:

- markup `on*={handler}` attributes are routing declarations, not arbitrary callback bodies
- `handler` must resolve to a directly publishable input signal declared as a body-less annotated `signal name : Signal T`
- the concrete GTK host must recognize the exact widget/event pair before the attribute is treated as live event routing
- the routed input signal payload type must match the concrete GTK event payload type
- handler resolution is performed once up front; GTK event payloads are then published directly into that input signal
- discrete GTK events publish one payload into the scheduler input signal and force their own runtime tick

Broader normalization of arbitrary handler expressions remains future work.

## 15.4 Inter-thread communication

Workers receive read-only cancellation observers and may publish source/task results back to the scheduler queue. They do not mutate GTK state or committed signal storage directly.

Library-level message-passing primitives:

```aivi
type Sender A
type Receiver A
type Mailbox A
```

Sending expressed through `Task`; receiving expressed through `source` integration.

---

## 16. Runtime architecture

## 16.1 Memory management

Target runtime: mostly-moving generational collector with incremental scheduling plus narrow stable-handle support at foreign boundaries.

Language-visible guarantees:

- ordinary values may move
- stable addresses are not guaranteed
- GTK/GObject/FFI interactions use stable handles, pinned wrappers, or copied values
- values crossing GTK, worker, source-provider, or other foreign seams use explicit detached boundary wrappers or ports; boundary detachment is never implicit

Initial GC rollout: only scheduler-committed runtime snapshots are in the moving-GC root set. Pending evaluator/source/task results remain ordinary Rust-owned values until commit.

## 16.2 Threads

Recommended runtime shape:

- one GTK UI island on an owned GLib main context
- worker threads for I/O, decoding, task execution, and heavy fragment evaluation
- immutable message passing from workers to scheduler-owned queues
- no direct GTK mutation from workers

The GLib driver reentry rule: scheduler/evaluator ownership sits behind one guarded critical section; same-thread reentry is a runtime invariant violation.

## 16.3 Scheduler

The scheduler owns:

- signal propagation
- source event ingestion
- task completion publication
- cancellation/disposal
- tick boundaries
- committed runtime snapshots

The scheduler must not:

- block the GTK main loop during heavy work
- deadlock on normal cross-thread publication
- recurse unboundedly during propagation
- leak torn-down subscriptions
- accept stale publications from superseded generations

Committed scheduler state is the source of truth. Worker-computed results are admitted only at tick-safe boundaries.

---

## 17. GTK / libadwaita embedding

The pure language core remains pure. UI effects cross a controlled boundary through a typed GTK bridge.

## 17.1 View model

AIVI uses typed markup-like view syntax and lowers it to a stable widget/binding graph. It does **not** use a virtual DOM.

### 17.1.1 Direct lowering rules

- HIR markup lowers to a typed `WidgetPlan` with stable identities, child operations, setter bindings, event hookups, and control branches
- `WidgetPlan` lowers to a `WidgetRuntimeAssembly` with concrete runtime handles and child-group structure
- the GTK executor consumes that runtime assembly through a bridge graph and applies direct GTK mutations

Each markup node compiles to:

- widget/control-node kind
- static property initializers
- dynamic property bindings
- signal/event handlers
- child-slot instructions
- teardown logic

Ordinary widget nodes are created once per node identity. Dynamic props update through direct setter calls. No generic diff engine over a virtual tree.

Live `aivi run` updates: the runtime snapshots committed globals on the main thread, evaluates selected view fragments on a worker, produces an immutable hydration plan, and applies GTK mutations back on the main thread via `idle_add`.

## 17.2 Property and event binding

```aivi
<Label text={statusLabel order} visible={isVisible} />
```

If an expression is reactive, the compiler extracts a derived signal and the runtime:

- computes the initial value
- subscribes once
- calls the concrete GTK setter on change

Interpolated markup text is genuinely dynamic. The GTK host routes interpolated text-valued attributes through runtime setter bindings.

### 17.2.1 Event hookups

Expression-valued markup attributes lower as live GTK event routes only when the widget schema catalog declares that exact widget/event pair.

```aivi
signal clicked : Signal Unit

<Button label="Click me" onClick={clicked} />
```

Event hookup rules:

- the handler expression must name a directly publishable input signal
- only direct input signals are legal; arbitrary callback expressions are future work
- the input signal's payload type must match the GTK event's concrete payload type
- unsupported event names on a given widget type remain ordinary attributes and are rejected by run-surface validation
- GTK discrete events force their own runtime ticks; rapid repeated events are processed as separate transactions

`on*` attributes are event-hook candidates only through this schema-backed rule. The host does not guess event semantics from spelling alone.

### 17.2.2 Executable widget schema metadata

One compiled widget schema catalog is shared by lowering, `aivi run` validation, and concrete GTK hookup.

Each widget schema entry defines:

- the current markup lookup key
- property descriptors: exact property name, semantic value shape, and GTK setter route
- event descriptors: exact event name, GTK signal route, and payload shape
- child-group descriptors: group name, container policy, and child-count bounds
- whether the widget is window-like for root validation/presentation

Unlabeled child content may populate only the schema's single default child group. Widgets needing multiple named child groups remain deferred.

Current executable catalog:

- `Window` — properties `title`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events; child group `content` accepting at most one child; treated as a window root
- `Box` — properties `orientation`, `spacing`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events; child group `children` with append-only sequence semantics
- `ScrolledWindow` — properties `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events; child group `content` accepting at most one child
- `Label` — properties `text`, `label`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events; no child groups
- `Button` — properties `label`, `visible`, `sensitive`, `hexpand`, `vexpand`; event `onClick` publishing `Unit`; no child groups
- `Entry` — properties `text`, `placeholderText`, `editable`, `visible`, `sensitive`, `hexpand`, `vexpand`; event `onActivate` publishing `Unit`; no child groups
- `Switch` — properties `active`, `visible`, `sensitive`, `hexpand`, `vexpand`; no markup events; no child groups

Widgets outside this catalog are not part of the current live GTK surface.

### 17.2.3 Host lifecycle attributes

`trackVisible={sig}` routes GTK `map` / `unmap` into a user-declared `Signal Bool` input signal.

Rules:

- the bound signal must be a body-less annotated `Signal Bool` input signal
- the host publishes `False` immediately at registration, `True` on first `map`, then `True` / `False` on later `map` / `unmap` transitions
- `map` / `unmap` is used rather than `show` / `hide` because a widget may be shown while not yet mapped through an unshown parent
- this is the canonical way to drive `source activeWhen` from visibility state

`hideOnClose={True}` on `ApplicationWindow` intercepts the delete event and calls `window.hide()` instead of destroying the window. This keeps the process alive and allows later restoration through normal presentation or D-Bus activation.

## 17.3 Control nodes

Control nodes are part of the view language and lower directly.

### 17.3.1 `<show>`

```aivi
<show when={isVisible}>
    <Label text="Ready" />
</show>
```

- `when` must be `Bool`
- `False`: subtree is absent
- `True`: subtree is present

Optional flag:

```aivi
<show when={isVisible} keepMounted={True}>
    ...
</show>
```

- `keepMounted = False` (default): `False` triggers full subtree teardown per §17.4
- `keepMounted = True`: subtree mounts once; hide/show becomes a visibility transition rather than unmount/remount; property bindings, signal subscriptions, source subscriptions, and event hookups remain installed while hidden; concrete input delivery while hidden follows the host toolkit — for the current GTK host, invisible widgets do not receive pointer or keyboard events even though their handlers remain connected

### 17.3.2 `<each>`

```aivi
<each of={items} as={item} key={item.id}>
    <Row item={item} />
</each>
```

- `of` must yield `List A`
- `as` binds the element within the body
- the body must produce valid child content for the parent slot
- `key` is required

Runtime behavior:

- child identity is maintained by key
- updates compute localized child edits rather than whole-tree replacement
- existing child subtrees are reused by key where possible
- GTK child insertion/removal/reordering happens directly

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

- `on` is any expression
- cases use ordinary AIVI patterns, including list patterns such as `[first, ...rest]`
- exhaustiveness follows ordinary match rules where the scrutinee type is locally provable
- lowering selects and deselects concrete subtrees directly

### 17.3.4 `<fragment>`

```aivi
<fragment>
    <Label text="A" />
    <Label text="B" />
</fragment>
```

Groups children without creating a wrapper widget.

### 17.3.5 `<with>`

```aivi
<with value={formatUser user} as={label}>
    <Label text={label} />
</with>
```

Introduces a pure local binding for the subtree. Does not create an independent signal node.

## 17.4 Teardown and lifecycle

Tearing down a subtree must:

- disconnect event handlers
- dispose source subscriptions owned by that subtree
- release widget handles
- preserve correctness under repeated show/hide and keyed list churn
- recursively deactivate owned runtime nodes so stale publications are rejected after teardown

GTK correctness is part of the language runtime contract.

---

## 18. Pattern matching and predicates

### 18.1 Rules

- sum matches must be exhaustive unless `_` is present
- boolean matches must cover `True` and `False` unless `_` is present
- record patterns may be field-subset patterns
- list patterns may match an exact prefix plus an optional final `...rest`
- nested constructor patterns are allowed
- ordered head/rest destructuring is defined for lists only; sets and maps do not use this syntax

### 18.2 Predicates

Predicates may use:

- ambient projections such as `.age > 18`
- `_` for the current subject
- `and`, `or`, `not`
- `==` / `!=` when an `Eq` instance is available for the operand type

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

Regex is a first-class compiled type with literal syntax:

```aivi
rx"\d{4}-\d{2}-\d{2}"
```

Invalid regex literals are compile-time errors. Validation happens in HIR validation; the compiler uses the Rust `regex-syntax` acceptance surface. This keeps the token stream lossless while making malformed regexes early, typed diagnostics.

---

## 20. Domains

Domains are nominal value spaces defined over an existing carrier type.

Use a domain when a value should:

- have the runtime representation of an existing type
- remain distinct at the type level
- optionally support domain-specific literal suffixes
- optionally expose domain-specific operators and smart constructors
- reject accidental mixing with the raw carrier or other domains over the same carrier

Typical examples: `Duration over Int`, `Url over Text`, `Path over Text`, `Color over Int`, `NonEmpty A over List A`.

A domain is not a type alias. A domain is not subtyping. A domain does not imply implicit casts.

### 20.1 Declaration form

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    millis      : Int -> Duration
    parse       : Int -> Result DurationError Duration
    value       : Duration -> Int
```

### 20.2 Core meaning

A domain introduces a nominal type over a carrier type while preserving explicit construction and elimination. The domain owns:

- literal suffixes
- smart construction
- explicit unwrapping
- domain-local operators
- optional decode/parse surfaces

### 20.3 Relation to opaque and branded types

Use `domain` when the nominal wrapper carries domain-owned literal, parsing, decode, or operator surfaces. Use `type` when an ordinary ADT or record suffices.

### 20.4 Construction and elimination

A domain may be introduced only through domain-owned constructors or smart constructors.

```aivi
domain Url over Text
    parse : Text -> Result UrlError Url
    value : Url -> Text

domain Duration over Int
    millis     : Int -> Duration
    trySeconds : Int -> Result DurationError Duration
    value      : Duration -> Int
```

Construction is explicit. Unwrapping is explicit. Unsafe construction should remain internal or be spelled as such.

Callable domain members enter ordinary term lookup when in scope. No projection syntax for domains in v1; the compiler may expose compiler-known qualified lookup surfaces such as `Duration.value` for disambiguation.

### 20.5 Literal suffixes

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    literal min : Int -> Duration
```

Enables `250ms`, `10sec`, `3min` as typed `Duration` values.

Literal-suffix rules:

- domain suffix names must be at least two ASCII letters long
- single-letter alphabetic suffixes are reserved for built-in numeric literal families
- compact `digits + suffix` is a suffix literal candidate; spaced forms are ordinary application
- only integer-family domain suffix literals are supported
- suffix resolution is compile-time only, against current-module domain literal declarations only
- no match is an error; more than one current-module match is an ambiguity error
- imported modules do not extend the literal-suffix search space

Examples:

- `250ms : Duration`
- `250 : Int`
- `250ms + 3min` is legal only if `Duration` defines `+`
- `250ms + 3` is illegal unless an explicit constructor or operator admits it

### 20.6 Domain operators

```aivi
domain Duration over Int
    literal ms : Int -> Duration
    (+)        : Duration -> Duration -> Duration
    (-)        : Duration -> Duration -> Duration
    (*)        : Duration -> Int -> Duration
    compare    : Duration -> Duration -> Ordering

domain Path over Text
    (/) : Path -> Text -> Path
```

Operator rules:

- operator resolution is static
- operators are not inherited from the carrier automatically
- operators must be declared by the domain or provided by explicit class evidence over the domain
- operators are type-checked before any fallback inference logic
- proven domain operators cross an explicit elaboration seam into typed core/backend; later layers do not rediscover them heuristically

### 20.7 Smart construction and invariants

Domains attach invariants stronger than the carrier type:

- `Url over Text`: may require URL parsing
- `Path over Text`: may normalize separators
- `Color over Int`: may require packed ARGB layout
- `NonEmpty A over List A`: may reject empty lists

```aivi
domain NonEmpty A over List A
    fromList : List A -> Option (NonEmpty A)
    head     : NonEmpty A -> A
    tail     : NonEmpty A -> List A
```

### 20.8 Parameterized domains

```aivi
domain ResourceId A over Text
domain NonEmpty A over List A
```

- parameters are ordinary type parameters
- kinds follow the ordinary kind system
- the carrier may use those parameters
- partial application is allowed when the resulting kind matches the expected constructor kind

### 20.9 Equality and instances

A domain does not automatically inherit all instances of its carrier.

- `Eq` may be compiler-derived for a domain if its carrier has `Eq` and the domain does not opt out
- domain identity is preserved even when equality is derived from the carrier's structure
- other class evidence is explicit unless separately declared

### 20.10 Runtime representation

A domain reuses its carrier runtime representation unless a later lowering layer documents a more specialized ABI. The nominal distinction is preserved in typing and diagnostics.

### 20.11 No implicit casts

Domains do not introduce implicit coercions to or from the carrier.

### 20.12 Diagnostics

Diagnostics should name the domain rather than erasing it to the carrier.

For literal/decode/operator failures, diagnostics should explain whether the failing surface was:

- unresolved suffix lookup
- ambiguous suffix or decode surface
- illegal raw-carrier use where a domain value was required
- missing domain operator or parser surface

### 20.13 Recommended examples

#### Duration

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    value       : Duration -> Int
    (+)         : Duration -> Duration -> Duration
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
    value : Path -> Text
    (/)   : Path -> Text -> Path
```

#### NonEmpty

```aivi
domain NonEmpty A over List A
    fromList : List A -> Option (NonEmpty A)
    head     : NonEmpty A -> A
    tail     : NonEmpty A -> List A
```

### 20.14 Design boundary

The implemented v1 domain slice:

- declarations, callable members, explicit construction/unwrapping, explicit decode surfaces, and domain-local operators are in scope
- literal suffixes are current-module integer-family surfaces only
- no implicit casts
- no projection syntax
- literal patterns remain on the existing integer/text-only slice; domain literal pattern widening is deferred

---

## 21. Diagnostics

Diagnostics must:

- identify the failed invariant
- point at the user-visible cause
- avoid leaking backend IR details unless requested in debug output
- suggest the intended construct when the misuse is obvious

Examples:

- using a signal in `value` should suggest `signal`
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

The formatter preserves and prefers the leading-cluster style when the spine is vertically scanned for independence.

```aivi
signal validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

This is a first-class canonical style.

---

## 23. Testing and hardening

Baseline implementation strategy:

- parser and decoder fuzzing in a standalone top-level `fuzz/` cargo-fuzz workspace
- stable corpus replay tests in ordinary CI so committed seeds are checked without requiring the `cargo fuzz` subcommand
- scheduler stress coverage that stays deterministic and in-process
- GTK subtree lifecycle tests
- stack-depth torture tests
- teardown/leak tests
- deterministic scheduling tests with generation-stamped publication scripts
- GLib wakeup and reentry tests

Decoder fuzzing is schema-owned: the fuzz target first parses and lowers source text, then executes only compiler-generated decode programs. Malformed inputs may fail, but failures must flow through typed decode errors whose field and variant names come from the generated schema.

Performance work is benchmark-gated. Each performance-oriented pass must land with:

- one checked-in corpus
- one machine-stable structural metric
- one release timing metric

Every bug fix must add a regression test naming the failed invariant.

---

## 24. Milestones

These milestones partition implementation work; they do not reduce scope.

Status legend: **COMPLETE** = fully implemented; **PARTIAL** = core slice implemented with known gaps; **PENDING** = not yet started.

### Milestone 1 — Surface and CST freeze — **COMPLETE**

- lexer ✓
- parser ✓
- CST (lossless for formatting and diagnostics) ✓
- formatter (canonical pipe, arrow, cluster alignment) ✓
- syntax for `type`, `data`, `class`, `instance`, `value`, `signal`, `source`, `result`, `view`, `adapter`, `use`, `export`, `provider`, markup, and pipe operators (`|>`, `?|>`, `||>`, `!|>`, `~|>`, `+|>`, `-|>`, `*|>`, `&|>`, `T|>`, `F|>`, `@|>`, `<|@`, `<|*`, `|`) ✓
- line/block/doc comment lexing (`//`, `/* */`, `/** **/`) and trivia retention ✓
- regex literal lexing plus HIR validation ✓
- compact suffix literal lexing (`250ms`) ✓

### Milestone 2 — HIR and names — **COMPLETE**

- name resolution ✓
- import resolution ✓
- import alias (`use module (x as y)`) ✓
- decorator attachment (`@recur.timer`, `@recur.backoff`) ✓
- explicit HIR nodes for applicative clusters and markup control nodes ✓
- domain declarations and suffix namespaces ✓
- `instance` blocks with same-module class resolution ✓
- provider declarations (`provider qualified.name`) ✓
- input signal declarations (body-less annotated `signal`) ✓
- module-aware expression typechecker in `aivi-hir` ✓

### Milestone 3 — Kinds and core typing — **COMPLETE**

- kind checking ✓
- class/instance resolution and evidence ✓
- constructor partial application ✓
- `Validation` ✓
- `Default` and record default elaboration ✓
- `Eq` compiler derivation ✓
- operator typechecking (`==`, `!=`, domain operators) ✓
- truthy/falsy branch handoff (`T|>`, `F|>`) including one-layer `Signal` lift ✓
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

- `Task` typed IR and scheduler completion ports ✓
- `source` declaration runtime contract and instance lifecycle ✓
- decode integration (structural decoder, domain parse method resolution) ✓
- worker/UI publication boundary ✓
- timer sources (`timer.every`, `timer.after`) — fully working ✓
- HTTP sources — runtime contract wired, provider execution slice partial
- `fs.read`, `fs.watch` — contract wired, provider execution slice partial
- socket / mailbox / process / D-Bus / window-event sources — partial or provider-specific
- full recurrent-task execution — pending

### Milestone 7 — GTK bridge — **PARTIAL**

- widget plan IR ✓
- runtime assembly ✓
- GTK bridge graph and child-group lowering ✓
- executor with direct setter/event/child management ✓
- `<show>` / `keepMounted` ✓
- `<each>` with required keys and localized child edits ✓
- `<empty>` ✓
- `<match>` ✓
- `<fragment>` ✓
- `<with>` ✓
- widget schema metadata for the current live widget surface ✓
- full widget property catalog — pending

### Milestone 8 — Backend and hardening — **PARTIAL**

- lambda IR with explicit closures and environments ✓
- backend IR with layouts, kernels, pipelines, source plans, and decode plans ✓
- Cranelift AOT codegen for scalars and item-body kernels ✓
- runtime startup linking (HIR → backend → scheduler) ✓
- inline helper pipe execution in item/source kernels ✓
- body-backed signal inline transform/tap/case/truthy-falsy execution against committed snapshots ✓
- general lambda/closure conversion for arbitrary bodies — pending
- scheduler-owned signal filter/fanout/recurrence pipeline execution — pending
- initial moving-GC integration — pending
- fuzzing and deterministic stress infrastructure — in progress
- performance pass plan frozen and benchmark-gated (see §28.8–§28.9)

---

## 25. Bottom-line implementation guidance

AIVI must be implemented as one coherent system:

- typed and lowered through explicit IR boundaries
- stack-safe by design
- scheduler-driven and deterministic
- pure in the language core
- explicit at all effect boundaries
- GTK-first without collapsing into callback-driven impurity
- direct-binding-oriented, not virtual-DOM-oriented

One correct algebraic model over many local patches:

- `&|>` must remain one applicative story across `Validation`, `Signal`, `Option`, `Result`, and `Task`
- record omission must remain explicit-default completion, not open records
- `Task` must remain the only user-visible one-shot effect carrier
- GTK markup must lower directly and predictably to widgets, setters, handlers, and child management

---

## 26. CLI reference

Module discovery uses the nearest ancestor `aivi.toml`; absent that, the entry file's parent directory is the workspace root. Module names come from relative `.aivi` paths under that root.

### 26.1 `aivi check <path>`

```
aivi check src/main.aivi
```

Pipeline: source → CST → HIR → typed core → lambda → backend (no code emission).

Reports diagnostics with source locations. Exits 0 if no errors, 1 if errors, 2 on internal failure.

`aivi <path>` with no subcommand is equivalent to `aivi check <path>`.

### 26.2 `aivi compile <path> [-o <output>]`

```
aivi compile src/main.aivi -o build/main.o
aivi compile src/main.aivi --output build/main.o
```

Pipeline: source → CST → HIR → typed core → lambda → backend → Cranelift → object file.

If `-o` / `--output` is omitted, no output file is written but the pipeline is validated. Exits 0 on success, 1 on compilation errors.

`aivi compile` stops at the honest compile boundary. Use `aivi build` when you want a runnable bundle directory; `compile` remains the object-code surface.

### 26.3 `aivi build <path> -o <output> [--view <name>]`

```
aivi build src/app.aivi -o build/app
aivi build src/app.aivi -o dist/users --view mainWindow
```

`aivi build` validates the same runnable surface as `aivi run`, then writes a bundle directory containing:

- a copied `aivi` runtime executable
- a bundled stdlib workspace
- the reachable workspace source closure plus `aivi.toml` when present
- a `run` launcher script pinned to the selected view

Run the packaged application via `./run` inside the emitted bundle directory.

The bundle is self-contained at the AIVI layer, but it still depends on the target system GTK stack. It is a runnable directory bundle, not yet a single native executable.

Exits 0 on success, 1 on validation/build errors.

### 26.4 `aivi run <path> [--view <name>]`

```
aivi run src/app.aivi
aivi run src/app.aivi --view mainWindow
```

View selection rules:

1. If `--view <name>` is given, the named top-level markup-valued `value` is used.
2. Otherwise, if a top-level markup-valued `value` named `view` exists, that is used.
3. Otherwise, if there is a unique top-level markup-valued `value`, that is used.
4. Otherwise, `--view <name>` is required.

The selected root must be a `Window`. The CLI does not auto-wrap arbitrary widgets into windows.

`aivi run` links the compiled runtime stack, evaluates the selected view fragments against committed runtime snapshots, re-evaluates after each meaningful committed tick, and applies GTK updates through the bridge executor.

The current cataloged widget/runtime slice includes `Window`, `HeaderBar`, `Paned`, `Box`, `ScrolledWindow`, `Frame`, `Viewport`, `Label`, `Button`, `Entry`, `Switch`, `CheckButton`, `ToggleButton`, `Image`, `Spinner`, `ProgressBar`, `Revealer`, and `Separator`. `Entry.onChange` publishes `Text`, `Switch.onToggle` publishes `Bool`, and JSON-backed source payloads may now decode `Float`, `Decimal`, `BigInt`, and `Bytes` through explicit contracts.

Widgets with a single default child group still accept ordinary unnamed children. Widgets with richer slot layouts now use explicit dotted child-group wrappers, for example:

```aivi
<Paned>
    <Paned.start>
        <Label text="Primary" />
    </Paned.start>
    <Paned.end>
        <HeaderBar>
            <HeaderBar.start>
                <Button label="Back" />
            </HeaderBar.start>
            <HeaderBar.titleWidget>
                <Label text="Inbox" />
            </HeaderBar.titleWidget>
            <HeaderBar.end>
                <Button label="More" />
            </HeaderBar.end>
        </HeaderBar>
    </Paned.end>
</Paned>
```

`HeaderBar.titleWidget` remains the default unnamed child group for backward compatibility. `Paned` requires explicit `start` / `end` child-group wrappers because it has no unnamed default slot.

Exits 0 on clean application close, 1 on startup/compilation error.

### 26.5 `aivi execute <path> [-- args...]`

```
aivi execute src/cli.aivi
aivi execute src/cli.aivi -- --model gpt-5.4 prompt.txt
```

`aivi execute` selects the top-level `value main`. The binding must be annotated as `Task E A`;
`signal main` and non-task values are rejected.

The command links the compiled runtime stack without GTK, settles any startup source activity,
evaluates `main`, and executes the resulting host task plan directly in the CLI process.

The current execute-time host surface includes:

- `source process.args`
- `source process.cwd`
- `source env.get "NAME"`
- `source stdio.read`
- `source path.home`
- `source path.configHome`
- `source path.dataHome`
- `source path.cacheHome`
- `source path.tempDir`
- `aivi.stdio.stdoutWrite`
- `aivi.stdio.stderrWrite`
- `aivi.fs.writeText`
- `aivi.fs.writeBytes`
- `aivi.fs.createDirAll`
- `aivi.fs.deleteFile`

Arguments after `--` are exposed through `process.args`. Exits 0 on success, 1 on validation or
runtime error.

### 26.6 `aivi fmt [--stdin | --check] [<path>...]`

```
aivi fmt src/app.aivi             # format to stdout
aivi fmt --stdin                  # read from stdin, write to stdout
aivi fmt --check src/a.aivi src/b.aivi   # verify formatting; exit 1 if any differ
```

The formatter is canonical: single deterministic output for any valid source. Formatting is part of the language contract (§22).

### 26.7 `aivi lex <path>`

```
aivi lex src/app.aivi
```

Tokenizes and prints the token stream. Useful for debugging lexer behavior, regex literal handling, or suffix literal resolution.

### 26.8 `aivi lsp`

```
aivi lsp
```

Starts the AIVI Language Server on stdin/stdout using the Language Server Protocol. Editor integrations launch this subprocess and communicate over stdio. See §27 for supported capabilities.

### 26.9 `aivi db migrate`

```
aivi db migrate
```

Diffs current record types against the last applied migration state and writes a new SQL file under `db/migrations/` with a timestamp-prefixed filename. The generated file is ordinary SQL intended for review and commit.

### 26.10 `aivi db apply`

```
aivi db apply
```

Applies pending SQL migrations in lexicographic order using a `_schema_migrations` tracking table inside one transaction. On failure, the whole application rolls back.

---

## 27. Language server (LSP)

`aivi lsp` is backed by the `aivi-query` incremental query database, which caches source, parse, HIR, diagnostic, symbol, and format results per revision.

### 27.1 Supported capabilities

| Capability | Status |
|---|---|
| Text document sync (full) | ✓ |
| Diagnostics (publish on open/change) | ✓ |
| Document formatting | ✓ |
| Document symbols | ✓ |
| Workspace symbols | Partial |
| Hover documentation | ✓ |
| Go-to-definition | ✓ |
| Completion (triggered on `.`) | ✓ |
| Semantic tokens (full) | Partial |

### 27.2 Architecture

All editor features go through the revision-keyed query database rather than invoking ad-hoc frontend passes. Incremental memoization is per file revision so rapid keystroke changes do not invalidate unrelated cached queries. When a workspace root is known, the server uses the same `aivi.toml` / relative-path module mapping as the CLI.

### 27.3 Current limitations

- whole-workspace semantic queries remain partial; the checked/open file set is the primary working set for symbols and diagnostics
- completion suggestions are basic; type-directed completion over expected record fields and constructor arguments is pending
- semantic token legend exists but token-type coverage is incomplete
- editor-facing project orchestration does not replace the CLI workflow for runtime, migrations, or provider startup validation

---

## 28. Pre-stdlib runtime and application surfaces

### 28.1 Workspace and module discovery

Multi-file workspace discovery is shared across `check`, `compile`, and `run`:

- the nearest ancestor `aivi.toml` is the workspace root when present
- otherwise the entry file's parent directory is the root
- module names come from relative `.aivi` paths under that root
- all commands must agree on this mapping

### 28.2 Database schema and migrations

AIVI record types are the schema source of truth.

Rules:

- migrations are CLI-generated SQL files, not an AIVI-specific migration DSL
- generated migrations live under `db/migrations/`
- runtime startup checks that the applied migration state matches the schema version the program was compiled against
- version mismatch: startup fails with `DbError.SchemaMismatch` before any query runs
- no auto-migration in production

### 28.3 D-Bus surface

- `dbus.ownName`: `source` for name ownership state
- `dbus.call`: `Task`
- `dbus.emit`: `Task`
- `dbus.signal`: `source` for inbound signal subscription
- `dbus.method`: `source` for fire-and-forget inbound method dispatch with immediate Unit reply semantics on the wire

Methods returning non-Unit values to the caller are deferred.

### 28.4 Local-first sync architecture

Reference email-oriented runtime shape:

- IMAP sync runs on a worker and writes fetched mail into SQLite through the database layer
- the UI reads via `db.query` over the local database rather than binding directly to a live IMAP stream
- the sync source publishes typed `SyncState`
- credential errors surface in `SyncState.error` and do not permanently tear down the source on the first auth failure
- SMTP send is a separate one-shot `Task SmtpError Unit`

### 28.5 Multi-process desktop architecture

Intended cooperating-process shape:

- a headless sync daemon
- a GTK UI process
- a GJS GNOME Shell extension

The daemon owns the D-Bus well-known name and SQLite write lock. The UI reads through SQLite and subscribes to daemon D-Bus signals. The extension communicates with the daemon through D-Bus only. SQLite WAL mode covers daemon writes plus UI reads.

`hideOnClose=True` on the main window hides rather than terminates the process. The existing instance is restored by presentation or D-Bus activation.

### 28.6 Moving-GC rollout boundary

- only scheduler-committed runtime snapshots are in moving-GC storage
- pending worker/source/task/evaluator results remain ordinary Rust-owned runtime values until commit
- GTK, worker, and provider seams keep explicit detached boundary wrappers so later GC expansion can happen without reopening those contracts

### 28.7 Runtime startup and linked ownership

Runtime startup links HIR runtime bindings to backend items, source kernels, and widget fragments. The long-lived linked runtime owns its compiled backend program behind shared ownership suitable for persistent GLib-driven sessions.

### 28.8 Hardening requirements

- scheduler stress uses existing runtime/unit harnesses, not a separate async test stack
- teardown, wakeup, and reentry behavior must be testable without sleep-driven flakiness
- parser and decoder fuzzing live in the standalone `fuzz/` workspace described in §23

### 28.9 Performance gate policy

Performance passes start only after typed-core validation. First-wave scope:

- typed-lambda capture pruning
- backend kernel simplification
- direct self-tail loop lowering
- scheduler frontier deduplication

HIR, typechecking, and typed core remain proof and diagnostic layers rather than speculative performance layers. Every performance pass must satisfy the benchmark gate policy in §23.
