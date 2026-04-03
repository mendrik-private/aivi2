# Thinking in AIVI

If you come from JavaScript, Python, Rust, or any imperative language, AIVI will feel unfamiliar at first. There are no loops, no `if`/`else` blocks, no mutable variables, and no code blocks with sequential statements.

This page shows you how to solve the same problems using AIVI's tools: **expressions**, **pattern matching**, **pipes**, and **collection combinators**.

## There are no variables, only values

In most languages, you declare a variable and change it later:

```js
// JavaScript
let score = 0;
score = score + 10;
```

In AIVI, every binding is permanent. If you want a different value, you create a new one:

```aivi
value score = 0
value updatedScore = score + 10
```

This might seem limiting, but it means you can always trust what a name refers to. There is no moment where `score` means one thing and later means another.

When you need values that change over time — a counter, a game state, user input — you use **signals**, which are covered below.

## There is no if/else — use pattern matching

In most languages:

```js
// JavaScript
function classify(score) {
  if (score >= 90) return "excellent";
  else if (score >= 50) return "pass";
  else return "fail";
}
```

In AIVI, branching is done through the values themselves. For boolean conditions, use `T|>` and `F|>`:

```aivi
type Int -> Text
func classify = score => score >= 50
  T|> "pass"
  F|> "fail"
```

For richer choices, use pattern matching with `||>`:

```aivi
type Direction =
  | North
  | South
  | East
  | West

type Direction -> Text
func label = dir => dir
  ||> North -> "up"
  ||> South -> "down"
  ||> East  -> "right"
  ||> West  -> "left"
```

The compiler checks that you have handled every case. If you add a fifth direction, every function that matches on `Direction` will need updating — and the compiler will tell you where.

### Chaining conditions

When you need multiple conditions, compute each one and combine them:

```aivi
type Int -> Bool -> Text
func describe = score active => (score > 50, active)
  ||> (True, True)   -> "active high scorer"
  ||> (True, False)  -> "inactive high scorer"
  ||> (False, True)  -> "active low scorer"
  ||> (False, False) -> "inactive low scorer"
```

Or use a pipe chain that builds up from simpler checks:

```aivi
type Int -> Text
func tier = score => score >= 90
  T|> "gold"
  F|> score >= 50
    T|> "silver"
    F|> "bronze"
```

## There are no loops — use collection combinators

In most languages:

```js
// JavaScript
const numbers = [1, 2, 3, 4, 5];
const doubled = numbers.map(n => n * 2);
const evens = numbers.filter(n => n % 2 === 0);
const total = numbers.reduce((sum, n) => sum + n, 0);
```

AIVI uses the same ideas, but as pipes:

```aivi
type Int -> Int
func double = n => n * 2

type Int -> Bool
func isEven = n => n % 2 == 0

value numbers = [1, 2, 3, 4, 5]

value doubled = numbers
  |> map double

value evens = numbers
  |> filter isEven

value total = numbers
  |> reduce add 0
```

Notice that each transformation is a named function. This makes the code self-documenting and each piece independently testable.

### Common patterns

| Instead of... | Use... |
| --- | --- |
| `for` loop that transforms each item | `map` with a function |
| `for` loop that keeps some items | `filter` with a predicate |
| `for` loop that builds up one result | `reduce` with a step function and seed |
| `for` loop that finds one item | `find` with a predicate |
| `for` loop that checks a condition | `any` or `all` with a predicate |
| Nested loops | `flatMap` or `map` inside `map` |

## There are no code blocks — just expressions

In most languages, a function body is a sequence of statements:

```js
// JavaScript
function process(user) {
  const name = user.name.trim();
  const greeting = `Hello, ${name}!`;
  return greeting;
}
```

In AIVI, a function body is a single expression. If you need intermediate steps, use a pipe:

```aivi
type User -> Text
func process = user => user.name
  |> trim
  |> "Hello, {.}!"
```

Or break the work into named helpers:

```aivi
type Text -> Text
func greet = name =>
    "Hello, {name}!"

type User -> Text
func process = user =>
    greet (trim user.name)
```

Both approaches are valid. The pipe style reads top-to-bottom; the helper style gives each step a reusable name.

## State that changes over time → signals

In imperative code, you model changing state with mutable variables and event handlers:

```js
// JavaScript
let count = 0;
button.addEventListener('click', () => {
  count += 1;
  label.textContent = `Count: ${count}`;
});
```

In AIVI, changing state lives in **signals**. A signal is a value in a dependency graph. When its inputs change, it recomputes:

```aivi
signal count = 0

signal label = count
  |> "Count: {.}"
```

The connection between `count` and `label` is **declared, not wired up manually**. The runtime handles the updates. You never write "when X changes, update Y" — you write "Y is derived from X."

### Accumulating state with `+|>`

When a signal needs to fold over a stream of events, use the accumulation pipe:

```aivi
type Event =
  | Increment
  | Decrement
  | Reset

type Event -> Int -> Int
func step = event count => event
  ||> Increment -> count + 1
  ||> Decrement -> count - 1
  ||> Reset     -> 0

signal events : Signal Event
signal count = events
  +|> 0 step
```

This declares: *"count starts at 0, and each time an event arrives, apply `step` to get the next value."* The state is managed by the signal system, not by a mutable variable.

## Talking to the outside world → sources

Pure functions cannot read files, make HTTP requests, or listen for keyboard input. In AIVI, all external input enters through **sources**:

```aivi
@source timer.every 1000ms
signal tick : Signal Unit

@source window.keyDown
signal keys : Signal Key
```

A source is a declared entry point. It tells the runtime: *"this signal gets its values from the outside world."* Everything downstream of a source is still pure computation.

Think of it this way:

```
Outside world  →  Source  →  Signal  →  Pure derivations  →  UI
   (messy)        (typed     (reactive     (deterministic)    (GTK
                  boundary)   graph)                          widgets)
```

## Reading AIVI code: a mental checklist

When you encounter AIVI code, ask yourself:

1. **What are the types?** Look at the `type` line above each `func`. It tells you exactly what goes in and what comes out.
2. **What is the subject?** In a pipe, `.` refers to the current value being transformed. `.field` projects a field from it.
3. **Is this a value or a signal?** `value` is computed once. `signal` participates in the reactive graph.
4. **Where does external data come from?** Look for `@source` annotations. Those are the boundaries.
5. **What pattern does the match cover?** The compiler guarantees exhaustiveness. If you see `||>`, every case is handled.

## Next steps

Now that you have the mental model:

- [Your First App](/guide/your-first-app) — put these ideas into practice
- [Values & Functions](/guide/values-and-functions) — the full reference for declarations
- [Pipes & Operators](/guide/pipes) — the complete pipe algebra
- [Signals](/guide/signals) — the reactive system in depth
