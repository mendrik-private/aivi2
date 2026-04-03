# Predicates & Selectors

Predicates are inline filter expressions used inside patch selectors and collection traversals. They select which elements to update without writing explicit loops or branches.

## Predicate syntax

A predicate appears inside square brackets and uses dot-prefixed field access:

```aivi
[.active == True]
```

The dot (`.`) refers to the current element being tested. The expression must evaluate to `Bool`.

## Predicates in patches

Combine predicates with the `<|` patch operator to update only matching elements:

```aivi
type User = {
    name: Text,
    role: Text,
    active: Bool
}

type { users: List User } -> { users: List User }
func promoteActive = .
 <| { users[.active == True].role: "admin" }
```

This updates the `role` field only for users where `.active` is `True`. Non-matching users are left unchanged.

## Predicate expressions

Predicates support the same comparison operators as regular expressions:

| Predicate | Meaning |
| --- | --- |
| `[.active == True]` | Field equals a value |
| `[.score >= 100]` | Numeric comparison |
| `[.name == "Ada"]` | Text equality |
| `[.role == "guest"]` | Match a specific field value |

The dot prefix accesses fields on each element:

```aivi
type Item = { name: Text, price: Int, inStock: Bool }

type { items: List Item } -> { items: List Item }
func discountExpensive = .
 <| { items[.price >= 50].price: halve }
```

## Selectors

Selectors are the path expressions inside patch braces that determine what to update. They chain left to right:

| Selector | Meaning |
| --- | --- |
| `field` | Select a record field |
| `.field` | Same as above (dot-prefixed form) |
| `a.b.c` | Nested field path |
| `[*]` | Traverse all `List` elements or `Map` values |
| `[predicate]` | Filter elements by predicate |
| `["key"]` | Select a `Map` entry by key |
| `[.key == "id"]` | Select `Map` entries matching a predicate |
| `Constructor` | Focus through a constructor with one payload |

Examples of chaining:

```aivi
// update all prices in a list
{ items[*].price: double }

// update prices of in-stock items only
{ items[.inStock == True].price: double }

// nested record field
{ profile.address.city: toUpperCase }

// focus through an Option
{ config.Some.retries: increment }
```

## Constructor focus

For single-payload constructors like `Some`, `Ok`, `Err`, `Valid`, and `Invalid`, the selector can focus through the constructor:

```aivi
type Config = {
    retries: Option Int,
    name: Text
}

value bumpRetries : (Config -> Config) =
    patch { retries.Some: increment }
```

If the value does not match the constructor (e.g. it is `None`), the patch leaves it unchanged.

## Store syntax

Use `:=` to store a function value as data instead of applying it:

```aivi
type Counter = {
    step: Int -> Int
}

value setStep : (Counter -> Counter) =
    patch { step: := increment }
```

Without `:=`, the function would be called during patch application. With `:=`, the function itself becomes the new field value.

## Removal syntax

Use `: -` to remove a field from a record:

```aivi
value cleaned = record <| { tempField: - }
```

The result type reflects the removal — it has one fewer field than the input. See [Record Patterns § Patch removal](/guide/record-patterns#patch-removal) for details.
