# Classes

A **class** in AIVI is a typeclass — a contract that a type can implement. It defines one or more operations that must be provided for any type that satisfies the class.

## Declaring a Class

```aivi
class Eq A
    (==) : A -> A -> Bool
```

This declares a class `Eq` with a type parameter `A`. Any type that is an instance of `Eq` must provide an `==` operator that compares two values of that type and returns a `Bool`.

## Using Class Constraints

Functions can require that a type parameter satisfies a class:

```aivi
fun equivalent: Bool left: Int right: Int =>
    left == right and left != 0
```

When you use `==` on a concrete type like `Int`, the compiler checks that `Int` is an instance of `Eq`.

## Standard Classes

AIVI's standard library defines several foundational classes:

### `Eq A`

Equality comparison:

```aivi
class Eq A
    (==) : A -> A -> Bool
```

Used everywhere that values need to be compared. `!=` is derived from `==`.

### `Ord A`

Ordered comparison (requires `Eq`):

```aivi
class Ord A
    compare: A -> A -> Ordering
```

Where `Ordering` is:

```aivi
type Ordering = Less | Equal | Greater
```

### `Default A`

A type with a sensible default value:

```aivi
class Default A
    default: A
```

### `Functor F`

A container that can be mapped over:

```aivi
class Functor F
    map: (A -> B) -> F A -> F B
```

This is what enables `*|>` (map pipe) to work on `Option`, `Result`, and `Signal`.

### `Semigroup A`

Types that can be combined:

```aivi
class Semigroup A
    (<>) : A -> A -> A
```

### `Monoid A`

A `Semigroup` with an identity element:

```aivi
class Monoid A
    empty: A
```

For `List`, `empty` is `[]` and `<>` is list concatenation.

### `Foldable F`

A container that can be folded:

```aivi
class Foldable F
    reduce: (B -> A -> B) -> B -> F A -> B
```

This is what enables `reduce` on lists.

### `Traversable F`

A container that can be traversed with effects:

```aivi
class Traversable F
```

### `Filterable F`

A container that supports filtering:

```aivi
class Filterable F
    filter: (A -> Bool) -> F A -> F A
```

### `Applicative F`

An enhanced `Functor` that supports combining:

```aivi
class Applicative F
    pure: A -> F A
    ap: F (A -> B) -> F A -> F B
```

### `Monad F`

An `Applicative` that supports sequential composition:

```aivi
class Monad F
    bind: F A -> (A -> F B) -> F B
```

### `Bifunctor F`

A container with two type parameters that can both be mapped:

```aivi
class Bifunctor F
    bimap: (A -> C) -> (B -> D) -> F A B -> F C D
```

`Result` is a `Bifunctor` — you can map over the error and success paths independently.

## Class Hierarchy

Classes can build on each other. `Monoid` requires `Semigroup`, and `Monad` requires `Applicative`, which requires `Functor`. When you implement `Monad` for a type, you automatically get all the `Functor` and `Applicative` operations too.

## Summary

| Class | Key operation | Meaning |
|---|---|---|
| `Eq` | `==` | Equality |
| `Ord` | `compare` | Ordering |
| `Default` | `default` | Default value |
| `Functor` | `map` | Transform inside a container |
| `Semigroup` | `<>` | Combine |
| `Monoid` | `empty` | Identity for combine |
| `Foldable` | `reduce` | Collapse to a single value |
| `Filterable` | `filter` | Keep matching elements |
| `Applicative` | `pure`, `ap` | Combine containers |
| `Monad` | `bind` | Sequential composition |
| `Bifunctor` | `bimap` | Map both sides |
| `Traversable` | — | Traverse with effects |
