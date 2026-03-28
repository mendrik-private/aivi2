
Aivi Language Specification (Clean Draft v1)

============================================================
0. Document Structure
============================================================

This document is split into:

- Normative Specification: strict, implementation-defining rules
- Non-Normative Notes: examples and clarifications

All duplication and ambiguity have been removed. Terminology is canonical.

============================================================
1. Glossary (Normative)
============================================================

- Value: immutable pure data (no lifecycle)
- Signal A: time-varying node carrying values of type A
- Domain: capability layer over a carrier (operators, pipes, semantics)
- Carrier: domain-level semantic type (may define behavior)
- Result E A: failure channel type (Err E | Ok A)
- Option A: optional value (None | Some A)
- Epoch: one invalidation cycle of a signal
- Source: acquisition/action definition
- Action: effectful operation exposed via .do
- Node: compiled graph unit

============================================================
2. Core Model (Normative)
============================================================

The language is statically typed and signal-first.

- Values are immutable and pure
- Signals form a dependency graph
- Signals are required for:
  - external input
  - async acquisition
  - UI/events
  - environment values

The compiler builds a graph and:
- schedules minimal recomputation
- identifies parallelism
- enforces dependency order

There are no:
- nulls
- loops
- mutation of values
- implicit side effects

============================================================
3. Signal Semantics (Normative)
============================================================

Signal A represents a node emitting values of type A.

Meta-state properties (fixed):

- .running : Signal Bool
- .done    : Signal Bool
- .error   : Signal (Option Error)
- .stale   : Signal Bool

.changed is NOT part of the language.

------------------------------------------------------------
Initial State
------------------------------------------------------------

At epoch creation:

- running = false
- done    = false
- stale   = true
- error   = None

Exception:

- Environment signals may start with done = true if value is immediately available.

------------------------------------------------------------
State Transitions
------------------------------------------------------------

- running = true during evaluation/acquisition
- done    = true when first settled value is available
- stale   = false when done becomes true
- error   = Some e on failure, otherwise None

.done never reverts within the same epoch.

============================================================
4. Types and Failure Model (Normative)
============================================================

All external data must be validated.

Failure channel:

- Represented using Result E A
- Success path carries A
- Failure path carries E

Rules:

- No silent failure
- No implicit success fallback
- Validation failure must propagate through Result

.error reflects current failure state:
- Some e if failed
- None otherwise

============================================================
5. Syntax (Normative)
============================================================

Keywords:
signal, source, value, type, data, domain, result, view, adapter

Lambda:
x => expr

Ambient:
. is lambda shorthand

Examples:
. + 1
.lastname

============================================================
6. Operators and Precedence (Normative)
============================================================

From highest to lowest:

1. Field access / projection (.)  
2. Function application  
3. Unary operators  
4. Multiplicative / additive operators  
5. Pipe operators  

Pipe precedence (left-associative):

 |>    (plain)
*|>   (map)
!|>   (validate)
?|>   (guard)
||>   (fallback)
&|>   (join)
~|>   (previous)
+|>   (accumulate)
-|>   (diff)
T|>, F|> (branch)
@|>   (source)

Example:
a &|> b |> f  ==  (a &|> b) |> f

============================================================
7. Pipes (Normative)
============================================================

|>    apply stage
*|>   map
!|>   validate
?|>   guard
||>   fallback
&|>   combine
~|>   previous state
+|>   accumulate state
-|>   diff
T|>, F|> branch
@|>   source boundary

============================================================
8. Accumulate Pipe +|> (Normative)
============================================================

Form:

signal +|> seed (state input => next)

Behavior:
- state persists across emissions
- first state = seed

Shorthand:

signal +|> prev + .

prev = previous state
.    = current value

============================================================
9. Source Model (Normative)
============================================================

Sources define acquisition behavior.

Identity is determined by:
- method
- URL
- headers
- body
- identity-marked fields

Rules:
- identical in-flight requests are deduplicated
- only one execution per identity
- stale cache may serve during refresh
- retries respect throttle

============================================================
10. Actions (.do) (Normative)
============================================================

Actions are typed effect nodes.

Typing:

signal.do.action : (Input?) -> ActionResult E A

Where:
- A = success value
- E = failure type

Rules:
- invocation is graph construction (not immediate execution)
- execution follows scheduling rules
- cancellation follows source rules
- result integrates into signal graph

Two kinds:

- source actions (namespace-level)
- resource actions (attached to signal)

============================================================
11. Domains (Normative)
============================================================

Domains define:

- operators
- branch semantics
- pipe support
- numeric suffixes
- diff rules

Capability resolution:

- must resolve to exactly one domain
- otherwise compile-time error

============================================================
12. Environment (Normative)
============================================================

env.NAME : Signal Text (default)

Rules:
- must validate into required type
- missing required value → runtime error
- validation failure → Result failure

============================================================
13. Results (Normative)
============================================================

result defines graph assembly.

- fields are nodes
- dependencies define execution
- no implicit ordering

============================================================
14. Non-Normative Notes
============================================================

- Signals behave like reactive graph nodes
- +|> is the primary state mechanism
- Domains act like typeclasses
- Sources unify async + caching + scheduling

============================================================
End of Spec
============================================================
