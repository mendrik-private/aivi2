# Classes

Classes are AIVI's typeclass-style abstraction mechanism. A class describes a set of operations that a type must provide.

## Declaring a class

```aivi
class Eq A
    (==):A -> A -> Bool
```

This says that any type used with `Eq` must support equality.

You can declare ordinary named methods too:

```aivi
class Display A
    display:A -> Text
```

## Superclass declarations

Use `with` inside the class body to declare that your class extends another class.
Any instance of the derived class must also provide an instance of each superclass.

```
class Functor F
    map: (A -> B) -> F A -> F B

class Applicative F
    pure: A -> F A
    ap: F (A -> B) -> F A -> F B

class Monad M
    bind: M A -> (A -> M B) -> M B
```
Multiple superclasses are listed as separate `with` lines:

```aivi
class Traversable F
    traverse: Applicative G
```

## Parameter constraints

Use `require` inside the class body to constrain a type parameter. This documents that any type substituted for that parameter must satisfy the given class.

```
class Container F
    contains: A -> F A -> Bool
```

## Using class-backed operators

When a type already has an instance, you can use the operator directly:

```aivi
fun equivalent:Bool left:Int right:Int =>
    left == right and left != 0

value sameNumber = equivalent 4 4
```
## Declaring an instance

Instances provide the implementation for a concrete type:

```aivi
class Eq A
    (==):A -> A -> Bool

data Blob =
  | Blob Bytes

fun blobEquals:Bool left:Blob right:Blob =>
    True

instance Eq Blob
    (==) left right = blobEquals left right
```

## Named class methods

A class can expose named operations instead of operators:

```aivi
class Compare A
    same:A -> A -> Bool

data Label =
  | Label Text

instance Compare Label
    same left right = left == right
```

## Eq constraints on functions

When a function needs to compare values of an open type parameter, use a constraint prefix on the annotation:

```
fun matchesKey: Eq K ->
    Bool k
```

Multiple constraints use a parenthesized comma-separated list:

```
fun bothEqual: (Eq A, Eq B) ->
    Bool a1
```

The constraint ensures the function can only be called when `K` (or `A`, `B`, etc.) has an `Eq` instance. Without the constraint, using `==` on an open type parameter is a type error.

## Why classes matter

Classes let generic code talk about capability instead of one hard-coded type. They are useful when you want a common interface for comparison, display, accumulation, or traversal.

## Summary

| Form | Meaning |
| --- | --- |
| `class Eq A` | Declare a class with a type parameter |
| `(==) : A -> A -> Bool` | Require an operator |
| `display : A -> Text` | Require a named method |
| `with Functor F` | Declare a superclass in the class body |
| `require Eq A` | Constrain a class type parameter |
| `instance Eq Blob` | Implement a class for one concrete type |
| `Eq K -> Bool` | Require `K` to have `Eq` in a function annotation |
