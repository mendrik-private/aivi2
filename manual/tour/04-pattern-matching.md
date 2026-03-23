# Pattern Matching

Pattern matching is AIVI's mechanism for inspecting the **shape** of a value and branching
based on what you find. It is more powerful than a `switch` statement because it matches
structure, not just equality.

## The match pipe \|\|>

`\|\|>` is the case pipe. It takes a value on the left and a pattern `=>` body arm on the right.
Multiple arms are written as successive `\|\|>` lines:

```text
-- declare a function 'directionText' mapping a Direction to a Text label
-- Up maps to "up"
-- Down maps to "down"
-- Left maps to "left"
-- Right maps to "right"
```

Each arm is: `\|\|> pattern => expression`.
The value (`direction`) is matched top-to-bottom against each pattern.
The body of the first matching arm is evaluated and returned.

## Matching on constructors

The most common use is matching on sum type variants:

```text
-- declare a sum type 'Status' with variants Running, Paused, GameOver
-- declare a function 'statusLabel' mapping a Status to a Text label
-- Running maps to "In progress"
-- Paused maps to "Paused"
-- GameOver maps to "Game over"
```

## Exhaustiveness

Pattern matching in AIVI is **exhaustive**: the compiler rejects any match that does not
cover all variants. If you add a new variant to a sum type, every match on that type becomes
a compile error until you handle the new case.

This is the key advantage over `switch` statements: you cannot accidentally forget a case.

```text
-- declare a sum type 'Color' with variants Red, Green, Blue
-- declare a function 'colorName' matching on Color
-- Red maps to "red"
-- Green maps to "green"
-- the Blue case is missing — this would be a compile error
```

## Wildcard patterns

When you want a catch-all, use `_`:

```text
-- declare a function 'growLength' matching on specific integer values
-- 1 maps to 2, 2 maps to 3, 3 maps to 4, 4 maps to 5, 5 maps to 6
-- any other value maps to 6 via the wildcard catch-all
```

`_` matches anything and does not bind the value.

## Matching on literal values

You can match on integer and text literals directly:

```text
-- declare a function 'fizzBuzz' taking an integer n
-- if n is divisible by 15, return "FizzBuzz"
-- else if n is divisible by 3, return "Fizz"
-- else if n is divisible by 5, return "Buzz"
-- otherwise return n converted to text
```

## Destructuring product types (records)

You can destructure a record in a pattern arm, binding its fields to names:

```text
-- declare a function 'describePoint' matching on a Vec2 value
-- destructure the Vec2 into its x and y components
-- format them as "(x, y)"
```

Record patterns work similarly:

```text
-- declare a function 'scoreOf' matching on a Game record
-- destructure the record to extract the 'score' field and return it
```

Here `{ score }` matches any `Game` record and binds the `score` field.

## Matching on data-carrying constructors

When a variant carries data, the pattern binds the inner values:

```text
-- Option A is a sum type: Some (carrying A) or None
-- declare a generic function 'unwrapOr' taking a default value and an Option A
-- if the option is Some, return the wrapped value
-- if the option is None, return the default
```

`Some value` binds the wrapped `A` to the name `value` in the body.

## Nested patterns

Patterns can be nested. In the snake game, the step logic matches on a record extracted
from a record:

```text
-- declare a function 'runningStep' taking boardSize, direction, and current game state
-- destructure the current game to extract snake, food, and score fields simultaneously
-- pass them to movedGame along with size and direction to produce the next game state
```

The record pattern `{ snake, food, score }` binds three fields of `Game` simultaneously,
without needing intermediate `let` bindings.

## \|\|> vs T\|>/F\|>

Use `\|\|>` when matching on a general sum type or literal. Use `T\|>` / `F\|>` when the
value is already a `Bool` and you want a two-branch conditional:

```text
-- when branching on a Bool: use the truthy/falsy pipe — if true use valueIfTrue, else valueIfFalse
-- when matching a sum type with two or more variants: match Some carrying x to call useIt on it, and None to use a fallback
```

## Summary

- `\|\|>` is the match pipe. Each arm is `\|\|> pattern => body`.
- Matching is exhaustive — every variant must be covered.
- `_` is the wildcard that matches anything.
- Patterns can destructure records and data-carrying constructors.
- Patterns can be nested arbitrarily.

[Next: Signals →](/tour/05-signals)
