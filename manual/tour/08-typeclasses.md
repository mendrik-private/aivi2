# Type Classes

Type classes describe shared capabilities that multiple types can implement.
They let generic code say what operations it needs, and they let the compiler
resolve the right implementation at typecheck time.

They are similar to interfaces, but they also carry the usual algebraic laws.
Those laws still matter, but the compiler is not required to prove them for you.

## Declaring a class

```aivi
class Eq A
    (==) : A -> A -> Bool
    (!=) : A -> A -> Bool
```

This declares a class `Eq` parameterised over a type variable `A`.
Any type that implements `Eq` must provide those members.

## Writing an instance

```aivi
instance Eq Color
```

For closed sum and product types whose fields already support structural
equality, the compiler can derive `Eq` automatically from the instance header.

## Exported type-class surface

The root `aivi` prelude now exports these type-class names directly:

- `Eq`
- `Default`
- `Ord`
- `Semigroup`
- `Monoid`
- `Functor`
- `Bifunctor`
- `Traversable`
- `Filterable`
- `Applicative`
- `Monad`
- `Foldable`

It also exports the ordering type used by `Ord`:

```aivi
type Ordering = Less | Equal | Greater
```

## Built-in classes and current compiler-backed instances

### `Eq` — structural equality

```aivi
class Eq A
    (==) : A -> A -> Bool
    (!=) : A -> A -> Bool
```

`Eq` is the structural equality class. The compiler provides equality for the
standard built-in shapes and can derive it for your own closed types when their
fields also support equality.

### `Default` — a fallback value

```aivi
class Default A
    default : A
```

`Default` is currently used most visibly by record-field elision.

Today, resolved-HIR default synthesis supports:

- same-module `Default` instances
- `Option A` as `None` when the instance is imported via `use aivi.defaults (Option)`

### `Ord` — total ordering

```aivi
class Eq A => Ord A
    compare : A -> A -> Ordering
```

`Ord` refines `Eq` with `compare`.
The current compiler-backed `compare` lowering supports:

- `Int`
- `Float`
- `Decimal`
- `BigInt`
- `Bool`
- `Text`
- `Ordering`

The result is one of `Less`, `Equal`, or `Greater`.

### `Semigroup` — associative combination

```aivi
class Semigroup A
    append : A -> A -> A
```

`append` combines two values of the same type.
The current compiler-backed builtin instances are:

- `Text`
- `List A`

### `Monoid` — combination with an identity

```aivi
class Semigroup A => Monoid A
    empty : A
```

`Monoid` adds an identity element on top of `Semigroup`.
The current compiler-backed builtin instances are:

- `Text`
- `List A`

### `Functor` — mapping over one-parameter contexts

```aivi
class Functor F
    map : (A -> B) -> F A -> F B
```

`Functor` describes contexts that can transform their payload while preserving
their outer shape.

The current compiler-backed `map` lowering supports:

- `List`
- `Option`
- `Result E`
- `Validation E`
- `Signal`

### `Bifunctor` — mapping both sides of a two-parameter context

```aivi
class Bifunctor F
    bimap : (A -> C) -> (B -> D) -> F A B -> F C D
```

`Bifunctor` lets you transform both type arguments of a two-parameter carrier.

The current compiler-backed `bimap` lowering supports:

- `Result`
- `Validation`

### `Applicative` — introducing and applying contextual values

The ambient hierarchy models applicative behavior in two steps:

```aivi
class Apply F
    apply : F (A -> B) -> F A -> F B

class Apply F => Applicative F
    pure : A -> F A
```

`Applicative` is the exported class name, and its ambient superclass `Apply`
supplies `apply`.

The current compiler-backed `pure` and `apply` lowering supports:

- `List`
- `Option`
- `Result E`
- `Validation E`
- `Signal`

For `Validation`, applicative accumulation currently expects `Invalid` payloads
shaped like `NonEmpty`.

### `Foldable` — reducing a structure to a value

```aivi
class Foldable F
    reduce : (B -> A -> B) -> B -> F A -> B
```

The current surface member is `reduce`, not `fold`.

The current compiler-backed `reduce` lowering supports:

- `List`
- `Option`
- `Result E`
- `Validation E`

### `Traversable` — traversing with an applicative effect

```aivi
class (Functor T, Foldable T) => Traversable T
    traverse : Applicative G => (A -> G B) -> T A -> G (T B)
```

`Traversable` sequences an applicative effect while rebuilding the original
shape.

The current compiler-backed `traverse` lowering supports traversing:

- `List`
- `Option`
- `Result`
- `Validation`

and rebuilding into these applicative results:

- `List`
- `Option`
- `Result`
- `Validation`
- `Signal`

### `Filterable` — map and discard in one pass

```aivi
class Functor F => Filterable F
    filterMap : (A -> Option B) -> F A -> F B
```

`filterMap` keeps values that map to `Some` and discards values that map to
`None`.

The current compiler-backed `filterMap` lowering supports:

- `List`
- `Option`

`Result` and `Validation` are intentionally not treated as builtin
`Filterable` carriers under this signature.

### `Monad` — sequencing dependent effects

The ambient hierarchy models monadic sequencing like this:

```aivi
class Apply M => Chain M
    chain : (A -> M B) -> M A -> M B

class (Applicative M, Chain M) => Monad M
    join : M (M A) -> M A
```

`Monad` remains part of the exported prelude surface.
In today's implementation, the most complete direct builtin lowering in this
area is centered on `map`, `apply`, and `pure`, while `Monad` still serves as
the higher-level abstraction you can constrain against.

## Using a class constraint in a function

When a function is generic but requires a class capability, declare it with
`with`:

```aivi
fun equals:Bool with Eq A #left:A #right:A =>
    left == right
```

Multiple constraints are separated by commas:

```aivi
fun resetThenAppend:A with Monoid A, Semigroup A #value:A =>
    append empty value
```

## Higher-kinded classes

`Functor`, `Filterable`, `Foldable`, `Traversable`, `Applicative`, and `Monad`
range over type constructors rather than ordinary values.

```aivi
class Functor F
    map : (A -> B) -> F A -> F B
```

Here `F` stands for a one-argument constructor such as `List`, `Option`,
`Result E`, `Validation E`, or `Signal`.

`Bifunctor` is similar, but its carrier takes two type arguments:

```aivi
class Bifunctor F
    bimap : (A -> C) -> (B -> D) -> F A B -> F C D
```

For the built-in surface today, that means `Result` and `Validation`.

## Why type classes instead of duck typing?

Type classes make generic requirements explicit:

1. A function states the capability it needs, like `with Ord A`.
2. The compiler checks that an instance exists for the type at that call site.
3. Dispatch stays static and type-directed instead of becoming a runtime guess.

## Summary

- `class Name T` declares a capability.
- `instance Name Type` provides an implementation.
- The public prelude now surfaces `Ord`, `Semigroup`, `Monoid`, `Bifunctor`, `Traversable`, and `Filterable` alongside the earlier core classes.
- `Ordering` is exported as `Less | Equal | Greater`.
- `Foldable` currently exposes `reduce`.
- Higher-kinded abstractions are part of the surface, but builtin lowering is still narrower than the full ambient hierarchy.

[Next: Domains →](/tour/09-domains)
