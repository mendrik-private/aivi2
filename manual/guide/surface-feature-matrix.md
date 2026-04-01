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
| Imported executable values across modules | yes | partial | partial | no | Checking passes, but `compile_accepts_workspace_value_imports` is currently red; cross-module executable value coverage is still narrower than type imports. |
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
| Text interpolation | yes | yes | yes | no | Runtime-backed, but current codegen explicitly rejects remaining dynamic-text lowering (`crates/aivi-backend/src/codegen.rs`). |
| Regex literals | yes | no | no | no | Regex literals are validated in HIR, but general-expression elaboration still blocks them before typed-core lowering (`crates/aivi-hir/src/general_expr_elaboration.rs`). |
| Record / tuple / list literals | yes | yes | yes | partial | Runtime support is broad; compile only covers part of the aggregate lowering space. |
| `Map { ... }` / `Set [ ... ]` literals | yes | yes | yes | no | The checker/runtime know these literals, but codegen still rejects remaining collection lowering in the first Cranelift slice. |
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
| Case split `||>` | yes | yes | yes | no | Runtime-backed, but codegen explicitly rejects inline-pipe `Case` stages in the current slice (`crates/aivi-backend/src/codegen.rs`). |
| Truthy/falsy branches `T|>` / `F|>` | yes | yes | yes | no | Runtime-backed, but codegen explicitly rejects inline-pipe `TruthyFalsy` stages in the current slice. |
| Fan-out `*|>` / join `<|*` | yes | yes | yes | no | The checker/runtime know these stages, but compile still lacks collection-lowering support for this slice. |
| Tap `|` | yes | yes | yes | no | Runtime-backed as an observing stage; codegen explicitly rejects inline-pipe debug/tap stages today. |
| Validation stage `!|>` | yes | partial | partial | no | The operator is in the surface/HIR, but there is no direct end-to-end runtime/codegen coverage yet, and general-expression elaboration blocks validate stages outside the supported scheduler boundary. |
| Accumulation `+|>` | yes | partial | yes | partial | Runtime startup tests prove accumulation works; `aivi execute` is not a long-lived signal host, and compile coverage is still narrower than the runtime slice. |
| Previous / diff `~|>` / `-|>` | yes | no | no | no | The operators are lexed and carried in HIR, but no current runtime/codegen path is evidenced in the repo. |
| Applicative clusters `&|>` | yes | yes | yes | partial | Runtime coverage is strong for builtin carriers, but `Task` is applicative-only and compile still depends on the first-slice builtin table. |
| Explicit recurrence `@|> ... <|@` | yes | no | partial | partial | Linked-runtime tests prove source-backed recurrence steps, but `syntax.md` already flags this area as cautionary and standalone compile/startup coverage is still narrower. |
| Structural patch apply / `patch { ... }` | partial | partial | partial | no | The checker accepts useful subsets, including list/map predicates and single-payload constructor focus, but the surface is still partial end to end. |
| Patch removal `field: -` | no | no | no | no | The checker still emits `hir::unsupported-patch-remove`; remove/shrink semantics are not implemented yet. |

## Signals, Tasks, And Sources

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Top-level `when` reactive updates | yes | partial | yes | yes | Guarded, source-pattern, and pattern-armed `when` forms have passing compile tests; the live runtime executes top-level `when` clauses. |
| `Task E A` | yes | yes | partial | partial | `aivi execute` is the direct task entrypoint; builtin executable support for `Task` is still applicative-only, and runtime traverse still rejects Task applicatives. |
| `timer.every` / `timer.after` | yes | partial | partial | partial | `immediate` works, but `jitter` is not executed yet and `coalesce` is only supported as `True` (`/guide/source-catalog`). |
| `http.get` / `http.post` | yes | partial | partial | partial | Provider tests exist, but option support is narrower than the syntax sheet: `http.get` still rejects `body`, and the live runtime is still request-slice specific. |
| `fs.watch` / `fs.read` | yes | partial | partial | partial | Provider tests exist; `fs.watch recursive` is still accepted-but-not-executed. |
| `socket.connect` | yes | partial | partial | partial | Provider tests exist; `heartbeat` remains accepted-but-not-executed. |
| `mailbox.subscribe` | yes | partial | partial | partial | Provider tests exist; `reconnect` and `heartbeat` remain contract-only in the current slice. |
| `process.spawn` | yes | partial | partial | partial | Provider tests exist; `stdout` / `stderr` still support `Ignore` and `Lines` only, not `Bytes`. |
| Host-context sources (`process.args`, `process.cwd`, `env.get`, `stdio.read`, `path.*`) | yes | yes | partial | partial | This is the best-covered `execute` source subset: `execute_reads_host_context_sources_and_writes_stdout` exercises it directly. |
| `db.connect` / `db.live` | yes | partial | partial | partial | Runtime tests exist, but `pool` is only validated, `optimistic` is still `False`-only, and `onRollback` is rejected in the current slice. |
| `window.keyDown` | yes | n/a | partial | partial | GTK-backed runtime tests exist, but `capture` and `focusOnly` are still fixed to their default values. |
| `dbus.ownName` / `dbus.signal` / `dbus.method` | yes | partial | partial | partial | Runtime tests exist; `dbus.method` still replies with `Unit` immediately and defers non-`Unit` reply payloads. |

## Markup / GTK Surface

| Surface form | Check | Execute | Run | Compile | Notes |
| --- | --- | --- | --- | --- | --- |
| Core markup roots and reactive attributes | yes | n/a | yes | n/a | `prepare_run_artifact` and bundle tests cover the current markup entry path. |
| Current widget catalog | yes | n/a | yes | n/a | The syntax-sheet catalog is supported; the repo also has run tests for additional common widgets, but arbitrary non-catalog widgets are still rejected. |
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

- `compile` is still a first-slice AOT boundary. It emits object code only and explicitly rejects inline `Case`, `TruthyFalsy`, tap/debug, remaining aggregate/collection lowering, and dynamic text.
- Imported user-authored higher-kinded instances and imported polymorphic class-member execution are still deferred.
- Custom `provider` declarations are currently contract/lowering features, not runtime-executable providers.
- Regex literals currently stop at checking/HIR; they do not lower through typed-core general expressions.
- Structural patch removal is not implemented yet, and broader patch lowering remains only partial.
- The source catalog is broad, but several options are still accepted-by-contract and not fully executed yet. Use `/guide/source-catalog` for the option-level truth table.
