# Snake: A Deeper Example

This is the **optional deep example** in the manual. If you are new to AIVI, start with
[Tutorials](/tutorials/) and build the task tracker first. Come here when you want to see the same
ideas stretched into a larger, more feature-dense program.

Snake is still a real program — you can find it in `demos/snake.aivi` — and it uses every major
language feature. We will build it up piece by piece and use it to show how types, functions,
signals, sources, domains, pattern matching, and markup work together in a larger application.

The live demo now renders the board with sprite assets from `demos/assets/`. This guide keeps the
smaller text-oriented snippets in a few places so the core game logic stays easier to read.

## The game at a glance

Our snake game has:

- A 30×20 grid where a snake moves in one of four directions
- Food that appears randomly on the grid
- A score that increases when the snake eats food
- Game over when the snake hits a wall or itself
- A keyboard-driven interface rendered in a GTK window

The entire game is about 230 lines of AIVI. There are no mutable variables, no loops, and no callbacks.

## Standard library

All standard library functions are available in every AIVI file without any
`use` statement — the stdlib modules self-hoist their exports project-wide.
`map`, `filter`, `indices`, `Duration`, and the rest are ready to use directly.
Text joining is one intentional exception: we import `aivi.text.join` locally so
bare `join` stays free for the generic `Monad.join` name.

Two small `use` blocks keep the names clean:

```aivi
use aivi.text (join as textJoin)

use aivi.nonEmpty (
    head as nelHead
    length as nelLength
    init as nelInit
)
```

The text alias makes rendering-specific string joining explicit. The `NonEmptyList`
aliases handle the other sharp edge: the `Snake` domain defines its own `head`
and `length` operations, so inside the domain body we still want unambiguous
references to the underlying helpers.

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
func opposite = arg1 => arg1
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
func outside = arg1 => arg1
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
func sameCell = target candidate =>
    candidate == target
```

`Cell` is a closed constructor type, so the helper body is just `==`. We use that typed predicate with
`any` when a list search wants an explicit boolean test.

## Domains: the snake itself

The snake is a non-empty list of cells, but we want richer operations than a plain list provides. This is where **domains** come in:

```aivi
domain Snake over NonEmptyList Cell = {
    type fromCells : NonEmptyList Cell -> Snake
    fromCells = cells => cells
    type head : Cell
    head = nelHead self
    type contains : Cell -> Bool
    contains = cell => any (sameCell cell) (toList self)
    type length : Int
    length = nelLength self
    type grow : Cell -> Snake
    grow = cell => cons cell self
    type move : Cell -> Snake
    move = cell => fromHeadTail cell (nelInit self)
    type nextHead : Direction -> Cell
    nextHead = dir => moveDir dir (nelHead self)
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
    snake: fromCells (cons (Cell 6 10) (cons (Cell 5 10) (singleton (Cell 4 10)))),
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

The entire game state still lives in this one signal so each tick remains atomic, but we can fan out stable sub-signals from it so unrelated UI work does not recompute:

```aivi
from state = {
    snake: .snake
    dir: .dir
    food: .food
    score: .score
    status: .status
}

signal boardText : Signal Text =
 &|> snake
 &|> food
  |> renderBoard

from dir = {
    dirLine: dirLabel
}

from status = {
    statusLine: statusLineFor
    gameOver: isGameOver
}

from score = {
    scoreLine: scoreLineFor
    finalScoreLine: finalScoreLineFor
}
```

This keeps the reducer where the rules need coherence, but moves rendering and labels onto a finer-grained signal graph. In particular, `boardText` now depends on `snake` and `food`, not on the whole `GameState`, so a pure direction change no longer invalidates the board render path.

## Rendering with text and indices

The board renders as text. Instead of nested loops, we use `indices` to generate coordinate sequences and `map` to transform them into glyphs:

```aivi
type List Cell -> Cell -> Cell -> Int -> Int -> Text
func cellGlyph = body head food y x => (Cell x y == head, any (sameCell (Cell x y)) body, Cell x y == food)
 ||> (True, _, _)          -> "@"
 ||> (_, True, _)          -> "o"
 ||> (_, _, True)          -> "*"
 ||> (False, False, False) -> "·"
```

This matches on a **triple of booleans** — is this cell the head, a body segment, or food? Every combination is covered. The renderer takes only `snake` and `food`, so direction-only turns do not force a board redraw. The `body` parameter is a pre-computed `List Cell` extracted from the snake once per frame.

Each row is rendered by mapping `cellGlyph body head food y` over the column indices, then the rows are joined with newlines:

```aivi
type List Cell -> Cell -> Cell -> Int -> Text
func renderRowAt = body head food y => indices boardW
  |> map (cellGlyph body head food y)
  |> textJoin ""

type Snake -> Cell -> Text
func renderBoard = snake food => indices boardH
  |> map (renderRowAt snake.cells snake.head food)
  |> textJoin "\n"
```

`indices boardW` produces `[0, 1, 2, ..., 29]`. The pipe maps each index through `cellGlyph body head food y` to produce a list of single-character strings, then `textJoin ""` joins them without a separator. The outer pipe does the same for rows with `textJoin "\n"`.

The expression `snake.cells` gives us the whole body list once in `renderBoard`, and `snake.head` gives us the head cell once. Those values are threaded through `renderRowAt` and `cellGlyph`, avoiding repeated conversions or unrelated `GameState` reads across all 600 cells.

### Display helper functions

Several small functions format the status display using text interpolation:

```aivi
type Direction -> Text
func dirLabel = arg1 => arg1
 ||> North -> "Up"
 ||> South -> "Down"
 ||> East  -> "Right"
 ||> West  -> "Left"

type Status -> Text
func statusLineFor = arg1 => arg1
 ||> Running  -> "Running"
 ||> GameOver -> "Game Over"

type Int -> Text
func scoreLineFor = arg1 =>
    "Score: {arg1}"

type Int -> Text
func finalScoreLineFor = arg1 =>
    "Final score: {arg1}"

type Status -> Bool
func isGameOver = arg1 => arg1
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
          ├── .snake ─┐
          ├── .food  ─┴──→ renderBoard  → boardText   → <Label>
          ├── .dir         → dirLine     → <Label>
          ├── .status      → statusLine  → <Label>
          ├── .score       → scoreLine   → <Label>
          └── .status      → gameOver    → <show>
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
| **Signals** | `state`, `snake`, `food`, `boardText`, `scoreLine` — the reactive graph |
| **Sources** | `timer.every`, `window.keyDown` — external input boundaries |
| **Event routing** | Signal merge connects sources to events |
| **Accumulation** | `+\|>` folds events into state over time |
| **Text interpolation** | `"Score: {.}"` — inline formatting with pipe subject |
| **Standard library** | All stdlib functions available project-wide — no `use` needed; `as` aliases resolve domain body name conflicts |
| **Markup** | `<Window>`, `<Label>`, `<show>` — type-checked GTK UI |

The game has zero mutable variables, zero loops, and zero callbacks. The entire architecture is a declared dependency graph with pure functions at every node.

## Next steps

- [Pipes & Operators](/guide/pipes) — the full pipe algebra reference
- [Signals](/guide/signals) — signals in depth
- [Domains](/guide/domains) — creating your own domains
- [Markup & UI](/guide/markup) — the complete widget system
