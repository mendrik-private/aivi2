# AIVI syntax sheet for LLM code generation

> Extracted from `AIVI_RFC.md` and spot-checked against real `.aivi` files in this repo.
>
> Goal: generate valid, non-hallucinated AIVI.
>
> Rule zero: if a form is not on this sheet, do **not** invent it.

## 1. Non-negotiable language shape

- Pure by default.
- Closed types by default.
- No `null` / `undefined`; use `Option A`.
- No `if` / `else`.
- No loops.
- Pipe algebra is core surface syntax.
- `Signal` is applicative, **not** monadic.
- `Validation` is applicative, **not** monadic.
- Text composition uses interpolation, not string concatenation operators.
- Top-level UI roots use `value`, not a dedicated `view` keyword.
- There is no `adapter` keyword.

If you need:

| Need | Use |
|---|---|
| Pure constant | `value` |
| Pure function | `func` |
| Reactive derived value | `signal` |
| External event/data stream | body-less `signal : Signal T` plus `@source` or GTK event routing |
| One-shot effect | `Task E A` |
| Independent validation/effect combination | `&\|>` applicative cluster |
| Immutable record update | `<\|` structural patch |
| Conditional expression flow | `\|\|>`, `T\|>`, `F\|>`, `?\|>` |
| Conditional UI | `<show>` or markup `<match>` |
| Repeated UI children | `<each key={...}>` |

## 2. Top-level forms

Allowed top-level forms:

- `type`
- `class`
- `instance`
- `domain`
- `value`
- `func`
- `signal`
- `from`
- `use`
- `export`
- `provider`
- decorators `@name` including `@source`, `@recur.timer`, `@recur.backoff`

### 2.1 Comments

```aivi
// line comment
/* block comment */
/** doc comment **/
```

### 2.2 `value` and `func`

```aivi
value answer = 42

value user : User = {
    name: "Ada",
    age: 36
}

type Int -> Int -> Int
func add = x y =>
    x + y

type Eq A => A -> Bool
func same = v =>
    v == v

type Int -> Int -> Int
func addFrom = amount value => value
  |> add amount

type Counter -> Int -> Counter
func bump = counter delta => counter
    <| {
        total: counter.total + delta
    }

type State -> Int
func readNested = state => state.x.y.z
  |> addOne
```

Rules:

- `value` = constant binding only; uses `=`.
- `func` = function declaration; uses `=` after the name, then parameters plus `=>`.
- Anonymous lambda expressions also use `=>` and may take one or more named parameters: `value isCell = coord => coord == cell`, `value next = 0 |> x => x + 1`.
- Function signatures live on a preceding `type` line: `type Int -> Int -> Int`.
- Inside `from` blocks, the same standalone `type` line form attaches to the immediately following entry.
- `func` headers keep parameters unannotated: `func add = x y => ...`.
- Shorthand subject lambdas are available only for unary composed dot-rooted expressions: `. == cell` means `value => value == cell`, `.score >= threshold` means `value => value.score >= threshold`. Bare `.` and `.field` keep their existing ambient-subject meaning.
- Ignored unary inputs stay explicit: `func constant = _ => ...`.
- Constraint prefixes, when present, live on the `type` line: `type Eq A => A -> Bool`.
- `value` is a contextual keyword and can still be a parameter name:

```aivi
type Int -> Int
func absolute = value =>
    value < 0 T|> 0 - value
     F|> value
```

### 2.3 `type`

```aivi
type Bool = True | False

type Option A = None | Some A

type Result E A = Err E | Ok A

type User = {
    name: Text,
    age: Int
}

type Player = {
    | Human
    | Computer

    type opponent : Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human
}
```

Rules:

- ADTs are closed by default.
- Constructors are curried ordinary values.
- Under-application is legal; over-application is a type error.
- Records are built with record literals, not curried record constructors.
- `type Name = { ... }` is parsed as a companion sum body only when the first significant token
  inside the braces is `|`; otherwise it remains record-type syntax.
- In a companion sum body, constructors must come before companion members.
- Companion members elaborate to ordinary top-level callables and use ordinary `use` / `export`
  rules.
- Companion member `type` lines spell the full function type, including the receiver.
- Companion member bodies use ordinary function forms such as `name = self => ...`.

### 2.4 `class` and `instance`

```aivi
class Eq A = {
    type (==) : A -> A -> Bool
}

class Applicative F = {
    with Apply F
    type pure : A -> F A
}

instance Eq A => Eq (Option A) = {
    (==) = left right => True
}
```

Rules:

- Class head: `class <Name> <TypeParam>+ = { ... }`
- Superclasses use body-level `with <Constraint>`.
- Per-parameter constraints use body-level `require <Constraint>`.
- Class members have shape `type <member> : [ConstraintPrefix =>] <Type>`.
- Instance heads may use constraint prefixes: `instance Eq A => Eq (Option A)`.
- Instance member bodies use `name = expr` or `name = arg1 arg2 => expr`.
- `implements` is **not** syntax.
- `requires` is **not** syntax; use `require`.
- Overlapping instances are not allowed.
- Orphan instances are fully disallowed.
- Imported user-instance lookup is deferred; same-module user instances are the safe assumption.

### 2.5 `domain`

```aivi
domain Duration over Int = {
    suffix ms
    type ms : Int
    ms = n => Duration n
    suffix sec
    type sec : Int
    sec = n => Duration (n * 1000)
    type millis : Int -> Duration
    millis = raw => raw
    type toMillis : Duration -> Int
    toMillis = duration => duration
}

domain Url over Text = {
    type parse : Text -> Result UrlError Url
    type scheme : Url -> Option Text
    type host : Url -> Option Text
}
```

Rules:

- A domain is nominal over a carrier type.
- Construction is explicit.
- Unwrapping is explicit.
- No implicit casts to/from the carrier.
- Domain bodies use brace syntax: `domain Name over Carrier = { ... }`.
- Domain members use `type member : TypeExpr`.
- Callable domain members participate in ordinary term lookup.
- Callable domain members may include an authored body of the form `name = expr`. When the body is function-shaped, the canonical surface is `name = arg1 arg2 => expr`.
- Authored bodies may use the contextual keyword `self` to refer to the domain-typed receiver. When `self` is used, the type annotation may omit the domain type from the first position because it is implicit. When `self` is not used, the annotation is the full type.
- Authored bodies are checked against the carrier view of the current domain, while the public signature remains nominal.
- Bodyless members stay as annotation-only declarations.
- Domain suffixes use `suffix name`, followed by `type name : BaseType` and an ordinary body binding.
- Unary domain methods can also be projected with `value.member`.
- Domain projection is still explicit member dispatch; it does not imply implicit carrier casts.

### 2.6 `signal`

```aivi
signal counter = 0
signal fullName = "{firstName} {lastName}"
signal clicked : Signal Unit
signal query : Signal Text
```

Rules:

- `signal name = expr` creates a derived signal.
- Body-less annotated `signal name : Signal T` creates an input signal.
- `value` must not depend on signals.
- Signals can depend on signals and pure helpers.

### 2.6.1 `from` signal fan-out sugar

```aivi
from state = {
    boardText: renderBoard
    dirLine: .dir |> dirLabel
    type Int -> Bool
    atLeast threshold: .score >= threshold
    type Bool
    gameOver: .status
        ||> Running -> False
        ||> GameOver -> True
}
```

Rules:

- `from source = { ... }` creates one top-level binding per entry.
- Zero-parameter entries lower to derived `signal`s.
- Parameterized entries require a preceding standalone `type` line inside the same block.
- That `type` line attaches only to the immediately following entry; leaving it orphaned at the end of the block is an error.
- Each entry body is desugared as if `source` were piped into it.
- `label: renderBoard` becomes `signal label = source |> renderBoard`.
- `label: .dir |> dirLabel` becomes `signal label = source |> .dir |> dirLabel`.
- `atLeast threshold: .score >= threshold` lowers to a top-level selector function. Its surface annotation is written without the final `Signal`, so `type Int -> Bool` means `Int -> Signal Bool` internally.
- Later entries may reference earlier entries from the same `from` block.
- Deeper-indented continuation lines belong to the current entry, so `||>` pipe-case arms stay inside that derived signal body.

### 2.7 `@source`

```aivi
@source http.get "/users"
signal users : Signal (Result HttpError (List User))

@source timer.every 120
signal tick : Signal Unit

@source http.post "/login" with {
    body: creds,
    headers: authHeaders,
    decode: Strict,
    timeout: 5sec
}
signal login : Signal (Result HttpError Session)
```

Rules:

- Form: `@source provider.variant args [with { ... }]` followed by a body-less `signal`.
- `@source` may decorate only a body-less `signal`.
- Provider and variant are statically resolved.
- Unknown or duplicate options are compile-time errors.
- Source args/options may be reactive expressions with statically known dependencies.

### 2.8 `provider`

```aivi
provider my.data.source
    wakeup: providerTrigger
    argument url : Url
    option timeout : Duration
    option retries : Int
```

### 2.9 `use` and `export`

```aivi
use aivi.defaults (
    Option
    defaultText
    defaultInt
    defaultBool
)

use my.client (fetch as clientFetch)

export main
export Url
export (Option, Result, Signal)
```

Rules:

- `use module (member)` imports selected names.
- Alias syntax: `use module (member as localName)`.
- No wildcard imports.
- No arbitrary value-level module qualification for imported members.
- Use aliases to disambiguate colliding names.
- Real repo usage confirms both `export name` and grouped `export (...)`.
- A module may export at most one `main` binding.

### 2.10 Top-level markup roots use `value`

```aivi
value mainWindow =
    <Window title="My App">
        <Box orientation="vertical">
            <Label text={greeting} />
        </Box>
    </Window>
```

There is no dedicated `view` declaration keyword.

## 3. Core types and literals

### 3.1 Core types

```text
Int
Float
Decimal
BigInt
Bool
Text
Unit
Bytes
List A
Map K V
Set A
Option A
Result E A
Validation E A
Signal A
Task E A
```

Kinds:

```text
Type
Type -> Type
Type -> Type -> Type
```

Examples:

- `Int : Type`
- `Option : Type -> Type`
- `Signal : Type -> Type`
- `Result : Type -> Type -> Type`
- `Task : Type -> Type -> Type`

Partial type-constructor application is allowed:

- valid: `Option`, `Signal`, `Result HttpError`, `Task FsError`
- invalid: using `Result` where a unary constructor is required

### 3.2 Numeric literals

Accepted surface:

```text
0
42
9000
0.5
3.14
19d
19.25d
123n
-1
-3.4
-19d
-123n
250ms
10sec
3min
-250ms
```

Rules:

- Unsuffixed integers are decimal only.
- `Float` uses one decimal point.
- `Decimal` uses trailing `d`.
- `BigInt` uses trailing `n`.
- Domain suffix literals use `digits + suffix` with a suffix of at least two ASCII letters.
- Spacing is semantic:
  - `250ms` = one suffixed literal candidate
  - `250 ms` = ordinary application
  - `-3` is valid
  - `- 3` is not a negative literal token
- No `_` separators.
- No hex/binary/octal integer literals.
- No exponent notation.
- Domain suffix resolution is compile-time only and current-module only.

### 3.3 Text and regex

```aivi
"{name} ({status})"
rx"\d{4}-\d{2}-\d{2}"
```

Rules:

- Text composition uses interpolation.
- String concatenation is not a core language feature.
- Regex is a first-class compiled type with literal syntax `rx"..."`.

### 3.4 Records, tuples, lists, maps, sets

```aivi
(1, 2)
{ name: "Ada", age: 36 }
[1, 2, 3]
Map { "x": 1, "y": 2 }
Set [1, 2, 4]
```

Rules:

- Plain `{ ... }` is always a record.
- Plain `[ ... ]` is always a list.
- `Map { ... }` is a map literal.
- `Set [ ... ]` is a set literal.
- Records and sums are closed by default.

### 3.5 Record row transforms

Type-level record transforms:

```aivi
Pick (name, age) User
Omit (isAdmin) User
Optional (nickname, email) User
Required (nickname) User
Defaulted (nickname) User
Rename { createdAt: created_at } User
```

Type-level piping is allowed:

```aivi
User
|> Omit (isAdmin)
|> Rename { createdAt: created_at }
```

## 4. Defaults and record shorthand

### 4.1 `Default`

```aivi
class Default A
    default : A
```

### 4.2 `aivi.defaults`

```aivi
use aivi.defaults (
    Option
    defaultText
    defaultInt
    defaultBool
)
```

### 4.3 Record omission

```aivi
type User = {
    name: Text,
    nickname: Option Text,
    email: Option Text
}

use aivi.defaults (Option)

value user : User = {
    name: "Ada"
}
```

This elaborates to:

```aivi
value user : User = {
    name: "Ada",
    nickname: None,
    email: None
}
```

Rules:

- Omission works only when the expected closed record type is known.
- Omitted fields must have supported default evidence.
- This does not open records.
- This does not weaken strict decoding.

### 4.4 Record shorthand

```aivi
value game:Game = {
    snake,
    food,
    status,
    score
}

game
 ||> { snake, food, status, score } -> score
```

Rules:

- Shorthand works only when the expected closed record type is known.
- Construction shorthand requires a same-named local binding in scope.
- Pattern shorthand must still be unambiguous.

## 5. Expression model

### 5.1 No `if` / `else`, no loops

- Branch with `||>`, `T|>`, `F|>`, or `?|>`.
- Repeat with recursion, collection combinators, or source/retry/interval flows.

### 5.2 Ambient subject

Inside a pipe:

- `.` = current subject
- `.field` = project field from current subject
- `.field.subfield` = chained projection
- `_` = discard only, never ambient subject

Important:

- Ordinary field access on named values still uses `value.field`.
- `.field` is illegal where no ambient subject exists.
- Selected-subject headers (`param!`, `param { path! }`) synthesize the initial subject so a
  function body can begin with `|>` or `<|` without an explicit `=>`.

### 5.3 Ordinary-expression precedence

From tighter to looser:

1. function application
2. prefix `not`
3. `*`, `/`, `%`
4. `+`, `-`
5. `>`, `<`, `>=`, `<=`, `==`, `!=`
6. `and`
7. `or`
8. `<|` patch application

Rules:

- `<|` is right-associative.
- Pipe operators are **not** part of the ordinary binary precedence table.
- A pipe spine starts from one ordinary expression and consumes stages left-to-right.
- Selected-subject function/companion headers desugar to that same ordinary-expression head before
  the continuation is parsed.
- `==` / `!=` typecheck through equality-class evidence.
- `<`, `>`, `<=`, and `>=` typecheck through `Ord.compare : A -> A -> Ordering`; the four
  surface operators, and sections like `(<)` / `(>=)`, are sugar over that primitive.

## 6. Pipe algebra

### 6.1 Core operators

| Operator | Shape | Meaning / guardrails |
|---|---|---|
| `|>` | `x |> f` or `x |> .field` | transform current subject |
| Pipe memo `#name` | `x |> #before f before #after` | name a stage input/result without leaving the pipe |
| `?|>` | `x ?|> predicate` | gate; predicate must be pure `Bool` |
| `||>` | `x ||> Pattern -> expr` | case split / pattern match |
| `T|>` / `F|>` | adjacent pair in one spine | truthy/falsy sugar for canonical carriers |
| `*|>` | `xs *|> body` | map / fan-out; pure mapping only |
| `<|*` | `xs *|> f <|* g` | explicit join after `*|>` only |
| `|` | `x | observer` | tap; ignores observer result |
| `!|>` | `x !|> validate` | dependent validation stage; body must return `Result` or `Validation`, preserving any existing carrier/error type |
| `~|>` | `signal ~|> seed` | previous committed value, seeded form |
| `+|>` | `signal +|> seed step` | stateful accumulation |
| `-|>` | `signal -|> diffFn` | diff current vs previous |
| `&|>` | applicative cluster stage | combine independent values under one applicative |
| `@|>` / `<|@` | recurrent region | explicit recurrence; avoid unless runtime lowering target is known |
| `<|` | `target <| patch` | structural patch application |

### 6.1.1 Pipe memos `#name`

```aivi
value total : Int = 20
 |> #before before + 1 #after
  |> after + before
```

Rules:

- `operator #name expr` binds the stage input for that stage body only.
- `operator expr #name` binds the stage result for the rest of the pipe after that stage.
- Both forms may appear on the same stage.
- Supported on ordinary pipe stages: `|>`, `|`, `?|>`, `||>`, `T|>`, `F|>`, `*|>`, `<|*`, `!|>`, `~|>`, `+|>`, `-|>`, `@|>`, and `<|@`.
- Temporal replay stages use ordinary `|>` with reserved heads: `|> delay <duration>` and `|> burst <duration> <count>`.
- `||>` runs and `T|>` / `F|>` pairs share memo flow across the grouped branches. Put the same result memo name on each arm when the merged branch result is needed later.
- `&|>` applicative clusters follow separate applicative-cluster semantics rather than the single-subject memo flow above.

### 6.2 `result { ... }`

```aivi
value total =
    result {
        left <- Ok 20
        right <- Ok 22
        left + right
    }
```

Rules:

- Each `<-` binding must produce `Result E A`.
- `Ok` payloads enter scope for the rest of the block.
- First `Err` short-circuits.
- Final line is wrapped in `Ok`.
- If the final line is omitted, the last bound name is returned implicitly.

### 6.3 `?|>` gate

```aivi
users ?|> .active
```

Rules:

- Predicate is typed against the ambient subject and must return `Bool`.
- Ordinary-value semantics: returns `Option A`.
- `Signal` semantics: forwards matching emissions and suppresses non-matching ones.
- `?|>` is not a general two-branch operator; use `||>` when branches compute unrelated shapes.

### 6.4 `||>` case split

```aivi
status
 ||> Paid    -> "paid"
 ||> Pending -> "pending"

xs
 ||> []                       -> 0
 ||> [first]                  -> first
 ||> [first, second, ...rest] -> first + second + sum rest
```

Rules:

- Current checked rollout supports pattern arms.
- Case-stage guard syntax is not wired end to end; do extra conditions in the arm body or via helper logic.
- `...rest` is list-only and must be final.
- Use `_` when an explicit catch-all is needed.

### 6.5 `T|>` and `F|>`

```aivi
ready
 T|> start
 F|> wait

maybeUser
 T|> greet .
 F|> showLogin

loaded
 T|> render .
 F|> showError .
```

Canonical truthy/falsy pairs:

- `True` / `False`
- `Some _` / `None`
- `Ok _` / `Err _`
- `Valid _` / `Invalid _`

Rules:

- `T|>` and `F|>` must appear as an adjacent pair in one pipe spine.
- The subject type must have a known canonical truthy/falsy pair.
- `.` is rebound to the matched payload when the matched constructor has exactly one payload.
- Use `||>` for named bindings, nested patterns, or more than two constructors.

### 6.6 `*|>` and `<|*`

```aivi
users
 *|> .email

users
 *|> .email
 <|* keepEmails
```

Rules:

- For `List A`, `*|>` maps `A -> B` to `List B`.
- For `Signal (List A)`, the map lifts pointwise.
- `*|>` does not flatten.
- `*|>` does not sequence `Task`s.
- `<|*` is legal only immediately after a `*|>` segment.

### 6.7 `|` tap

```aivi
value
 |> compute
 |  debug
 |> finish
```

Rules:

- Tap observes the subject and leaves it unchanged.
- The tap result is ignored.

### 6.8 `&|>` applicative clusters

```aivi
signal validatedUser =
 &|> validateName nameText
 &|> validateEmail emailText
 &|> validateAge ageText
  |> UserDraft
```

Rules:

- Every cluster member must have the same outer applicative constructor.
- Safe builtin carriers: `List`, `Option`, `Result`, `Validation`, `Signal`, `Task`.
- Finalizer must be a pure function or constructor.
- If no explicit finalizer appears before pipe end, the cluster defaults to a tuple constructor.
- Pipe memos are part of the ordinary single-subject pipe flow and do not currently participate inside `&|>` clusters.
- Inside an unfinished cluster:
  - no ambient `.field` projections unless nested under an explicit subject
  - no `?|>` or `||>`

### 6.9 Structural patches

```aivi
updated = target <| {
    profile.name: "Grace"
    items[.active].price: 3
}

promote : User -> User
promote = patch {
    isAdmin: True
}

stored = target <| {
    callback: := someFunction
}

smaller = target <| {
    obsoleteField: -
}
```

Rules:

- `target <| { ... }` applies a patch.
- `patch { ... }` creates a reusable patch function `A -> A`.
- Root selectors may omit the leading dot: `name` and `.name` are equivalent at patch root.
- Nested selectors require explicit dots.
- Patch entries are **not** record fields; they are path updates.
- RFC examples format patch entries one per line; do not assume record-style comma syntax.
- Plain patch instructions replace the selected value, or transform it when the expression has type `A -> A`.
- `:=` stores a function value as data rather than applying it.
- `field: -` removes a field and shrinks the record type; it is a compile error if the field does not exist or downstream typing still requires it.
- Current executable support is partial for some patch-lowering paths; syntax is still normative.

## 7. Signals, tasks, and sources

### 7.1 Signal rules

```aivi
signal x = 3
signal y = x + 5
```

Rules:

- A signal referenced inside a `signal` is read as its current committed value.
- Signal dependencies are extracted after elaboration.
- Derived-signal dependency graphs are static after elaboration.
- `Signal` is not a `Monad`.

### 7.2 Input signals

```aivi
signal clicked : Signal Unit
signal query : Signal Text
```

Rules:

- Type annotation is mandatory for body-less input signals.
- Input signals are externally publishable entry points.
- GTK events and runtime completions route into these.

### 7.3 `+|>` accumulation

```aivi
signal counter : Signal Int = tick
 +|> 0 step

type Unit -> Int -> Int
func step = tick current =>
    current + 1
```

Rules:

- Checked form: `signalSource +|> seed step`
- Step function shape: `input -> state -> state`

### 7.4 Signal merge and reactive arms

```aivi
signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowUp") => Turn North
  ||> keyDown (Key "ArrowDown") => Turn South
  ||> _ => Tick

signal total : Signal Int = ready
  T|> left + right
  F|> 0
```

Rules:

- The merge expression (`sig1 | sig2`) lists the source signals that feed the declaring signal.
- Each source must name a previously declared local `signal`.
- Multi-source arms: `||> <source-name> <pattern> => <body>` — source name prefix required, must match a signal in the merge list.
- Single-source arms: `||> <pattern> => <body>` — no source name prefix needed.
- Default arm: `||> _ => <body>` — required; provides the initial value before any source fires and handles unmatched cases.
- Pattern binders introduced by an arm are only in scope for that arm body.
- Body type must match the declaring signal's payload type.
- No ambient subject (`.`) inside arm bodies.
- Self-reference: the declaring signal cannot read itself from its own arm bodies.
- If multiple sources fire in one tick, later arm in source order wins.

Current implementation note:

- Signal merge arms lower into the same reactive update clause mechanism as before.
- Guards and bodies are ordinary expressions with direct signal references.
- The surface lowers through existing pipe/case machinery rather than a separate pattern runtime.
- Standalone compile/startup integration remains narrower than the fully checked frontend/runtime test surface.

### 7.5 `Task E A`

Rules:

- `Task E A` is the one-shot user-visible effect carrier.
- Use it for effectful computation that may fail with `E` or succeed with `A`.
- Builtin executable support is applicative today.
- Do not assume broad monadic `Task` ergonomics are currently executable.

### 7.6 `@source` providers

Builtin provider families in the RFC:

- `http.*`
- `timer.*`
- `fs.watch`
- `fs.read`
- `socket.connect`
- `mailbox.subscribe`
- `process.spawn`
- `window.*`
- `dbus.*`

Conservative rules:

- Provider kind is static.
- Reactive source config changes reconfigure runtime instances; they do not change the static graph shape.
- Decode happens before scheduler publication.
- Decode errors flow through typed error channels.

## 8. Markup / GTK surface

### 8.1 Core markup shape

```aivi
value mainWindow =
    <Window title="My App">
        <Box orientation="vertical" spacing={8}>
            <Label text={greeting} />
            <Button label="Click me" onClick={clicked} />
            <Button label="Select Alpha" onClick={selectRow "Alpha"} />
        </Box>
    </Window>
```

Rules:

- Markup roots are ordinary `value` bindings.
- Attributes may be literal (`label="Click me"`) or expression-valued (`text={greeting}`).
- Reactive expressions lower to derived bindings/setters; this is not a virtual DOM.

### 8.2 Executable widget catalog

Current live GTK widget catalog named in the RFC:

- `Window`
- `Box`
- `ScrolledWindow`
- `Label`
- `Button`
- `Entry`
- `Switch`

Do not hallucinate arbitrary JSX/HTML/DOM widgets as automatically valid.

### 8.3 Event routing

```aivi
signal clicked : Signal Unit
```

Rules:

- The handler expression must name a directly publishable input signal.
- Only direct input signals are legal in the current executable slice.
- Arbitrary callback expressions are future work.
- Unsupported widget/event pairs are rejected by the run surface.

### 8.4 Control nodes

#### `<show>`

```aivi
<show when={isVisible}>
    <Label text="Ready" />
</show>
```

Optional:

```aivi
<show when={isVisible} keepMounted={True}>
    ...
</show>
```

#### `<each>`

```aivi
<each of={items} as={item} key={item.id}>
    <Row item={item} />
    <empty>
        <Label text="No items" />
    </empty>
</each>
```

Rules:

- `of` must yield `List A`.
- `key` is required.

#### `<match>`

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

- Cases use ordinary AIVI patterns.
- Exhaustiveness follows ordinary match rules where the scrutinee type is locally provable.

#### `<fragment>`

```aivi
<fragment>
    <Label text="A" />
    <Label text="B" />
</fragment>
```

#### `<with>`

```aivi
<with value={formatUser user} as={label}>
    <Label text={label} />
</with>
```

Rules:

- `<with>` introduces a pure local binding for a subtree.
- It does not create an independent signal node.

## 9. Patterns and predicates

### 9.1 Pattern rules

Allowed patterns include:

- constructors: `Some value`, `Ok item`, `Paid`
- wildcard: `_`
- record field-subset patterns: `{ snake, food, status, score }`
- list patterns: `[]`, `[first]`, `[first, second, ...rest]`
- nested constructor patterns

Rules:

- Sum matches must be exhaustive unless `_` is present.
- Boolean matches must cover `True` and `False` unless `_` is present.
- `...rest` is list-only and final.
- Ordered head/rest destructuring is for lists only, not maps/sets.

### 9.2 Predicates

```aivi
users |> filter (.active and .age > 18)
xs    |> takeWhile (. < 10)
```

Allowed predicate forms:

- `.`
- `.field`
- `.field.subfield`
- `and`, `or`, `not`
- `==`, `!=` when `Eq` exists

Important:

- `x == y` desugars to `(==) x y`.
- `x != y` desugars to `not (x == y)`.
- `(!=)` is not its own class member.

## 10. Anti-patterns: do not write AIVI like another language

### 10.1 Elm / Haskell / OCaml style mistakes

| Do not write | Use instead |
|---|---|
| `if cond then a else b` | `cond T|> a F|> b` or `expr ||> Pattern -> ...` |
| `case x of ...` | `x ||> Pattern -> expr` |
| `match x with ...` | `x ||> Pattern -> expr` or markup `<match on={x}>` |
| `Maybe a` / `Either e a` | `Option A` / `Result E A` |
| `do` notation / `>>=` on `Signal` | `&|>` or explicit source/runtime nodes |
| open records / row-polymorphic record assumptions | closed records plus `Pick` / `Omit` / `Rename` |
| `{ record | field = value }` | `record <| { field: value }` |
| `\x -> ...` | not specified here; prefer named `func` or existing in-repo precedent only |
| OCaml-style `fun x -> ...` | not AIVI `func`; AIVI uses a named `func` with a leading `type` signature |
| `where` blocks | not specified here |
| `let ... in ...` | not specified here |

### 10.2 JS / TS / React style mistakes

| Do not write | Use instead |
|---|---|
| `null`, `undefined` | `Option A` |
| mutable component state / hooks | `signal` |
| `useEffect(...)` | `Task` or `@source` |
| JSX fragments `<>...</>` | `<fragment>...</fragment>` |
| arbitrary callback expressions in events | direct input signal routing like `onClick={clicked}` or `onClick={selectRow item}` |
| DOM / virtual DOM mental model | direct GTK widget lowering |
| arbitrary HTML tags | current GTK widget catalog only |

### 10.3 Import / module mistakes

| Do not write | Use instead |
|---|---|
| `import Foo exposing (...)` | `use foo (...)` |
| wildcard imports | explicit `use module (member, other)` |
| arbitrary qualified imported value access | import the member and optionally alias it |
| assume module header syntax like `module Foo exposing (...)` | not defined by this sheet; current toolchain derives module names from file paths |

### 10.4 Signal / FRP mistakes

| Do not write | Use instead |
|---|---|
| `Signal.bind`, monadic dynamic rewiring | static signal graph plus explicit source/runtime nodes |
| use a `Signal` inside a `value` | move the computation to `signal` |
| assume `Signal` callbacks or imperative observers | explicit pipes, input signals, GTK routing |

### 10.5 Text / record / patch mistakes

| Do not write | Use instead |
|---|---|
| `"a" ++ "b"` or `"a" ^ "b"` | `"{a}{b}"` or `"{left} {right}"` |
| map literal as plain `{ ... }` | `Map { ... }` |
| set literal as plain `{ ... }` | `Set [ ... ]` |
| patch as record update syntax | `target <| { path.to.field: value }` |
| assume patch literals use ordinary record commas | patch entries are path updates; follow existing patch formatting |

## 11. Not specified here: avoid inventing

The RFC and repo evidence above do **not** give a stable authoring contract here, so avoid emitting these unless you have direct in-repo precedent for the exact form:

- standalone local `let ... in ...`
- `where` clauses
- standalone `match ... with` expression syntax
- Haskell-style guards on `||>` arms
- open-record row-polymorphic programming styles
- arbitrary imported user instances from other modules
- non-catalog GTK widgets/events

## 12. Conservative executable subset

Safest subset for code generation today:

- `value`, `func`, `type`, `domain`, `class`, `instance`
- `use`, `export`
- `signal name = expr`
- body-less input signals `signal name : Signal T`
- `@source ...` on body-less signals
- `result { ... }`
- `|>`, `?|>`, `||>`, `T|>`, `F|>`, `*|>`, `<|*`, `|`, `!|>`
- `&|>` over builtin applicative carriers
- records, tuples, lists, `Map {}`, `Set []`
- markup `value` roots with the current GTK widget catalog
- `<show>`, `<each>`, `<match>`, `<fragment>`, `<with>`
- record shorthand and default omission only when the expected type is known

Use extra caution with:

- signal merge reactive arms
- `@|> ... <|@` recurrent flows
- advanced structural patch lowering paths
- provider-specific runtime behavior outside the common `@source` shapes
- imported user-instance-heavy abstractions

## 13. Minimal canonical examples

### Pure code

```aivi
type User = {
    name: Text,
    active: Bool
}

value greeting = "hello"

type User -> Text
func greet = user =>
    "{greeting}, {user.name}"
```

### Reactive code

```aivi
signal firstName : Signal Text
signal lastName : Signal Text

signal fullName =
 &|> firstName
 &|> lastName
  |> joinName
```

### Result chaining

```aivi
value total =
    result {
        left <- Ok 20
        right <- Ok 22
        left + right
    }
```

### Markup

```aivi
signal clicked : Signal Unit

value view =
    <Window title="Demo">
        <Box orientation="vertical">
            <Label text="Ready" />
            <Button label="Click" onClick={clicked} />
        </Box>
    </Window>
```
