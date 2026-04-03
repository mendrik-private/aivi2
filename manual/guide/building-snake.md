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

The entire game is about 260 lines of AIVI. There are no mutable variables, no loops, and no callbacks.

## Modeling the world with types

The first thing to do in AIVI is define what things exist. We start with types:

```aivi
type Direction =
  | North
  | South
  | East
  | West

type Status = Running | GameOver

type Cell =
  | Cell Int Int
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
```

`Tick` advances the snake one step. `Turn` changes direction. `Restart` resets the game. Notice that `Turn` carries a `Direction` payload — the constructor holds data.

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

## Domains: the snake itself

The snake is a list of cells, but we want richer operations than a plain list provides. This is where **domains** come in:

```aivi
domain Snake over List Cell = {
    type List Cell -> Snake
    fromCells cells = cells

    type Snake -> Cell
    head snake = getOrElse (Cell 0 0) (listHead snake)

    type Snake -> Cell -> Bool
    contains snake cell = any (cellEq cell) snake

    type Snake -> Int
    length snake = listLength snake

    type Snake -> Cell -> Snake
    grow snake cell = append [cell] snake

    type Snake -> Cell -> Snake
    move snake cell = takeCells (listLength snake) (append [cell] snake)

    type Snake -> Direction -> Cell
    nextHead snake dir = moveDir dir (getOrElse (Cell 0 0) (listHead snake))
}
```

A domain wraps a carrier type (`List Cell`) with a semantic name (`Snake`) and domain-specific operations. You call these operations with dot notation: `st.snake.head`, `st.snake.contains h`, `st.snake.grow h`.

The key insight is that outside the domain, you cannot accidentally treat a `Snake` as a raw `List Cell`. The domain boundary prevents mixing up snake-specific logic with general list operations.

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
    snake: fromCells [Cell 6 10, Cell 5 10, Cell 4 10],
    dir: East,
    food: spawnFood seed0,
    score: 0,
    seed: seed0,
    status: Running
}
```

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

### Advancing the snake

```aivi
type GameState -> Cell -> GameState
func advance = st h => outside h or (st.snake.contains h)
  T|> { snake: st.snake, dir: st.dir, food: st.food,
        score: st.score, seed: st.seed, status: GameOver }
  F|> resolveMove st h
```

If the new head is outside the board or collides with the snake body, the game is over. Otherwise, resolve the move. The `T|>` / `F|>` pipes branch on a boolean expression.

### Resolving a move

```aivi
type GameState -> Cell -> GameState
func resolveMove = st h => h == st.food
  T|> { snake: st.snake.grow h, dir: st.dir,
        food: spawnFood (nextSeed st.seed),
        score: st.score + 1, seed: nextSeed st.seed, status: Running }
  F|> { snake: st.snake.move h, dir: st.dir, food: st.food,
        score: st.score, seed: nextSeed st.seed, status: Running }
```

If the head lands on food, grow the snake and spawn new food. Otherwise, move the snake (grow at the head, drop the tail). Notice how each branch constructs a complete new `GameState` record — there is no mutation.

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

## Routing events with `when`

The `when` clause connects sources to the event signal:

```aivi
signal event : Signal Event

when tick _ => event <- Tick
when keyDown (Key "ArrowLeft") => event <- Turn West
when keyDown (Key "ArrowRight") => event <- Turn East
when keyDown (Key "ArrowUp") => event <- Turn North
when keyDown (Key "ArrowDown") => event <- Turn South
when keyDown (Key "Space") => event <- Restart
```

Each `when` watches a signal for a specific pattern. When `keyDown` receives `Key "ArrowLeft"`, the event signal gets `Turn West`. This is pattern-based routing, not imperative event handling.

## Accumulating state with `+|>`

```aivi
signal state : Signal GameState = event
  +|> initial step
```

This is the accumulation pipe. It reads: *"start with `initial`, and each time `event` fires, apply `step` to compute the next `GameState`."*

The entire game state lives in this one signal. All other signals derive from it:

```aivi
signal boardText = state |> renderBoard
signal dirLine = state |> .dir |> dirLabel
signal statusLine = state |> .status |> statusLineFor
signal scoreLine = state |> .score |> scoreLineFor
signal gameOver = state |> .status |> isGameOver
```

Each derived signal projects a piece of state and transforms it. When `state` changes, all of these recompute automatically.

## Rendering with text

The board renders as text using `reduce` — AIVI's replacement for loops:

```aivi
type GameState -> Int -> Text
func renderRow = st y => boardColumns
  |> reduce (rowTextStep st y) ""
```

`boardColumns` is `[0..29]`, a range. `reduce` folds each column index into a text string using `rowTextStep`, which looks up the cell glyph:

```aivi
type GameState -> Int -> Int -> Text
func cellGlyph = st y x =>
    (Cell x y == st.snake.head, st.snake.contains (Cell x y), Cell x y == st.food)
  ||> (True, _, _)          -> "@"
  ||> (_, True, _)          -> "o"
  ||> (_, _, True)          -> "*"
  ||> (False, False, False) -> "·"
```

This matches on a **triple of booleans** — is this cell the head, a body segment, or food? Every combination is covered.

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
          when clauses route to event signal
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
| **Domains** | `Snake` wraps `List Cell` with domain-specific operations |
| **Domain literals** | `120ms` — type-safe duration with suffix syntax |
| **Signals** | `state`, `boardText`, `scoreLine` — the reactive graph |
| **Sources** | `timer.every`, `window.keyDown` — external input boundaries |
| **Event routing** | `when` clauses connect sources to events |
| **Accumulation** | `+\|>` folds events into state over time |
| **Reduce** | Replaces loops for rendering rows and the board |
| **Markup** | `<Window>`, `<Label>`, `<show>` — type-checked GTK UI |

The game has zero mutable variables, zero loops, and zero callbacks. The entire architecture is a declared dependency graph with pure functions at every node.

## Next steps

- [Pipes & Operators](/guide/pipes) — the full pipe algebra reference
- [Signals](/guide/signals) — signals in depth
- [Domains](/guide/domains) — creating your own domains
- [Markup & UI](/guide/markup) — the complete widget system
