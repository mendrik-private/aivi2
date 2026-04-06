# Compiler Pipeline

AIVI's compilation is a layered descent from surface syntax to native Cranelift code. Each crate owns exactly one layer; no layer reaches past its boundary.

## Stage 1 — `aivi-syntax`: Surface Frontend

**Sources**: `crates/aivi-syntax/src/`

- `lex.rs` — tokeniser: produces `LexedModule` with a `Vec<Token>` and a flat token table.
- `parse.rs` — recursive-descent parser: produces `ParsedModule` (a CST `Module` + diagnostics).
- `cst.rs` — Concrete Syntax Tree node types: `Item`, `Expr`, `TypeExpr`, `Pattern`, `MarkupNode`, `PipeExpr`, `SignalMergeBody`, `PatchBlock`, etc.
- `format.rs` — canonical formatter: idempotent pretty-printer over the CST.

The CST is a faithful, lossless representation of the source — every token is recoverable. Errors are represented as `ErrorItem` nodes rather than aborting the parse.

## Stage 2 — `aivi-hir`: Name Resolution & HIR

**Sources**: `crates/aivi-hir/src/`

The largest layer. Responsible for:

1. **Name resolution** (`resolver.rs`) — resolves identifiers to `HirId`s, builds module symbol tables.
2. **HIR lowering** (`lower.rs`) — lowers CST items into typed HIR nodes: `Value`, `Func`, `Signal`, `Source`, `Class`, `Instance`, `Domain`, `Use`.
3. **Type checking** (`typecheck.rs`, `typecheck_context.rs`) — bidirectional type checking with constraint solving.
4. **Elaboration passes** — separate focused passes that enrich the HIR:
   - `gate_elaboration.rs` — pipe stages → gate runtime plans
   - `fanout_elaboration.rs` — fanout/join plans
   - `decode_elaboration.rs` / `decode_generation.rs` — source decode programs
   - `signal_metadata_elaboration.rs` — signal merge metadata
   - `temporal_elaboration.rs` — recurrence/temporal plans
   - `source_lifecycle_elaboration.rs` — source lifecycle actions
   - `truthy_falsy_elaboration.rs` — `?` / `!` operators
   - `general_expr_elaboration.rs` — ambient item and runtime expression elaboration
   - `capability_handle_elaboration.rs` — custom source capability handles
   - `domain_operator_elaboration.rs` — domain operator methods

**Key HIR types** (in `hir.rs`):
- `HirModule` — top-level module with arenas for all node types
- `HirValue`, `HirFunc`, `HirSignal`, `HirSource`, `HirClass`, `HirInstance`, `HirDomain`
- `ApplicativeSpine` — spine of function applications (head + arguments)
- `GateRuntimeExpr` / `GateRuntimePipeExpr` — elaborated gate expressions for runtime

## Stage 3 — `aivi-typing`: Type-Side Semantics

**Sources**: `crates/aivi-typing/src/`

Focused structural derivation plans consumed by HIR elaboration:

- `kind.rs` — kind checking (`*`, `* → *`, `* → * → *`, etc.), `KindChecker`, `KindStore`
- `eq.rs` — structural `Eq` derivation, `EqDeriver`, `EqContext`, `TypeStore`
- `decode.rs` — source decode planning, `DecodePlanner`, `DecodeSchema`
- `fanout.rs` — fanout carrier and plan derivation
- `gate.rs` — gate carrier and plan derivation
- `recurrence.rs` — recurrence/wakeup planning
- `source_contracts.rs` — custom source contract resolution

## Stage 4 — `aivi-core`: Typed Core IR

**Sources**: `crates/aivi-core/src/`

Post-HIR intermediate representation:

- `ty.rs` — `Type` (ground types, type applications, type parameters, HKT)
- `expr.rs` — `Expr`, `ExprKind` — typed core expressions including `PipeExpr`, `PipeStage`, `Pattern`, builtin carriers
- `lower.rs` — `lower_module()`, `lower_runtime_fragment()` — consume HIR elaboration reports and produce `CoreModule`
- `validate.rs` — structural validation of core modules
- `ids.rs` — typed arenas: `ExprId`, `ItemId`, `PipeId`, `StageId`, `SourceId`, `DecodeProgramId`

Intentionally narrow: consumes only elaboration reports the frontend can already justify. Blocked handoffs are rejected explicitly rather than guessed.

## Stage 5 — `aivi-lambda`: Closure/Lambda IR

**Sources**: `crates/aivi-lambda/src/`

Sits between core and backend. Makes closure structure explicit:

- `analysis.rs` — capture analysis: identifies free variables per closure boundary
- `module.rs` — `LambdaModule`, `Closure`, `Capture`, `CaptureId`, `ClosureId`, `ClosureKind`
- `lower.rs` — `lower_module()` consumes `CoreModule`, emits `LambdaModule`
- `validate.rs` — validates closure metadata consistency

Does not yet commit to backend ABI, layout, or calling convention — those are backend concerns.

## Stage 6 — `aivi-backend`: Cranelift Codegen

**Sources**: `crates/aivi-backend/src/`

- `layout.rs` — value layout: `Layout`, domain layouts, record layouts, sum layouts
- `codegen.rs` — main Cranelift IR emission
- `lower.rs` — lowers lambda IR into backend IR nodes
- `program.rs` — `BackendProgram` — the compiled artifact
- `runtime.rs` — runtime-facing entry points
- `gc.rs` — GC integration stubs
- `kernel.rs` — built-in kernel function implementations
- `numeric.rs` — numeric operation lowering
- `validate.rs` — backend IR structural validation
- `cache.rs` — compilation caching

Cranelift is used for both AOT compilation and JIT execution. The `BackendLinkedRuntime` (in `aivi-runtime`) is the bridge between the compiled program and the live runtime.

## Query Layer

The `aivi-query` crate wraps all compilation stages in an incremental, memoised query layer (see [query-layer.md](query-layer.md)). The LSP and CLI both go through this layer rather than calling compiler stages directly.

*See also: [architecture.md](architecture.md), [type-system.md](type-system.md), [runtime.md](runtime.md)*
