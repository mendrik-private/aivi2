# AIVI RFC Gap Analysis

> Based on: `AIVI_RFC.md` Draft v0.4  
> Scope: implementation-facing gaps, underspecified semantics, missing definitions, and suggested resolutions.

Gaps are organized by severity: **🔴 HIGH** (blocks implementation), **🟡 MEDIUM** (blocks correctness or tooling), **🟢 LOW** (polish or future work).

//-

## 1. `Action` Type — Undefined Internal Type

**Severity:** 🔴 HIGH  
**Section:** §15.3 Event handler normalization

### Problem

Section 15.3 states that UI event handlers may elaborate to one of:

- a pure patch/state update
- an `Action`
- a `Task E Action`

`Action` is never declared, described, or given a structure anywhere in the RFC. Section 25 reinforces that "`Task` must remain the only user-visible one-shot effect carrier" — which implies `Action` is **not** user-visible. But nothing specifies what it is.

### Suggested Fix

Add a subsection **§15.2.1 The `Action` type** (internal runtime type):

```
Action is a sealed internal runtime type owned by the scheduler.
It represents a committed UI state change instruction produced after
a Task completes or an event handler is evaluated.

From the user's perspective, event handlers return either pure
expressions or `Task E A` where A carries the update payload.
The elaborator wraps the result in an Action before dispatching
to the scheduler.

Users never name, construct, or inspect Action directly.
The runtime's event dispatch layer owns the Action-to-scheduler
pipeline. This is the Elm-architecture equivalent of a `Msg`.
```

An alternative stronger fix: replace `Action` with a user-visible `Msg A` convention and document the event-to-scheduler loop explicitly, as it is a key part of the GTK integration story.

//-

## 2. Comment Syntax — Not Specified

**Severity:** 🔴 HIGH  
**Section:** §5 Top-level forms / §19 Strings

### Problem

No syntax for comments exists anywhere in the RFC. The formatter section (§22) cannot be implemented without knowing comment syntax. The CST section (§4.1) says it is "lossless enough for formatting and diagnostics" — which requires comment preservation.

### Suggested Fix

Add **§5.1 Comments**:

```
Single-line comments use `//`:

    // this is a comment
    val x = 42 // inline comment

Multi-line comments use `{- -}`, nestable:

    /*
      This is a block comment.
      /* Nested block comment is legal. */
    */

Documentation comments use `/*> <*/` and attach to the
immediately following declaration:

    /** The canonical empty value for a type. **/
    class Default A
        default : A

The CST must preserve all comment tokens and their positions.
The formatter must round-trip comments without loss.
```

//-

## 3. Module System — Scoping and Qualified Names

**Severity:** 🔴 HIGH  
**Section:** §5 Top-level forms

### Problem

`use` and `export` appear in examples but the module system is not specified:
- No qualified name syntax (is it `Http.get` or `http.get`?)
- No rules for what `use` brings into scope (all exports? named?)
- No circular-dependency handling
- No default visibility (is every declaration exported unless marked?)
- No module file mapping (does `use aivi.network` map to `aivi/network.aivi`?)

### Suggested Fix

Add **§5.2 Module system**:

```
A module corresponds to a single source file. The module name
is declared at the top of the file or inferred from the file path.

    module aivi.network

Imports:
    use aivi.network (http, socket)     // named import
    use aivi.network                    // import all exports
    use aivi.network hiding (socket     // hides an import
    use aivi.network as Net             // qualified alias

Exports:
    export (MyType, myFun)              // explicit export list
    export ..                           // export everything (default)

Visibility:
- All top-level declarations are module-private by default.
- `export` opts them into the public surface.

Qualified access uses `.` after the alias:
    Net.http, Net.socket

Circular dependencies are a compile-time error. The compiler
reports the cycle and stops.
```

//-

## 4. Literal Syntax — Underspecified

**Severity:** 🔴 HIGH  
**Section:** §6.2 Core primitive types

### Problem

`Int`, `Float`, `Decimal`, `BigInt` are listed but:
- `Int` range is unspecified (32-bit? 64-bit? arbitrary precision?)
- `Float` precision is unspecified (IEEE 754 f64? f32?)
- `Decimal` semantics are entirely absent
- No hex, binary, or octal literal syntax
- No underscores in numeric literals
- No `Char` type or character literal syntax
- Integer overflow behavior is unspecified

### Suggested Fix

Add **§6.2.1 Numeric literal syntax and semantics**:

```
Int:
- 64-bit signed integer (i64)
- Overflow is a compile-time error for literals, runtime trap for computed values
- Decimal: 42, -7, 1_000_000
- Hex: 0xFF, 0xDEAD_BEEF
- Binary: 0b1010_0011
- Octal: 0o755

Float:
- IEEE 754 double (f64)
- NaN and Infinity are not constructible by user code; sources
  that decode them produce a decode error
- Literals: 3.14, -0.5, 1.0e10, 1_000.5

BigInt:
- Arbitrary-precision integer; no overflow
- Literal form: 123n (suffix n)

Decimal:
- Arbitrary-precision decimal (base-10 safe arithmetic)
- Intended for financial/currency use
- Literal form: 19.99d (suffix d)

There is no Char type in v1. Single characters are Text of length 1.
```

//-

## 5. Operator Precedence Table — Missing

**Severity:** 🔴 HIGH  
**Section:** §11 Pipe algebra

### Problem

Eight pipe operators are defined (`|>`, `?|>`, `||>`, `*|>`, `&|>`, `@|>`, `<|@`, `|`, `<|*`) alongside standard arithmetic and comparison operators, but no precedence table or associativity rules exist. This makes the grammar ambiguous and the parser unimplementable without guessing.

### Suggested Fix

Add **§11.0 Operator precedence**:

```
Precedence (low to high binding), all left-associative unless noted:

1. |>   ?|>   ||>   *|>   &|>   @|>   <|@   |   <|*   (pipe operators)
2. or
3. and
4. not                                                  (right-assoc, unary)
5. ==   !=
6. <   <=   >   >=
7. +   -                                               (arithmetic additive)
8. *   /   %                                           (arithmetic multiplicative)
9. unary -                                             (prefix)
10. function application                                (left-assoc, highest)

Pipe operators bind lower than all arithmetic and comparison so that:

    xs ?|> .age > 18

parses as:

    xs ?|> (.age > 18)

and not as:

    (xs ?|> .age) > 18

Within pipe operators themselves, all are left-to-right sequential at the
same precedence level. Mixing pipe variants in a single spine is legal
subject to the cluster and recurrence rules in §11.
```

//-

## 6. Type Inference and Constraint Solving — No Algorithm

**Severity:** 🔴 HIGH  
**Section:** §4.3 Typed core, §6.1 Kinds, §7.1 Resolution

### Problem

Three distinct compile-time algorithms are required but never specified:
1. Kind inference / checking for HKT partial application
2. Instance resolution (dictionary passing or specialization)
3. Type inference (how much Hindley-Milner? bidirectional? local only?)

Without these, the typed-core pass cannot be implemented coherently.

### Suggested Fix

Add **§4.3.1 Typing algorithm**:

```
AIVI uses local bidirectional type checking, not global HM inference.

Inference flows:
- Checking mode: expected type is pushed inward (e.g., record literals,
  lambda bodies, constructor arguments).
- Synthesis mode: type is derived outward from sub-expressions.

Type variables are resolved locally within a val/fun/sig definition.
Cross-definition inference is not performed; annotations are required
at definition boundaries where the type cannot be locally synthesized.

Kind checking:
- Kind of each named type constructor is recorded at declaration.
- Partial application is legal when the resulting kind is structurally
  consistent with the expected kind at the use site.
- Kind mismatches are reported with the expected kind and the actual kind.

Instance resolution:
- At each class-constrained use site, the compiler searches the set of
  in-scope instances for the required class and type argument.
- Search is depth-limited (v1: max depth 10 for superclass chains).
- Overlapping instances are an immediate compile error at the instance
  declaration site, not at use sites.
- Orphan instances: an instance is orphan if neither the class nor the
  type being instantiated is declared in the current module. Orphan
  instances are rejected in v1.
- If no instance is found, the error names the missing class and type.
```

//-

## 7. IR Specifications — No Formal Definitions

**Severity:** 🔴 HIGH  
**Section:** §4 Compiler pipeline, §3.5 IR invariants

### Problem

Section 3.5 requires each IR to define ownership model, identity strategy, source span strategy, validation rules, and pretty-print form. None of the seven IR boundaries (CST, HIR, typed core, lambda IR, backend IR) provide this. The pipeline is named but not specified.

### Suggested Fix

Add **§4.0 IR boundary contract** (template applied to each IR):

```
Each IR must document:

Ownership:
  - Are nodes owned by an arena? A bump allocator? A Vec index?
  - Can nodes be mutated after construction?

Identity:
  - How are nodes uniquely identified? (index, interned ID, pointer?)
  - Are IDs stable across passes?

Source spans:
  - Every node that can produce a diagnostic carries a SourceSpan.
  - Desugared synthetic nodes carry the span of the surface construct
    they were derived from.

Validation:
  - The invariants that must hold on a well-formed IR node.
  - The pass responsible for establishing each invariant.
  - A validate() entry point that checks all invariants in debug builds.

Pretty-print:
  - A Display implementation suitable for //dump-ir flags.
  - Sufficient to reconstruct the structure for snapshot tests.
```

Then apply this to each: CST, HIR, TypedCore, LambdaIR, BackendIR.

//-

## 8. Scheduler — No Transaction Semantics or Deadlock Proof

**Severity:** 🔴 HIGH  
**Section:** §13.3, §16.3

### Problem

The scheduler is required to be "transactional per tick", "topologically ordered", "glitch-free", and "deadlock-free" but none of these are formally defined. "Transactional" is especially vague — it could mean snapshot isolation, linearizability, or simply "no reads of partially-updated state during a tick".

### Suggested Fix

Add **§13.3.1 Scheduler tick model**:

```
A scheduler tick is the unit of propagation.

A tick begins when one or more source events arrive (from HTTP
completion, timer fire, file event, user input, etc.).

Within a tick:
1. All newly arrived source events are collected into the pending set.
2. The dirty signal set is computed: all signals that directly depend
   on a changed source or another dirty signal.
3. Dirty signals are evaluated in topological order (dependencies
   evaluated before dependents). Any signal evaluated more than once
   in a tick is an implementation error.
4. Readers see either all-old or all-new values for any signal pair.
   No signal observes a mix of tick-N and tick-N+1 values from its
   dependencies during a single evaluation. This is the glitch-free
   guarantee.
5. After all signals are stable, GTK property setters are called for
   changed reactive bindings.
6. The tick ends. New source events arriving during a tick are queued
   for the next tick.

Cycles in the signal graph are a compile-time error.
The compiler must verify acyclicity during graph extraction.

Deadlock prevention:
- The scheduler owns a single-writer queue per worker.
- Workers publish into the queue and return immediately.
- The GTK main thread drains the queue at the start of each tick.
- No worker ever waits on the scheduler; no scheduler step ever
  waits on a worker. Communication is one-directional.
- This design makes deadlock structurally impossible under normal
  operation. Pathological cases (e.g., a Task that submits work
  back to a full bounded queue) are documented as implementation
  errors with a queue-full error, not silent deadlock.
```

//-

## 9. Signal Cycles — Detection Unspecified

**Severity:** 🔴 HIGH  
**Section:** §13.1, §13.3

### Problem

What happens if a user defines `sig a = a + 1`? Or if `sig a` depends on `sig b` which depends on `sig a`? The RFC says "dependency graphs are static after elaboration" but never specifies how cycles are detected or reported.

### Suggested Fix

Add **§13.1.1 Cycle detection**:

```
After elaboration, the compiler extracts the signal dependency graph.
If the graph contains a cycle, it is a compile-time error:

    sig a = b + 1   // error: signal 'a' depends on 'b'
    sig b = a + 1   // error: signal 'b' depends on 'a'

The error reports the full cycle path:
    error: cyclic signal dependency: a -> b -> a

Self-referential signals such as `sig x = x` are also illegal
unless expressed through the explicit recurrence operators
`@|>` and `<|@`, which lower to runtime-owned loop nodes.
```

//-

## 10. `<each>` Key Migration Algorithm — Unspecified

**Severity:** 🔴 HIGH  
**Section:** §17.3.2

### Problem

`<each>` requires a `key` for identity-stable child management, and "existing child subtrees are reused by key where possible". But the algorithm for what happens when keys are added, removed, or reordered is not defined. This is a well-known subtle problem (cf. React reconciliation, virtual DOM diffing).

### Suggested Fix

Add **§17.3.2.1 Key reconciliation algorithm**:

```
Given a previous key list [k1, k2, k3] and a new key list [k2, k4, k1]:

1. Build a map from key -> existing child widget for all previously
   mounted children.
2. For each key in the new list, in order:
   a. If the key exists in the old map, reuse the existing child widget.
      Update its bound props to reflect the new item value.
   b. If the key is new, create a new child widget subtree.
3. Remove all old children whose keys are absent from the new list.
   Teardown must follow the rules in §17.4 (disconnect handlers,
   dispose subscriptions, release widget handles).
4. Reorder surviving children to match the new key order using
   GTK's child reordering API directly.

Keys must be of a type with an `Eq` instance. `Text` and `Int` are
the recommended key types.

Key uniqueness within a single `<each>` render is a runtime assertion
in debug builds and silently de-duplicates (last wins) in release builds.
```

//-

## 11. GTK Widget Mapping Table — Missing

**Severity:** 🔴 HIGH  
**Section:** §17.1.1

### Problem

Section 17.1.1 says "each markup node compiles to a widget/control-node kind" but provides no table. Without this, the GTK bridge cannot be implemented.

### Suggested Fix

Add **§17.1.2 Widget mapping** (partial example to be completed):

```
The following AIVI markup nodes map to GTK4/libadwaita widgets:

Primitive layout:
  <Box orientation={Horizontal}>   -> GtkBox (horizontal)
  <Box orientation={Vertical}>     -> GtkBox (vertical)
  <Grid>                           -> GtkGrid
  <Stack>                          -> GtkStack
  <ScrolledWindow>                 -> GtkScrolledWindow

Content:
  <Label text={...}>               -> GtkLabel
  <Image source={...}>             -> GtkImage
  <Separator>                      -> GtkSeparator

Input:
  <Button label={...}>             -> GtkButton
  <Entry text={...}>               -> GtkEntry
  <Switch active={...}>            -> GtkSwitch
  <CheckButton active={...}>       -> GtkCheckButton
  <SpinButton value={...}>         -> GtkSpinButton
  <Slider value={...}>             -> GtkScale

Containers (libadwaita):
  <Window title={...}>             -> AdwApplicationWindow
  <HeaderBar>                      -> AdwHeaderBar
  <Clamp>                          -> AdwClamp
  <PreferencesGroup>               -> AdwPreferencesGroup
  <ActionRow title={...}>          -> AdwActionRow

Control nodes (AIVI-internal, not GTK widgets):
  <show>, <each>, <match>, <fragment>, <with>

This table is normative. Unrecognized markup tags are compile-time errors.
Each widget's legal props, prop types, and event names must be declared
in the GTK bridge schema so the compiler can type-check them.
```

//-

## 12. Standard Library — No Inventory

**Severity:** 🔴 HIGH  
**Section:** (entire document)

### Problem

No standard library modules are documented. `aivi.network`, `aivi.defaults` are referenced by name but not specified. Core functions like `map`, `filter`, `fold`, `Text.join`, `List.head` appear in examples but are never declared.

### Suggested Fix

Add **§26 Standard library modules** (outline):

```
aivi.defaults     // Default instances (Option, List, Result, etc.)
aivi.list         // List A operations: map, filter, fold, head, tail,
                     zip, take, drop, range, flatten, reverse, length,
                     find, any, all, groupBy, sortBy, uniqueBy
aivi.text         // Text operations: length, concat, join, split,
                     trim, contains, startsWith, endsWith, toUpper,
                     toLower, slice, lines, words
aivi.math         // Numeric: abs, min, max, clamp, floor, ceil, round,
                     sqrt, pow, log, mod, rem
aivi.option       // Option utilities: fromMaybe, toList, map, filter
aivi.result       // Result utilities: map, mapErr, fromOption,
                     toValidation, sequence
aivi.map          // Map K V operations: fromList, toList, get, insert,
                     delete, merge, keys, values, size, member
aivi.set          // Set A operations: fromList, toList, member, insert,
                     delete, union, intersection, difference, size
aivi.bytes        // Byte buffer operations
aivi.network      // http, socket source providers
aivi.fs           // fs.watch, fs.read source providers
aivi.process      // process.spawn source provider
aivi.timer        // timer.every, timer.after source providers
aivi.regex        // Regex matching, capture groups
aivi.json         // JSON encode/decode utilities
aivi.gtk          // Window events, key names, clipboard, dialogs
```

Each module's full type signatures belong in a companion stdlib RFC or
implementation spec, not in the language RFC itself.

//-

## 13. `Eq` for `Map`, `Set`, `Bytes` — Deferred Without Plan

**Severity:** 🟡 MEDIUM  
**Section:** §7.3

### Problem

`Eq` is explicitly not derived for `Bytes`, `Map`, `Set`, and several other types. The deferral reason is "equality semantics have not yet been specified." This blocks any meaningful use of these types in pattern matching or validation.

### Suggested Fix

Add **§7.3.1 Equality for collection types**:

```
Bytes:
  Equality is byte-by-byte comparison of the underlying buffer.
  Compiler-derives Eq for Bytes.

Map K V:
  Two maps are equal iff they contain the same set of key-value pairs.
  Requires: Eq K, Eq V.
  Key ordering is irrelevant to equality.

Set A:
  Two sets are equal iff they contain the same elements.
  Requires: Eq A.
  Element ordering is irrelevant to equality.

Signal A:
  Signals do not have Eq. Two Signal values cannot be compared.
  Rationale: signal identity is a runtime graph concept, not a value.

Task E A:
  Tasks do not have Eq. A Task is a description of work, not a result.

Function values:
  Functions do not have Eq in v1.

GTK/foreign handles:
  Not Eq in user code. Handles are opaque runtime references.
```

//-

## 14. `Default` Instances — Only `Option` Bundle Specified

**Severity:** 🟡 MEDIUM  
**Section:** §9.2

### Problem

Only the `Option` bundle is given. Without `Default` instances for other common types, record omission is useless in practice.

### Suggested Fix

Add **§9.2.1 Standard Default bundles**:

```
Bundles provided by aivi.defaults:

use aivi.defaults (Option)    // Default (Option A)  = None
use aivi.defaults (List)      // Default (List A)    = []
use aivi.defaults (Text)      // Default Text        = ""
use aivi.defaults (Int)       // Default Int         = 0
use aivi.defaults (Float)     // Default Float       = 0.0
use aivi.defaults (Bool)      // Default Bool        = False
use aivi.defaults (Map)       // Default (Map K V)   = Map {}
use aivi.defaults (Set)       // Default (Set A)     = Set []

Users may define their own Default instances for custom types:

instance Default MyConfig
    default = { theme: Light, fontSize: 14 }
```

//-

## 15. `?|>` Gate — Subscription and Reactive Predicate Semantics

**Severity:** 🟡 MEDIUM  
**Section:** §11.3

### Problem

For `Signal A`, updates with `False` predicate are "suppressed" but:
1. Do downstream signals still subscribe? (They should — the graph is static.)
2. What if the predicate is itself a signal? Can you write `signal ?|> otherSignal`?

### Suggested Fix

Add **§11.3.1 Gate semantics for signals**:

```
Gate subscription:
  A gated signal is always part of the dependency graph regardless of
  the predicate's current value. Downstream signals subscribe to the
  gated signal normally. A suppressed update simply does not propagate
  a new value — the downstream signal retains its previous value.

Reactive predicates:
  The predicate body of `?|>` may reference signals. If it does, the
  gated signal depends on both the subject signal and the predicate's
  signal dependencies:

    users ?|> (.active and isLoggedIn)
    // result depends on: users, isLoggedIn

  When the predicate changes from False to True, the gate immediately
  forwards the subject's current value.
  When the predicate changes from True to False, the gate suppresses
  further updates. No synthetic "undo" update is emitted.

  The predicate must remain pure aside from signal reads.
```

//-

## 16. `<show keepMounted>` — Handler/Subscription Behavior During Hide

**Severity:** 🟡 MEDIUM  
**Section:** §17.3.1

### Problem

When `keepMounted=True` and the subtree is hidden, it's unclear whether event handlers fire, whether signals continue to propagate, and whether sources continue to run.

### Suggested Fix

Add **§17.3.1.1 keepMounted semantics**:

```
keepMounted = False (default):
  On hide: child subtree is fully torn down per §17.4 rules.
  On show: child subtree is fully recreated and subscribed.
  Cost: widget allocation/deallocation on each toggle.
  Use when: the subtree is expensive to keep alive.

keepMounted = True:
  On hide: the GTK widget is set to invisible (gtk_widget_set_visible false).
  Signal subscriptions remain active and continue to propagate.
  Event handlers remain connected but GTK will not deliver input events
  to invisible widgets (GTK4 behavior: invisible widgets do not receive
  pointer or keyboard events).
  Source subscriptions remain active.
  On show: visibility is restored; no re-creation occurs.
  Cost: continued signal and source activity even while hidden.
  Use when: the subtree is cheap to maintain and toggled frequently.
```

//-

## 17. HTTP `activeWhen` — Non-Normative Language

**Severity:** 🟡 MEDIUM  
**Section:** §14.1.2

### Problem

"in-flight work **may be** cancelled or marked stale" is non-normative. An implementation cannot be correct without a definitive answer.

### Suggested Fix

Replace the sentence with:

```
activeWhen gates all request issuance and polling.

When activeWhen transitions from True to False:
- Any in-flight request is cancelled at the network layer if
  cancellation is supported by the underlying runtime.
- If the request cannot be cancelled (e.g., already in the OS
  network stack), its response is discarded and not published
  into the scheduler. The signal retains its last published value.
- Polling is suspended immediately.

When activeWhen transitions from False to True:
- If refreshEvery or refreshOn is configured, the source behaves
  as if a refresh trigger just fired.
- Otherwise, the source re-issues its initial request.
```

//-

## 18. `refreshOn` Backpressure — Unspecified

**Severity:** 🟡 MEDIUM  
**Section:** §14.1.2

### Problem

If `refreshOn` fires while a request is already in flight, the behavior is unspecified. Queuing, dropping, or coalescing are all valid but produce different UX behavior.

### Suggested Fix

Add to the HTTP source contract:

```
When a refreshOn trigger fires while a request is already in flight:
- The current in-flight request is cancelled if cancellable.
- A new request is issued immediately with the latest source configuration.
- This is a "latest-wins" policy, consistent with the signal model.
- No queue of pending requests is maintained. A new trigger always
  supersedes the previous in-flight request.

If dropping/cancellation is undesirable (e.g., a form submission),
use Task directly instead of an HTTP source, as Task gives explicit
control over concurrency and queuing.
```

//-

## 19. Recurrence (`@|>` / `<|@`) — Termination and Representation

**Severity:** 🟡 MEDIUM  
**Section:** §11.7

### Problem

Recurrent pipes are "scheduler-owned loop nodes" but the runtime representation, termination conditions, and tail-call semantics are entirely unspecified.

### Suggested Fix

Add **§11.7.1 Recurrence runtime model**:

```
A recurrent pipe compiles to a named runtime loop node owned by
the scheduler. The loop node holds:
- current iteration state (type S, the ambient subject)
- a wakeup trigger (timer, source event, or signal edge)
- a step function (S -> S or S -> Task E S)
- a termination condition (optional; if absent, runs until owner teardown)

The step function is never called recursively. The scheduler
enqueues the next wakeup after each step completes.

Termination:
  Recurrence nodes stop when:
  a. The owning sig or view subtree is torn down.
  b. An explicit `@|> done` or equivalent termination step is reached.
  c. The step function returns an error type (for Task-backed recurrence).

Stack safety:
  The step function is invoked as a normal scheduler callback.
  It must not recurse into itself. The compiler rejects step bodies
  that would produce unbounded stack depth.

Lowering targets:
  - Timer-driven: `@|>` on a timer source → GLib timeout callback
  - Signal-driven: `@|>` on a signal edge → scheduler event callback
  - Task-driven: step returns Task → Task completion triggers next step
```

//-

## 20. Orphan and Overlapping Instance Rules — Vague

**Severity:** 🟡 MEDIUM  
**Section:** §7.1

### Problem

"Orphan instances are disallowed or tightly restricted" is not a rule. An implementation must choose one.

### Suggested Fix

Replace with:

```
Orphan instances are disallowed in v1.

An instance is orphan if the module declaring it is not the module
that declares either the class or the type being instantiated.

Rationale: orphan instances cause coherence violations when the same
instance is declared in two different modules that are both imported.

To provide an instance for a foreign type and foreign class, users
must use a newtype wrapper declared in their own module.

Overlapping instances are a compile-time error at the instance
declaration site. Two instances overlap if there exists any type
for which both would apply. The error names both instances and
the overlapping type pattern.
```

//-

## 21. Domain `Eq` Opt-Out — Syntax Unspecified

**Severity:** 🟡 MEDIUM  
**Section:** §20.9

### Problem

"`Eq` may be compiler-derived for a domain if its carrier has `Eq` and the domain does not opt out" — the opt-out mechanism has no syntax.

### Suggested Fix

Add to §20.9:

```
Compiler-derived Eq for a domain is opt-in, not opt-out.

To derive Eq for a domain, add `derive Eq` to the domain body:

    domain Duration over Int
        derive Eq
        literal ms : Int -> Duration
        ...

Without `derive Eq`, the domain does not have Eq. This is safer
than opt-out because accidental Eq derivation could expose
implementation-detail equality semantics.

The derive mechanism is extensible: v1 supports `derive Eq` only.
Later versions may add `derive Ord`, `derive Hash`, etc.
```

//-

## 22. Diagnostics — No Taxonomy or Error Codes

**Severity:** 🟡 MEDIUM  
**Section:** §21

### Problem

Diagnostics are described in terms of examples only. No taxonomy, no error codes, no severity levels. This makes it impossible to write diagnostic regression tests referencing specific error IDs.

### Suggested Fix

Add **§21.1 Diagnostic taxonomy**:

```
Each diagnostic has:
- A code: E followed by a four-digit number (e.g., E0001)
- A severity: error, warning, hint
- A primary span: the main cause location
- Zero or more secondary spans with labels
- An optional suggestion (machine-applicable fix where possible)

Error code ranges:
  E0001-E0099: Syntax errors (lexer/parser)
  E0100-E0199: Name resolution errors
  E0200-E0299: Kind errors
  E0300-E0399: Type errors
  E0400-E0499: Instance resolution errors
  E0500-E0599: Signal graph errors
  E0600-E0699: Source / decode errors
  E0700-E0799: GTK bridge errors
  E0800-E0899: Module system errors
  E0900-E0999: Domain errors

Warning code ranges:
  W0001-W0099: Unused bindings, unreachable arms, redundant imports
```

//-

## 23. `<match>` Pattern Syntax — Not Formally Specified

**Severity:** 🟡 MEDIUM  
**Section:** §17.3.3

### Problem

`<case pattern={Paid}>` is shown but there is no formal spec of what pattern syntax is legal here. Can you write `<case pattern={Some x}>`? Can you use guards?

### Suggested Fix

Add **§17.3.3.1 Match pattern grammar**:

```
The `pattern` attribute of `<case>` accepts any pattern legal in
an ordinary `||>` case split, including:

- Constructor patterns:       Paid, None, Some x, Err e, Ok value
- Wildcard:                   _
- Literal patterns:           42, "hello", True
- Record subset patterns:     { name, age }
- Nested patterns:            Ok (Some x)

Guards are NOT supported in `<match>` case patterns in v1.
Use `<show when={...}>` inside the case body for additional conditions.

Bindings introduced in a pattern are in scope in the case body.
Pattern matching in <match> follows the same exhaustiveness rules
as ordinary sum matches.
```

//-

## 24. Formatter — No Algorithm

**Severity:** 🟡 MEDIUM  
**Section:** §22

### Problem

The formatter is "part of the language contract" but has no algorithm, line-length limit, indentation unit, or conflict-resolution rules.

### Suggested Fix

Add **§22.0 Formatter contract**:

```
Line length: 100 characters (hard limit for canonical output).
Indentation: 4 spaces. Tabs are not canonical.
The formatter is idempotent: formatting formatted output is a no-op.
The formatter is total: it must not fail on any syntactically valid file.

Pipe alignment:
  All pipe stages in one spine are vertically aligned at the operator:

      items
       |>  filter .active
       *|> .email
       <|* Text.join ", "

  The subject is on its own line when the pipe has two or more stages.
  Single-stage pipes may be inline: `items |> filter .active`

Match arm alignment:
  Contiguous `||>` arms align their `=>` tokens:

      status
       ||> Paid    => "paid"
       ||> Pending => "pending"
       ||> _       => "unknown"

Applicative cluster alignment:
  `&|>` stages align at the operator; the finalizer `|>` aligns with them:

      sig validatedUser =
       &|> validateName nameText
       &|> validateEmail emailText
       &|> validateAge ageText
        |> UserDraft

Comments: preserved in place. The formatter does not reflow comment text.
```

//-

## 25. Panic / Unrecoverable Error Semantics

**Severity:** 🟡 MEDIUM  
**Section:** (absent)

### Problem

What happens when the runtime encounters an unrecoverable error? (E.g., assertion failure, allocator OOM, FFI contract violation.) There is no panic model.

### Suggested Fix

Add **§16.4 Unrecoverable errors**:

```
AIVI distinguishes recoverable errors (Result E A, Validation E A,
decode errors) from unrecoverable runtime faults.

Unrecoverable faults include:
- Allocator out-of-memory
- Signal cycle at runtime (implementation bug)
- GTK assertion failure
- FFI contract violation
- Stack overflow detected by the runtime

On an unrecoverable fault:
- The runtime prints a structured diagnostic to stderr including
  the fault kind, a source location if available, and a stack trace
  in debug builds.
- The process exits with a non-zero status code.
- No cleanup handlers run. GTK shutdown is attempted on a best-effort
  basis.

There is no user-catchable panic in v1. Recoverable error handling
uses Result and Task; unrecoverable faults abort the process.
```

//-

## 26. Anonymous Function Syntax — Absent

**Severity:** 🟡 MEDIUM  
**Section:** §5, §11

### Problem

`fun add:Int #x:Int #y:Int => x + y` is shown for named functions but there is no syntax for anonymous lambdas, which are required by higher-order functions like `filter`, `map`, and `sortBy`.

### Suggested Fix

Add **§10.4 Anonymous functions**:

```
An anonymous function is written with `\` and `=>`:

    \x => x + 1
    \x y => x + y

Type annotations are optional when the expected type is known:

    users |> filter (\u => u.age > 18)

Named keyword arguments are not supported on anonymous functions.
Use named functions when keyword arguments improve clarity.

Inside a pipe, the ambient subject is accessible as `_`, so short
lambdas are often replaceable by ambient projection:

    users |> filter (.age > 18)    // equivalent to above
```

//-

## 27. `Map` / `Set` Key Constraints — Unspecified

**Severity:** 🟡 MEDIUM  
**Section:** §6.7

### Problem

`Map K V` is declared but key constraints are absent. Without knowing whether `K` requires `Eq`, `Hash`, or both, the implementation cannot choose a data structure.

### Suggested Fix

Add to §6.7:

```
Map K V requires: Eq K, Hash K
  Keys are compared using Eq and bucketed using Hash.
  Iteration order is unspecified (hash-map semantics).
  For ordered maps, use SortedMap K V from aivi.map (requires Ord K).

Set A requires: Eq A, Hash A
  Same structural rules as Map with unit values.
  For ordered sets, use SortedSet A from aivi.map (requires Ord A).

The Hash class is:

    class Hash A
        hash : A -> Int

Compiler-derived Hash is provided for:
  Int, Text, Bool, Unit, Bytes,
  tuples, records, sums, and Lists
  whose element/field types are all Hash.
```

//-

## 28. String Operations — Only Interpolation Shown

**Severity:** 🟢 LOW  
**Section:** §19

### Problem

Text interpolation is specified but basic Text operations (length, concat, split, etc.) are only implied. Without them, most real programs cannot be written.

### Suggested Fix

Add **§19.3 Text operations** (pointing to stdlib):

```
Core Text operations are provided by aivi.text.
See §26 for the full inventory.

Interpolation:
    "{firstName} {lastName}"

Concatenation is done via interpolation or aivi.text.concat:
    aivi.text.concat [firstName, " ", lastName]

There is no `+` operator for Text. Use interpolation or concat.
```

//-

## 29. Integer Overflow and Float NaN — Unspecified

**Severity:** 🟢 LOW  
**Section:** §6.2

### Problem

Integer overflow and floating-point edge cases (NaN, Infinity, -0.0) are never addressed.

### Suggested Fix

Add to §6.2.1 (covered in gap #4 above, repeating for completeness):

```
Int overflow: runtime trap (panic) in both debug and release builds.
  Wrapping or saturating arithmetic is available via aivi.math:
    Math.addWrapping, Math.addSaturating, etc.

Float NaN: NaN is never constructible by user code directly.
  Division by zero produces a runtime error (not NaN).
  Decode sources that encounter NaN/Infinity produce a decode error.
  Rationale: silent NaN propagation is a well-known source of bugs
  in functional programs.

Float -0.0: -0.0 == 0.0 is True per Eq Float.
  aivi.math.isNegativeZero is provided for the rare case where
  the distinction matters.
```

//-

## 30. LSP and Tooling — Entirely Absent

**Severity:** 🟢 LOW  
**Section:** (absent)

### Problem

No LSP, REPL, or debugger spec. These are important for adoption but not blocking for v1 compilation.

### Suggested Fix

Add **§27 Tooling roadmap** (non-normative):

```
The following tooling is expected but not normative for v1:

Language Server (LSP):
  - Hover: show inferred type at cursor
  - Completion: top-level names, record fields, pipe continuations
  - Go-to-definition
  - Find references
  - Rename
  - Inline diagnostics mirroring the compiler's error taxonomy (§21.1)

REPL:
  - Expression evaluation in an isolated signal-free context
  - sig declarations create a minimal scheduler loop
  - Useful for exploring types and pure expressions

Debugger:
  - Signal graph inspector (show current signal values and dep graph)
  - Scheduler trace (which signals propagated in last N ticks)
  - No source-level step debugging in v1 (Cranelift constraint)
```

//-

## Summary Table

| # | Gap | Severity | Section |
|//-|////-|//////////|////////-|
| 1 | `Action` type undefined | 🔴 | §15.3 |
| 2 | Comment syntax absent | 🔴 | §5 |
| 3 | Module system (scoping, qualified names) | 🔴 | §5 |
| 4 | Literal syntax (Int range, Float, Decimal, overflow) | 🔴 | §6.2 |
| 5 | Operator precedence table | 🔴 | §11 |
| 6 | Type inference and constraint-solving algorithm | 🔴 | §4.3, §7.1 |
| 7 | IR node/ownership/span/validation/pretty-print specs | 🔴 | §3.5, §4 |
| 8 | Scheduler transaction semantics and deadlock proof | 🔴 | §13.3, §16.3 |
| 9 | Signal cycle detection | 🔴 | §13.1 |
| 10 | `<each>` key reconciliation algorithm | 🔴 | §17.3.2 |
| 11 | GTK widget mapping table | 🔴 | §17.1.1 |
| 12 | Standard library inventory | 🔴 | global |
| 13 | `Eq` for `Map`, `Set`, `Bytes` deferred | 🟡 | §7.3 |
| 14 | `Default` bundles — only `Option` specified | 🟡 | §9.2 |
| 15 | `?|>` gate subscription and reactive predicate | 🟡 | §11.3 |
| 16 | `<show keepMounted>` handler/subscription behavior | 🟡 | §17.3.1 |
| 17 | HTTP `activeWhen` — non-normative "may be" | 🟡 | §14.1.2 |
| 18 | `refreshOn` backpressure | 🟡 | §14.1.2 |
| 19 | Recurrence (`@|>` / `<|@`) termination and repr | 🟡 | §11.7 |
| 20 | Orphan/overlapping instance rules vague | 🟡 | §7.1 |
| 21 | Domain `Eq` opt-out syntax missing | 🟡 | §20.9 |
| 22 | Diagnostics: no taxonomy or error codes | 🟡 | §21 |
| 23 | `<match>` pattern grammar | 🟡 | §17.3.3 |
| 24 | Formatter: no algorithm or line-length rules | 🟡 | §22 |
| 25 | Panic / unrecoverable error semantics | 🟡 | absent |
| 26 | Anonymous function syntax | 🟡 | §10 |
| 27 | `Map`/`Set` key constraints (`Hash`?) | 🟡 | §6.7 |
| 28 | Text operations (only interpolation shown) | 🟢 | §19 |
| 29 | Integer overflow and float NaN | 🟢 | §6.2 |
| 30 | LSP, REPL, debugger tooling | 🟢 | absent |
