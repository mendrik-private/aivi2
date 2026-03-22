# AIVI Language Specification

## Draft v0.4 — implementation-facing

> Status: normative working draft for the first implementation.

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
- validation rules
- pretty-print/debug output

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

### 4.1 CST

The CST is source-oriented and lossless enough for formatting and diagnostics.

### 4.2 HIR

HIR responsibilities:

- names resolved
- imports resolved
- decorators attached
- markup nodes represented explicitly
- pipe clusters represented explicitly
- surface sugar preserved where useful for diagnostics

### 4.3 Typed core

Typed core responsibilities:

- all names resolved
- kinds checked
- class constraints attached
- `&|>` normalized into applicative spines
- pattern matching normalized
- record default elision elaborated
- markup control nodes typed
- signal dependency graph extracted

### 4.4 Closed typed lambda IR

Responsibilities:

- explicit closures
- explicit environments
- explicit runtime nodes for sources/tasks/signals where needed
- dictionary passing or monomorphization decisions applied
- no remaining surface sugar

### 4.5 Backend IR and codegen

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
- orphan instances are disallowed or tightly restricted in v1
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

?? can we do without haskel code, it will confuse the agent
Applicative behavior: 

```aivi
pure x = Valid x
```

```aivi
Valid f      <*> Valid x      = Valid (f x)
Invalid e    <*> Valid _      = Invalid e
Valid _      <*> Invalid e    = Invalid e
Invalid e1   <*> Invalid e2   = Invalid (e1 ++ e2)
```

`Validation` is the canonical carrier for form validation under `&|>`.

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

For signals, this filters updates. For ordinary value flow, it lowers through the chosen flow carrier.

?? what does this mean?

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
 T|> render
 F|> showError
```

elaborates to:

```aivi
loaded
 ||> Ok _  => render
 ||> Err _ => showError
```

The canonical truthy / falsy constructor pairs in v1 are:

- `True` / `False`
- `Some _` / `None`
- `Ok _` / `Err _`
- `Valid _` / `Invalid _`

Rules:

- `T|>` and `F|>` may appear only as an adjacent pair within one pipe spine
- the subject type must have a known canonical truthy / falsy pair
- they do not bind constructor payloads
- use `||>` when payload binding is required
- user-defined truthy / falsy overloads are not supported in v1
- in T/F cases _ carries the first inner constructor value: showError _ 


### 11.5 `*|>` map / fan-out


Maps over an ordinary or reactive collection.

```aivi
users
 *|> .email
```

Each element becomes the ambient subject within the fan-out body.

### 11.6 `|` tap

Observes the subject without changing it.

```aivi
value
 |> compute
 |  debug
 |> finish
```

### 11.7 `@|>` and `<|@`

These mark explicit recurrent flows used for retry/poll/stream-style pipelines. Their exact runtime lowering is scheduler-owned and must remain stack-safe.

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
?? Text interpolation with signals must work in args. so we can add env specific baseUrls for example.

### 14.1 Source contract

A source is a runtime-owned producer that publishes typed values into the scheduler.

Sources may represent:

- HTTP
- file watching
- sockets
- D-Bus
- timers
- process events
- mailboxes/channels

### 14.1.1 Source declaration shape

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
    decode: strict,
    retry: 3,
    timeoutMs: 5000
}
sig users : Signal (Result HttpError (List User))
```

Rules:

- the provider and variant are resolved statically
- positional arguments are provider-defined and typed
- options are a closed record whose legal fields are provider-defined
- unknown options are a compile-time error
- duplicate options are a compile-time error

### 14.1.2 Recommended v1 source variants

The following provider variants are recommended for v1.

#### HTTP

```aivi
@source http.get "/users"
sig users : Signal (Result HttpError (List User))

@source http.post "/login" with {
    body: creds,
    headers: authHeaders,
    decode: strict,
    timeoutMs: 5000
}
sig login : Signal (Result HttpError Session)
```

Recommended HTTP options:

- `headers : Map Text Text`
- `query : Map Text Text`
- `body : A`
- `decode : DecodeMode`
- `timeoutMs : Int`
- `retry : Int`

?? how can we schedule refreshes? signal deps at least, but db life cycles would be also good.

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

#### File watching

```aivi
@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
sig fileEvents : Signal FsEvent
```

?? how can we load the file on change?

Recommended file-watch options:

- `events : List FsWatchEvent`
- `recursive : Bool`
- `decode : DecodeMode`

#### Socket / channel / mailbox

```aivi
@source socket.connect "ws://localhost:8080" with {
    decode: strict
}
sig inbox : Signal (Result SocketError Message)

@source mailbox.subscribe "jobs"
sig jobs : Signal Job
```

Recommended socket and mailbox options:

- `decode : DecodeMode`
- `buffer : Int`
- `reconnect : Bool`
- `heartbeatMs : Int`

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

### 14.1.3 Decode and delivery modes

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
- delivery into the scheduler remains typed and transactional

### 14.2 Decoding


AIVI includes compiler-generated structural decoding by default.

Default decoding rules:

- closed records reject missing required fields
- extra fields are rejected in strict mode by default
- sum decoding is explicit
- decoder override remains possible where necessary

Record default elision for user-written literals does **not** weaken source decoding by default.

### 14.3 Cancellation and lifecycle

Source subscriptions must carry explicit runtime cancellation/disposal semantics. When the owning graph or view is torn down, the source subscription is disposed.

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

## 15.3 Event handler normalization

UI event handlers may elaborate to one of:

- a pure patch/state update
- an `Action`
- a `Task E Action`
- a runtime-normalized batch of those

Normalization is internal. The user model remains pure plus explicit `Task`.

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

Event props lower to direct signal connections, not callback spaghetti exposed to user code.

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
- if `False`, hide means dispose/unsubscribe; show means recreate
- if `True`, the subtree remains mounted and toggles visibility-sensitive state instead

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

Domains bind carrier types, literal suffixes, operators, and smart construction semantics.

Examples include:

- duration
- color
- path
- URL
- geometry
- calendar/date arithmetic

Numeric literal suffixes such as `10min` and `250ms` are part of the domain story. Implicit casts across domains do not exist.

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

### Milestone 1 — Surface and CST freeze

- lexer
- parser
- CST
- formatter skeleton
- syntax for `type`, `class`, `instance`, `val`, `fun`, `sig`, `use`, `export`, markup, and pipe operators

### Milestone 2 — HIR and names

- name resolution
- import resolution
- decorator attachment
- explicit HIR nodes for applicative clusters and markup control nodes

### Milestone 3 — Kinds and core typing

- kind checking
- class/instance resolution
- constructor partial application
- `Validation`
- `Default`
- `Eq`

### Milestone 4 — Pipe normalization

- exact `&|>` normalization
- recurrence node representation
- diagnostics for illegal unfinished clusters

### Milestone 5 — Reactive core and scheduler

- signal graph extraction
- topological scheduling
- transactional ticks
- deterministic propagation
- cancellation/disposal

### Milestone 6 — Tasks and sources

- `Task` runtime
- `@source` runtime contract
- decode integration
- worker/UI publication boundary

### Milestone 7 — GTK bridge

- direct widget graph lowering
- dynamic setter bindings
- event hookups
- `<show>`, `<each>`, `<empty>`, `<match>`, `<fragment>`, `<with>`
- keyed child management without VDOM

### Milestone 8 — Backend and hardening

- lambda IR
- backend IR
- Cranelift AOT/JIT
- performance passes
- fuzzing and stress infrastructure

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