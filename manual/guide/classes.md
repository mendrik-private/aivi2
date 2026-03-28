# Classes

Classes are AIVI's typeclass-style abstraction mechanism. A class describes a set of operations that a type must provide.

## Declaring a class

```aivi
class Eq A
    (==): A -> A -> Bool
```

This says that any type used with `Eq` must support equality.

You can declare ordinary named methods too:

```aivi
class Display A
    display: A -> Text
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
    (==): A -> A -> Bool

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
    same: A -> A -> Bool

data Label =
  | Label Text

instance Compare Label
    same left right = left == right
```

## Why classes matter

Classes let generic code talk about capability instead of one hard-coded type. They are useful when you want a common interface for comparison, display, accumulation, or traversal.

## Summary

| Form | Meaning |
| --- | --- |
| `class Eq A` | Declare a class with a type parameter |
| `(==) : A -> A -> Bool` | Require an operator |
| `display : A -> Text` | Require a named method |
| `instance Eq Blob` | Implement a class for one concrete type |
