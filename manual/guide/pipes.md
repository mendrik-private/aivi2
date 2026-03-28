# Pipes & Operators

Pipes are the primary way to compose data transformations in AIVI. Instead of deeply nested function calls, you write a linear left-to-right flow.

## The Basic Pipe `|>`

`|>` applies a function to the value on its left:

```aivi
fun double: Int n: Int => n * 2
fun addOne: Int n: Int => n + 1

value result = 5 |> double |> addOne   -- 11
```

This is equivalent to `addOne (double 5)`, but reads in the order of execution.

In multi-line form, align the pipes for readability:

```aivi
value result =
    5
     |> double
     |> addOne
```

### Passing Extra Arguments

When the function you're piping into takes more than one argument, supply the extra arguments immediately after the function name. The piped value is used as the **last** argument:

```aivi
fun multiply: Int factor: Int n: Int => factor * n

value result =
    5
     |> multiply 3   -- multiply 3 5 = 15
```

## Map `*|>`

`*|>` applies a function inside a container. It works on `Option`, `Result`, `Signal`, and any `Functor`:

```aivi
fun doubleOpt: Option Int opt: Option Int =>
    opt
     *|> double
```

If `opt` is `None`, the result is `None`. If it is `Some 5`, the result is `Some 10`.

On signals, `*|>` maps over each emitted value:

```aivi
signal doubled: Signal Int =
    counter
     *|> double
```

## Validate `!|>`

`!|>` applies a validation function. The function returns a `Result`, and any `Err` short-circuits the pipeline:

```aivi
type ValidationError = TooShort | TooLong

fun validateLength: Result ValidationError Text s: Text =>
    s
     ||> _ if length s < 3  -> Err TooShort
     ||> _ if length s > 50 -> Err TooLong
     ||> _                   -> Ok s

signal validName: Signal (Result ValidationError Text) =
    nameInput
     !|> validateLength
```

## Guard `?|>`

`?|>` keeps a value only if a predicate holds, wrapping it in `Option`:

```aivi
type User = {
    active: Bool,
    age: Int,
    name: Text
}

fun activeAdults: Option User user: User =>
    user
     ?|> (.active and .age >= 18)
```

The `.` shorthand accesses fields on the tested value.

## Combine `&|>`

`&|>` combines multiple signals into a tuple. The combined signal emits a new value whenever any input changes:

```aivi
signal firstName: Signal Text = ...
signal lastName: Signal Text  = ...

signal fullName: Signal Text =
 &|> firstName
 &|> lastName
  |> (\first last => "{first} {last}")
```

Notice the leading `&|>` — when combining signals at the start of a pipeline you write the operator before the first signal too.

After combining, `|>` receives all the combined values as arguments to the next function. Here a lambda `\first last => ...` is used, but a named function works just as well.

### Combining Into a Type

A common pattern is to collect several validated signals into a record or constructor:

```aivi
type UserDraft = UserDraft Text Text Int

signal nameText: Signal Text  = ...
signal emailText: Signal Text = ...
signal ageValue: Signal Int   = ...

signal draft: Signal UserDraft =
 &|> nameText
 &|> emailText
 &|> ageValue
  |> UserDraft
```

## Accumulate `+|>`

`+|>` folds incoming values into state. It replaces mutable variables.

```aivi
fun addToTotal: Int total: Int n: Int =>
    total + n

signal runningTotal: Signal Int =
    numbers
     +|> 0 addToTotal
```

The first argument is the **seed** (initial state). The function receives the current state and the new value, and returns the next state.

### Shorthand Form

For simple arithmetic accumulation, use the shorthand with `prev` (the previous state) and `.` (the current value):

```aivi
signal total: Signal Int =
    numbers
     +|> prev + .
```

`prev` refers to the previous accumulated value, and `.` is the current incoming value.

## Diff `-|>`

`-|>` emits the difference between the current and previous value:

```aivi
signal delta: Signal Int =
    score
     -|>
```

For numeric types, this is subtraction. For custom types, the domain defines what "diff" means.

## Previous `~|>`

`~|>` pairs each new value with the previous one, yielding a tuple `(previous, current)`:

```aivi
signal transition: Signal (Status, Status) =
    status
     ~|> Idle
```

The argument is the **initial previous value** used before any real previous value is available.

## Boolean Branches `T|>` and `F|>`

Split a boolean value into two branches:

```aivi
fun label: Text active: Bool =>
    active
     T|> "active"
     F|> "inactive"
```

These can be used inline in a pipeline:

```aivi
signal displayText: Signal Text =
    isActive
     T|> "On"
     F|> "Off"
```

## Source Boundary `@|>`

`@|>` marks a pipeline step as a source boundary, meaning the step performs an effect (like an HTTP request) and is scheduled by the runtime source system. This is typically used internally by `@source`-declared signals.

## Operator Precedence

Pipes are **left-associative** and have lower precedence than arithmetic and function application. From highest to lowest:

1. Field access `.`
2. Function application
3. Arithmetic operators (`*`, `/`, `+`, `-`)
4. Comparison operators (`==`, `!=`, `<`, `>`, `<=`, `>=`)
5. Boolean operators (`and`, `or`)
6. Pipe operators (all `|>` variants)

Within the pipe operators themselves, they all have the same precedence and associate left-to-right:

```aivi
-- This:
a &|> b |> f

-- Is the same as:
(a &|> b) |> f
```

## Combining Operators

Operators compose naturally in a pipeline:

```aivi
type User = {
    active: Bool,
    age: Int,
    email: Text
}

fun formatUser: Text user: User =>
    "({user.age}) {user.email}"

signal activeUserText: Signal (Option Text) =
    userSignal
     ?|> (.active and .age > 18)
     *|> formatUser
```

This reads: "take `userSignal`, keep only active adults, then format each one."

## Summary

| Operator | Name | Description |
|---|---|---|
| `\|>` | Apply | `value \|> f` → `f value` |
| `*\|>` | Map | Apply inside a container or signal |
| `!\|>` | Validate | Apply a `Result`-returning function |
| `?\|>` | Guard | Keep value only if predicate holds |
| `\|\|>` | Case-split | Pattern match |
| `&\|>` | Combine | Merge multiple signals |
| `+\|>` | Accumulate | Fold values into state |
| `-\|>` | Diff | Difference from previous value |
| `~\|>` | Previous | Pair with previous value |
| `T\|>` | True branch | Result when boolean is `True` |
| `F\|>` | False branch | Result when boolean is `False` |
