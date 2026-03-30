# AIVI Readability Improvements

## Context

`demos/snake.aivi` is 362 lines. The core game logic is straightforward, but five gaps in the
expression language force verbose workarounds: no method call syntax, no `let` bindings, no
flat multi-condition expressions, no variant test operator, and no way to merge multiple event
sources into one signal stream.

These are general-purpose improvements — they benefit every AIVI program, not just games.

**Key discovery:** the `<|` patch operator already exists (`record <| { .field: val }`) but
snake.aivi doesn't use it. All five features below are genuinely new.

---

## Feature 1 — Method call syntax

`expr.method(arg₁, arg₂)` desugars to `method expr arg₁ arg₂`.

Disambiguation: `expr.field` with no parens stays a plain projection; `expr.method(args)` with
parens is a method call. No ambiguity, no new AST node — just a rewrite to `Apply` during parse.

Layers: `parse.rs` (peek for `(` after `.identifier`), `format.rs` (round-trip), LSP
completion (offer functions whose first parameter type matches the receiver).

---

## Feature 2 — `let` binding in expressions

```
let name = expr
body
```

Layout-sensitive. Chains naturally. Desugars to `(\name -> body) expr` — no mutation.

Needed to name intermediate values (e.g. `nextHead`) without inventing a top-level function or
threading through pipe memo syntax.

Layers: new `let` keyword in `lex.rs`, `ExprKind::Let` in `cst.rs`, desugar in `hir/lower.rs`,
formatter, LSP scope tracking.

---

## Feature 3 — `when/otherwise` cond expression

```
when cond₁ -> expr₁
when cond₂ -> expr₂
otherwise  -> exprN
```

First true condition wins. `otherwise` is mandatory (exhaustiveness). Desugars to nested
`T|>` / `F|>` chains — no new runtime semantics.

Also adds **guards on `||>` case arms**: `||> Pattern when boolExpr -> body`, so the same
priority-chain style works inside ADT dispatch.

Layers: `when`/`otherwise` keywords, `ExprKind::Cond` in `cst.rs`, `guard: Option<Expr>` on
`PipeCaseArm`, desugar in `hir/lower.rs`, formatter.

---

## Feature 4 — `is` variant test

`expr is Constructor` → `Bool`.

- Unit variant: desugars to `expr == Constructor`
- Parameterized variant: desugars to `expr ||> Constructor _ -> True ||> _ -> False`

Layers: `is` keyword, `ExprKind::IsTest` in `cst.rs`, desugar in `hir/lower.rs`, LSP completion
(offer constructors of the expression's type after `is`).

---

## Feature 5 — Multi-source signal merge

```
signal events : Signal GameEvent = merge {
  tick    |> \_ -> Tick,
  keyDown ||>
    ArrowLeft  -> Turn West
    ArrowRight -> Turn East
    ArrowUp    -> Turn North
    ArrowDown  -> Turn South
    Space      -> Restart
}

signal state : Signal GameState = events
  +|> initialState stepGame
```

`merge { ... }` fires whenever any branch fires. Each branch maps its source to the shared result
type. The accumulate then receives a cleanly-typed event stream.

This eliminates the `TickState` / `seenRestartCount` coordination hack entirely. Direction becomes
part of `GameState`, updated by `Turn` events in the same stream as `Tick` and `Restart`.

Layers: `merge` keyword, `ExprKind::Merge` in `cst.rs`, HIR `SignalItem` gets a `MergeBody`
variant, new multi-source input signal kind in `aivi-runtime` (graph, hir_adapter, scheduler).

---

## Impact summary

| Before | After |
|--------|-------|
| `TickState`, `seenRestartCount` (~40 lines) | Eliminated by merge signal |
| `CellHeadState`, `cellHeadStep`, `firstCell` (~8 lines) | `snake.head` via stdlib + method call |
| `willCrash`, `willEat`, `applyRunningStep`, `gameOverState`, `nextSnake`, `nextScore`, `nextFood`, `stepRunning`, `stepGame`, `restartIfRequested`, `restartPending`, `countRestart`, `stepOnTick`, `stepTickState` (~75 lines) | One `when/otherwise` chain in `stepGame` |
| Explicit record construction listing all fields | `<| { .field: val }` patches |
| Separate `direction`/`restartCount` accumulate signals with coordination dance | `direction` is part of `GameState`, driven by `Turn` events |

**Estimated result: ~160–180 lines from 362.**

---

## Layers affected per feature

All features follow:
```
lex.rs → parse.rs → cst.rs → hir/lower.rs → typecheck.rs → format.rs → LSP
```

Features 1, 2, 4: pure syntactic sugar, no backend changes.
Feature 3: desugars in HIR lowering, touches `PipeCaseArm`.
Feature 5: requires `aivi-runtime` changes (graph, hir_adapter, scheduler).

---

## Verification

1. Rewrite `demos/snake.aivi` using all five features as the integration test
2. `cargo test --workspace` must remain green
3. Run rewritten snake through MCP tools (`launch_app`, `emit_gtk_event`) to confirm identical
   behaviour
4. Add HIR/backend fixtures for each new form
5. Formatter round-trip: `format(parse(format(source))) == format(source)`

---

## Improved snake.aivi (target state)

```aivi
domain Duration over Int
    literal ms:Int -> Duration

type Direction =
  | North
  | South
  | East
  | West

type Cell =
  | Cell Int Int

type Status =
  | Running
  | GameOver

type Board = { width: Int, height: Int }

type GameState = {
    snake: List Cell,
    food: Cell,
    score: Int,
    status: Status,
    direction: Direction,
    seed: Int
}

type GameEvent =
  | Tick
  | Turn Direction
  | Restart

value board:Board = { width: 30, height: 20 }
value boardColumns = [0..29]
value boardRows = [0..19]
value initialSeed = 2463534242

value initialSnake = [Cell 6 10, Cell 5 10, Cell 4 10]

-- stdlib assumed: List.head, List.contains, List.append, List.take, List.length

fun moveCell:Cell direction:Direction cell:Cell => (direction, cell)
  ||> (North, Cell x y) -> Cell x (y - 1)
  ||> (South, Cell x y) -> Cell x (y + 1)
  ||> (East,  Cell x y) -> Cell (x + 1) y
  ||> (West,  Cell x y) -> Cell (x - 1) y

fun isOutside:Bool board:Board cell:Cell => cell
  ||> Cell x y -> x < 0 or x >= board.width or y < 0 or y >= board.height

fun nextSeed:Int seed:Int =>
    (seed * 1103515245 + 12345) % 2147483648

fun spawnFood:Cell snake:(List Cell) seed:Int => [0..599]
  |> reduce (\state \_ => state.found
      T|> state
      F|> let candidate = Cell (state.seed % board.width) ((state.seed / board.width) % board.height)
          when snake.contains(candidate) -> { seed: nextSeed state.seed, found: False, value: Cell 0 0 }
          otherwise                      -> { seed: state.seed, found: True, value: candidate })
    { seed, found: False, value: Cell 0 0 }
  |> .value

value initialState:GameState = {
    snake: initialSnake,
    food: spawnFood initialSnake initialSeed,
    score: 0,
    status: Running,
    direction: East,
    seed: initialSeed
}

fun oppositeDirection:Direction d:Direction => d
  ||> North -> South
  ||> South -> North
  ||> East  -> West
  ||> West  -> East

fun stepGame:GameState event:GameEvent state:GameState => event
  ||> Restart ->
      initialState

  ||> Turn dir when state.status is Running ->
      when state.direction == oppositeDirection dir -> state
      otherwise -> state <| { .direction: dir }

  ||> Turn _ -> state

  ||> Tick when state.status is GameOver -> state

  ||> Tick ->
      let next = state.snake.head.moved(state.direction)
      when next.isOutside(board)          -> state <| { .status: GameOver }
      when state.snake.contains(next)     -> state <| { .status: GameOver }
      when next == state.food             ->
          let newSeed = nextSeed state.seed
          state <| { .snake:  state.snake.prepend(next),
                     .score:  state.score + 1,
                     .seed:   newSeed,
                     .food:   spawnFood state.snake.prepend(next) newSeed }
      otherwise ->
          state <| { .snake: state.snake.prepend(next).take(state.snake.length) }

@source window.keyDown with { repeat: False, focusOnly: True }
signal keyDown: Signal Key

@source timer.every 120ms with { immediate: False, coalesce: True }
signal tick: Signal Unit

signal events : Signal GameEvent = merge {
    tick    |> \_ -> Tick,
    keyDown ||>
        ArrowLeft  -> Turn West
        ArrowRight -> Turn East
        ArrowUp    -> Turn North
        ArrowDown  -> Turn South
        Space      -> Restart
}

signal state : Signal GameState = events
  +|> initialState stepGame

fun directionLabel:Text d:Direction => d
  ||> North -> "Up"
  ||> South -> "Down"
  ||> East  -> "Right"
  ||> West  -> "Left"

fun cellGlyph:Text snake:(List Cell) food:Cell cell:Cell =>
  when cell == snake.head   -> "@"
  when snake.contains(cell) -> "o"
  when cell == food         -> "*"
  otherwise                 -> "·"

fun rowText:Text snake:(List Cell) food:Cell row:Int => boardColumns
  |> reduce (\text \col => "{text}{cellGlyph snake food (Cell col row)}") ""

fun boardText:Text s:GameState => boardRows
  |> reduce (\acc \row => acc == "" ? rowText s.snake s.food row : "{acc}\n{rowText s.snake s.food row}") ""

value main =
    <Window title="AIVI Snake">
        <Box orientation="vertical" spacing={8}>
            <Label text={"Direction: {directionLabel state.direction}"} />
            <Label text={"Score: {state.score}"} />
            <Label text={state.status ||> Running -> "Running" ||> GameOver -> "Game Over"} />
            <Label text={boardText state} monospace />
            <show when={state.status is GameOver}>
                <Label text={"Final score: {state.score}"} />
            </show>
        </Box>
    </Window>

export main
```

### What the new version demonstrates

| Feature | Where used |
|---------|-----------|
| Method call `expr.method(args)` | `snake.head.moved(direction)`, `snake.contains(cell)`, `state.snake.prepend(next)`, `state.snake.length` |
| `let` binding | `let next = ...` and `let newSeed = ...` inside `stepGame` |
| `when/otherwise` expression | `stepGame` Tick handler, `cellGlyph`, `boardText` |
| Guards on `\|\|>` arms | `\|\|> Turn dir when state.status is Running` |
| `is` variant test | `state.status is Running`, `state.status is GameOver`, `show when={...}` |
| `merge { }` signal | Merging tick + keyDown into one typed `GameEvent` stream |
| `<\|` patch (already exists) | `state <\| { .status: GameOver }`, `state <\| { .direction: dir }` |

Lines: **~130** vs current **362**.

The remaining complexity is inherent to snake itself: food spawning, board rendering,
`moveCell` dispatch. No boilerplate.
