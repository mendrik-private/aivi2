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

- `aivi compile` lowers through backend codegen and can write a Cranelift object file, but it still ends by printing that runtime startup/link integration is not available yet (`crates/aivi-cli/src/main_parts/build_tools.rs:96-157`).
- `aivi build` is the runnable packaging path: it copies the runtime, stdlib, reachable workspace sources, and emits a launcher (`crates/aivi-cli/src/main_parts/build_tools.rs:168-223`, `crates/aivi-cli/tests/build.rs:46-171`).
- Backend execution programs can retain compiled object artifacts while still creating lazy-JIT execution engines (`crates/aivi-backend/src/engine.rs:151-263`, `crates/aivi-backend/tests/execution_engine.rs:109-131`).

**Conclusion:** JIT execution is implemented and tested. AOT object emission is implemented and tested. A fully linked native runnable artifact from `aivi compile` is still a product gap.

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

- `aot-runtime-link-boundary`
- `stdlib-undocumented-low-level-modules`

## Follow-up implementation slice

The current follow-up slice closed the highest-confidence docs/API parity issues:

- documented `aivi.api`
- documented the compiler-backed low-level modules `aivi.arithmetic`, `aivi.bits`, and
  `aivi.data.json`
- updated stdlib navigation/index entries so those modules are discoverable
- corrected the CLI help/wiki wording so `compile` vs `build` matches the real runtime boundary
- shifted pair docs and prelude guidance toward `first` / `second` / `mapFirst` / `mapSecond`

## Remaining tracked gap

- `aot-runtime-link-boundary` — native object emission exists, but `aivi compile` still does not
  produce a fully linked runnable artifact. `aivi build` remains the current deployment path.
