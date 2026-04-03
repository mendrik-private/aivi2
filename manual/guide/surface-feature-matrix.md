# Surface Feature Matrix

This page turns `syntax.md` into a conservative implementation matrix backed by the current repo.
It answers “what checks, executes, runs, or compiles today?” rather than “what the RFC eventually wants.”

## Legend

- `check` = `aivi check` / syntax + HIR validation.
- `execute` = `aivi execute` / one-shot non-GTK task entry (`value main : Task ...`).
- `run` = `aivi run` / live GTK + runtime path.
- `compile` = `aivi compile` / Cranelift object-code boundary only.
- `yes` = directly covered and currently working on that path.
- `partial` = accepted, but narrowed by same-module limits, provider-option gaps, runtime restrictions, or codegen slice limits.
- `no` = currently blocked or rejected on that path.
- `n/a` = not the intended delivery path for that surface.

## Important scope notes

- The `compile` column does **not** mean “produces a runnable GTK binary”. `aivi compile` currently stops at object emission; `aivi build` is the runnable bundle path.
- The matrix is intentionally conservative. When docs and executable evidence differ, the lower status wins.
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
| `value` / `func` | yes | yes | yes | yes | Core declaration surface is stable; later rows capture body-specific gaps. |
| `type` aliases, records, and sums | yes | yes | yes | yes | Closed ADTs and records are broadly accepted across the pipeline. |
| `class` / `instance` | yes | partial | partial | partial | Builtins and same-module user instances work; imported user-authored instances and imported polymorphic class-member execution remain deferred (`manual/guide/typeclasses.md`, `crates/aivi-core/src/lower.rs`). |
| `domain` declarations and member lookup | yes | yes | yes | yes | Domain binary operators (+, -, *, /, %) and comparisons compile via native Int operations on the unwrapped carrier value. Domain `value`/`unwrap` members compile as representational identity. |
| Derived `signal name = expr` | yes | partial | yes | partial | The live runtime supports derived signals with reactive re-evaluation. `aivi execute` settles signals once and cannot observe subsequent derivations. Compile depends on the codegen slice used by the signal body expression. |
| Body-less input `signal name : Signal T` | yes | partial | yes | yes | Input signals route GTK/runtime publications in `run`. `aivi execute` settles source-backed input signals once but cannot receive subsequent publications since it is not an interactive signal host. |
| Built-in `@source ...` on a body-less signal | yes | partial | partial | partial | Broadly lowered and runtime-backed, but option-level support is intentionally narrower; see the source rows below and `/guide/source-catalog`. |
| Custom `provider qualified.name` declarations | yes | partial | partial | partial | Contract checking and lowering exist. The runtime now accepts custom providers as inert instances (no crash), but they do not yet produce values. |
| `use` / `export` for types and constructors | yes | yes | yes | yes | Workspace type imports are covered by passing `run` and `compile` tests. |
| Imported executable values across modules | yes | partial | partial | partial | Checking passes and the primary workspace-import compile test succeeds; coverage still narrows for cross-module values whose bodies use unsupported codegen forms. |
| Top-level markup roots via `value` | yes | n/a | yes | n/a | `run` and `build` treat markup-valued top-level `value`s as the deployment surface. |

## Types And Literals

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core scalar types (`Int`, `Float`, `Bool`, `Text`, `Unit`) | yes | yes | yes | yes | Directly exercised across pure, runtime, and compile-safe tests. |
| Extended scalar/runtime types (`Decimal`, `BigInt`, `Bytes`) | yes | yes | yes | yes | Literals compile as symbol-value pointers. Arithmetic (+, -, *, /, %) and comparisons (==, !=, <, >, <=, >=) emit runtime bridge calls (`aivi_decimal_add`, `aivi_bigint_mul`, etc.). Bytes intrinsics (`length`, `get`, `slice`, `fromText`, `repeat`) compile. |
| Collection and effect types (`List`, `Map`, `Set`, `Option`, `Result`, `Validation`, `Signal`, `Task`) | yes | yes | yes | partial | `List`/`Map`/`Set` literals compile via runtime constructors. `Option` niche/inline, `Result` Ok/Err, and `Validation` Valid/Invalid compile via sum construction. `Signal` and `Task` effect-type lowering remains outside the current codegen slice. |
| Partial type-constructor application / HKTs | yes | partial | partial | partial | Checked and same-module executable in the current higher-kinded slice, but not yet a general cross-module evidence system. |
| Numeric literals (`Int`, `Float`, `Decimal`, `BigInt`) | yes | yes | yes | yes | All four numeric types have passing compile tests with verified Cranelift emission (Int/Float as immediates, Decimal/BigInt as symbol-value pointers). |
| Domain suffix literals (`250ms`, `10sec`, `3min`) | yes | yes | yes | yes | Domain suffix literals compile as by-reference domain values. Domain arithmetic and comparisons on suffixed values compile via native Int operations on the unwrapped carrier. |
| Text interpolation | yes | yes | yes | yes | Both static and dynamic text interpolation compile. Static interpolation folds at compile time; dynamic interpolation (e.g. `"{host}/path"` referencing a computed value) emits runtime text-concat calls (`cranelift_codegen_compiles_interpolated_text`). |
| Regex literals | partial | no | no | no | Regex literals in expression position produce `hir::regex-in-expression`; the only valid use is as `@source` option values (e.g. `pattern: rx"..."` in an HTTP or filesystem source). Use the `aivi.regex` module for runtime pattern matching. |
| Record / tuple / list literals | yes | yes | yes | yes | Records and tuples compile for scalar and by-reference fields. List literals compile via `aivi_list_new` runtime constructor calls. |
| `Map { ... }` / `Set [ ... ]` literals | yes | yes | yes | yes | Compile emits runtime constructor calls (`aivi_list_new`, `aivi_set_new`, `aivi_map_new`); element evaluation and stack marshalling are code-generated. |
| Type-level record row transforms (`Pick`, `Omit`, `Rename`, `Optional`, `Required`, `Defaulted`) | yes | yes | yes | yes | These are type-surface features; once checking succeeds, the later runtime/compile path sees the elaborated shape. |
| `Default` and record omission | yes | yes | yes | yes | Omission and defaults elaborate at check time, producing fully-specified records that compile through the standard aggregate codegen path. |
| Record shorthand | yes | yes | yes | yes | Shorthand desugars at check time before reaching the backend; resulting records compile successfully. |

## Pipe Algebra And Expression Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Ambient subject (`.`, `.field`, `.field.subfield`) | yes | yes | yes | yes | This is part of the stable checked/runtime expression surface. |
| Basic transform pipe `|>` | yes | yes | yes | yes | Core transform pipelines are compile-safe when their stage bodies stay inside the codegen slice. |
| `result { ... }` | yes | yes | yes | partial | Checked and runtime-backed; compile still depends on the payload/body shapes used inside the block. |
| Gate `?|>` | yes | yes | yes | partial | Simple gate compilation works, but domain/operator-rich gate expressions still hit codegen limits (`crates/aivi-cli/tests/compile.rs`). |
| Case split `||>` | yes | yes | yes | partial | Runtime-backed. Codegen emits Cranelift IR for inline-pipe `Case` stages with pattern matching, branching, and merge blocks; coverage is still narrower than the full runtime pattern set. |
| Truthy/falsy branches `T|>` / `F|>` | yes | yes | yes | partial | Runtime-backed. Codegen emits Cranelift IR for inline-pipe `TruthyFalsy` stages with boolean branching and merge blocks; coverage is still narrower than the full runtime carrier set. |
| Fan-out `*|>` / join `<|*` | yes | yes | yes | partial | Fan-out and join work in both derived signal pipelines and general expression contexts. Codegen now emits Cranelift loop IR for FanOut stages (list length, indexed get, map body, result construction); backend lowering for general-expression fan-out still has layout constraints. |
| Tap `|` | yes | yes | yes | partial | Runtime-backed as an observing stage. Codegen emits the tap body and discards the result, preserving the pipeline subject; coverage is narrower than the runtime for side-effect-heavy tap expressions. |
| Debug stage | yes | yes | yes | yes | Runtime-backed debugging/logging stage. Codegen emits debug stages as no-op pass-throughs, preserving the pipeline subject value. |
| Validation stage `!|>` | yes | yes | yes | yes | The validation stage is fully elaborated for general expression contexts (lowered as a Transform with `Replace` mode) and compiles through the standard transform codegen path. For signal pipelines it remains a scheduler boundary. |
| Accumulation `+|>` | yes | partial | yes | partial | Accumulation fully works at runtime with per-wakeup stepping (`linked_runtime_applies_accumulate_steps_once_per_wakeup`). `aivi execute` is one-shot and not a long-lived accumulator host. Compile coverage is still narrower than the runtime slice. |
| Previous / diff `~|>` / `-|>` | yes | partial | yes | yes | Full pipeline through core → lambda → backend → runtime; `startup.rs` implements temporal state caching for derived signals. `aivi execute` is one-shot and not a long-lived signal host. `compile_accepts_temporal_signal_programs` emits object code for `~|>` pipelines. |
| Applicative clusters `&|>` | yes | yes | yes | partial | Runtime coverage is strong for builtin carriers, but `Task` is applicative-only and compile still depends on the first-slice builtin table. |
| Explicit recurrence `@|> ... <|@` | yes | partial | partial | yes | Recurrence seed, start, step, and wakeup-witness kernels all compile. `aivi execute` settles sources once but is not a long-lived recurrence host; `aivi run` drives recurrence steps reactively. |
| Structural patch apply / `patch { ... }` | partial | partial | partial | partial | The checker accepts useful subsets, including list/map predicates and single-payload constructor focus. Codegen compiles single-segment named Replace and Remove patches (desugared to record construction). Complex selectors and nested patches are not yet supported. |
| Patch removal `field: -` | yes | yes | yes | yes | The checker accepts patch removal and computes the result type via field omission. The runtime elaborator omits removed fields from the result record. Codegen compiles removal as desugared record construction. |

## Signals, Tasks, And Sources

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Top-level `when` reactive updates | yes | partial | yes | yes | Guarded, source-pattern, and pattern-armed `when` forms all compile and run. `aivi execute` fires `when` clauses once for initial signal values but cannot observe subsequent reactive updates since it is one-shot. |
| `Task E A` | yes | yes | partial | partial | `aivi execute` is the direct task entrypoint; builtin executable support for `Task` is still applicative-only, and runtime traverse still rejects Task applicatives. |
| `timer.every` / `timer.after` | yes | partial | yes | partial | `immediate`, `jitter`, and `coalesce` options are all supported. `aivi execute` fires once for the initial tick but is not a long-lived timer host. Compile depends on the codegen slice used by the timer body. |
| `http.get` / `http.post` | yes | partial | yes | partial | Provider option support is broad: `http.get` accepts `body` (RFC 9110). `aivi execute` performs one request/response cycle; live streaming requires `run`. Compile is still request-slice specific. |
| `fs.watch` / `fs.read` | yes | partial | yes | partial | `fs.watch` supports `recursive: True` for directory-tree watching. `aivi execute` performs one read/snapshot; continuous watching requires `run`. |
| `socket.connect` | yes | partial | yes | partial | `heartbeat` is supported via periodic TCP keepalive writes. `aivi execute` opens one connection cycle; persistent socket streams require `run`. |
| `mailbox.subscribe` | yes | partial | yes | partial | `reconnect` retries on disconnection; `heartbeat` publishes periodic Unit events. `aivi execute` receives one initial message; continuous subscription requires `run`. |
| `process.spawn` | yes | partial | yes | partial | All three `StreamMode` values are supported: `Ignore`, `Lines`, and `Bytes`. `aivi execute` captures one output cycle; long-lived process streams require `run`. |
| Host-context sources (`process.args`, `process.cwd`, `env.get`, `stdio.read`, `path.*`) | yes | yes | yes | partial | All host-context sources are immediate-value providers that resolve at startup via `publish_immediate_value`; they work identically in `execute` and `run` modes. Compile remains narrower than the runtime ABI. |
| `db.connect` / `db.live` | yes | partial | yes | partial | `optimistic` and `onRollback` are accepted; `pool` is validated. `aivi execute` runs one query cycle; live subscriptions (`db.live`) require `run`. |
| `window.keyDown` | yes | n/a | yes | partial | GTK-backed runtime tests exist; `capture` and `focusOnly` options are now accepted and stored for the GTK event controller. |
| `dbus.ownName` / `dbus.signal` / `dbus.method` | yes | partial | partial | partial | Runtime tests exist. `dbus.method` supports `reply` with static GLib variant strings; dynamic runtime-computed reply payloads are not yet supported. |

## Markup / GTK Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core markup roots and reactive attributes | yes | n/a | yes | n/a | `prepare_run_artifact` and bundle tests cover the current markup entry path. |
| Current widget catalog | yes | n/a | yes | n/a | The syntax-sheet catalog is supported; `Button` also exposes typed `opacity: Float` and `animateOpacity: Bool` properties for signal-driven fade transitions, but arbitrary non-catalog widgets and generic CSS-property maps are still rejected. |
| Event routing to input signals / payload publication | yes | n/a | yes | n/a | 8 event types across 7 widgets: ButtonClicked, EntryChanged, EntryActivated, SwitchToggled, CheckButtonToggled, ToggleButtonToggled, SpinButton.onValueChanged (F64), Scale.onValueChanged (F64). |
| `<show>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<each>` | yes | n/a | yes | n/a | Covered by run/control-node tests and keyed collection handling. |
| `<match>` | yes | n/a | yes | n/a | Covered by run/control-node tests and shared pattern machinery. |
| `<fragment>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<with>` | yes | n/a | yes | n/a | Covered by run tests for markup-local bindings and payload-derived bindings. |

## Patterns And Predicates

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Constructor / wildcard / nested patterns | yes | yes | yes | yes | Codegen emits tag comparisons for sum constructors, field extraction for nested record/option patterns, integer literal matching, and wildcard/binding pass-through. Tested by `cranelift_codegen_compiles_constructor_pattern_with_sum_variants`, `cranelift_codegen_compiles_nested_constructor_pattern`, `cranelift_codegen_compiles_wildcard_and_integer_patterns`, and the catalog foundation fixture. |
| Record and list patterns (`{ ... }`, `[]`, `[x, ...rest]`) | yes | yes | yes | yes | Record patterns: field offset computation with recursive sub-pattern matching. List patterns: `aivi_list_len` length discrimination, `aivi_list_get` element extraction, `aivi_list_slice` rest binding. Recursive list patterns are rejected (`compile_rejects_recursive_list_pattern_fixture_with_cycle_error`) because self-referencing functions require loop/tail-call support. |
| Predicate mini-language (`.field`, `and`, `or`, `not`, `==`, `!=`) | yes | yes | yes | partial | Gate predicates compile for Bool/Option carriers with `True`/`False`/`Some`/`None` truthiness. The checked/runtime slice is broad; compile still narrows when predicates rely on unsupported domain literal or complex inline-pipe codegen forms. |

## Biggest Gaps

- `compile` covers a broad AOT slice: collection literals, Decimal/BigInt arithmetic, Result/Validation constructors, domain operators, pattern matching (constructor/record/list/wildcard), fan-out loops, recurrence kernels, text interpolation, and suffixed integer domain literals all compile. The remaining compile gap is recursive self-referencing functions (no loop/tail-call support) and `Signal`/`Task` effect-type lowering.
- `aivi execute` is one-shot: worker-thread sources now have a bounded 5-second settlement window, but continuous streaming and reactive signal scheduling require `aivi run`.
- Imported user-authored higher-kinded instances and imported polymorphic class-member execution are still deferred.
- Custom `provider` declarations are accepted at runtime as inert instances but do not yet produce values.
- Regex literals are only valid as `@source` option values; regex literals in expression position produce `hir::regex-in-expression`. Use the `aivi.regex` module for runtime pattern matching.
- Structural patch apply supports single-segment `Named Replace` and `Named Remove` on closed records. More complex patch selectors and nested patches are not yet supported.
- The source catalog is broadly executed. The main remaining contract-only option is `dbus.method` dynamic runtime-computed reply payloads. Use `/guide/source-catalog` for the option-level truth table.
