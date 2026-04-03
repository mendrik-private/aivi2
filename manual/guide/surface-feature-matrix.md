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
| `domain` declarations and member lookup | yes | yes | yes | partial | Runtime foundation tests cover domain operators and authored members; codegen only supports a narrow representational slice (`crates/aivi-backend/tests/foundations.rs`, `crates/aivi-backend/src/codegen.rs`). |
| Derived `signal name = expr` | yes | partial | yes | partial | The live runtime supports derived signals; `aivi execute` is one-shot, and compile still depends on the codegen slice used by the signal body. |
| Body-less input `signal name : Signal T` | yes | partial | yes | yes | Input signals route GTK/runtime publications in `run`; `execute` can observe settled sources but is not a general interactive signal host. |
| Built-in `@source ...` on a body-less signal | yes | partial | partial | partial | Broadly lowered and runtime-backed, but option-level support is intentionally narrower; see the source rows below and `/guide/source-catalog`. |
| Custom `provider qualified.name` declarations | yes | no | no | partial | Contract checking and lowering exist, but the runtime provider manager still rejects unsupported/custom providers (`crates/aivi-runtime/src/providers.rs`). |
| `use` / `export` for types and constructors | yes | yes | yes | yes | Workspace type imports are covered by passing `run` and `compile` tests. |
| Imported executable values across modules | yes | partial | partial | partial | Checking passes and the primary workspace-import compile test succeeds; coverage still narrows for cross-module values whose bodies use unsupported codegen forms. |
| Top-level markup roots via `value` | yes | n/a | yes | n/a | `run` and `build` treat markup-valued top-level `value`s as the deployment surface. |

## Types And Literals

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core scalar types (`Int`, `Float`, `Bool`, `Text`, `Unit`) | yes | yes | yes | yes | Directly exercised across pure, runtime, and compile-safe tests. |
| Extended scalar/runtime types (`Decimal`, `BigInt`, `Bytes`) | yes | yes | yes | partial | Runtime-backed, but compile support stays within the current literal-cell / narrow ABI contracts in `codegen.rs`. |
| Collection and effect types (`List`, `Map`, `Set`, `Option`, `Result`, `Validation`, `Signal`, `Task`) | yes | yes | yes | partial | The checker/runtime know these shapes; compile coverage remains narrower for aggregate and effect-heavy lowering. |
| Partial type-constructor application / HKTs | yes | partial | partial | partial | Checked and same-module executable in the current higher-kinded slice, but not yet a general cross-module evidence system. |
| Numeric literals (`Int`, `Float`, `Decimal`, `BigInt`) | yes | yes | yes | partial | Scalar literals compile in the first slice; some wider aggregate/codegen uses still stop at codegen. |
| Domain suffix literals (`250ms`, `10sec`, `3min`) | yes | yes | yes | partial | Surface and runtime support are real; compile still depends on the domain-member/codegen subset. |
| Text interpolation | yes | yes | yes | partial | Static text interpolation compiles. Dynamic interpolation still relies on `Reduce(List)` / `Append(List)` lowering which remains outside the current codegen slice. |
| Regex literals | partial | no | no | no | Regex literals in expression position produce `hir::regex-in-expression`; the only valid use is as `@source` option values (e.g. `pattern: rx"..."` in an HTTP or filesystem source). Use the `aivi.regex` module for runtime pattern matching. |
| Record / tuple / list literals | yes | yes | yes | partial | Runtime support is broad; compile covers scalar/by-reference aggregates and list literals via runtime constructor calls. |
| `Map { ... }` / `Set [ ... ]` literals | yes | yes | yes | partial | Compile emits runtime constructor calls (`aivi_list_new`, `aivi_set_new`, `aivi_map_new`); element evaluation and stack marshalling are code-generated. |
| Type-level record row transforms (`Pick`, `Omit`, `Rename`, `Optional`, `Required`, `Defaulted`) | yes | yes | yes | yes | These are type-surface features; once checking succeeds, the later runtime/compile path sees the elaborated shape. |
| `Default` and record omission | yes | yes | yes | partial | Omission elaborates in the checker, but the resulting record-heavy runtime values still inherit aggregate codegen limits. |
| Record shorthand | yes | yes | yes | partial | Checked and runtime-backed; compile inherits record/aggregate narrowing. |

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
| Accumulation `+|>` | yes | partial | yes | partial | Runtime startup tests prove accumulation works; `aivi execute` is not a long-lived signal host, and compile coverage is still narrower than the runtime slice. |
| Previous / diff `~|>` / `-|>` | yes | partial | yes | partial | Full pipeline through core → lambda → backend → runtime; `startup.rs` implements temporal state caching for derived signals. `aivi execute` is one-shot and not a long-lived signal host. Compile accepts temporal signal programs. |
| Applicative clusters `&|>` | yes | yes | yes | partial | Runtime coverage is strong for builtin carriers, but `Task` is applicative-only and compile still depends on the first-slice builtin table. |
| Explicit recurrence `@|> ... <|@` | yes | no | partial | partial | Linked-runtime tests prove source-backed recurrence steps, but `syntax.md` already flags this area as cautionary and standalone compile/startup coverage is still narrower. |
| Structural patch apply / `patch { ... }` | partial | partial | partial | no | The checker accepts useful subsets, including list/map predicates and single-payload constructor focus, but the surface is still partial end to end. |
| Patch removal `field: -` | yes | yes | yes | no | The checker accepts patch removal and computes the result type via field omission. The runtime elaborator now omits removed fields from the result record construction. Codegen does not cover patch apply in the current slice. |

## Signals, Tasks, And Sources

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Top-level `when` reactive updates | yes | partial | yes | yes | Guarded, source-pattern, and pattern-armed `when` forms have passing compile tests; the live runtime executes top-level `when` clauses. |
| `Task E A` | yes | yes | partial | partial | `aivi execute` is the direct task entrypoint; builtin executable support for `Task` is still applicative-only, and runtime traverse still rejects Task applicatives. |
| `timer.every` / `timer.after` | yes | partial | yes | partial | `immediate`, `jitter`, and `coalesce` are all supported. Compile still depends on the codegen slice used by the timer body. |
| `http.get` / `http.post` | yes | partial | yes | partial | Provider option support is now broad: `http.get` accepts `body` (RFC 9110). The live runtime is still request-slice specific for compile. |
| `fs.watch` / `fs.read` | yes | partial | yes | partial | `fs.watch` now supports `recursive: True` for directory-tree watching. |
| `socket.connect` | yes | partial | yes | partial | Provider tests exist; `heartbeat` is now supported via periodic TCP keepalive writes. |
| `mailbox.subscribe` | yes | partial | yes | partial | Provider tests exist; `reconnect` retries on disconnection and `heartbeat` publishes periodic Unit events. |
| `process.spawn` | yes | partial | yes | partial | All three `StreamMode` values are supported: `Ignore`, `Lines`, and `Bytes`. |
| Host-context sources (`process.args`, `process.cwd`, `env.get`, `stdio.read`, `path.*`) | yes | yes | partial | partial | This is the best-covered `execute` source subset: `execute_reads_host_context_sources_and_writes_stdout` exercises it directly. |
| `db.connect` / `db.live` | yes | partial | yes | partial | Runtime tests exist; `optimistic` and `onRollback` are now accepted. `pool` is only validated. |
| `window.keyDown` | yes | n/a | yes | partial | GTK-backed runtime tests exist; `capture` and `focusOnly` options are now accepted and stored for the GTK event controller. |
| `dbus.ownName` / `dbus.signal` / `dbus.method` | yes | partial | partial | partial | Runtime tests exist. `dbus.method` supports `reply` with static GLib variant strings; dynamic runtime-computed reply payloads are not yet supported. |

## Markup / GTK Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core markup roots and reactive attributes | yes | n/a | yes | n/a | `prepare_run_artifact` and bundle tests cover the current markup entry path. |
| Current widget catalog | yes | n/a | yes | n/a | The syntax-sheet catalog is supported; `Button` also exposes typed `opacity: Float` and `animateOpacity: Bool` properties for signal-driven fade transitions, but arbitrary non-catalog widgets and generic CSS-property maps are still rejected. |
| Event routing to input signals / payload publication | yes | n/a | partial | n/a | Direct signal hooks and payload-publishing event hooks are covered, but unsupported widget/event pairs are still rejected by the run surface. |
| `<show>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<each>` | yes | n/a | yes | n/a | Covered by run/control-node tests and keyed collection handling. |
| `<match>` | yes | n/a | yes | n/a | Covered by run/control-node tests and shared pattern machinery. |
| `<fragment>` | yes | n/a | yes | n/a | Covered by run/control-node tests. |
| `<with>` | yes | n/a | yes | n/a | Covered by run tests for markup-local bindings and payload-derived bindings. |

## Patterns And Predicates

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Constructor / wildcard / nested patterns | yes | yes | yes | partial | Runtime-backed; compile still loses coverage once those patterns lower through unsupported inline case/codegen paths. |
| Record and list patterns (`{ ... }`, `[]`, `[x, ...rest]`) | yes | yes | yes | partial | Checked and runtime-backed, but recursive list-pattern compile coverage is still incomplete (`compile_rejects_recursive_list_pattern_fixture_with_cycle_error`). |
| Predicate mini-language (`.field`, `and`, `or`, `not`, `==`, `!=`) | yes | yes | yes | partial | The checked/runtime slice is broad; compile still narrows when predicates rely on unsupported domain or inline-pipe codegen forms. |

## Biggest Gaps

- `compile` is a first-slice AOT boundary. It emits object code with runtime constructor imports for collection literals (`aivi_list_new`/`aivi_set_new`/`aivi_map_new`). Fan-out stages now have Cranelift loop emission but backend lowering still constrains general-expression fan-out. Dynamic text interpolation relying on `Reduce(List)`/`Append(List)` remains outside the codegen slice.
- Inline-pipe `Case`, `TruthyFalsy`, and `Tap` stages now have Cranelift emission code but coverage is still narrower than the runtime pattern/carrier set.
- `Previous`/`Diff` temporal stages work end to end for derived signals in the live runtime. General-expression contexts still block them (by design: temporal state requires a signal host).
- Imported user-authored higher-kinded instances and imported polymorphic class-member execution are still deferred.
- Custom `provider` declarations are currently contract/lowering features, not runtime-executable providers.
- Regex literals are only valid as `@source` option values; regex literals in expression position produce `hir::regex-in-expression`. Use the `aivi.regex` module for runtime pattern matching.
- Structural patch apply supports single-segment `Named Replace` and `Named Remove` on closed records. Replace substitutes the field value; Remove omits the field from the result record with a narrowed result type. More complex patch selectors and nested patches are not yet supported.
- The source catalog is now broadly executed. The main remaining contract-only option is `dbus.method` dynamic runtime-computed reply payloads. Use `/guide/source-catalog` for the option-level truth table.
