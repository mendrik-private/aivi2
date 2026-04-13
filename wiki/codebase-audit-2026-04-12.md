# Codebase Audit — 2026-04-12

Broad audit of the current Rust crates, CLI execution boundary, stdlib/manual parity, and user-facing API naming.

## Scope

Validated directly from source and current test runs:

- `cargo build --bin aivi`
- `cargo test -p aivi-backend --test foundations`
- `cargo test -p aivi-cli --test compile`
- `cargo test -p aivi-backend --test execution_engine`
- `cargo test -p aivi-cli --test build`
- `cargo clippy --all-targets --all-features`

## Verified JIT / AOT boundary

- `aivi compile` lowers through backend codegen and can write a Cranelift object file, but it still stops at object emission rather than producing the final runnable app.
- `aivi build` is now the runnable packaging path: it emits a source-free bundle with the runtime executable, `run-artifact.json`, serialized backend metadata payloads, precompiled native-kernel sidecars, and a launcher (`crates/aivi-cli/src/main_parts/build_tools.rs`, `crates/aivi-cli/src/main_parts/run_artifact.rs`, `crates/aivi-cli/tests/build.rs`).
- `aivi run` can launch either from source/workspace input or directly from the serialized run artifact emitted by `build`.
- Backend execution programs can retain compiled object artifacts while still creating lazy-JIT execution engines (`crates/aivi-backend/src/engine.rs:151-263`, `crates/aivi-backend/tests/execution_engine.rs:109-131`).

**Conclusion:** JIT execution is implemented and tested. AOT object emission is implemented and tested. The old source-carrying runtime-link gap and the later compiled-payload launch gap are both closed for runnable bundles: `aivi build` now emits source-free artifacts plus precompiled native sidecars that `aivi run` consumes without launch-time re-JIT for supported kernels. `aivi compile` still remains object-emission only by CLI contract.

## Highest-confidence crate findings

1. **GC future-proofing gap** — `crates/aivi-backend/src/gc.rs:195-206` has an explicit TODO noting that in-place root replacement bypasses any write barrier. This blocks safe future generational or incremental GC work.
2. **Clone-heavy pipe type walking** — `crates/aivi-hir/src/typecheck_context/helpers.rs:133-240` stores a full `GateExprEnv` by value, clones it at construction, clones `current` subjects per stage, and clones the environment again for non-transform stages.
3. **Clone-heavy LSP symbol search** — `crates/aivi-lsp/src/navigation.rs:32-53` builds `Vec<LspSymbol>` by cloning the whole tree and clones candidate symbols again while searching. A reference-based stack would avoid this churn on every hover/go-to-definition lookup.
4. **Redundant query/workspace allocations** — `crates/aivi-query/src/workspace.rs:81-85` calls `text.to_string()` even though `fs::read_to_string` already produced a `String`. `crates/aivi-query/src/queries/hir.rs:289-291` also materialises extra `Vec`/`Arc` copies for diagnostics.
5. **Representation pressure flagged by clippy**:
   - `crates/aivi-openapi/src/model.rs:251-256` — `SecuritySchemeOrRef`
   - `crates/aivi-hir/src/fanout_elaboration.rs:56-60` — `FanoutSegmentOutcome`
   - `crates/aivi-hir/src/gate_elaboration.rs:318-340` — `GateRuntimePipeStageKind`
   - `crates/aivi-core/src/lower/api.rs:82-159` — large `LoweringError` driving repeated `result_large_err` warnings in `module_lowerer`

These are good candidates for boxing or smaller payload factoring where hot-path size matters.

## Stdlib / manual parity

- `manual/stdlib/index.md:35-38` is stale for key exports:
  - it advertises `aivi.option` names like `withDefault` and `andThen`,
  - and `aivi.result` names like `andThen`, `onOk`, and `onErr`,
  - but current source exports are `getOrElse` / `flatMap` / `mapOr` in `stdlib/aivi/option.aivi:35-112` and `flatMap` / `fold` / `mapBoth` in `stdlib/aivi/result.aivi:38-100`.
- `manual/guide/openapi-source.md:79-97` imports `aivi.api`, but there is no `manual/stdlib/api.md`. The actual module exists at `stdlib/aivi/api.aivi:1-22`.
- `manual/guide/sources.md:21-24` still names `aivi.data.json` as part of the public boundary, but there is no stdlib reference page or index entry for it. The module exists at `stdlib/aivi/data/json.aivi:1-98`.
- Low-level modules `stdlib/aivi/arithmetic.aivi:1-34` and `stdlib/aivi/bits.aivi:1-10` exist but are not surfaced in `manual/stdlib/index.md`. The project should either document them or make the internal/public boundary explicit.

## Naming pressure against AIVI style

- `manual/guide/types.md:165-169` still teaches `value mkRow = Cell 5`. That example should avoid `mk*` naming and instead use a plain descriptive partial application, e.g. `value rowAtX5 = Cell 5`.
- `stdlib/aivi/pair.aivi:3-46` exports `fst`, `snd`, `mapFst`, `mapSnd`, plus both `fromPair` and `toPair` even though `fromPair` and `toPair` have identical implementations. A more AIVI-like surface would prefer `first`, `second`, `mapFirst`, `mapSecond`, and tuple literals (or one helper) over duplicate constructor synonyms.
- `stdlib/aivi/core/fn.aivi:9-45` exposes `flip`, `compose`, and `andThen`. Those helpers are still useful, but the manual should frame pipe algebra and argument order as the primary AIVI style and keep these as secondary tools.

## Follow-up todo IDs

- `aot-runtime-link-boundary` — closed by the source-free run-artifact bundle path
- `stdlib-undocumented-low-level-modules` — closed by new manual/index coverage
- `compiled-aot-artifact-launch` — closed by native-sidecar bundle launch

## Follow-up implementation slice

The current follow-up slice closed the highest-confidence docs/API parity issues:

- documented `aivi.api`
- documented the compiler-backed low-level modules `aivi.arithmetic`, `aivi.bits`, and
  `aivi.data.json`
- updated stdlib navigation/index entries so those modules are discoverable
- corrected the CLI help/wiki wording so `compile` vs `build` matches the real runtime boundary
- replaced the old source-carrying build bundle with a source-free serialized run-artifact bundle
- shifted pair docs and prelude guidance toward `first` / `second` / `mapFirst` / `mapSecond`

## AOT bundle status

- Runnable bundles still keep backend `Program` metadata, but that is now a deliberate runtime-link
  boundary rather than an execution gap: launch uses precompiled native sidecars for supported
  kernels and falls back to existing interpreter/JIT behavior only where the first Cranelift slice
  still cannot lower a kernel.
- `aivi compile` remains object-emission only. Turning that object output into a final linked app is
  separate product work, not the old runtime-link correctness gap.
