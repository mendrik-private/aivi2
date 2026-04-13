# Indexed collections and companion bodies

Sources: `stdlib/aivi/list.aivi`, `stdlib/aivi/option.aivi`, `stdlib/aivi/matrix.aivi`, `stdlib/aivi/prelude.aivi`, `manual/guide/typeclasses.md`, `manual/guide/types.md`, `manual/stdlib/list.md`, `manual/stdlib/option.md`, `manual/stdlib/matrix.md`, `manual/stdlib/prelude.md`, `syntax.md`, `AIVI_RFC.md`.

## Implemented today

### `aivi.list`

- `indexed : List A -> List (Int, A)`
- `mapWithIndex : (Int -> A -> B) -> List A -> List B`
- `reduceWithIndex : (B -> Int -> A -> B) -> B -> List A -> B`
- `filterMap : (A -> Option B) -> List A -> List B`

These are the practical stopgaps for indexed collection work until indexed HKTs exist.

### `aivi.option`

- `fold : B -> (A -> B) -> Option A -> B`
- `mapOr : B -> (A -> B) -> Option A -> B`
- `isSomeAnd : (A -> Bool) -> Option A -> Bool`

These close a small ergonomics gap around “unwrap or default” flows without adding new surface syntax.

### `aivi.matrix`

- `MatrixIndex` is now an explicit coordinate type with constructor `coord : Int -> Int -> MatrixIndex`
- indexed helpers: `mapWithIndex`, `reduceWithIndex`, `coords`, `entries`, `positionsWhere`, `count`, `modifyAt`, `replaceMany`
- `Matrix` now defines user-authored `Functor` and `Foldable` instances, so ambient `map` and `reduce` work when `aivi.matrix` is in scope

Important nuance: `replaceAt` still accepts the original tuple-shaped `(Int, Int)` input for compatibility, while the newer helper family uses `MatrixIndex`.

### `aivi.prelude`

The ambient prelude now re-exports:

- `mapOr`, `isSomeAnd`
- `indexed`, `mapWithIndex`, `reduceWithIndex`
- `count`, `findMap`

This keeps the “common path” concise while leaving matrix-specific helpers in `aivi.matrix`.

### ADT companion bodies

Closed sums now support colocated companion helpers in a brace body:

```aivi
type Player = {
    | Human
    | Computer

    type Player -> Player
    opponent = self => self
     ||> Human    -> Computer
     ||> Computer -> Human
}
```

Important rules:

- the brace form is treated as a companion sum only when the first significant token inside the body is `|`
- constructors come first; companion members follow
- companion members elaborate to ordinary top-level functions
- companion members keep ordinary export/import rules rather than piggybacking on the type export
- companion member `type` lines spell the full function type, including the receiver
- companion bodies use ordinary function forms such as `name = self => ...` or `name = . ...`

## Current higher-kinded reality

- Builtin carrier lowering is still the main executable slice for `List`, `Option`, `Result`, `Validation`, `Signal`, and `Task`
- Imported unary user-authored higher-kinded instances are executable for ordinary member lookup such as `map` and `reduce`
- The path works through authored executable evidence rather than adding a new builtin carrier
- This is enough for `Matrix` to participate in ambient `map` / `reduce` today

## Deferred proposals

### Indexed HKTs

The missing abstraction is not another container-specific API; it is executable evidence for indexed containers.

Proposed shape:

```aivi
class FunctorWithIndex F I
    mapWithIndex : (I -> A -> B) -> F A -> F B

class FoldableWithIndex F I
    reduceWithIndex : (B -> I -> A -> B) -> B -> F A -> B
```

Current blocker: the executable instance path is clearly unary today. Multi-parameter indexed heads are not yet proven end to end.
