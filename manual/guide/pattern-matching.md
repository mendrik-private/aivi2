# Pattern Matching

AIVI has no `if`/`else` or `switch` statements. Branching is done through pattern matching using the `||>` pipe operator. It is exhaustive — the compiler requires you to handle every case.

## The Case-Split Operator `||>`

The `||>` operator matches a value against a series of patterns. Each arm uses `->` to separate the pattern from its result:

```aivi
fun describeNumber: Text n: Int =>
    n
     ||> 0 -> "zero"
     ||> 1 -> "one"
     ||> _ -> "many"
```

The `_` wildcard matches anything. Arms are tried top-to-bottom; the first matching arm wins.

## Matching Union Types

```aivi
type Direction = Up | Down | Left | Right

fun directionLabel: Text dir: Direction =>
    dir
     ||> Up    -> "up"
     ||> Down  -> "down"
     ||> Left  -> "left"
     ||> Right -> "right"
```

Each constructor must be covered. If you forget one, the compiler reports an error.

### Constructors With Data

When a constructor carries a value, bind it to a name in the pattern:

```aivi
type LoadState =
  | NotAsked
  | Loaded User
  | Failed Text

fun describe: Text state: LoadState =>
    state
     ||> NotAsked        -> "waiting"
     ||> Loaded user     -> "loaded: {user.name}"
     ||> Failed message  -> "error: {message}"
```

## The Wildcard `_`

`_` matches any value without binding it:

```aivi
fun isRunning: Bool status: Status =>
    status
     ||> Running  -> True
     ||> _        -> False
```

## Guards

Add a condition after the pattern using `if`:

```aivi
fun classify: Text n: Int =>
    n
     ||> _ if n > 0  -> "positive"
     ||> _ if n < 0  -> "negative"
     ||> _           -> "zero"
```

Guards can use any boolean expression. A wildcard with a guard acts like `else if`.

## Record Patterns

Destructure a record by listing the fields you want:

```aivi
type Profile = {
    name: Text,
    score: Int
}

fun summary: Text profile: Profile =>
    profile
     ||> { name, score } -> "{name} scored {score}"
```

You can mix pattern-bound fields with literal checks:

```aivi
fun isTopScore: Bool profile: Profile =>
    profile
     ||> { score } if score >= 100 -> True
     ||> _                         -> False
```

## Tuple Patterns

Match on tuples by wrapping the pattern in parentheses:

```aivi
fun movePixel: Pixel head: Pixel direction: Direction =>
    (head, direction)
     ||> (Pixel px py, Up)    -> Pixel px (py - 1)
     ||> (Pixel px py, Down)  -> Pixel px (py + 1)
     ||> (Pixel px py, Left)  -> Pixel (px - 1) py
     ||> (Pixel px py, Right) -> Pixel (px + 1) py
```

This simultaneously destructures both the `Pixel` constructor and the `Direction` constructor.

## Nested Patterns

Patterns can be nested arbitrarily:

```aivi
type Inner = A | B
type Outer = Outer Inner

fun describe: Text outer: Outer =>
    outer
     ||> Outer A -> "outer A"
     ||> Outer B -> "outer B"
```

## The Option Type

`Option` is a union type, so pattern matching handles it naturally:

```aivi
fun displayName: Text opt: Option Text =>
    opt
     ||> Some name -> name
     ||> None      -> "anonymous"
```

## The Result Type

```aivi
fun handleResult: Text result: Result Text Int =>
    result
     ||> Ok value    -> "got {value}"
     ||> Err message -> "failed: {message}"
```

## Boolean Branches: `T|>` and `F|>`

For simple true/false decisions, `T|>` and `F|>` are more readable than a full `||>` match on `Bool`:

```aivi
fun label: Text active: Bool =>
    active
     T|> "active"
     F|> "inactive"
```

This is equivalent to:

```aivi
fun label: Text active: Bool =>
    active
     ||> True  -> "active"
     ||> False -> "inactive"
```

`T|>` gives the value for `True`, `F|>` for `False`.

## Guard-Only Filtering: `?|>`

The `?|>` operator passes a value through only when a predicate holds. It returns `Option A`:

```aivi
type User = {
    active: Bool,
    age: Int,
    email: Text
}

fun activeAdult: Option User user: User =>
    user
     ?|> (.active and .age > 18)
```

The `.` shorthand accesses fields on the value being tested. This is useful for filtering signals:

```aivi
signal adultUsers: Signal (Option User) =
    userSignal
     ?|> (.active and .age > 18)
```

## Exhaustiveness

Pattern matches must cover every possible case. The compiler will reject incomplete matches:

```aivi
-- This will NOT compile if Direction has four constructors:
fun label: Text dir: Direction =>
    dir
     ||> Up   -> "up"
     ||> Down -> "down"
     -- Left and Right are missing!
```

Use a wildcard `_` to cover remaining cases when you don't need to distinguish them:

```aivi
fun isVertical: Bool dir: Direction =>
    dir
     ||> Up   -> True
     ||> Down -> True
     ||> _    -> False
```

## Summary

| Pattern | What it matches |
|---|---|
| `Constructor` | Exact constructor |
| `Constructor name` | Constructor and binds its payload to `name` |
| `Constructor (Nested p)` | Nested constructor pattern |
| `{ field, other }` | Record with those fields bound |
| `(a, b)` | Tuple, binding both elements |
| `_` | Anything (no binding) |
| `name` | Anything, bound to `name` |
| Guard: `pattern if cond` | Pattern plus boolean condition |

| Operator | Purpose |
|---|---|
| `||>` | Case-split / pattern match |
| `T|>` | True branch of a boolean |
| `F|>` | False branch of a boolean |
| `?|>` | Guard — filter to `Option` |
