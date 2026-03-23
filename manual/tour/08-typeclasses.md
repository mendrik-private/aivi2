# Type Classes

Type classes let you define a shared interface for multiple types.
They are similar to TypeScript interfaces, but with one important difference:
type class instances come with **laws** — invariants the implementation must uphold.

## Declaring a class

```text
-- declare a type class 'Eq' for any type A
-- requires an equality operator returning Bool
-- requires an inequality operator returning Bool
```

This declares a class `Eq` parameterized over a type `A`.
Any type that implements `Eq` must provide `==` and `!=`.

## Writing an instance

```text
-- declare a sum type 'Color' with variants Red, Green, Blue
-- declare that Color implements the Eq class (compiler derives equality from the type structure)
```

The compiler derives structural equality for closed product and sum types whose fields all
have `Eq` instances. You declare the instance header; the compiler fills in the implementation.

Now `Red == Green` evaluates to `False`, and you can use `==` anywhere `Color` is expected.

## Built-in classes

AIVI ships three fundamental classes:

### Eq — equality

```text
-- define the Eq class: requires equality and inequality operators for type A
```

Most built-in types (`Int`, `Bool`, `Text`, `List A`) are instances of `Eq`.
Your product and sum types get `Eq` for free if all their fields have `Eq` instances.

### Show — text representation

```text
-- declare a type class 'Show' for any type A
-- requires a 'show' function that converts A to a human-readable Text
```

`show` converts a value to a human-readable `Text`.
The snake game uses this pattern with hand-written text functions rather than the class,
but `Show` is the standard interface:

```text
-- declare that Direction implements the Show class (compiler derives text representation from constructor names)
```

The instance body is filled in by the compiler based on the constructor names.

### Ord — ordering

```text
-- declare a type class 'Ord' for any type A (requires Eq as a superclass)
-- requires a 'compare' function returning an Ordering value
-- Ordering is a sum type with variants LT (less than), EQ (equal), GT (greater than)
```

`Ord` requires `Eq` as a superclass constraint. Any type with a meaningful ordering can
implement `Ord`, enabling use with sorting and comparison functions.

## Using a class constraint in a function

When a function is generic but requires a class capability, you express this with a constraint:

```text
-- declare a generic function 'matches' comparing two values of type A, requiring Eq for A
-- declare a generic function 'contains' checking whether a list includes a given item, requiring Eq for A
-- filter the list to items matching the target, count them, and return True if the count is greater than zero
```

The `with Eq A` syntax says: "this function works for any type `A`, but only if `A` has
an `Eq` instance."

## Why type classes instead of duck typing?

In a dynamically typed language, you can call `.toString()` on anything and hope for the best.
Type classes make the contract explicit:

1. The function declares exactly which capabilities it needs (`with Show A`).
2. The compiler checks that the type you pass has the required instance.
3. The instance documents and enforces the laws.

This means no runtime surprises, no "undefined is not a function", and no accidental
implicit coercions.

## Summary

- `class Name T` declares an interface with required methods.
- `instance Name Type` provides a concrete implementation.
- Built-in classes: `Eq` (equality), `Show` (display), `Ord` (ordering).
- Functions use `with ClassName T` to declare constraints.
- The compiler derives structural instances for closed types whose fields already have instances.
- Type classes make contracts explicit and compiler-checked.

That completes the Language Tour. Next: [The AIVI Way →](/aivi-way/)
