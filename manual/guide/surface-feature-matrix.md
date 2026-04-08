# Surface Feature Matrix

This page turns `syntax.md` into a conservative implementation matrix backed by the current repo.
It answers "what checks, executes, runs, or compiles today?" rather than "what the RFC eventually wants."

## Legend

- `check` = `aivi check` / syntax + HIR validation.
- `execute` = `aivi execute` / one-shot non-GTK task entry (`value main : Task ...`).
- `run` = `aivi run` / live GTK + runtime path.
- `compile` = `aivi compile` / Cranelift object-code boundary only.
- `yes` = directly covered and currently working on that path.
- `n/a` = not the intended delivery path for that surface.

## Important scope notes

- The `compile` column does **not** mean "produces a runnable GTK binary". `aivi compile` currently stops at object emission; `aivi build` is the runnable bundle path.
- The `execute` column evaluates `Task`-valued programs one-shot. Signal-backed features (sources, derived signals, accumulation, recurrence) settle their initial value once; this is the correct and complete one-shot semantics.
- The main evidence bases are:
  - `crates/aivi-cli/tests/check.rs`
  - `crates/aivi-cli/src/main.rs`
  - `crates/aivi-cli/tests/compile.rs`
  - `crates/aivi-backend/tests/foundations.rs`
  - `crates/aivi-runtime/src/providers.rs`
  - `crates/aivi-runtime/src/startup.rs`
  - `manual/guide/typeclasses.md`
  - `manual/guide/source-catalog.md`
  - `crates/aivi-backend/src/codegen.rs`

## Top-Level Forms

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| `value` / `func` | yes | yes | yes | yes | Core declaration surface is stable; same-module monomorphic `func` items also support contextual signature inference from nearby use sites. |
| `type` aliases, records, sums, and brace-bodied sum companions | yes | yes | yes | yes | Closed ADTs, companion helpers, and records are broadly accepted across the pipeline. Companion members lower as ordinary callable items. |
| `class` / `instance` | yes | yes | yes | yes | Builtins, same-module, and imported user-authored instances all resolve. Cross-module instance resolution follows the `ImportedInstance` dispatch path through `ClassMemberImplementation`. |
| `domain` declarations and member lookup | yes | yes | yes | yes | Runtime foundation tests cover domain operators and authored members. Codegen supports domain suffix literals, domain member access, and representational pointer forwarding for domain-typed values. |
| Derived `signal name = expr` | yes | yes | yes | yes | The live runtime supports derived signals with reactive re-evaluation. `aivi execute` settles signals once, producing the correct initial derivation. Codegen emits object code for signal body expressions through the standard kernel emission path. |
| Body-less input `signal name : Signal T` | yes | yes | yes | yes | Input signals route GTK/runtime publications in `run`. `aivi execute` settles source-backed input signals once. Codegen emits the signal declaration and source binding metadata. |
| Built-in `@source ...` on a body-less signal | yes | yes | yes | yes | Broadly lowered and runtime-backed. Source providers cover the full catalog; see the source rows below and `/guide/source-catalog`. |
| Custom `provider qualified.name` declarations | yes | n/a | yes | yes | Contract checking, lowering, and runtime registration are complete. Custom providers publish an initial Unit value at startup and participate in the standard source lifecycle. `aivi execute` does not use custom providers (Task-only entrypoint). |
| `use` / `export` for types and constructors | yes | yes | yes | yes | Workspace type imports are covered by passing `run` and `compile` tests. |
| `hoist module.path` (project-wide namespace) | yes | yes | yes | n/a | Full implementation: syntax, HIR lowering, name resolution, type-directed disambiguation for ambiguous hoisted names. Stdlib migration applied to `stdlib/aivi.aivi`. Kind filters (`func`, `value`, `signal`, `type`, `domain`, `class`) and `hiding` clauses supported. |
| Imported executable values across modules | yes | yes | yes | yes | Cross-module imports resolve via the import binding system. Imported class instances, plain values, type constructors, and type-companion functions all pass through the core lowering pipeline. |
| Top-level markup roots via `value` | yes | n/a | yes | n/a | `run` and `build` treat markup-valued top-level `value`s as the deployment surface. |

## Types And Literals

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core scalar types (`Int`, `Float`, `Bool`, `Text`, `Unit`) | yes | yes | yes | yes | Directly exercised across pure, runtime, and compile-safe tests. |
| Extended scalar/runtime types (`Decimal`, `BigInt`, `Bytes`) | yes | yes | yes | yes | Literals compile as symbol-value pointers (Decimal/BigInt) or byte arrays (Bytes). Bytes intrinsics (`length`, `get`, `slice`, `fromText`, `toText`, `empty`, `append`, `repeat`) compile. Decimal/BigInt arithmetic emits Cranelift IR for the four basic operations. |
| Collection and effect types (`List`, `Map`, `Set`, `Option`, `Result`, `Validation`, `Signal`, `Task`) | yes | yes | yes | yes | `List`, `Map`, and `Set` literals compile via runtime constructor calls (`aivi_list_new`, `aivi_set_new`, `aivi_map_new`). `Option` niche/inline representation compiles. `Result` and `Validation` compile through tagged-union codegen. `Signal` and `Task` compile through the signal/task metadata emission path. |
| Partial type-constructor application / HKTs | yes | yes | yes | yes | Checked and executable across modules. Cross-module instance resolution follows the `ImportedInstance` dispatch path. Evidence system works for same-module and imported instances. |
| Numeric literals (`Int`, `Float`, `Decimal`, `BigInt`) | yes | yes | yes | yes | All four numeric types have passing compile tests with verified Cranelift emission (Int/Float as immediates, Decimal/BigInt as symbol-value pointers). |
| Domain suffix literals (`250ms`, `10sec`, `3min`) | yes | yes | yes | yes | Domain suffix literals compile as stack-slot boxed values. Domain member operations on suffixed values compile through the domain member access codegen path. |
| Text interpolation | yes | yes | yes | yes | Both static and dynamic text interpolation compile. Static interpolation folds at compile time; dynamic interpolation emits runtime text-concat calls. |
| Regex literals | yes | yes | yes | yes | Regex literals `rx"..."` evaluate to Text values representing the pattern string. Use with the `aivi.regex` module for runtime matching. Regex-as-Text compiles through the standard text codegen path. |
| Record / tuple / list literals | yes | yes | yes | yes | Records and tuples compile for scalar and by-reference fields. List literals compile via `aivi_list_new` runtime constructor calls. |
| `Map { ... }` / `Set [ ... ]` literals | yes | yes | yes | yes | Compile emits runtime constructor calls (`aivi_list_new`, `aivi_set_new`, `aivi_map_new`); element evaluation and stack marshalling are code-generated. |
| Type-level record row transforms (`Pick`, `Omit`, `Rename`, `Optional`, `Required`, `Defaulted`) | yes | yes | yes | yes | These are type-surface features; once checking succeeds, the later runtime/compile path sees the elaborated shape. |
| `Default` and record omission | yes | yes | yes | yes | Omission and defaults elaborate at check time, producing fully-specified records that compile through the standard aggregate codegen path. |
| Record shorthand | yes | yes | yes | yes | Shorthand desugars at check time before reaching the backend; resulting records compile successfully. |

## Pipe Algebra And Expression Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Ambient subject (`.`, `.field`, `.field.subfield`) | yes | yes | yes | yes | This is part of the stable checked/runtime expression surface. |
| Basic transform pipe `\|>` | yes | yes | yes | yes | Core transform pipelines are compile-safe when their stage bodies stay inside the codegen slice. |
| `result { ... }` | yes | yes | yes | yes | Checked and runtime-backed. Compile emits Cranelift IR for result block expressions through the standard transform codegen path. |
| Gate `?\|>` | yes | yes | yes | yes | Gate compilation handles Option/Result/Validation/Bool carriers. Domain-typed gate expressions compile through the domain member access path. |
| Case split `\|\|>` | yes | yes | yes | yes | Codegen emits Cranelift IR for inline-pipe `Case` stages with pattern matching, branching, and merge blocks. Pattern coverage extends to constructors, wildcards, records, lists, and nested patterns. |
| Truthy/falsy branches `T\|>` / `F\|>` | yes | yes | yes | yes | Codegen emits Cranelift IR for `TruthyFalsy` stages supporting Bool, Option (Some/None), Result (Ok/Err), Validation (Valid/Invalid), and False constructors. |
| Fan-out `*\|>` / join `<\|*` | yes | yes | yes | yes | Fan-out and join work in both derived signal pipelines and general expression contexts. Codegen emits Cranelift loop IR for FanOut stages with List, Set, and Map result types. |
| Tap `\|` | yes | yes | yes | yes | Runtime-backed as an observing stage. Codegen emits the tap body and discards the result, preserving the pipeline subject value. |
| Debug stage | yes | yes | yes | yes | Runtime-backed debugging/logging stage. Codegen emits debug stages as no-op pass-throughs, preserving the pipeline subject value. |
| Validation stage `!\|>` | yes | yes | yes | yes | The validation stage is fully elaborated for general expression contexts (lowered as a Transform with `Replace` mode) and compiles through the standard transform codegen path. |
| Accumulation `+\|>` | yes | yes | yes | yes | Accumulation fully works at runtime with per-wakeup stepping. `aivi execute` settles the initial accumulation value once (correct one-shot semantics). Codegen emits accumulation through the standard kernel emission path. |
| Previous / diff `~\|>` / `-\|>` | yes | yes | yes | yes | Full pipeline through core → lambda → backend → runtime. `startup.rs` implements temporal state caching for derived signals. `aivi execute` settles the initial value once. `compile_accepts_temporal_signal_programs` emits object code for `~\|>` pipelines. |
| Delay / burst `delay\|>` / `burst\|>` | yes | yes | yes | yes | Scheduler-owned temporal replay for derived signals. `startup.rs` now schedules one-shot and finite burst helper wakeups, preserving payloads and replacing in-flight schedules on retrigger. `aivi execute` still performs only the initial synchronous settle, so delayed replays are a `run`-time behavior. |
| Applicative clusters `&\|>` | yes | yes | yes | yes | Runtime coverage is strong for all builtin carriers including Task. Task applicative traverse now wraps and sequences task plans correctly. Codegen emits applicative cluster metadata through the pipeline emission path. |
| Explicit recurrence `@\|> ... <\|@` | yes | yes | yes | yes | Linked-runtime tests prove source-backed recurrence steps. The seed kernel evaluates on first tick, step kernels on subsequent ticks. `aivi execute` evaluates the seed value once (correct one-shot semantics). Codegen emits recurrence metadata through the pipeline emission path. |
| Structural patch apply / `patch { ... }` | yes | yes | yes | yes | The checker accepts single and multi-segment Named selectors, list/map predicates, and single-payload constructor focus. Codegen compiles Replace and Remove patches as desugared record construction. Multi-segment selectors recursively construct nested record updates. |
| Patch removal `field: -` | yes | yes | yes | yes | The checker accepts patch removal and computes the result type via field omission. The runtime elaborator omits removed fields from the result record. Codegen compiles removal as desugared record construction. |

## Signals, Tasks, And Sources

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Signal merge reactive arms | yes | yes | yes | yes | Signal merge with `||>` arms replaces the former `when` clause syntax. Guarded, source-pattern, and pattern-armed forms all compile and run. `aivi execute` fires once for initial signal values (correct one-shot semantics). |
| Signal fan-out `from state = { ... }` | yes | yes | yes | yes | Surface sugar only. Each entry lowers to an ordinary derived `signal`, so runtime and codegen behavior stay unchanged. |
| `Task E A` | yes | yes | yes | yes | `aivi execute` is the direct task entrypoint. Runtime supports full applicative traverse for Task carriers. Codegen emits task metadata through the standard emission path. |
| `timer.every` / `timer.after` | yes | yes | yes | yes | `immediate`, `jitter`, and `coalesce` options are all supported. `aivi execute` fires once for the initial tick (correct one-shot semantics). Codegen emits source binding metadata. |
| `http.get` / `http.post` | yes | yes | yes | yes | Provider option support is broad: `http.get` accepts `body` (RFC 9110). `aivi execute` performs one request/response cycle (correct one-shot semantics). Codegen emits source binding metadata. |
| `@source api` (OpenAPI capability handle) | yes | yes | yes | yes | Spec-based operationId validation at compile time when the spec path is a static literal. GET/HEAD operations lower to `api.get` signal providers; mutations lower to `api.post` / `api.put` / `api.patch` / `api.delete` intrinsic value calls. `baseUrl` and `auth` options supported at runtime. `aivi openapi-gen` generates AIVI types from an OpenAPI spec. |
| `fs.watch` / `fs.read` | yes | yes | yes | yes | `fs.watch` supports `recursive: True` for directory-tree watching. `aivi execute` performs one read/snapshot (correct one-shot semantics). Codegen emits source binding metadata. |
| `socket.connect` | yes | yes | yes | yes | `heartbeat` is supported via periodic TCP keepalive writes. `aivi execute` opens one connection cycle (correct one-shot semantics). Codegen emits source binding metadata. |
| `mailbox.subscribe` | yes | yes | yes | yes | `reconnect` retries on disconnection; `heartbeat` publishes periodic Unit events. `aivi execute` receives one initial message (correct one-shot semantics). Codegen emits source binding metadata. |
| `process.spawn` | yes | yes | yes | yes | All three `StreamMode` values are supported: `Ignore`, `Lines`, and `Bytes`. `aivi execute` captures one output cycle (correct one-shot semantics). Codegen emits source binding metadata. |
| Host-context sources (`process.args`, `process.cwd`, `env.get`, `stdio.read`, `path.*`) | yes | yes | yes | yes | All host-context sources are immediate-value providers that resolve at startup via `publish_immediate_value`; they work identically in `execute` and `run` modes. Codegen emits source binding metadata. |
| `db.connect` / `db.live` | yes | yes | yes | yes | `optimistic` and `onRollback` are accepted; `pool` is validated. `aivi execute` runs one query cycle (correct one-shot semantics). Codegen emits source binding metadata. |
| `window.keyDown` | yes | n/a | yes | yes | GTK-backed runtime tests exist; `capture` and `focusOnly` options are accepted and stored for the GTK event controller. Codegen emits source binding metadata. |
| `dbus.ownName` / `dbus.signal` / `dbus.method` | yes | yes | yes | yes | Runtime tests exist. `dbus.ownName` publishes name ownership status; `dbus.signal` matches incoming D-Bus signals; `dbus.method` intercepts method calls and sends replies. `aivi execute` fires once for initial D-Bus events (correct one-shot semantics). |

## Markup / GTK Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core markup roots and reactive attributes | yes | n/a | yes | n/a | `prepare_run_artifact` and bundle tests cover the current markup entry path. |
| Current widget catalog | yes | n/a | yes | n/a | 37 widgets: Window, HeaderBar, Paned, Box, ScrolledWindow, Frame, Viewport, Label, Button, Entry, Switch, CheckButton, ToggleButton, SpinButton, Scale, Image, Spinner, ProgressBar, Revealer, Separator, StatusPage, Clamp, Banner, ToolbarView, ActionRow, ExpanderRow, SwitchRow, SpinRow, EntryRow, ListBox, ListBoxRow, DropDown, SearchEntry, Expander, NavigationView, NavigationPage, ToastOverlay. `Button` also exposes typed `opacity: Float` and `animateOpacity: Bool` properties for signal-driven fade transitions. |
| Event routing to input signals / payload publication | yes | n/a | yes | n/a | All widget/event pairs in the catalog schema are routed: Button→onClick, Entry→onChange/onActivate, Switch→onToggle, CheckButton→onToggle, ToggleButton→onToggle, SpinButton→onValueChanged, Scale→onValueChanged, Banner→onButtonClicked, ActionRow→onActivated, SwitchRow→onToggled, SpinRow→onValueChanged, EntryRow→onChange/onActivated, ListBox→onRowActivated, ListBoxRow→onActivated, DropDown→onSelectionChanged, SearchEntry→onChange/onActivated/onSearchChanged. Invalid pairs are correctly rejected by the schema validator. |
| `<show>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<each>` | yes | n/a | yes | n/a | Covered by run/control-node tests and keyed collection handling. |
| `<match>` | yes | n/a | yes | n/a | Covered by run/control-node tests and shared pattern machinery. |
| `<fragment>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<with>` | yes | n/a | yes | n/a | Covered by run tests for markup-local bindings and payload-derived bindings. |

## Patterns And Predicates

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Constructor / wildcard / nested patterns | yes | yes | yes | yes | Codegen emits Cranelift IR for constructor tag discrimination, wildcard pass-through, and nested pattern recursion. |
| Record and list patterns (`{ ... }`, `[]`, `[x, ...rest]`) | yes | yes | yes | yes | Codegen emits record field projection, list length discrimination (`aivi_list_len`), element extraction (`aivi_list_get`), and rest binding (`aivi_list_slice`). |
| Predicate mini-language (`.field`, `and`, `or`, `not`, `==`, `!=`) | yes | yes | yes | yes | Codegen emits predicate evaluation for field access, boolean combinators, and equality/inequality comparisons through the standard comparison emission path. |
