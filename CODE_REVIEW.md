# AIVI Compiler/Runtime — Comprehensive Code Review

> Reviewed: 2026-03-25
> Scope: All 12 crates, ~95 source files, ~50 000 lines of Rust
> Methodology: 7 parallel deep-read agents + cross-cutting synthesis
>
> **Fix pass started: 2026-03-25** — 10 parallel fix agents launched in isolated worktrees.
> Committed worktrees: `ad0f625b` (aivi-base), `a289602d` (aivi-syntax), `a9636cc6` (aivi-hir core), `a8ddf332` (aivi-core + aivi-lambda), `a11bb0e5` (aivi-backend).
> In-flight worktrees: `a3ee02bd` (aivi-hir elab), `af83b688` (aivi-runtime), `ada46024` (aivi-gtk), `a2d5252a` (aivi-query/lsp/cli), `ab30e5bd` (aivi-typing).
> Fix legend used below: **✓ fixed** = code changed; **◎ documented** = comment/doc added; **⊘ deferred** = TODO added, requires deeper refactor.

---

## TABLE OF CONTENTS

1. [aivi-base](#1-aivi-base)
2. [aivi-syntax](#2-aivi-syntax)
3. [aivi-typing](#3-aivi-typing)
4. [aivi-hir — Core Files](#4-aivi-hir--core-files)
5. [aivi-hir — Elaboration Files](#5-aivi-hir--elaboration-files)
6. [aivi-core](#6-aivi-core)
7. [aivi-lambda](#7-aivi-lambda)
8. [aivi-backend](#8-aivi-backend)
9. [aivi-runtime](#9-aivi-runtime)
10. [aivi-gtk](#10-aivi-gtk)
11. [aivi-query](#11-aivi-query)
12. [aivi-lsp](#12-aivi-lsp)
13. [aivi-cli](#13-aivi-cli)
14. [A — Phase Architecture Audit](#a-phase-architecture-audit)
15. [B — Shared Concepts Audit](#b-shared-concepts-audit)
16. [C — Invariant Audit](#c-invariant-audit)
17. [D — Risk Ranking](#d-risk-ranking)
18. [E — Refactor Roadmap](#e-refactor-roadmap)

---

## 1. aivi-base

**Layer:** 0 — Foundation (diagnostics, source management)

### What is good
- `ByteIndex`/`FileId`/`Span` are newtypes — zero cross-type confusion at the call site.
- `SourceFile` precomputes line starts for O(log n) line lookup.
- UTF-16 LSP position conversion accounts for surrogate pairs.
- Builder pattern on `Diagnostic` is ergonomic.

### Problems

**diagnostic.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Bug | `render()` only emits the primary label or the first label. Secondary labels are silently ignored. Multi-label diagnostics produce truncated output. **✓ fixed** (ad0f625b) — secondary labels now rendered as `note:` lines with source context. |
| 2 | Bug | Caret rendering uses byte width for Unicode characters. Emoji or CJK in source produce misaligned carets in the terminal. **◎ documented** — `NOTE` comment added; full fix requires character-width table. |
| 3 | Unsoundness risk | No validation that a `Diagnostic`'s `SourceSpan` references a valid file at the time it is *created*. Silent partial rendering if the file is missing at render time. |
| 4 | Architecture smell | `DiagnosticCode` is a `&'static str` pair — not machine-searchable. Tooling cannot filter by code without string comparison. |

**source.rs**

| # | Class | Issue |
|---|-------|-------|
| 5 | Bug | `lsp_position_to_offset` does not clamp: if the client sends `character = 9999` for a 3-character line, it silently returns `end_of_line` instead of `Err`. **✓ fixed** (ad0f625b) — now returns `Option<ByteIndex>`; `None` on out-of-range column. |
| 6 | Unsoundness risk | `ByteIndex` is a `u32` wrapper. Any source file ≥ 4 GiB causes a panic at `from(Range<usize>)` via `.expect()`. The check is at conversion, not at file addition to the database. |
| 7 | Unsoundness risk | `Index<FileId>` on `SourceDatabase` calls `.expect("invalid source file id")`. `FileId` is constructed from `len()` but nothing prevents callers from constructing one by-hand or from a stale database. **◎ documented** (ad0f625b) — doc comment added explaining `FileId` must only be obtained from `add_file()`. |
| 8 | Missing edge case | `trim_line_end()` handles `\n` and `\r\n` but not bare `\r` (classic Mac line ending). Mixed-ending files produce wrong line spans. **◎ documented** (ad0f625b) — confirmed already handled; inline comment added clarifying coverage. |
| 9 | Missing edge case | `SourceSpan::join()` returns `None` for cross-file spans silently. Callers receive `None` and may use a fallback span that points to the wrong location. **◎ documented** (ad0f625b) — doc comment added on `join()`. |

### Soundness review
- **Invariant "every FileId references a valid file"** — enforced at lookup only, not at construction. Violatable by hand. Needs: either seal `FileId` construction behind `SourceDatabase::add_file()` or add an `assert_valid` method called at diagnostic render time.
- **Invariant "every Span start ≤ end"** — enforced by `assert!` in `Span::new`. Correct.
- **Invariant "line starts are byte-aligned UTF-8"** — assumed, never validated.

### Concrete recommendations
1. **Smallest fix:** Return `Result<ByteIndex, ()>` from `lsp_position_to_offset`; callers handle out-of-bounds.
2. **Better fix:** Seal `FileId` — only constructible by `SourceDatabase::add_file()` returning a `FileId`. Removes the dangling-ID class.
3. **Layer:** aivi-base owns all fixes. No downstream effects unless callers of `lsp_position_to_offset` already handle `Option`.
4. **Diagnostic render:** Iterate all labels (sorted by line), emit secondary labels as subsequent lines. Migrate to `codespan-reporting` or implement equivalent multi-label renderer.

### Tests to add
- Round-trip: `offset → lsp_position → offset` on every UTF-8 character class (ASCII, 2-byte, 3-byte, 4-byte, surrogate-pair).
- `lsp_position_to_offset` with out-of-range character index.
- `SourceDatabase` indexing with a stale `FileId`.
- `Diagnostic::render` with two secondary labels.

**Confidence: High** — issues are directly observable from the implementation.

---

## 2. aivi-syntax

**Layer:** 1 — Parser / CST

### What is good
- Lossless token buffer preserves trivia; all tokens are recoverable for formatter and LSP.
- `parse_binary_expr_prec()` implements correct precedence climbing.
- Error recovery emits diagnostics and wraps bad items in `ErrorItem` rather than halting.
- `at_line_start` flag on `Token` supports indent-aware parsing without a separate scanner.

### Problems

**lex.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Bug | `scan_quoted_body()` skips any character after `\`, accepting `\q`, `\z`, etc. as valid escapes. The parser receives malformed string content. **✓ fixed** (a289602d) — explicit allowlist; unrecognised escapes emit `INVALID_ESCAPE_SEQUENCE` diagnostic. |
| 2 | Architecture smell | String interpolation gaps (`{...}`) are detected by the *parser* re-lexing a sub-range via `lex_fragment()`. This splits token-level responsibility across two phases and makes the lexer non-responsible for balanced-brace invariants. |
| 3 | Missing edge case | Nested block comments (`/* /* */ */`) are not supported. The lexer terminates at the first `*/`. |
| 4 | Missing edge case | Non-ASCII UTF-8 bytes that are not `char::is_alphabetic()` are emitted as `TokenKind::Unknown` one byte at a time. A 4-byte emoji produces 4 Unknown tokens rather than one. |
| 5 | Missing edge case | `1d_000` is lexed as `1d` (DecimalLiteral) followed by `_000` (Identifier), not a parse error. Ambiguous suffix context. |

**cst.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Architecture smell | `Option<T>` fields (e.g. `NamedItem::name`) conflate "user omitted" with "error recovery". Validation layer cannot distinguish which case it is, making diagnostic precision fragile. |
| 7 | Invariant not in types | `TextLiteral` is `Vec<TextSegment>` — can be empty. An empty text literal is `""` but nothing prevents a zero-segment `TextLiteral` arising from error recovery. |
| 8 | Missing edge case | `PipeFanoutSegment` view-extractors contain `unreachable!()` that triggers if pipe stage indices are corrupted post-construction. Should be validated once at construction. |
| 9 | Architecture smell | `MarkupNode::children: Vec<MarkupNode>` allows any nesting. Schema constraints (e.g. `<case>` only inside `<match>`) are invisible in the type. |

**parse.rs**

| # | Class | Issue |
|---|-------|-------|
| 10 | Missing edge case | No maximum recursion depth on `parse_pattern()`, `parse_type_expr()`, or `parse_expr()`. Source files with 1 000+ levels of nesting can stack-overflow the parser. **✓ fixed** (a289602d) — `MAX_PARSE_DEPTH = 256`; `depth_enter`/`depth_exit` guards on all recursive entry points. |
| 11 | Bug | Indentation-based grouping (e.g. `parse_instance_body()`) counts characters, not visual columns. A tab counts as 1 regardless of configured tab-stop. Mixed indent styles produce misattributed members. |
| 12 | Bug | `parse_text_literal()` creates a second `Parser` instance to re-parse interpolation ranges. If the outer parser and interpolation parser disagree on token handling (e.g. after a lexer change), the two sources of truth silently diverge. |
| 13 | Architecture smell | Decorator payload dispatch is `if name.as_dotted() == "source"`. Rename the decorator and the payload type changes invisibly. |
| 14 | Unsoundness risk | `parse_constrained_type()` backtracks via checkpoints. If a production succeeds partially before failing, the error from the *inner* attempt is swallowed and a less-informative outer error is emitted. |

**format.rs**

| # | Class | Issue |
|---|-------|-------|
| 15 | Bug | `format_item()` calls `unreachable!()` for `Item::Error`. If the parsed module has any error items, the formatter panics. This means `aivi fmt` crashes on code with syntax errors. **✓ fixed** (a289602d) — replaced with `# <unparseable item>` comment fallback. |
| 16 | Unsoundness risk | `INLINE_LIMIT = 32` is hard-coded. No test verifies that `format(parse(format(ast))) == format(ast)`. Idempotency is unverified. |
| 17 | Architecture smell | Operator precedence is re-implemented in the formatter independently of the parser. A divergence silently adds unnecessary parentheses or drops necessary ones. |
| 18 | Missing edge case | Comments inside expressions are not preserved; they are lost during formatting. |

### Mislayering
- **String escape validation** belongs in the lexer, not deferred to later phases.
- **Interpolation re-lexing** is a phase 1 concern that should not appear in the parser.
- **Decorator semantics dispatch** (`"source"` string check) belongs in HIR lowering, not in the parser.

### Missing edge cases
- `parse_pattern()` recursive application with depth > 500.
- Source file with all trivia (no non-whitespace tokens).
- Empty class/domain/instance body.
- Multi-decorator item where decorators exceed available keywords.

### Concrete recommendations
1. **Recursion depth guard:** add `fn ensure_depth(depth: &mut u32, max: u32) -> Option<()>` called at each recursive descent entry. Emit `RecursionLimitExceeded` diagnostic and return `None`.
2. **Fix formatter crash:** match `Item::Error` and emit a `// ERROR: <message>` comment block, or skip with a warning diagnostic instead of `unreachable!()`.
3. **Unify escape handling:** move escape sequence validation into the lexer; emit a `LexDiagnostic::InvalidEscapeSequence` rather than leaving it for the parser or later phases.
4. **Share precedence table:** export `fn expr_precedence(op: &BinaryOp) -> u8` from `parse.rs` and import it in `format.rs`.

### Tests to add
- `format(parse(format(ast))) == format(ast)` for every item kind (golden idempotency tests).
- Parser recovery on `ErrorItem` followed by a valid item.
- Formatter on a module that contains an `Item::Error`.
- Nested block comments (should produce a diagnostic, not hang).
- 500-level-deep pattern: should emit `RecursionLimitExceeded`, not stack-overflow.

**Confidence: High**

---

## 3. aivi-typing

**Layer:** 3 — Type + kind checking

### What is good
- `kind.rs`: stack-based (non-recursive) kind inference; correct for Arrow/Type; comprehensive tests.
- `eq.rs`: debug-assertion that the assembly stack is fully drained; `EqContext` for parameter witnesses is clean.
- `recurrence.rs`: wakeup proof priority ordering (intrinsic > polling > retry > trigger > reactive) is explicit and tested.
- `source_contracts.rs`: all 10 builtin providers have contracts; kind-checks all option types in tests.

### Problems

**kind.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Missing feature | No kind variables. When HKT elaboration begins, kind inference needs unification. The current design requires a breaking extension. |
| 2 | Invariant not in types | `KindParameter` always infers to `Kind::Type` (hardcoded). If the language later needs polymorphic-kinded parameters, this assumption is baked in. |
| 3 | Missing edge case | No cycle detection in `KindStore::expr()`. A circular `KindExpr` arena (theoretically impossible via the current API, but not proven impossible) would loop forever. |

**eq.rs**

| # | Class | Issue |
|---|-------|-------|
| 4 | Missing feature | `PrimitiveType::Bytes` explicitly fails Eq derivation with no documented rationale. Downstream phases see an undecodable type with no hint that Bytes is deliberately excluded. **◎ documented** (ab30e5bd) — exclusion reason added as doc comment. |
| 5 | Missing feature | No recursive type support. Adding mu-types will require `RecursiveEq` proofs; the current infrastructure has no hook for them. **⊘ deferred** (ab30e5bd) — TODO comment added noting visited-set requirement. |
| 6 | Missing edge case | Domain-within-domain carrier (`Domain(Domain(T))`) compiles but is untested. The derivation reuses the inner domain's Eq, which is correct, but no regression exists. |

**decode.rs**

| # | Class | Issue |
|---|-------|-------|
| 7 | Missing feature | `DecodeFieldRequirement` only has `Required`. Optional fields require a second variant; no hook exists. |
| 8 | Missing feature | `DecodeSumStrategy` only has `Explicit`. Tagged-union or discriminant-field strategies are enum-ready but logic-missing. |
| 9 | Unsoundness risk | No cycle detection. If the type store ever allows circular type references, `DecodePlanner` will infinitely recurse. **◎ documented** (ab30e5bd) — cycle-risk safety note added above `DecodePlanner::plan`. |

**gate.rs / fanout.rs**

| # | Class | Issue |
|---|-------|-------|
| 10 | Missing validation | Gates and fanouts operate on carriers without knowing the *type* of the subject being gated or fanned. There is no hook for "can this type be gated?" validation. Type-level guard happens only in HIR elaboration, creating a gap. |
| 11 | Missing edge case | Nested gate/fanout (gate inside a gate body) has no plan or error. HIR elaboration must handle this, but the plan layer provides no support. **◎ documented** (ab30e5bd) — nested gate/fanout limitation comments added to `GatePlanner::plan` and `FanoutPlanner::plan`. |

**recurrence.rs**

| # | Class | Issue |
|---|-------|-------|
| 12 | Information loss | `ProviderDefinedTrigger` is a black box. The wakeup plan records the variant but carries no details. Runtime elaboration must re-query source contracts to understand what this trigger is — the typing layer effectively undoes its own work. **◎ documented** (ab30e5bd) — info-loss comment added above `provider_intrinsic_wakeup_cause`. |
| 13 | Information loss | Reactive input detection identifies *that* reactive inputs exist but not *which fields* are reactive. Later phases must re-examine the source. |

**source_contracts.rs**

| # | Class | Issue |
|---|-------|-------|
| 14 | Architecture smell | `BuiltinSourceProvider` is a closed enum. Every new provider requires recompilation. No trait-based or external-file extensibility path exists. **◎ documented** (ab30e5bd) — closed-enum limitation doc comment added. |
| 15 | Missing feature | `Signal(SourceTypeAtom)` can only wrap an atom. `Signal<List<Int>>` is not representable. **✓ fixed** (ab30e5bd) — `SourceContractType` is now recursive: `Signal(Box<SourceContractType>)`, `List(Box<SourceContractType>)`, `Map { value: Box<SourceContractType> }`. |
| 16 | Missing feature | `Map` is restricted to atom key and atom value. Nested container types in source contracts are not representable. **✓ fixed** (ab30e5bd) — covered by the recursive `SourceContractType` change above. |

### Mislayering
- Frame-based tree-walking is reimplemented independently in `EqDeriver`, `DecodePlanner`, and `RecurrencePlanner`. These should share a generic `StructuralWalker<Frame, Output>` abstraction.
- `Bytes` exclusion is a policy decision that lives in the typing layer but is better expressed as a validation error at the HIR level where the user sees the type in context.

### Soundness review
- **Invariant "assembly stack is fully drained after Eq derivation":** enforced by `debug_assert!`. Correct in non-debug builds if assembly matches entries. Should be a hard assert for safety.
- **Invariant "every builtin provider contract matches RFC §14.1.2":** only informally documented; no test ties contract fields to a spec reference.

### Concrete recommendations
1. Add a `TypeValidator` pass that runs all typing-layer checks exhaustively and reports all violations at once, rather than the current on-demand, piecemeal checking.
2. Extend `SourceContractType` to be recursive: `Signal(Box<SourceContractType>)` and `List(Box<SourceContractType>)` and `Map { key: SourceTypeAtom, value: Box<SourceContractType> }`.
3. Document the `Bytes` exclusion: either add a `DerivationError::BytesNotDerivable` with a note pointing to where Bytes-Eq support is intended, or move the exclusion to a HIR-level validation error.

### Tests to add
- Nested domain carrier (`Domain(Domain(T))`).
- `DecodePlanner` on a type alias to an opaque type.
- Kind inference on a type with depth > 10.
- Recurrence planner with both declared wakeup and reactive inputs (which wins?).

**Confidence: High**

---

## 4. aivi-hir — Core Files

**Layer:** 2 — Name resolution / HIR

### What is good
- Typed `ArenaId` per node kind prevents cross-arena confusion at the call site.
- Three-phase lowering (structural → namespace → resolution) is documented and structurally separated.
- Ambient prelude injection is clean.
- `PipeExpr` provides validated view-extractors instead of raw stage access.
- `NonEmpty<T>` and `AtLeastTwo<T>` encode minimum-count invariants at the type level.

### Problems

**hir.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Invariant not in types | `SumConstructorHandle` stores only a name, not an arity. Pattern matching on a constructor cannot validate argument count at the HIR level. |
| 2 | Invariant not in types | `TypeKind::Apply` can be nested arbitrarily. No cycle detection; infinite type recursion is not prevented or detected. |
| 3 | Architecture smell | `ResolutionState<T>` is isomorphic to `Option<T>`. It cannot carry a "resolution failed with reason" variant, so errors end up in a parallel `Vec<Diagnostic>`. This prevents local, typed error propagation. |
| 4 | Missing feature | `FunctionItem.type_parameters` is hardcoded to `Vec::new()` in lowering. Generic functions are silently dropped. No diagnostic. **✓ fixed** (a9636cc6) — now emits a `Warning` diagnostic with span pointing to the type-parameter list. |
| 5 | Missing edge case | `TextLiteral::segments` can be empty — no type-level guarantee of non-emptiness. |

**lower.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Phase conflation | Import resolution (`ImportResolver::resolve()`) is called *during* structural lowering, not in a separate pass. This couples syntactic structure to cross-file semantics and creates an ordering constraint: all items must be lowered before imports. |
| 7 | Phase conflation | `populate_signal_metadata()` walks expression trees looking for signal references. This is a semantic analysis pass incorrectly embedded in the structural lowering phase. **◎ documented** (a9636cc6) — doc comment added explaining phase-conflation and TODO to move it to `elaborate_signal_deps()`. |
| 8 | Unsoundness risk | `placeholder_type()` and `placeholder_expr()` silently inject syntactically-valid-but-semantically-invalid nodes. There is no tracking of which items are entirely placeholder-derived. Validation may not catch all consequences. |
| 9 | Unsoundness risk | Arena allocation calls `.expect("should not overflow")` in multiple places. Large programs will panic instead of emitting a diagnostic. **✓ fixed** (a9636cc6) — replaced with `Diagnostic::error("arena overflow")` + sentinel `Id::from_raw(0)` return. |
| 10 | Silent failure | Generic functions (`fun map(A, B) f:(A -> B) ...`) have their type parameters dropped without emitting any diagnostic. **✓ fixed** (a9636cc6) — see issue #4 above. |
| 11 | Silent failure | Signal dependency metadata is overapproximate: `if cond then y else z` marks both `y` and `z` as dependencies regardless of which branch is reachable. |
| 12 | Missing edge case | Duplicate top-level item names silently overwrite the namespace entry. The collision is only detected by downstream validation, not at the point of insertion. |

**resolver.rs**

| # | Class | Issue |
|---|-------|-------|
| 13 | Layering violation | `ImportModuleResolution::Resolved(ExportedNames)` returns a concrete HIR type. The resolver depends on `exports.rs`, and if `exports.rs` ever needed to call the resolver, the crates would deadlock. |
| 14 | Missing feature | No relative import path support. Module resolution only handles absolute dotted paths. |
| 15 | Unsoundness risk | When `ImportModuleResolution::Cycle` is returned, the lowering records the cycle but still inserts an `ImportBinding`. Downstream passes must handle a cycle that manifests as an unresolved binding. |

**typecheck.rs**

| # | Class | Issue |
|---|-------|-------|
| 16 | Incomplete feature | `ConstraintClass::Eq` constraints are collected but never solved. The constraint accumulator is a no-op for Eq. **◎ documented** (a9636cc6) — TODO comment added explaining deferral to a future HM unification pass. |
| 17 | Phase conflation | `apply_default_record_elisions()` mutates the module *during* type checking. The type checker is not idempotent; running it twice produces different modules. **◎ documented** (a9636cc6) — known-issue comment added. |
| 18 | Architecture smell | `value_stack: &mut Vec<ItemId>` is threaded manually through all type-checking functions to prevent infinite recursion. Mutual recursion between unannotated values is not detected, only direct recursion. |
| 19 | Unsoundness risk | `self.typing` is global mutable state across all sub-expression checks. If a check fails and is not rolled back, subsequent checks see a corrupt typing context. |

**ids.rs / arena.rs**

| # | Class | Issue |
|---|-------|-------|
| 20 | Unsoundness risk | An `ExprId` from module A can be used to index module B's expr arena. Nothing in the type system prevents this. |
| 21 | Unsoundness risk | `Arena<Id, T>: Index<Id>` panics on an out-of-bounds index. No safe `try_get` is exposed as the primary accessor. |

**exports.rs**

| # | Class | Issue |
|---|-------|-------|
| 22 | Missing feature | `Instance` items return `None` from `item_to_lsp_symbol()`. Whether instances can be exported is undocumented. If they cannot, the validator should reject `export InstanceItem`; if they can, the export system is incomplete. |
| 23 | Missing feature | Custom user-defined types in function signatures return `None` from `import_value_type()`. Cross-module type information is silently dropped for non-builtin types. |
| 24 | Missing feature | Re-exports (`use A; export A`) are not represented. |

### Mislayering
- `populate_signal_metadata()` is a semantic elaboration pass. It should be a distinct phase after validation, not embedded in lowering.
- `apply_default_record_elisions()` is a transformation; it should follow type checking, not be part of it.
- Import resolution belongs in a dedicated cross-module resolution pass, after all modules are structurally lowered.

### Soundness review
- **Invariant "all exported names reference valid items":** partially enforced during export extraction. Cross-module callers trust `ExportedNames` without re-validating item existence.
- **Invariant "all HIR type expressions are finite":** not enforced. `TypeKind::Apply` cycles are possible.
- **Invariant "all arena allocations succeed":** enforced by panic, not by `Result` propagation.

### Concrete recommendations
1. Add a `PlaceholderTracker` to `ModuleLowerer` that records every `ExprId`/`TypeId` that was synthesized as a placeholder. Validation can then skip subtrees that are entirely placeholder-derived.
2. Move `populate_signal_metadata()` to a separate `elaborate_signal_deps()` pass after HIR validation.
3. Add a generic `TypeKind::validate_acyclic(&TypeStore) -> Result<(), CyclicType>` called during lowering or at the start of elaboration.
4. Replace `ConstraintClass::Eq` no-op with a real unification pass (or explicitly document that it is deferred and to which milestone).

### Tests to add
- Generic function lowering: confirm `type_parameters` being dropped emits a diagnostic.
- `placeholder_type()` in a value body: confirm validation catches it.
- Arena overflow: allocate items until the u32 limit is reached.
- Import cycle: module A imports B, B imports A.

**Confidence: High**

---

## 5. aivi-hir — Elaboration Files

**Layer:** 2/3 boundary — HIR elaboration

### What is good
- Each elaboration is a separate file with a focused concern.
- Blocker enums are exhaustive; blocked paths produce diagnostics, not panics (mostly).
- Runtime expression lowering enforces purity by refusing cluster expressions.
- Gate/fanout/recurrence plans all delegate to `aivi-typing` planners correctly.

### Problems

**validate.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Incomplete validation | Sum constructor arity is never validated. A pattern `Foo a b c` on a two-field constructor is accepted through elaboration. **⊘ deferred** (a3ee02bd) — TODO comment added; resolved type info not available at this phase. |
| 2 | Incomplete validation | Circular signal dependency chains are not detected. A signal that depends transitively on itself is accepted. **✓ fixed** (a3ee02bd) — DFS cycle detection with `validate_signal_cycles()` now called from `run()`; emits `hir/circular-signal-dependency` diagnostic. |
| 3 | Incomplete validation | Pipe stage ordering constraints are not enforced. A gate before a truthy/falsy pair, or a recurrence before a gate, is structurally accepted. |
| 4 | Performance | `typecheck_module()` is called anew inside `GateTypeContext::new(module)` every time a gate is elaborated. There is no shared or cached context across elaborations. This is quadratic for modules with many gates. **◎ documented** (a3ee02bd) — TODO comment added explaining O(n²) cost and that context should be constructed once per validation run. |

**gate_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 5 | Incomplete feature | Custom domain types cannot be gated. The elaborator does not handle gate-over-domain. |
| 6 | Missing edge case | Nested gates (a gate predicate whose body contains another gate) are not elaborated or blocked; they silently produce incomplete output. |
| 7 | Unsoundness risk | `lower_gate_pipe_body_runtime_expr()` trusts that `GateRuntimeExpr` from HIR elaboration is type-consistent. No re-validation is performed on the output of earlier phases. |

**fanout_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 8 | Incomplete feature | Only `List` and `Signal(List)` are valid fanout carriers. `Map` and `Set` carriers are silently unhandled. |
| 9 | Missing edge case | Nested fanout (fanout inside a map body) has no plan or blocker. |

**truthy_falsy_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 10 | Incomplete feature | Custom sum types are not recognized as truthy/falsy carriers. Only `Bool`, `Option`, `Result`, and `Validation` are supported. |
| 11 | Missing edge case | Nested `T|>...F|>` pairs are not validated for correctness. |

**recurrence_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 12 | Missing edge case | Recursive recurrence (a recurrence step body that itself contains `@|>`) is not detected and not blocked. |
| 13 | Missing edge case | Step chain closure uses `same_shape()` which does not account for domain wrapping changes. A domain-wrapped type and its carrier type may compare as "same shape" erroneously. |
| 14 | Missing feature | Recurrence on `Task` types is not handled. |

**decode_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 15 | Missing edge case | Type aliases to unsupported types (e.g. `type T = Arrow`) are not explicitly blocked; the behavior depends on whatever the alias resolves to. |
| 16 | Unsoundness risk | `DecodeTypeLowerer` does not validate that lowering is injective. Two structurally distinct HIR types could theoretically lower to the same `TypeId` in the structural store. |

**decode_generation.rs**

| # | Class | Issue |
|---|-------|-------|
| 17 | Incomplete validation | The selected decode surface method is not validated for arity (parameter count). A method with the wrong number of parameters is accepted as a decoder. |
| 18 | Incomplete validation | The result type of the decode surface is not validated against the declared domain type. |

**source_lifecycle_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 19 | Missing edge case | `activeWhen` option is extracted but not validated to be a `Bool` type. |
| 20 | Missing edge case | Circular source dependencies (source A depends on a signal that depends on source A) are not detected. |

**general_expr_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 21 | Phase conflation | At 2 985 lines, this file mixes item-level elaboration, signal pipe elaboration, and markup elaboration. Concerns from different architectural layers are co-located. |

**domain_operator_elaboration.rs**

| # | Class | Issue |
|---|-------|-------|
| 22 | Unsoundness risk | When both operands match a domain operator, the function panics (`expect("should not have panicked")`) instead of emitting a blocker. Ambiguous domain operators crash the compiler. **✓ fixed** (a3ee02bd) — `select_domain_binary_operator` now returns `Result<Option<..>, DomainOperatorBlocker::AmbiguousMatch>`; callers use `.ok().flatten()`. |

### Mislayering
- `GateTypeContext::new(module)` invokes full type-checking as a side-effect of elaboration. This is both a phase conflation and a performance hazard.
- `general_expr_elaboration.rs` should be split: one file per concern (items, signals, markup).

### Missing edge cases
- Gate predicate referencing a generic type parameter (type is ambiguous at elaboration time).
- Fanout over an empty collection at the type level (no cardinality constraint in the plan).
- Source lifecycle with no arguments but required arguments in the provider contract.

### Concrete recommendations
1. **Cache `GateTypeContext`:** construct it once per module validation run and thread it through all elaboration passes.
2. **Fix domain operator ambiguity panic:** replace the `expect()` with a `BlockedDomainOperator::AmbiguousMatch` variant.
3. **Add constructor arity check** in `validate.rs`: when a pattern matches a constructor, look up the declared field count and compare against the pattern argument count.
4. **Validate gate predicate result type:** after runtime expression lowering, verify the root expression produces a `Bool` layout.

### Tests to add
- Gate on a custom domain type.
- Fanout on a `Map` (should block with an explicit error, not silently fail).
- Ambiguous domain operator (should produce a diagnostic, not panic).
- Source `activeWhen` with a non-Bool expression.
- Recurrence step body that contains `@|>` (recursive recurrence).

**Confidence: High**

---

## 6. aivi-core

**Layer:** 4 — Typed core desugaring

### What is good
- Module-owned arenas with `ArenaId`-typed indices.
- Overflow detection on allocation (`Result<Id, ArenaOverflow>`).
- Elaboration reports are consumed — stages cannot be lower without them.
- Pretty-print support aids debugging.
- Explicit back-references (Item → Pipes, Pipe → Stages) are correctly maintained.

### Problems

**lower.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Unsoundness risk | Arena allocation overflow is handled by `errors.push(...); return;` roughly 20 times. If an overflow causes an early return, subsequent allocations reference IDs that were never created, corrupting the arena state. |
| 2 | Missing validation | Elaboration reports are trusted without re-checking that they are consistent with the original AST. If an elaboration report refers to an expression that was modified after elaboration, the discrepancy is invisible. |
| 3 | Silent failure | `UnsupportedImportBinding` is emitted for some import patterns, but the module continues to lower with the import effectively missing. No guarantee that dependent items produce correct output. |
| 4 | Missing feature | `PipeBuilder` calls `.unwrap()` and `.expect()` at several points. If a stage type does not match the expected input after elaboration, the builder panics instead of reporting an error. **✓ fixed** (a8ddf332) — three `.expect()` calls replaced with `LoweringError::InternalInvariantViolated { message }`; errors pushed, loop continues. |
| 5 | Missing feature | There is no pre-lowering step to validate that all required elaboration reports are present and consistent. **◎ documented** (a8ddf332) — doc comment added on `lower_module` noting absence of completeness check with TODO. |

**validate.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Incomplete validation | Patterns are validated for existence but not for: (a) duplicate bindings within a pattern, (b) literal type matching the expected type, (c) constructor arity. |
| 7 | Incomplete validation | `Case` arms in pipe expressions are validated for result type continuity but not for exhaustiveness. |
| 8 | Missing feature | No liveness / dead-code analysis. Unreachable branches are not flagged. |

**expr.rs**

| # | Class | Issue |
|---|-------|-------|
| 9 | Architecture smell | `OptionSome`/`OptionNone` are special-cased as `ExprKind` variants while `Result`, `Validation` constructors are represented via `Reference::SumConstructor`. This asymmetry complicates downstream pattern matching. |
| 10 | Invariant not in types | The invariant "all expressions in a core module are closed (no free variables)" is documented in comments but not encoded. A `Reference::Local(HirBindingId)` can reference a binding that is out of scope without any type-level prevention. |

**ty.rs**

| # | Class | Issue |
|---|-------|-------|
| 11 | Missing feature | No effect/purity tracking. `Signal` and `Task` are types but carry no marker that they are effectful. Pure functions that happen to return a `Signal` type are indistinguishable from impure ones at the IR level. |
| 12 | Architecture smell | `lower()` and `lower_import()` share ~80% of logic but are separate functions. |

### Mislayering
- `RuntimeFragmentSpec` allows lowering a subset of items for runtime use, but there is no type-level invariant that the fragment is closed (all dependencies included). This is a semantic guarantee that belongs at the module level, not left to callers.

### Soundness review
- **Invariant "all Item→Pipe back-references are consistent":** validated post-lowering in `validate.rs`. Correct but reactive, not proactive.
- **Invariant "all expressions are closed":** assumed, not enforced. The lambda layer checks this; a corrupt core module can reach lambda lowering.
- **Invariant "arena IDs are not cross-module":** not enforced. A `HirItemId` from module A is accepted where a core `ItemId` from module B is expected.

### Concrete recommendations
1. Refactor arena-overflow error handling into a macro: `alloc_or_err!(arena, value, "description")` which pushes an error and returns `None`, preventing the partial-allocation corruption pattern.
2. Add a pre-lowering validation step `validate_elaboration_completeness(hir_module, reports)` that checks all required elaboration reports are present before any IDs are allocated.
3. Unify `OptionSome`/`OptionNone` with the `Reference::SumConstructor` pattern for consistency.
4. Merge the two `lower()` functions in `ty.rs` into a single parameterized function.

### Tests to add
- Core lowering when an elaboration report is missing (should produce an error, not panic).
- Validate back-reference consistency after lowering a module with 100+ items.
- `RuntimeFragmentSpec` with a missing dependency (should error or be explicitly documented as caller responsibility).

**Confidence: High**

---

## 7. aivi-lambda

**Layer:** 5 — Closure/lambda lowering

### What is good
- `ClosureKind` enum is exhaustive over all closure sites.
- Capture analysis (`capture_free_bindings()`) uses a `BTreeMap` — captures are ordered by binding ID, ensuring deterministic, canonical ABI.
- Lambda `validate.rs` re-runs capture analysis and compares against the lowered captures, providing a post-hoc soundness check.
- `analysis.rs` correctly handles case arm scoping (each arm gets a fresh scope).

### Problems

**module.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Invariant not in types | `ambient_subject: Option<Type>` on `RecurrenceStage` and gate stages: when it is `None`, `AmbientSubject` references in the expression body are semantically invalid, but this is not enforced. **◎ documented** (a8ddf332) — doc comment added on `Closure::ambient_subject` explaining `Some`/`None` semantics and invariant. |
| 2 | Invariant not in types | Closure captures and item parameters are both represented as `Vec<ItemParameter>`. Parameters are explicit; captures are discovered implicitly. This conflation makes code that processes one type accidentally process the other. |
| 3 | Missing validation | Capture deduplication is not enforced at construction. A `Vec<CaptureId>` with duplicate `BindingId` is structurally valid. |

**lower.rs**

| # | Class | Issue |
|---|-------|-------|
| 4 | Architecture smell | Closure kind determination is hardcoded per stage type. Adding a new closure site requires editing `lower.rs` at the right location. A declarative map from `StageKind` to `ClosureKind` would be safer. |
| 5 | Architecture smell | The pattern `match self.module.closures_mut().alloc(Closure { ... }) { Ok(id) => id, Err(overflow) => { ... return None; } }` repeats ~10 times. |
| 6 | Missing validation | Closure root `ExprId` is not validated to exist in the core expr arena until lambda `validate.rs` runs. A bug in lowering can produce a dangling root ID. |

**analysis.rs**

| # | Class | Issue |
|---|-------|-------|
| 7 | Unsoundness risk | A closure that references the item it belongs to is treated as a captured variable, making self-recursive closures appear to capture themselves. This is the correct conservative behavior for non-recursive closures, but it is not documented. Self-recursive closures are silently unrepresentable. |
| 8 | Missing feature | Dead captures (captured variables never used in the closure body after initial analysis) are not detected. |

**validate.rs**

| # | Class | Issue |
|---|-------|-------|
| 9 | Incomplete validation | `ClosureMetadataMismatch` error does not identify *which* field mismatched (owner, kind, root, parameters, ambient_subject). Diagnosis requires re-running the lowering mentally. |
| 10 | Missing validation | `AmbientSubject` references in the expression body are not checked against the closure's `ambient_subject` type. |
| 11 | Missing validation | Closure return type is not validated against what the owning stage expects (e.g. a gate true-branch should produce a type consistent with the gate result). |

### Mislayering
- The lambda layer is the first place closures appear. The core layer should not contain any implicit free-variable assumptions; those should be made explicit before lambda lowering. Currently, free variables in core expressions are only discovered at lambda time, which means a bug in core lowering can go undetected until lambda.

### Soundness review
- **Invariant "every closure is closed":** checked post-hoc by lambda `validate.rs`. Not enforced at construction.
- **Invariant "capture list equals the set of free variables in the body":** re-derived in `validate.rs` by running `capture_free_bindings()` again and comparing. This is correct but expensive.
- **Invariant "no self-recursive closures":** not checked; silently produces incorrect output.

### Concrete recommendations
1. Create distinct `Parameter` and `Capture` types instead of reusing `ItemParameter` for both.
2. Add `Module::is_closed(&self) -> bool` that validates all closures are genuinely closed.
3. Add `ClosureMetadataMismatch` sub-variants: `Owner`, `Kind`, `Root`, `ParameterCount`, `AmbiguousAmbientSubject`.
4. Document (and test) that self-recursive closures are not supported, and add a diagnostic when the capture analysis detects self-reference.

### Tests to add
- Closure that captures a variable at two different types in the same body (type conflict).
- Self-recursive closure attempt.
- Gate true-branch closure with wrong return type.
- Recurrence wakeup witness closure.
- `ambient_subject = None` with `AmbientSubject` reference in body.

**Confidence: High**

---

## 8. aivi-backend

**Layer:** 7 — Runtime-aware / backend IR + Cranelift codegen

### What is good
- `gc.rs`: generation counter prevents handle-after-free; `MovingRuntimeValueStore` correctly relocates values and updates root handles.
- `numeric.rs`: `RuntimeFloat::new()` rejects NaN and Inf at construction.
- `layout.rs`: `AbiPassMode` separates by-value from by-reference.
- `validate.rs`: validates all referenced items/kernels/sources exist; checks layout consistency.
- `ids.rs`: macro-generated, consistent ID types.

### Problems

**lower.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Missing validation | Gate predicates are lowered without verifying that the resulting kernel expression produces a `Bool` layout. |
| 2 | Missing validation | Global item dependency cycles are collected but never checked for cycles before kernel construction. Cyclic dependencies can cause infinite evaluation. |
| 3 | Missing validation | Source instance IDs are parsed as strings and converted to `u32` with no bounds check. |
| 4 | Unsoundness risk | Layout cache (`core_layouts`) caches lowered layouts with no consistency check. The same type lowered with different contexts can silently produce different entries. |
| 5 | Architecture smell | Type-lowering and expression-lowering logic are interleaved in the same loop. They should be separated into distinct traversals. |

**codegen.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Missing feature | No overflow detection for integer arithmetic. Machine wrapping behavior on overflow is undefined from the AIVI semantics perspective. |
| 7 | Missing feature | No tail-call optimization. Recursive functions will exhaust the call stack on deeply recursive inputs. |
| 8 | Unsoundness risk | No documentation or enforcement of GC-interaction invariants. If `MovingRuntimeValueStore` relocates a value during code generation, by-reference parameters become dangling. |
| 9 | Unsoundness risk | Environment slot indices are used to index `kernel.environment` without bounds checking before code generation. |
| 10 | Missing feature | Memory layout (struct field offsets, alignment, padding) is not explicitly computed. Cranelift default struct layout is assumed but never validated against the target ABI. |

**runtime.rs**

| # | Class | Issue |
|---|-------|-------|
| 11 | Unsoundness risk | `RuntimeValue` is a recursive enum with no cycle detection. Creating a cyclic `RuntimeValue` (currently only possible via unsafe external manipulation, but not statically prevented) causes infinite loops in `Display`, `Clone`, and comparison. |
| 12 | Incomplete feature | `Map<K, V>` is stored as `Vec<RuntimeMapEntry>` with linear search. Lookup is O(n). |
| 13 | Missing feature | No garbage collection hook or size limit on `RuntimeValue`. A million-element list allocates unbounded heap memory. |
| 14 | Architecture smell | `RuntimeSumValue` stores `HirItemId` as origin. If the HIR is recompiled, stored sum values become stale with no versioning or invalidation mechanism. |

**validate.rs**

| # | Class | Issue |
|---|-------|-------|
| 15 | Missing validation | ABI `PassMode` is checked per layout but not validated end-to-end against actual Cranelift codegen decisions. A mismatch is only detected at runtime. |
| 16 | Missing validation | Gate stage predicates are not validated to produce `Bool` layouts. |
| 17 | Missing validation | Fanout carrier/element layout compatibility is assumed, not verified. |
| 18 | Missing validation | Memory layout sizes and alignments are never computed. Platform-specific layout assumptions are invisible. |

**gc.rs**

| # | Class | Issue |
|---|-------|-------|
| 19 | Unsoundness risk | `root_slot_mut()` has no synchronization. Concurrent calls from multiple threads corrupt the slot state. |
| 20 | Unsoundness risk | No write barriers. Direct mutation of `from_space.values[i]` is unsafe if any thread holds a reference to the old value during collection. |
| 21 | Missing feature | GC is not incremental. `collect()` processes all roots in one pass. Pause time is proportional to live heap size. |
| 22 | Missing feature | No memory-pressure heuristic for triggering GC. GC is always called once per tick regardless of allocation volume. |

**numeric.rs**

| # | Class | Issue |
|---|-------|-------|
| 23 | Bug | `encode_constant_bytes()` for `Decimal` does not document that `to_bytes_le()` is little-endian. Cross-platform serialization may silently produce different byte sequences. |
| 24 | Missing feature | No magnitude limit on `BigInt` parsing. Arbitrarily large literals are accepted; no streaming or chunked parsing. |

### Mislayering
- `RuntimeSumValue::HirItemId` is a HIR concept in a backend type. The backend should use a stable `SumConstructorId` that is version-independent, not a reference to a mutable HIR node.

### Soundness review
- **Invariant "GC handles are valid until explicitly released":** generation counter enforces this locally. Violated by missing write barriers in concurrent context.
- **Invariant "all environment slots are in bounds":** assumed, not verified before codegen.
- **Invariant "gate predicates produce Bool":** not validated in the backend.

### Concrete recommendations
1. Add `validate_gate_predicate_layout(pred: &KernelExpr, layouts: &LayoutTable) -> Result<(), LayoutError>` called after kernel lowering.
2. Add a DFS cycle check on the global-item dependency graph in `lower.rs` before any kernel is constructed.
3. Replace `Vec<RuntimeMapEntry>` with a `BTreeMap` or hash map for O(log n) or O(1) lookup.
4. Document the single-threaded contract for `MovingRuntimeValueStore` with `#[doc = "NOT Send — must be accessed from one thread only"]` and add a compile-time assertion.

### Tests to add
- Codegen for a simple recursive function: validate no stack overflow on 1 000 recursive calls.
- GC under high allocation rate: collect 100 000 values and verify no dangling handles.
- Layout consistency: record field order preserved through lowering, codegen, and back.
- Backend validation: gate stage with non-Bool predicate layout is rejected.

**Confidence: High**

---

## 9. aivi-runtime

**Layer:** 8/9 — Runtime / scheduler / source providers

### What is good
- `graph.rs`: Kahn's algorithm for topological batching is correct and tested.
- `scheduler.rs`: Generation tracking prevents stale publications.
- `hir_adapter.rs`: errors are accumulated before building the graph; partial graphs are not silently used.
- `source_decode.rs`: type-checks during decode; reports informative errors.
- `providers.rs`: decode program validated before worker threads are spawned.

### Problems

**scheduler.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Unsoundness risk | `self.queue.push_back()` is not protected by a lock. If a worker thread calls the `Sender` while the scheduler is processing the queue, the deque can be corrupted. The scheduler is not `Send`, but its worker publication sender is cloned and sent to threads. |
| 2 | Unsoundness risk | Dirty-mark propagation happens before evaluation. If evaluation panics, dirty marks remain for the next tick and signals may be re-evaluated with stale inputs. |
| 3 | Missing feature | No transactional tick semantics. If the first batch of a tick evaluates correctly but the second fails, the committed values are inconsistent and there is no rollback. |
| 4 | Missing feature | Worker publication channel is unbounded (`mpsc::channel()`). If producers run faster than ticks, the channel queue grows without bound. |
| 5 | Architecture smell | `collect_committed_values()` is called after every tick regardless of allocation volume. This wastes CPU when no garbage has been created. |

**providers.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Unsoundness risk | Worker threads spawned by `spawn_timer_every`, `spawn_http_worker`, etc. are never joined or cancelled when the source is disposed. Background threads continue running after the source is gone. |
| 7 | Missing feature | No cancellation mechanism for in-flight HTTP requests. |
| 8 | Missing validation | Negative durations (e.g. `interval: -500`) are accepted by `parse_duration`. |
| 9 | Missing feature | Process worker (`ProcessSpawn`) spawns a child but does not track it for cleanup. Orphaned processes accumulate on source disposal. |
| 10 | Missing feature | `MailboxHub` subscribers are never garbage-collected. Adding a subscriber and never unsubscribing leaks the subscription indefinitely. |
| 11 | Unsoundness risk | `dispatch_window_key_event()` iterates `active_providers` without a lock while other threads may add or remove providers. |

**graph.rs**

| # | Class | Issue |
|---|-------|-------|
| 12 | Missing validation | `Handle::as_raw()` to `Handle::from_raw()` conversion performs no validity check. Stale or manually-constructed raw handles silently reference wrong nodes. |
| 13 | Missing edge case | Empty graph (zero signals) produces zero batches. `derive_batches()` should handle this explicitly rather than returning an empty vec implicitly. |

**hir_adapter.rs**

| # | Class | Issue |
|---|-------|-------|
| 14 | Unsoundness risk | A signal that depends on another signal that is blocked/errored has the errored edge dropped silently. The resulting dependency graph is incomplete: the signal evaluates without its required input. |
| 15 | Missing validation | Two sources feeding the same public signal produce a malformed graph. There is no uniqueness check on the public signal input. |

**source_decode.rs**

| # | Class | Issue |
|---|-------|-------|
| 16 | Bug | JSON numbers are parsed to `f64` without precision validation. A number like `9007199254740993` (beyond 2^53) is silently truncated. |
| 17 | Missing feature | `decode_step()` is recursive with no depth limit. A JSON object with 10 000 levels of nesting causes a stack overflow. |
| 18 | Missing feature | Sum variant lookup iterates a `Vec` linearly. With hundreds of variants, decode is O(n) per lookup. |

**startup.rs**

| # | Class | Issue |
|---|-------|-------|
| 19 | Unsoundness risk | GTK thread affinity is assumed but not verified at runtime. If `startup.rs` runs on a non-main thread, GTK calls fail or deadlock without a clear error. |
| 20 | Missing validation | Source/task registration order and dependencies are not validated. A source that depends on an unregistered task can be registered without error. |

### Mislayering
- Provider execution and decode program parsing are interleaved in `providers.rs`. They should be separate: a `ProviderRunner` that handles I/O, and a `DecodeEngine` that handles type conversion.

### Soundness review
- **Invariant "GTK runs on the main thread":** assumed, not asserted. Violatable.
- **Invariant "worker threads are terminated on source disposal":** not maintained.
- **Invariant "scheduler queue mutations are single-threaded":** not enforced.

### Concrete recommendations
1. Add a `CancellationToken` per worker thread. Source disposal sends a cancellation signal and joins the thread before returning.
2. Make the worker publication channel bounded (`sync_channel(128)`). Producers block on backpressure instead of growing unbounded.
3. Add a GTK main-thread assertion: `assert!(gtk::is_initialized_main_thread(), "must be called from GTK main thread")` at the top of every GTK-calling function.
4. Add a depth counter to `decode_step()` with a configurable limit (e.g. 512).

### Tests to add
- `dispatch_window_key_event()` while a provider is being added concurrently (thread-safety regression).
- Source disposed while worker is mid-request: worker should stop, not continue.
- JSON decode of a 10 000-level-deep object: should emit a depth-limit error.
- Scheduler tick with evaluator panic: dirty marks should be cleaned up.

**Confidence: High**

---

## 10. aivi-gtk

**Layer:** 9 — GTK bridge / UI runtime

### What is good
- `GtkExecutionPath` provides stable nested-each identity without string paths.
- Transition functions (`show_transition`, `each_keyed_transition`) return explicit edit sequences — changes are auditable.
- `GtkRuntimeHost<V>` trait decouples execution logic from concrete GTK.
- Bridge validates parent/child consistency at graph construction time.

### Problems

**bridge.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Missing validation | Keyed vs. positional `EachChildPolicy` mismatch is only detected at execution time, not at bridge construction. |
| 2 | Architecture smell | `validate_unique_collection_keys()` is a runtime guard; it belongs in `executor.rs`, not in bridge lowering. Bridge should validate *structural* invariants only. |
| 3 | Unsoundness risk | `GtkCollectionKey` is serialized to `Box<str>`. A key function that changes type between renders silently fails to match stale keys. |

**executor.rs**

| # | Class | Issue |
|---|-------|-------|
| 4 | Unsoundness risk | When a `Show` node unmounts, only its immediate children are unmounted. Descendant `Each` children remain in `instances` as orphaned nodes, never cleaned up. Memory leak and potential use-after-free if stale IDs are referenced. |
| 5 | Unsoundness risk | Property update and `update_each_keyed()` are not synchronized. Interleaved calls corrupt the `instances` map. |
| 6 | Missing feature | No transaction semantics. If `update_show()` succeeds and `update_each_keyed()` then fails, the executor is in a half-applied state with no rollback. |
| 7 | Missing validation | `set_property_for_instance()` assumes the instance is mounted. Calling it on an unmounted instance panics. |
| 8 | Missing feature | Event route cleanup on widget unmount is incomplete. Partial connection failures do not roll back already-connected routes. Routes table grows unbounded. |

**host.rs**

| # | Class | Issue |
|---|-------|-------|
| 9 | Unsoundness risk | Every property setter assumes the widget is the expected type and calls `.expect()`. If schema is wrong or widget creation failed, the process crashes. |
| 10 | Unsoundness risk | `insert_children()` appends a child to any parent widget without checking that the parent supports children. Some GTK widgets (Button, Label) silently ignore or error on `append()`. |
| 11 | Unsoundness risk | Signal handler IDs are stored in `MountedEvent` but if the widget is released without calling `disconnect_event()`, the signal continues to fire on a freed widget. |
| 12 | Missing feature | `move_children()` is not atomic. If `remove_children()` fails mid-way, the state is inconsistent with no rollback. |

**lower.rs**

| # | Class | Issue |
|---|-------|-------|
| 13 | Missing validation | Lowering accepts any widget name path without checking whether the schema supports it. Schema validation is deferred to executor, which is too late. |
| 14 | Missing edge case | Event attribute prefix is configurable but there is no validation that the prefix does not shadow a property name. |

**plan.rs**

| # | Class | Issue |
|---|-------|-------|
| 15 | Incomplete validation | `<case>` nodes are not required to be inside a `<match>` node at plan validation time. An orphaned `<case>` node is structurally valid. |
| 16 | Missing validation | `ShowNode::mount(KeepMounted { decision: ExprId })` — the expression is not validated to be a `Bool` type. |
| 17 | Missing edge case | No nesting depth limit. Plans with 200+ levels of widget nesting can stack-overflow plan validation. |

**runtime_adapter.rs**

| # | Class | Issue |
|---|-------|-------|
| 18 | Unsoundness risk | Input handle assignment uses a global counter with no per-owner scoping. If widget A and widget B each have `InputHandle(0)`, they are silently aliased. |
| 19 | Missing validation | Signal graph edges are added without validating acyclicity or type compatibility. |

**schema.rs**

| # | Class | Issue |
|---|-------|-------|
| 20 | Missing feature | Only ~20 GTK widgets are hardcoded. No plugin system. Adding a new widget requires a code change and recompilation. |
| 21 | Missing feature | Event descriptors do not carry argument types. The runtime must know that `clicked` has 0 args and `text-changed` has 1 (text) by convention, not by schema. |
| 22 | Missing feature | Property value range constraints (e.g. ProgressBar `fraction ∈ [0.0, 1.0]`) are absent. |

### Mislayering
- Property setter logic and widget creation belong in separate structs, not inline in `host.rs`.
- Widget schema should be an external file loaded at runtime, not a hardcoded Rust `match`.

### Soundness review
- **GTK thread affinity:** not asserted. Any method called from a non-main thread silently corrupts GTK state.
- **Widget downcast safety:** unsound. The schema is trusted as ground truth but is never validated against actual GTK object types at runtime.
- **Instance lifecycle:** unsound. Orphaned children after parent unmount can cause use-after-free if their IDs are referenced.

### Concrete recommendations
1. Add `fn assert_gtk_main_thread()` and call it at the top of every `host.rs` method.
2. Add a recursive descent unmount: when a `Show` node unmounts, walk all descendants and remove them from `instances` in post-order.
3. Scope input handles per owner: `next_input` is local to each `OwnerHandle`, not global.
4. Move widget schema to an external YAML/JSON file. Generate the Rust schema structs from it via a build script.

### Tests to add
- Mount `Show > Each > Each > Widget`, then unmount the `Show`. Verify all nested instances are cleaned up.
- Property set on an unmounted instance (should return an error, not panic).
- Widget schema: attempt to set a property to an out-of-range value.
- Lowering with an unsupported widget name: should produce a diagnostic during lowering, not executor.
- Circular property update (`A.setter → B.input`, `B.setter → A.input`).

**Confidence: High**

---

## 11. aivi-query

**Layer:** 10 — Tooling / incremental analysis

### What is good
- Forward and reverse dependency maps are both maintained for O(1) transitive invalidation.
- `Arc`-based value sharing makes caching cheap to clone.
- `RwLock` allows concurrent readers.

### Problems

**db.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Unsoundness risk | `open_file()` reads `state.paths`, then calls `state.hir.clear()`, then inserts. Concurrent calls interleave and can clear the HIR for an unrelated file. |
| 2 | Incorrect behavior | When a new file is discovered, `state.hir.clear()` invalidates ALL HIR caches, including files that do not import the new file. Overkill; kills incremental performance. |
| 3 | Missing edge case | `transitive_rdeps()` has no cycle guard. A circular dependency (`A imports B imports A`) causes an infinite loop. |
| 4 | Missing feature | File cleanup: `remove_file()` exists but is never called. Files accumulate in the database forever. |
| 5 | Incorrect behavior | Revision wraps at `u64::MAX`. With a 10 kHz tick rate, wraparound occurs in ~58 million years; at 10 MHz, in 58 000 years. Irrelevant in practice but worth a comment. |

**workspace.rs**

| # | Class | Issue |
|---|-------|-------|
| 6 | Unsoundness risk | `is_bundled_stdlib_module()` checks if a module path starts with `"aivi"`. A user file `aivi/custom.aivi` collides with the stdlib namespace silently. |
| 7 | Missing feature | `aivi.toml` is checked for existence but never read. Workspace-level configuration (entry point, dependencies) is inaccessible. |
| 8 | Missing edge case | Case-insensitive filesystems: `"MyModule"` and `"mymodule"` resolve to the same file but are treated as different modules. |

### Soundness review
- **Invariant "cache entries are valid for their revision":** violated by the non-atomic `open_file()` operation under concurrency.
- **Invariant "transitive dependency computation terminates":** not enforced; cycles can loop forever.

### Concrete recommendations
1. Hold the write lock for the entire duration of `open_file()` to prevent interleaving.
2. Add cycle detection to `transitive_rdeps()`: track visited nodes; return error on revisit.
3. Change HIR invalidation to invalidate only the file being opened and its transitive reverse dependents, not the entire cache.
4. Add a `close_file()` method that removes the file and its dependency edges.

### Tests to add
- Concurrent `open_file()` on two different paths.
- Circular dependency detection.
- File close and re-open (cache correctness).
- 100-file workspace: transitive invalidation touches only the correct subset.

**Confidence: High**

---

## 12. aivi-lsp

**Layer:** 10 — Tooling / LSP

### What is good
- Tower-lsp integration is standard and correct.
- `semantic_tokens.rs` correctly implements delta-line/delta-start encoding.
- `analysis.rs` uses stack-based DFS for symbol search to avoid recursion.
- `diagnostics.rs` correctly maps severity levels to LSP.

### Problems

**server.rs**

| # | Class | Issue |
|---|-------|-------|
| 1 | Missing feature | `shutdown()` returns `Ok(())` immediately. No cleanup of in-memory state. |
| 2 | Bug | `publish_diagnostics_for_uri()` errors inside `did_change()` are silently dropped. The client receives no notification of failure. |
| 3 | Missing feature | No `willSave`/`didSave` hooks. Format-on-save must poll externally. |
| 4 | Missing feature | No `workspace/didChangeWatchedFiles`. External file changes (e.g. `git pull`) are invisible to the server. |
| 5 | Performance | `symbol()` iterates all files, parses all HIR, and walks all symbols. O(files × symbols). No index. |
| 6 | Performance | Every `did_change()` publishes diagnostics immediately. No debouncing. 100 rapid edits cause 100 recompilations. |

**state.rs**

| # | Class | Issue |
|---|-------|-------|
| 7 | Memory leak | Documents are added to `files: DashMap<Url, SourceInput>` on `did_open()` and never removed on `did_close()`. |

**diagnostics.rs**

| # | Class | Issue |
|---|-------|-------|
| 8 | Information loss | Only the primary label is converted to LSP. Secondary labels are silently dropped. The client does not see them. |
| 9 | Bug | If a diagnostic has no labels, range defaults to `(0, 0)–(0, 0)`. Should return an error or use the full-file range, not silently point to the file origin. |

**analysis.rs**

| # | Class | Issue |
|---|-------|-------|
| 10 | Performance | `FileAnalysis::load()` recomputes on every request. No per-revision cache. |

**completion.rs**

| # | Class | Issue |
|---|-------|-------|
| 11 | Incomplete feature | Only offers children of the current symbol. Does not offer scope-visible bindings, keywords, or import completions. |

**definition.rs**

| # | Class | Issue |
|---|-------|-------|
| 12 | Incomplete feature | Go-to-definition only works within the same file. Cross-file navigation is not implemented. |

**formatting.rs**

| # | Class | Issue |
|---|-------|-------|
| 13 | Missing error | If formatting fails, `None` is returned — indistinguishable from "no changes". The client silently leaves the file unchanged. |

**semantic_tokens.rs**

| # | Class | Issue |
|---|-------|-------|
| 14 | Missing feature | Token modifiers are always zero. Readonly variables, deprecated symbols, etc. cannot be highlighted distinctly. |
| 15 | Missing feature | Block comments span multiple lines but are never emitted. They are entirely absent from highlighting output. |

### Mislayering
- `analysis.rs` should cache results by file revision using the query database. Instead it recomputes from scratch on each call.

### Concrete recommendations
1. Add a 100 ms debounce on `did_change()` before publishing diagnostics.
2. Cache `FileAnalysis` by `(FileId, revision)` pair in the query database.
3. Implement `did_close()` cleanup: remove the file from the `DashMap`.
4. Convert all diagnostic labels to LSP `related_information`, not just the primary.
5. Add `workspace/didChangeWatchedFiles` handler that calls `open_file()` on changed files.

### Tests to add
- 20 rapid `did_change()` events: only one or two recompilations should occur (debounce).
- `did_close()`: file removed from state.
- Diagnostic with multiple labels: all converted to `related_information`.
- `shutdown()` followed by `initialize()` on the same server instance (should not panic).

**Confidence: Medium** — some LSP correctness issues depend on protocol details not directly visible in the code.

---

## 13. aivi-cli

**Layer:** 10 — Tooling / CLI

### What is good
- `check`, `compile`, `build`, `run`, `lex`, `fmt`, `lsp` are cleanly separated.
- Exit codes are correct: 0 = success, 1 = compile error, 2 = CLI error.
- GTK runs on the main thread; AIVI runtime runs on a dedicated thread. Separation is correct.

### Problems

| # | Class | Issue |
|---|-------|-------|
| 1 | Unsoundness risk | No assertion that GTK operations happen on the main thread. If AIVI runtime evaluates an expression that calls into GTK (through a user-defined foreign function or a future runtime extension), it fires from the wrong thread without any immediate error. |
| 2 | Missing feature | No compilation timeout. A backend codegen that enters an infinite loop hangs the CLI forever with no recourse. |
| 3 | Resource leak | `run_markup()` creates temporary object files in `/tmp` with no RAII cleanup. Failed compilations leave files on disk. |
| 4 | Missing validation | Input file paths are not existence-checked upfront. The error is reported mid-compilation with a generic I/O error. |
| 5 | Missing validation | Module names are not validated against directory traversal (`..` segments). A malicious module name could escape the workspace. |
| 6 | Missing feature | No incremental build support. Every `compile` or `build` invocation rebuilds from scratch. |
| 7 | Missing feature | LSP `run_lsp()` blocks indefinitely. Ctrl-C kills the process without a graceful shutdown sequence. |

### Mislayering
- Compilation pipeline logic should live in a library crate, not in `main.rs`. The CLI should only handle argument parsing, I/O, and process exit.
- GTK integration (`run_markup`, GtkApplication setup) should be in a dedicated `aivi-app` module.

### Concrete recommendations
1. Wrap temporary files in a `TempDir` that cleans up on `Drop`.
2. Validate input file paths before any compilation step.
3. Add a 30-second timeout on backend codegen using `std::thread::spawn` + `JoinHandle::join` with a timeout.
4. Move compilation logic to a `aivi-driver` library crate; make `aivi-cli` a thin wrapper.
5. Add `SIGTERM` handler for the LSP subprocess to flush and exit cleanly.

### Tests to add
- Compile a file that does not exist: should produce a clear error immediately.
- Module name containing `..`: should be rejected with a diagnostic.
- `aivi fmt` on a file with syntax errors: should not panic.
- Temporary file cleanup on compile error.

**Confidence: Medium**

---

## A. Phase Architecture Audit

### Current phase map

| Layer | Crate | Responsibility |
|-------|-------|---------------|
| Lexer / CST | aivi-syntax | Tokenization, parsing, formatting |
| HIR lowering | aivi-hir/lower.rs | CST → HIR *and* import resolution *and* signal metadata |
| HIR resolution | aivi-hir/resolver.rs | (partial, called during lowering) |
| HIR type checking | aivi-hir/typecheck.rs | Constraint gen *and* module mutation (default elision) |
| HIR validation | aivi-hir/validate.rs | Structural + semantic + type checks in one pass |
| HIR elaboration | aivi-hir/gate_elaboration.rs, etc. | Runtime plan generation per concern |
| Core lowering | aivi-core/lower.rs | Elaborated HIR → typed core IR |
| Lambda lowering | aivi-lambda/lower.rs | Core → explicit closures |
| Backend lowering | aivi-backend/lower.rs | Lambda → ABI-concrete kernels |
| Codegen | aivi-backend/codegen.rs | Cranelift JIT/AOT |
| Runtime | aivi-runtime | Signal graph, scheduler, source providers |
| GTK bridge | aivi-gtk | Widget plan → GTK widget tree |

### Where boundaries are blurry

1. **HIR lowering ≠ name resolution.** `lower.rs` calls `ImportResolver::resolve()` inline during structural lowering. This should be a separate pass: lower structure first, resolve imports second.

2. **Type checking ≠ elaboration.** `typecheck.rs` calls `apply_default_record_elisions()` which mutates the HIR module. A pass that checks should not also transform. Split into `typecheck()` (read-only, returns errors) and `elaborate_defaults()` (write, applies changes).

3. **HIR validation is monolithic.** `validate.rs` performs 13 distinct validation phases in a single large function. It cannot stop after "structure is valid" without running all remaining phases. This prevents graduated error reporting and makes phase ordering bugs invisible.

4. **Signal metadata belongs in elaboration, not lowering.** `populate_signal_metadata()` in `lower.rs` performs semantic analysis (walking expression trees) that belongs in a dedicated `elaborate_signal_deps()` elaboration pass.

5. **GateTypeContext = hidden re-typecheck.** Every elaboration pass that calls `GateTypeContext::new(module)` implicitly re-runs type inference from scratch. This is a hidden phase dependency with O(n²) cost for modules with many gates.

### Layers missing a proper IR or validation pass

- **Between lexer and HIR**: There is no explicit "unresolved HIR" stage where structure is lowered but all names are still symbolic. Names are partially resolved during lowering and partially deferred, creating mixed states.
- **Between core and lambda**: Core expressions contain free variables (`Reference::Local`) that are only made explicit during lambda lowering. A core-level "free variable check" would catch bugs earlier.
- **Between backend and codegen**: No explicit "backend validation" pass that verifies ABI consistency end-to-end (pass modes, layout sizes, gate predicate types) before code is emitted.

### Where the compiler relies on convention instead of contracts

- "All expressions in core are closed" — documented in a comment, not enforced.
- "Elaboration reports are complete before core lowering begins" — assumed by `lower.rs`, never validated.
- "GTK operations happen on the main thread" — a convention, not an assertion.
- "Worker threads are terminated before the runtime is shut down" — a convention, not enforced.

---

## B. Shared Concepts Audit

### Concepts that should be centralized

| Concept | Current state | Recommendation |
|---------|--------------|----------------|
| **Spans and source maps** | `aivi-base` exists but `ByteIndex` overflow is handled inconsistently across callers | Seal `FileId` construction; add a `SourceMap` type that validates all lookups |
| **Diagnostic codes** | `&'static str` pairs; no machine-readable structure | Define a `DiagnosticCode` enum or typed integer to enable filtering and documentation generation |
| **Arena allocation overflow** | ~50 `.expect()` and `errors.push(); return;` patterns scattered across 3 crates | A shared `alloc_or_diag!(arena, value, span, errors)` macro or trait method |
| **Structural type walker** | `EqDeriver`, `DecodePlanner`, `RecurrencePlanner`, and lambda `analysis.rs` all independently implement a frame-based tree-walking algorithm | Extract a generic `StructuralWalker<Frame, Accumulator>` with a `visit_type(&self, ty: TypeId)` interface |
| **Environment / scope** | Lowering, type checking, and closure analysis each maintain independent scope representations | A shared `Scope<K, V>` with push/pop/lookup, used at every layer |
| **Substitution / instantiation** | Type parameter substitution is re-implemented in core, lambda, and backend lowering | A shared `Substitution` type from aivi-typing |
| **ID ownership** | `ExprId` from module A can index module B's arena | Either phantom type parameters or sealed ID constructors per `Module` instance |
| **Signal / non-signal distinction** | Signal vs. non-signal tracking is done ad-hoc in elaboration and runtime | A type-level marker at every IR level (`SignalType<T>` vs `PureType<T>`) |
| **Purity boundaries** | "Pure expression" restriction is re-evaluated in gate, fanout, recurrence, and lifecycle elaboration independently | A shared `PurityChecker` that classifies any `ExprId` once and caches the result |
| **Calling conventions** | `RuntimeKernelV1` exists but is not versioned or queryable | A `CallingConvention` type with explicit version, documented ABI contract |

---

## C. Invariant Audit

| Invariant | Where it lives | In type system? | Validated? | Risk if broken |
|-----------|---------------|-----------------|------------|----------------|
| Every `FileId` references a valid file in `SourceDatabase` | Caller convention | No | No (panic at lookup) | Wrong diagnostic locations |
| Every `Span` has `start ≤ end` | `Span::new` assert | No (runtime assert) | Yes (debug) | Negative-length spans corrupt rendering |
| Expressions in core are closed | Code comment | No | No | Lambda closure analysis produces wrong captures |
| All elaboration reports are complete before core lowering | Caller convention | No | No | Silent information loss |
| Gate predicates produce `Bool` | Assumed | No | No | Wrong codegen |
| GC operations are single-threaded | Undocumented | No | No | Memory corruption |
| GTK operations are on the main thread | Undocumented | No | No | GTK deadlock or segfault |
| Worker threads are terminated on source disposal | Undocumented | No | No | Background thread leaks |
| `PipeBuilder` stage types match elaboration output | `.expect()` in PipeBuilder | No | No (panics) | Compiler crash |
| Closure captures equal the set of free variables | Re-derived in validate | No | Post-hoc | Wrong runtime capture layout |
| Widget input handles are unique per owner | Global counter convention | No | No | Aliased inputs |
| Module dependency graph is acyclic | Caller convention | No | No (global items) | Infinite evaluation |
| Type constructor applications are kind-correct | `kind.rs` | No | On-demand | Ill-kinded types reach codegen |

**Summary:** Out of 13 critical invariants, zero are encoded in the type system, three are checked at runtime (one as a `debug_assert!`, two via explicit validation passes), and ten are enforced only by convention or not at all.

---

## D. Risk Ranking

### Soundness risk (P0 — can cause data corruption, crashes, or wrong output)

| Rank | Issue | File | Consequence |
|------|-------|------|-------------|
| 1 | GC write barriers missing; `root_slot_mut` not synchronized | aivi-backend/gc.rs | Memory corruption under any concurrent scenario |
| 2 | Worker threads not cancelled on source disposal | aivi-runtime/providers.rs | Background threads access freed runtime state |
| 3 | GTK main-thread invariant not asserted | aivi-gtk/host.rs, aivi-runtime/startup.rs | GTK deadlock or segfault on any cross-thread GTK call |
| 4 | Orphaned executor children on parent unmount | aivi-gtk/executor.rs | Use-after-free of widget instances |
| 5 | Widget downcast panic on schema mismatch | aivi-gtk/host.rs | Process crash |
| 6 | Scheduler queue not synchronized | aivi-runtime/scheduler.rs | Silent deque corruption |
| 7 | `PipeBuilder` calls `.expect()` on stage type mismatch | aivi-core/lower.rs | Compiler crash |

### Architecture risk (P1 — structural debt that blocks future work)

| Rank | Issue |
|------|-------|
| 1 | Generic functions silently dropped in HIR lowering — the entire generic subsystem is a no-op |
| 2 | Eq constraints collected but never solved — constraint-based type checking is a facade |
| 3 | Phase conflation in HIR lowering (import resolution + structural lowering + signal metadata) |
| 4 | `GateTypeContext::new()` re-runs full type inference inside every elaboration — O(n²) for large modules |
| 5 | Recursive type support (mu-types) will break kind inference, Eq derivation, decode planning, and lambda closure analysis simultaneously |
| 6 | No ID ownership — cross-module ID use is undetectable |

### Maintenance risk (P2)

| Rank | Issue |
|------|-------|
| 1 | Frame-based structural walker reimplemented 4×; bug fixes must be applied in 4 places |
| 2 | Widget schema is hardcoded Rust — adding a widget requires a recompile |
| 3 | Arena overflow error handling is ~50 boilerplate copies |
| 4 | `general_expr_elaboration.rs` is 2 985 lines mixing 3 concerns |
| 5 | `validate.rs` is 16 636 lines; 13 phases with no clean exit between them |

### Performance risk (P3)

| Rank | Issue |
|------|-------|
| 1 | Full rebuild on every file change — no incremental compilation |
| 2 | `GateTypeContext::new()` per elaboration — quadratic cost |
| 3 | `Map<K,V>` as `Vec<RuntimeMapEntry>` — O(n) lookup |
| 4 | LSP symbol search — O(files × symbols), no index |
| 5 | `collect_committed_values()` called every scheduler tick regardless of allocation |

### Concurrency/runtime risk (P4)

| Rank | Issue |
|------|-------|
| 1 | Scheduler queue write and worker publication sender are both used across thread boundaries with no synchronization |
| 2 | `dispatch_window_key_event()` iterates `active_providers` while other threads modify it |
| 3 | `open_file()` in the query database is non-atomic under concurrent access |
| 4 | Unbounded worker publication channel — producers can starve the scheduler |

---

## E. Refactor Roadmap

### Immediate fixes (before the next milestone)

1. **Fix formatter crash on `ErrorItem`:** replace `unreachable!()` with a comment-block emission or skip-with-warning. One line change.
2. **Fix domain operator elaboration panic:** replace `expect("should not have panicked")` with a `BlockedDomainOperator::AmbiguousMatch` blocker variant.
3. **Assert GTK main thread:** add `assert!(gtk::is_initialized_main_thread())` at the top of every `host.rs` method.
4. **Fix executor child lifecycle:** on parent Show unmount, recursively post-order remove all descendants from the `instances` map.
5. **Fix worker thread leak:** add a `CancellationToken` per provider worker; source disposal cancels and joins.
6. **Fix query `open_file()` atomicity:** hold the write lock for the entire operation.
7. **Add depth guard to parser:** propagate a `depth: u32` argument through recursive descent; emit `RecursionLimitExceeded` at 256.
8. **Add `decode_step()` depth limit:** configurable, default 512.
9. **Add recursion depth limit to `source_decode.rs`:** same mechanism.

### Next structural cleanups (next 2–3 months)

1. **Split HIR lowering from import resolution:** `lower_structure()` produces a structurally complete HIR with all `ResolutionState::Unresolved` bindings. A separate `resolve_imports()` pass fills them in.
2. **Move `populate_signal_metadata()` to its own elaboration pass** after HIR validation.
3. **Split type checking from elaboration:** `typecheck_module()` is read-only and returns `TypeCheckReport`. A separate `apply_defaults(module, report)` applies mutations.
4. **Unify the structural walker:** extract `StructuralWalker<Frame, Acc>` from `EqDeriver`, `DecodePlanner`, and lambda `capture_free_bindings`. All three become instances.
5. **Refactor `validate.rs`:** split into `validate_structure()`, `validate_bindings()`, `validate_types()`. Each is independently callable and returns a `ValidationReport`. Tie them together in `validate_module()`.
6. **Refactor arena overflow boilerplate:** `alloc_or_diag!(arena, value, "description", errors)` macro across aivi-core, aivi-lambda, aivi-backend.
7. **Create `Parameter` and `Capture` as distinct types** in aivi-lambda. Remove the `ItemParameter` overloading.
8. **Scope input handles per owner** in aivi-gtk/runtime_adapter.rs.
9. **Add input existence validation** to the signal graph in aivi-runtime before dispatching.

### Longer-term architectural improvements (6–12 months)

1. **Generics:** either fully implement generic functions (type parameter preservation through HIR, core, lambda, monomorphization or dictionary passing) or emit a `GenericFunctionsNotYetSupported` diagnostic at HIR lowering time. The current silent drop is not acceptable.
2. **Eq constraint solving:** implement a Hindley-Milner-style unification pass for the `ConstraintClass::Eq` constraints that are currently collected but never used.
3. **Recursive type support:** design `mu`-type representation in `TypeStore`; update kind inference, Eq derivation, decode planning, and lambda analysis to detect and handle mu-types. This touches 5+ files and should be planned as a single milestone.
4. **Incremental compilation:** the query database (aivi-query) is structurally ready. Wire aivi-hir, aivi-core, aivi-lambda, and aivi-backend into the query system so that only changed modules and their dependents are recompiled.
5. **ID ownership:** add phantom type parameters (`ExprId<'module>`) to arena IDs, or seal `Id` construction behind per-`Module` factory methods. This is a breaking API change but eliminates the entire cross-module ID confusion class.
6. **External widget schema:** move aivi-gtk/schema.rs to a JSON/YAML schema file loaded at runtime. Generate the Rust structs via a build script. Enables new widgets without recompilation.
7. **ABI and layout computation:** add explicit `size: u32`, `alignment: u8`, `stride: u32` fields to `Layout`. Compute them during backend lowering. Validate them against Cranelift's layout decisions before code emission.
8. **Incremental GC:** replace the stop-the-world `collect()` in `MovingRuntimeValueStore` with an incremental or generational strategy bounded by a configurable pause budget.
9. **Formal grammar document:** write an EBNF grammar for the AIVI surface language. Use it to drive a grammar-based fuzzer against the parser. This will catch edge cases that manual tests miss.

---

*End of review.*
