# Type Classes

Type classes define a shared interface that multiple types can implement.
They are similar to TypeScript interfaces, but each instance comes with **laws** —
invariants the implementation must uphold, checked by the compiler.

## Declaring a class

```text
class Eq A
    (==) : A -> A -> Bool
```

This declares a class `Eq` parameterised over a type variable `A`.
Any type that implements `Eq` must provide `==`.

## Writing an instance

```text
instance Eq Color
```

For closed sum and product types whose fields all have `Eq` instances, the compiler
**derives** the implementation automatically. You declare the instance header; the
compiler fills in the body.

`Red == Green` now evaluates to `False` and `==` works anywhere `Color` appears.

## Built-in classes

AIVI's standard library exports these classes from `aivi` (available without import):

### `Eq` — equality

```text
class Eq A
    (==) : A -> A -> Bool
```

`Int`, `Bool`, `Text`, `List A`, `Option A`, and `Result E A` all have `Eq` instances
when their type arguments do. Your own closed types get `Eq` for free via derivation.

### `Default` — a sensible zero value

```text
class Default A
    default : A
```

Provides a canonical empty/zero value for a type. Useful when you need an initial
accumulator without an explicit seed.

### `Functor` — mapping over a container

```text
class Functor F
    map : (A -> B) -> F A -> F B
```

`Functor` describes types that can be mapped over. `List`, `Option`, `Result E`, and
`Signal` are all functors. In pipe notation, `*|>` is the `map` for `List`; `T|>` on
`Option` lifts the function over the `Some` case.

### `Applicative` — combining independent effects

```text
class Applicative F
    pure  : A -> F A
    apply : F (A -> B) -> F A -> F B
```

`Applicative` lets you combine multiple independent values in the same context.
`&|>` in pipe notation is `apply` for `Signal` — it zips two signals and applies a
function pointwise.

### `Monad` — sequencing dependent effects

```text
class Monad F
    flatMap : (A -> F B) -> F A -> F B
```

`Monad` sequences computations where the next step depends on the result of the previous
one. `Option`, `Result E`, and `Task E` are monads. `flatMap` on `Option` short-circuits
on `None`; on `Result` it short-circuits on `Err`.

### `Foldable` — reducing a structure to a value

```text
class Foldable F
    fold : (B -> A -> B) -> B -> F A -> B
```

`Foldable` describes structures that can be reduced to a single value. `List` is the
primary instance. The `reduce` function used throughout the stdlib is `fold` on `List`.

## Using a class constraint in a function

When a function is generic but requires a class capability, declare a constraint with
`with`:

```text
fun findDuplicate:(Option A) with Eq A #items:(List A) =>
    // TODO: add a verified AIVI example here
```

`with Eq A` says: "this function works for any `A`, but only if `A` has an `Eq`
instance." Multiple constraints are separated by commas:

```text
fun display:Text with Eq A, Display A #value:A =>
    // TODO: add a verified AIVI example here
```

## Higher-kinded types

`Functor`, `Applicative`, `Monad`, and `Foldable` are **higher-kinded** classes — their
type parameter is itself a type constructor (`F` expects one argument, e.g. `List`,
`Option`, `Signal`). AIVI supports this through a narrow HKT mechanism:

```text
class Functor F
    map : (A -> B) -> F A -> F B
```

Here `F` stands in for `List`, `Option`, `Result E`, `Signal`, or any other
single-argument type constructor. A function constrained by `Functor F` works uniformly
across all of them:

```text
fun lift:F B with Functor F #transform:(A -> B) #container:(F A) =>
    // TODO: add a verified AIVI example here
```

You do not need to interact with HKT mechanics directly for most application code —
the stdlib functions (`map`, `flatMap`, `fold`) handle it. HKT constraints appear
when writing generic library functions that must work over multiple container types.

## Why type classes instead of duck typing?

In a dynamically typed language, you call `.toString()` and hope for the best.
Type classes make the contract explicit:

1. The function declares exactly which capabilities it needs (`with Functor F`).
2. The compiler checks that the type you pass has the required instance.
3. The instance enforces the laws structurally.

No runtime surprises, no `undefined is not a function`.

## Summary

- `class Name T` declares an interface with required methods.
- `instance Name Type` provides a concrete implementation; the compiler derives it for closed types.
- Core classes: `Eq`, `Default`, `Functor`, `Applicative`, `Monad`, `Foldable`.
- `with ClassName T` on a function declares a constraint.
- `Functor` / `Applicative` / `Monad` / `Foldable` are higher-kinded — their parameter is a type constructor.

[Next: Domains →](/tour/09-domains)
