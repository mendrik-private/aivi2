# Outstanding implementation gaps

- LSP cursor-offset migration is still incomplete after `aivi_base::SourceFile::lsp_position_to_offset` began returning `Option<ByteIndex>`.
  - Evidence: `crates/aivi-lsp/src/completion.rs`, `crates/aivi-lsp/src/definition.rs`, and `crates/aivi-lsp/src/hover.rs` all still carry `TODO(aivi-base)` comments for that update.

- Runtime source lowering is still partial for some provider shapes.
  - Evidence: `crates/aivi-runtime/src/hir_adapter.rs` still emits `source owner {owner} uses a provider the runtime adapter cannot lower yet: {provider:?}` for unsupported providers.

- Eq constraints are collected but still do not go through a dedicated solver pass.
  - Evidence: `crates/aivi-hir/src/typecheck.rs` explicitly warns that collected `Eq` constraints can otherwise remain unsolved.

- Formatting still has a live `todo!()` for standalone case pipe stages.
  - Evidence: `crates/aivi-syntax/src/format.rs` still contains `todo!("format PipeStageKind::Case as a standalone stage line")`.

- Nested gate/fanout semantics and gate-predicate Bool validation are still incomplete.
  - Evidence: `crates/aivi-typing/src/gate.rs`, `crates/aivi-typing/src/fanout.rs`, `crates/aivi-hir/src/gate_elaboration.rs`, and `crates/aivi-backend/src/validate.rs` all carry explicit TODOs for these invariants.

- Core lowering still lacks a completeness check for partially elaborated items.
  - Evidence: `crates/aivi-core/src/lower.rs` warns that lowering can otherwise continue with incomplete elaboration output.

- Mailbox subscriptions and runtime maps still carry known runtime-structure gaps.
  - Evidence: `crates/aivi-runtime/src/providers.rs` documents unbounded subscriber growth in `MailboxHub`, and `crates/aivi-backend/src/runtime.rs` still defers the ordered-map representation behind a TODO.

- Higher-kinded user-authored class and instance support is still only partial end to end.
  - Evidence: ambient prelude declarations exist, but public `aivi check` still rejects examples like `instance Applicative Option` and class members shaped like `F Int`.

- Value-level duplicate record fields and duplicate-set literal handling are still missing from the checker/runtime path.
  - Evidence: duplicate record fields currently pass `aivi check`, and set literals accept duplicates without warning or canonicalization.

- Record-default evidence is still narrower than the RFC originally claimed.
  - Evidence: `use aivi.defaults (Option)` is a compiler-recognized special path, while other `Default` support is limited to same-module instances rather than general imported evidence.

- Ambient `Monad` / `Chain` declarations still do not have end-to-end builtin lowering support.
  - Evidence: the executable carrier list includes `Functor`, `Apply`, `Applicative`, `Foldable`, `Bifunctor`, `Traversable`, and `Filterable`, but not `Monad` / `Chain`.

- `+|>` accumulation is still only partially wired while legacy `scan` remains the working checked surface.
  - Evidence: parser/HIR support exists, but checked fixtures and runtime tests still use `|> scan seed step`, and user-authored `+|>` programs fail in practice.

- `&|>` applicative clusters are still HIR/typechecker-only for many executable paths.
  - Evidence: valid cluster surfaces type-check, but typed-core lowering still rejects general executable use sites.

- Built-in D-Bus source providers and some RFC source-option claims are still missing/outdated.
  - Evidence: no `dbus.*` providers exist in the built-in catalog, `timer.jitterMs` is stale in favor of `jitter : Duration`, and `fs.read.activeWhen` is not a current option.
