# LLM AIVI Authoring Guide

Use this file when an agent must write **valid, non-hallucinated AIVI code**.

The goal is not to be clever. The goal is to write code that is:

- semantically grounded in `AIVI_RFC.md`
- architecturally disciplined by `AGENTS.md`
- conservative where the current implementation is narrower than the RFC
- explicit about effects, signals, domains, and control flow

## Source hierarchy

When writing AIVI, resolve questions in this order:

1. `AIVI_RFC.md` for language semantics and surface syntax
2. `AGENTS.md` for architecture, invariants, layering, and non-hallucination method
3. `choices_made.md` for the current implementation slice when it is narrower than the RFC
4. `plan/choice_gaps.md` for known mismatches, deferrals, and unstable areas

If the RFC is explicit, follow it.

If the RFC is silent or ambiguous, follow `AGENTS.md`: choose the **narrowest coherent** interpretation, keep later refinement cheap, and document the choice.

If the RFC and current implementation choices differ, prefer the **intersection** for emitted user-facing code unless the task is explicitly about resolving the gap.

## Hard non-hallucination rules

Do not invent:

- undocumented syntax
- an `update` keyword, patch block, record-diff literal, or ad hoc patch DSL
- undocumented top-level forms
- undocumented decorators
- undocumented provider options
- implicit coercions
- open records
- hidden mutation
- signal monadic binding
- wildcard imports, import aliases, or module-qualified names unless the local file already proves they exist
- custom decoder hooks unless the task is explicitly about that missing feature
- custom provider declarations unless the task is explicitly about provider-contract work

If you cannot point to an RFC section, a local example, or a documented implementation choice, do not emit that construct.

## Core mental model

These are the stable ideas you should preserve in every generated module:

- AIVI is pure by default.
- `val` and `fun` are pure.
- `sig` is the reactive boundary.
- `@source` on `sig` is the external-input boundary.
- `Task E A` is the only user-visible one-shot effect carrier.
- `domain` creates nominal value spaces over existing carriers.
- pipe algebra is the primary control-flow surface.
- there is no `if` / `else`.
- there are no imperative loops in surface syntax.
- `Signal` is `Functor` and `Applicative`, not `Monad`.
- GTK markup lowers directly to widgets and control nodes; there is no virtual DOM.

## Authoring workflow

When generating AIVI code, use this order:

1. Pick the right carrier.
   - use `val` for pure non-reactive values
   - use `fun` for pure reusable logic
   - use `sig` for derived reactive state
   - use `@source` + `sig` for runtime-owned external inputs
   - use `Task E A` for one-shot effects
   - use `domain` when a value must stay nominally distinct from its carrier

2. Model data with closed ADTs and closed records.
   - prefer explicit `type` and `domain`
   - keep illegal states unrepresentable

3. Compose with the documented operator surface instead of imperative control flow.
   - `|>` for transforms
   - `?|>` for pure `Bool` gates
   - `||>` for pattern-based branching
   - `*|>` and `<|*` for fan-out and explicit join
   - `&|>` for independent applicative combination

4. Keep reactive structure static.
   - derive signals from statically known dependencies
   - do not write code that requires dynamic signal rewiring

5. Keep effect boundaries explicit.
   - source inputs stay in `@source`
   - one-shot effects stay in `Task`
   - UI events normalize through pure updates, `Action`, or `Task`

6. Use markup control nodes for UI branching and repetition.
   - use `<show>`, `<each>`, `<empty>`, `<match>`, `<fragment>`, `<with>`

## Surface forms you may rely on

These top-level forms are documented and safe to use:

- `type`
- `class`
- `instance`
- `val`
- `fun`
- `sig`
- `use`
- `export`
- decorators via `@name`
- `domain`

Canonical top-level example from the RFC:

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

Prefer the documented `fun` style above when you need a function definition. Do not invent alternate declaration syntax.

For maximum compileability today:

- prefer compiler-derived structural `Eq` over authoring new `Eq` instances
- author `instance` declarations only when the task or nearby code already proves that instance surface is implemented end to end

## Safe syntax and semantics by feature

### 1. Types, records, constructors, and equality

Use closed types.

Safe patterns:

```aivi
type Bool = True | False

type Option A =
  | None
  | Some A

type User = {
    name: Text,
    age: Int
}

val user:User = {
    name: "Ada",
    age: 36
}
```

Rules:

- non-record constructors are ordinary curried values
- records are built with record literals
- tuples, records, and lists are distinct
- equality is available only for documented structural cases
- do not compare `Signal`, `Task`, functions, or foreign handles with `==`

### 2. Domains

Use `domain` for nominal wrappers over carrier types.

Safe pattern:

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    value       : Duration -> Int

domain Url over Text
    parse : Text -> Result UrlError Url
    value : Url -> Text
```

Domain rules:

- no implicit casts between a domain and its carrier
- construction is explicit
- unwrapping is explicit
- operators are not inherited from the carrier automatically
- equality may be derived when the carrier supports `Eq`

For maximum compileability with current implementation choices:

- define any literal suffix you use in the same module
- prefer explicit constructors like `Duration.millis 250` when suffix support is uncertain
- do not assume imported suffixes resolve unless the surrounding code already proves that path is supported

### 3. Text and regex

Use interpolation, not string concatenation.

```aivi
"{name} ({status})"
```

Regex literals are first-class:

```aivi
rx"\d{4}-\d{2}-\d{2}"
```

### 4. Expression model and branching

There is no `if` / `else`.

Use:

- `||>` for general branching
- `T|>` / `F|>` for canonical truthy/falsy carriers
- `?|>` when you want "keep this only if the predicate holds"

Safe examples:

```aivi
status
 ||> Paid    => "paid"
 ||> Pending => "pending"
```

```aivi
ready
 T|> start
 F|> wait
```

```aivi
user
 ?|> .active
 T|> .email
 F|> "inactive"
```

Gate rules:

- the gate predicate must be pure
- the gate result must be `Bool`
- for `Signal A`, failed updates are suppressed; no fake opposite update is emitted
- for ordinary `A`, `?|>` lowers to `Option A`

### 5. Pipe operators

Use the documented pipe operators as the main flow surface.

Documented operators:

- `|>` transform
- `?|>` gate
- `||>` case split
- `*|>` fan-out map
- `<|*` fan-out join
- `&|>` applicative cluster stage
- `@|>` recurrent flow start
- `<|@` recurrence step
- `|` tap

Safe examples:

```aivi
order |> .status
```

```aivi
users
 *|> .email
 <|* Text.join ", "
```

```aivi
value
 |> compute
 |  debug
 |> finish
```

Do not invent extra pipe operators or flattening behavior.

Important restrictions:

- `*|>` is pure mapping only
- it does not flatten nested lists
- it does not sequence `Task`
- it does not merge nested `Signal`
- `<|*` is legal only immediately after `*|>`

### 6. Applicative clusters with `&|>`

Use `&|>` when combining independent values under one applicative carrier.

Safe pattern:

```aivi
sig validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

Use `&|>` for:

- `Validation E`
- `Signal`
- `Option`
- `Result E`
- `Task E`

Rules:

- every member must have the same outer applicative constructor
- the finalizer must be a pure function or constructor
- do not place `?|>` or `||>` inside an unfinished cluster
- do not rely on ambient `.field` projections inside an unfinished cluster unless the nested expression has its own explicit subject

Use `Validation` when failures should accumulate.

```aivi
type Validation E A =
  | Invalid (NonEmptyList E)
  | Valid A
```

### 7. Signals

`sig` is the reactive boundary.

Safe pattern:

```aivi
sig x = 3
sig y = x + 5
```

Rules:

- `val` must not depend on signals
- `fun` stays pure even when used inside `sig`
- signal dependency graphs are static after elaboration
- `Signal` is applicative, not monadic
- do not generate code that requires dynamic dependency rewiring

Prefer derived signals and applicative combination over nested reactive effects.

### 8. Sources and reactive architecture

External inputs enter through `@source` on `sig`.

Safe surface forms:

```aivi
@source timer.every 120
sig tick : Signal Unit
```

```aivi
@source http.get "/users" with {
    decode: Strict
}
sig users : Signal (Result HttpError (List User))
```

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

Reactive source configuration is allowed:

```aivi
val baseUrl = "https://example.com"
sig authToken = "token"
sig authHeaders = Map { "Authorization": authToken }

@source http.get "{baseUrl}/users" with {
    headers: authHeaders,
    decode: Strict
}
sig users : Signal (Result HttpError (List User))
```

Source rules:

- only use `@source` on `sig`
- keep provider and variant statically known
- use built-in providers unless the task is explicitly about custom provider declarations
- use closed option records
- keep option values simple and typed
- prefer same-module bindings, constructors, and literals for source options when you want maximum compileability today
- use `Strict` or `Permissive` for decode mode
- do not invent custom decoder hooks

Polling option caution:

- the docs currently mention both `refreshEvery` and `refreshEveryMs`
- do not guess the spelling in new code
- reuse the spelling already used by the target file, local tests, or current compiler surface

Important architectural rules:

- `fs.watch` reports events only
- `fs.read` performs explicit snapshot reads
- HTTP refresh must be explicit
- reactive source arguments reconfigure sources transactionally
- stale results from superseded request-like sources must not publish into the live graph

### 9. Repetition and recurrence

Repetition in AIVI comes from:

- recursion
- collection combinators
- source-driven triggers
- explicit recurrent pipe forms

However, recurrence is an unstable area in the current implementation backlog.

For maximum non-hallucination safety:

- prefer built-in timers, `refreshOn`, `reloadOn`, or other documented source triggers
- use `@|>` / `<|@` only when the local file or task explicitly targets documented recurrence support
- do not invent new recurrence syntax
- do not rely on `@recur.*` in user-facing AIVI unless the task is specifically about the current compiler-internal recurrence gap

### 10. `Task`

`Task E A` is the only user-visible one-shot effect carrier.

Rules:

- ordinary `val` and `fun` remain pure
- use `Task` for scheduled one-shot effects
- use `Task` applicatively or monadically where the task model is explicit
- do not smuggle effects through pure helpers

The RFC does not fully spell out all library-level task constructors in this repo, so do not invent task APIs. Reuse existing local task helpers when present.

### 11. Patch/state update surface

The RFC explicitly allows UI event handlers to normalize to:

- a pure patch/state update
- an `Action`
- a `Task E Action`
- a runtime-normalized batch of those

However, the RFC does **not** define one standalone user-facing patch DSL in this workspace.

For non-hallucination safety:

- treat patch/update as a semantic category, not as a guaranteed surface syntax
- do not invent an `update` keyword
- do not invent patch records, diff literals, record-spread updates, or callback DSLs
- do not assume Elm-style `update model msg = ...` syntax unless the target file already proves that exact surface
- if a module already has an `Action` type or reducer/update helper, reuse that exact local convention
- otherwise, prefer pure named helpers and explicit closed-value construction using syntax already documented by the RFC

Practical rule:

- if a requested event handler needs patch syntax that is not already present in the local file or another grounded example, stop short of inventing it and say the patch surface is currently undocumented

### 12. Markup and reactive UI

Use typed markup-like syntax for UI.

Safe examples:

```aivi
<Label text={statusLabel order} visible={isVisible} />
```

```aivi
<show when={isVisible}>
    <Label text="Ready" />
</show>
```

```aivi
<each of={items} as={item} key={item.id}>
    <Row item={item} />
    <empty>
        <Label text="No items" />
    </empty>
</each>
```

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

```aivi
<with value={formatUser user} as={label}>
    <Label text={label} />
</with>
```

Markup rules:

- use documented control nodes only
- prefer widget/control names already seen in the local file or RFC examples
- include a `key` on every `<each>` for current implementation safety
- keep `<with>` pure; it does not create a new signal node

## Current implementation-safe subset

When you want the highest chance that generated code matches both the RFC and the current implementation wave, stay inside these constraints:

- use simple `use module (name1 name2)` imports only
- avoid import aliases, wildcards, and module-qualified names
- keep source option values simple and explicitly typed
- prefer same-module domain literals and same-module source helper values
- give every `<each>` a `key`
- use built-in decode modes only
- avoid custom provider declarations in application code
- avoid decoder override hooks
- avoid recurrence syntax unless the target file already uses it
- avoid inventing patch/update syntax that is not already proven locally
- avoid new user-authored instances unless the local module already proves the surface
- do not assume broader expression typing for hard source-option expressions unless the local compiler tests prove it

## Copyable grounded templates

### Domain plus explicit construction

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    value       : Duration -> Int

val shortDelay:Duration = 250ms
```

### Derived signals

```aivi
sig firstName = "Ada"
sig lastName = "Lovelace"

fun joinName:Text #first:Text #last:Text =>
    "{first} {last}"

sig fullName =
 &|> firstName
 &|> lastName
  |> joinName
```

### Explicit watch-read pipeline

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

### Fan-out and explicit join

```aivi
emails
 *|> .address
 <|* Text.join ", "
```

### Markup with control nodes

```aivi
<show when={isReady}>
    <each of={items} as={item} key={item.id}>
        <Row item={item} />
        <empty>
            <Label text="No items" />
        </empty>
    </each>
</show>
```

## Preflight checklist before emitting code

Before finalizing generated AIVI, check all of these:

- every feature used appears in the RFC or a local proven example
- `val` and `fun` remain pure
- reactive state uses `sig`
- external input uses `@source` on `sig`
- domains have explicit construction and elimination
- there are no implicit casts
- there is no `if` / `else`
- there are no imperative loops
- pipe operators are used with their documented carriers and restrictions
- `&|>` members share one applicative outer constructor
- `Signal` is not treated as a monad
- source options are closed and explicit
- every `<each>` includes `key`
- no invented patch/update DSL appears in handlers
- no speculative decorators, imports, providers, or decoder hooks were invented

## If you are still unsure

If a requested construct is not clearly documented:

1. look for a local example in the target file or nearby tests
2. fall back to the RFC example surface
3. choose the narrower coherent subset from `choices_made.md`
4. if uncertainty remains, do not invent code for that feature; emit a simpler documented form or state the blocker explicitly

This is better than producing plausible but false AIVI.
