# Classes

Classes are AIVI's typeclass-style abstraction mechanism. A class describes a set of operations that a type must provide.
For the current higher-kinded hierarchy, builtin executable support matrix, and user-authored instance limits, see [Typeclasses & Higher-Kinded Support](/guide/typeclasses).

## Declaring a class

```aivi
class Eq A
```

This says that any type used with `Eq` must support equality.

You can declare ordinary named methods too:

```aivi
class Display A
```

## Block body syntax

When a class has multiple members, group them inside `= { ... }`:

```aivi
class Eq A = {
    (==) : A -> A -> Bool
}

class Display A = {
    display : A -> Text
    label : A -> Text
}
```

Instance declarations use the same block form:

```aivi
instance Eq Blob = {
    (==) left right = blobEquals left right
}
```

## Superclass declarations

Use `with` inside the class body to declare that your class extends another class.
Any instance of the derived class must also provide an instance of each superclass.

```aivi
class Named A

class Displayed A

class Logged A = {
    with Named A
    with Displayed A
}
```

Multiple superclasses are listed as separate `with` lines.

```aivi
class CacheKey A = {
    with Eq A
    with Hashable A
}
```

## Parameter constraints

Use `require` inside the class body to constrain a type parameter. This documents that any type substituted for that parameter must satisfy the given class.

```aivi
class Container A = {
    require Eq A
}
```

## Using class-backed operators

When a type already has an instance, you can use the operator directly:

```aivi
type Int -> Int -> Bool
func equivalent = left right =>
    left == right and left != 0

value sameNumber = equivalent 4 4
```
## Declaring an instance

Instances provide the implementation for a concrete type:

```aivi
class Eq A

type Blob = Blob Bytes

type Blob -> Blob -> Bool
func blobEquals = left right =>
    True

instance Eq Blob
```

## Named class methods

A class can expose named operations instead of operators:

```aivi
class Compare A

type Label = Label Text

instance Compare Label
```

## Eq constraints on functions

When a function needs to compare values of an open type parameter, use a constraint prefix on the annotation:

```aivi
type Eq K => K -> K -> Bool
func matchesKey = key candidate =>
    key == candidate
```

Multiple constraints use a parenthesized comma-separated list:

```aivi
type (Eq A, Eq B) => A -> A -> B -> B -> Bool
func bothEqual = leftA rightA leftB rightB =>
    leftA == rightA and leftB == rightB
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
| `type Eq K => K -> K -> Bool` | Require `K` to have `Eq` in a function annotation |
| `class Name A = { ... }` | Group class members in a block |

---

**See also:** [Typeclasses & Higher-Kinded Support](typeclasses.md) — HKT hierarchy, built-in support matrix, and user-authored instance limits
