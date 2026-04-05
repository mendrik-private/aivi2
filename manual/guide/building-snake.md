# Building Snake

This tutorial walks through a complete Snake game written in AIVI. It is a real program — you can find it in `demos/snake.aivi` — and it uses every major language feature. We will build it up piece by piece, explaining each concept as it appears.

By the end, you will understand how types, functions, signals, sources, domains, pattern matching, and markup work together in a real application.

## The game at a glance

Our snake game has:

- A 30×20 grid where a snake moves in one of four directions
- Food that appears randomly on the grid
- A score that increases when the snake eats food
- Game over when the snake hits a wall or itself
- A keyboard-driven interface rendered in a GTK window

The entire game is about 230 lines of AIVI. There are no mutable variables, no loops, and no callbacks.

## Imports

The game imports several standard library modules:

```aivi
use aivi.nonEmpty (
    NonEmptyList
    singleton
    head as nelHead
    cons as nelCons
    length as nelLength
    toList as nelToList
    init as nelInit
    fromHeadTail as nelFromHeadTail
)

use aivi.list (
    contains as listContains
    any
)

use aivi.text (
    join
    concat
)

use aivi.duration (Duration)

use aivi.matrix (
    Matrix
    MatrixError
    indices as matrixIndices
)
```

The `as` keyword renames imports to avoid ambiguity — `head` from `aivi.nonEmpty` becomes `nelHead` so it does not collide with other modules.

## Modeling the world with types

The first thing to do in AIVI is define what things exist. We start with types:

```aivi
type Direction =
  | North
  | South
  | East
  | West

type Status = Running | GameOver

type Cell = Cell Int Int
```

`Direction` is a sum type with four constructors. `Status` has two. `Cell` wraps two integers — an x and y position on the grid.

These types are **closed**. You cannot add a fifth direction later without updating every function that matches on `Direction`. The compiler ensures exhaustiveness.

## Events that drive the game

The game responds to three kinds of events:

```aivi
type Event =
  | Tick
  | Turn Direction
  | Restart

type Key = Key Text
```

`Tick` advances the snake one step. `Turn` changes direction. `Restart` resets the game. Notice that `Turn` carries a `Direction` payload — the constructor holds data. `Key` wraps a text string representing a keyboard key name.

## Pure functions for game logic

Every piece of game logic is a pure function. Let us start with direction:

```aivi
type Direction -> Direction
func opposite = .
 ||> North -> South
 ||> South -> North
 ||> East  -> West
 ||> West  -> East
```

This function takes a direction and returns its opposite. It uses `||>` for exhaustive pattern matching — every constructor is handled.

### Moving a cell

```aivi
type Direction -> Cell -> Cell
func moveDir = d c => (d, c)
 ||> (North, Cell x y) -> Cell x (y - 1)
 ||> (South, Cell x y) -> Cell x (y + 1)
 ||> (East, Cell x y)  -> Cell (x + 1) y
 ||> (West, Cell x y)  -> Cell (x - 1) y
```

This matches on a **tuple** of direction and cell. Each arm destructures the `Cell` to extract `x` and `y`, then constructs a new `Cell` with the adjusted position.

### Boundary checking

```aivi
value boardW = 30
value boardH = 20

type Cell -> Bool
func outside = .
 ||> Cell x y -> x < 0 or x >= boardW or y < 0 or y >= boardH
```

### Pseudo-random food placement

```aivi
value seed0 = 2463534242

type Int -> Int
func nextSeed = s =>
    (s * 1103515245 + 12345) % 2147483647

type Int -> Cell
func spawnFood = s =>
    Cell (s % boardW) (nextSeed s % boardH)
```

The game uses a simple linear congruential generator for pseudo-random numbers. `spawnFood` converts a seed into a grid coordinate.

### Cell equality

```aivi
type Cell -> Cell -> Bool
func cellEq = a b =>
    a == b
```

This wraps the built-in equality operator into a named function, which we pass to higher-order list functions like `listContains`.

## Domains: the snake itself

The snake is a non-empty list of cells, but we want richer operations than a plain list provides. This is where **domains** come in:

```aivi
domain Snake over NonEmptyList Cell = {
    type NonEmptyList Cell -> Snake
    fromCells cells = cells
    type Cell
    head = nelHead self
    type Cell -> Bool
    contains cell = listContains cellEq cell (nelToList self)
    type Int
    length = nelLength self
    type Cell -> Snake
    grow cell = nelCons cell self
    type Cell -> Snake
    move cell = nelFromHeadTail cell (nelInit self)
    type Direction -> Cell
    nextHead dir = moveDir dir (nelHead self)
}
```

A domain wraps a carrier type (`NonEmptyList Cell`) with a semantic name (`Snake`) and domain-specific operations. Inside the body, `self` refers to the domain-typed receiver, so `nelHead self` unwraps a `Snake` as the underlying `NonEmptyList Cell`. You call these operations with dot notation: `st.snake.head`, `st.snake.contains h`, `st.snake.grow h`.

Using `NonEmptyList` rather than `List` as the carrier type guarantees the snake always has at least one cell — making `head` total (no need for a fallback value).

The key insight is that outside the domain, you cannot accidentally treat a `Snake` as a raw `NonEmptyList Cell`. The domain boundary prevents mixing up snake-specific logic with general list operations. When you *do* need the underlying value — for example, to pass it to a generic list function — every domain has a built-in `.carrier` accessor that returns the carrier value at zero cost: `st.snake.carrier` yields the `NonEmptyList Cell`.

## Game state as a record

With types and the snake domain in place, we can define the full game state:

```aivi
type GameState = {
    snake: Snake,
    dir: Direction,
    food: Cell,
    score: Int,
    seed: Int,
    status: Status
}
```

And an initial state:

```aivi
value initial : GameState = {
    snake: fromCells (nelCons (Cell 6 10) (nelCons (Cell 5 10) (singleton (Cell 4 10)))),
    dir: East,
    food: spawnFood seed0,
    score: 0,
    seed: seed0,
    status: Running
}
```

The initial snake is built by consing cells onto a singleton non-empty list. The initial food position comes from `spawnFood seed0`.

## The step function: events → state changes

The heart of the game is a single pure function that takes an event and a state, and returns the next state:

```aivi
type Event -> GameState -> GameState
func step = ev st => ev
 ||> Restart -> initial
 ||> Turn d  -> handleTurn d st
 ||> Tick    -> handleTick st
```

Each event is routed to a handler. Let us trace through `handleTick`:

```aivi
type GameState -> GameState
func handleTick = st => st.status
 ||> GameOver -> st
 ||> Running  -> advance st (st.snake.nextHead st.dir)
```

If the game is over, return the state unchanged. If running, compute the next head position and advance. The pattern match on `st.status` replaces what would be an `if` statement in other languages.

### Handling turns

```aivi
type Direction -> GameState -> GameState
func handleTurn = d st => st.status
 ||> GameOver -> st
 ||> Running  -> applyTurn d st

type Direction -> GameState -> GameState
func applyTurn = d st => d == opposite st.dir
 T|> st
 F|> st <| { dir: d }
```

Turns are ignored when the game is over. When running, a turn in the opposite direction is also ignored (you cannot reverse into yourself). Otherwise, the direction is updated with `<|`.

### Advancing the snake

```aivi
type GameState -> Cell -> GameState
func advance = st h => outside h or (st.snake.contains h)
 T|> st <| { status: GameOver }
 F|> resolveMove st h
```

If the new head is outside the board or collides with the snake body, the game is over. Otherwise, resolve the move. The `T|>` / `F|>` pipes branch on a boolean expression.

The `<|` operator applies a structural patch: it copies every field of `st` and replaces only `status`. No mutation occurs — a new `GameState` value is returned with all other fields unchanged.

### Resolving a move

```aivi
type GameState -> Cell -> GameState
func resolveMove = st h => h == st.food
 T|> st <| { snake: st.snake.grow h, food: spawnFood (nextSeed st.seed), score: st.score + 1, seed: nextSeed st.seed }
 F|> st <| { snake: st.snake.move h, seed: nextSeed st.seed }
```

If the head lands on food, grow the snake, spawn new food, and increment the score. Otherwise, move the snake and advance the seed. In both cases `<|` copies the unchanged fields — only the named fields are overridden.

## Sources: connecting to the real world

Two sources drive the game — a timer and the keyboard:

```aivi
use aivi.duration (Duration)

@source timer.every 120ms with {
    immediate: False,
    coalesce: True
}
signal tick : Signal Unit

@source window.keyDown with {
    repeat: False,
    focusOnly: True
}
signal keyDown : Signal Key
```

The timer fires every 120 milliseconds. `120ms` is a **domain suffix literal** — the `ms` suffix comes from the standard library's `Duration` domain, which converts the integer to a `Duration` at compile time.

The keyboard source captures key presses without repeat, so holding a key does not flood the game with events.

## Merging sources into events

Signal merge syntax lists the source signals separated by `|`, then `||>` arms discriminate by
source name and payload pattern:

```aivi
signal event : Signal Event = tick | keyDown
  ||> tick _ => Tick
  ||> keyDown (Key "ArrowLeft") => Turn West
  ||> keyDown (Key "ArrowRight") => Turn East
  ||> keyDown (Key "ArrowUp") => Turn North
  ||> keyDown (Key "ArrowDown") => Turn South
  ||> keyDown (Key "Space") => Restart
```

Each arm names a source signal from the merge list, matches a pattern on its payload, and produces
the event value. When `keyDown` receives `Key "ArrowLeft"`, the event signal gets `Turn West`.
This is declarative routing, not imperative event handling.

## Accumulating state with `+|>`

```aivi
signal state : GameState = event
 +|> initial step
```

This is the accumulation pipe. It reads: *"start with `initial`, and each time `event` fires, apply `step` to compute the next `GameState`."*

The entire game state lives in this one signal. All other signals derive from it:

```aivi
signal boardText = state
  |> renderBoard

signal dirLine = state
  |> .dir
  |> dirLabel

signal statusLine = state
  |> .status
  |> statusLineFor

signal scoreLine = state
  |> .score
  |> scoreLineFor

signal gameOver = state
  |> .status
  |> isGameOver

signal finalScoreLine = state
  |> .score
  |> finalScoreLineFor
```

Each derived signal projects a piece of state and transforms it. When `state` changes, all of these recompute automatically.

## Rendering with text and indices

The board renders as text. Instead of nested loops, we use `matrixIndices` to generate coordinate sequences and `map` to transform them into glyphs:

```aivi
type List Cell -> GameState -> Int -> Int -> Text
func cellGlyph = body st y x => (Cell x y == st.snake.head, listContains cellEq (Cell x y) body, Cell x y == st.food)
 ||> (True, _, _)          -> "@"
 ||> (_, True, _)          -> "o"
 ||> (_, _, True)          -> "*"
 ||> (False, False, False) -> "·"
```

This matches on a **triple of booleans** — is this cell the head, a body segment, or food? Every combination is covered. The `body` parameter is a pre-computed `List Cell` extracted from the snake via `.carrier` (see below), so the conversion from `NonEmptyList` happens once per frame rather than once per cell.

Each row is rendered by mapping `cellGlyph body st y` over the column indices, then the rows are joined with newlines:

```aivi
type List Cell -> GameState -> Int -> Text
func renderRowAt = body st y => matrixIndices boardW
  |> map (cellGlyph body st y)
  |> concat

type GameState -> Text
func renderBoard = st => matrixIndices boardH
  |> map (renderRowAt (nelToList st.snake.carrier) st)
  |> join "\n"
```

`matrixIndices boardW` produces `[0, 1, 2, ..., 29]`. The pipe maps each index through `cellGlyph body st y` to produce a list of single-character strings, then `concat` joins them without a separator. The outer pipe does the same for rows, joining with newlines.

The expression `st.snake.carrier` uses the built-in `.carrier` accessor to get the `NonEmptyList Cell` from the `Snake` domain, then `nelToList` converts it to a `List Cell`. This list is computed once in `renderBoard` and threaded through `renderRowAt` and `cellGlyph`, avoiding repeated conversions across all 600 cells.

### Display helper functions

Several small functions format the status display using text interpolation:

```aivi
type Direction -> Text
func dirLabel = .
 ||> North -> "Up"
 ||> South -> "Down"
 ||> East  -> "Right"
 ||> West  -> "Left"

type Status -> Text
func statusLineFor = .
 ||> Running  -> "Running"
 ||> GameOver -> "Game Over"

type Int -> Text
func scoreLineFor = "Score: {.}"

type Int -> Text
func finalScoreLineFor = "Final score: {.}"

type Status -> Bool
func isGameOver = .
 ||> Running  -> False
 ||> GameOver -> True
```

The `{.}` syntax is text interpolation — the `.` refers to the current pipe subject (the function parameter).

## The UI

Finally, the markup:

```aivi
value main =
    <Window title="AIVI Snake">
        <Box orientation="vertical" spacing={8}>
            <Label text={dirLine} />
            <Label text={scoreLine} />
            <Label text={statusLine} />
            <Label text={boardText} monospace />
            <show when={gameOver}>
                <Label text={finalScoreLine} />
            </show>
        </Box>
    </Window>

export main
```

Each `<Label>` binds its `text` attribute to a signal. When the signal updates, the label updates. `<show when={gameOver}>` conditionally renders the final score.

## The complete data flow

```
Timer (120ms)  ──→  tick signal
                         ↓
Keyboard  ──→  keyDown signal
                    ↓
          signal merge routes to event signal
                    ↓
          +|> accumulates into state signal
                    ↓
          ├── renderBoard  → boardText   → <Label>
          ├── .dir         → dirLine     → <Label>
          ├── .status      → statusLine  → <Label>
          ├── .score       → scoreLine   → <Label>
          └── isGameOver   → gameOver    → <show>
```

Every arrow is a declared dependency. The runtime propagates changes through the graph automatically.

## What this game teaches

| Concept | How the game uses it |
| --- | --- |
| **Closed types** | `Direction`, `Status`, `Event` — every case must be handled |
| **Pattern matching** | Every function branches with `\|\|>`, `T\|>`, `F\|>` |
| **Pure functions** | `step`, `advance`, `resolveMove` — no mutation, no side effects |
| **Patch** | `<\|` copies a record updating only named fields |
| **Domains** | `Snake` wraps `NonEmptyList Cell` with domain-specific operations |
| **Domain literals** | `120ms` — type-safe duration with suffix syntax |
| **Signals** | `state`, `boardText`, `scoreLine` — the reactive graph |
| **Sources** | `timer.every`, `window.keyDown` — external input boundaries |
| **Event routing** | Signal merge connects sources to events |
| **Accumulation** | `+\|>` folds events into state over time |
| **Text interpolation** | `"Score: {.}"` — inline formatting with pipe subject |
| **Imports** | `use aivi.nonEmpty (...)` — named imports with `as` renaming |
| **Markup** | `<Window>`, `<Label>`, `<show>` — type-checked GTK UI |

The game has zero mutable variables, zero loops, and zero callbacks. The entire architecture is a declared dependency graph with pure functions at every node.

## Next steps

- [Pipes & Operators](/guide/pipes) — the full pipe algebra reference
- [Signals](/guide/signals) — signals in depth
- [Domains](/guide/domains) — creating your own domains
- [Markup & UI](/guide/markup) — the complete widget system
