# Pipe algebra

Pipe expressions come in two shapes in the implementation:

1. ordinary single-subject pipe stages, stored as `ExprKind::Pipe(PipeExpr { head, stages, ... })`
2. applicative clusters, normalized into `ExprKind::Cluster(ApplicativeCluster)`

That split matters for pipe memos.

## Pipe memos `#name`

`#name` is the surface for remembering values inside a pipe without introducing a separate helper.

- `operator #name expr` binds the stage input for that stage body
- `operator expr #name` binds the stage result for later stages in the same pipe

The implementation now carries those memo bindings across the ordinary pipe-stage surface instead of
restricting them to plain transform/tap stages.

Covered ordinary stages:

- transform `|>` and tap `|`
- gate `?|>`
- case runs `||>`
- truthy/falsy pairs `T|>` / `F|>`
- fan-out `*|>` and join `<|*`
- validation `!|>`
- previous/diff `~|>` / `-|>`
- delay/burst `delay|>` / `burst|>`
- accumulation `+|>`
- recurrence `@|>` / `<|@`

## Grouped branch memo handling

Two surface forms are grouped before later passes reason about them:

- consecutive `||>` arms form one case run
- one adjacent `T|>` / `F|>` pair forms one truthy/falsy branch group

HIR lowering normalizes memos across those grouped stages so later name-resolution, typing, and
general-expression lowering see one canonical memo carrier for the group result. In practice that
lets the same `#resolved` name flow out of a branch group the same way it does for a one-stage
transform.

## Formatter and fixture coverage

The syntax layer now preserves memos in the formatter for:

- case groups
- `burst|>`
- `+|>`

The repo also has a dedicated valid fixture covering broad stage usage:

- `fixtures/frontend/milestone-2/valid/pipe-stage-memos/main.aivi`

and focused CLI coverage in:

- `crates/aivi-cli/tests/check.rs`
- `crates/aivi-cli/tests/compile.rs`

## Cluster boundary

`&|>` applicative clusters lower through `ApplicativeCluster` / `ApplicativeSpine`, not the
ordinary `PipeStage` path. Because of that, the current memo work applies to the ordinary
single-subject pipe flow, while cluster members/finalizers remain a separate applicative surface.

## Sources

- `crates/aivi-syntax/src/parse.rs`
- `crates/aivi-syntax/src/format.rs`
- `crates/aivi-hir/src/lower.rs`
- `crates/aivi-hir/src/typecheck_context.rs`
- `crates/aivi-hir/src/general_expr_elaboration.rs`
- `manual/guide/pipes.md`
- `syntax.md`
