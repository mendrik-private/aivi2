# AIVI Readability Improvements

## What already exists (and is simply underused in snake.aivi)

| Feature | Syntax | Used in snake.aivi? |
|---------|--------|---------------------|
| Pipe memos | `\|> #before transform #after` | Yes, but inconsistently |
| Reactive `when` updates | `when guard => signal <- expr` | No — all state via `+\|>` accumulate |
| `<\|` patch operator | `record <\| { .field: val }` | No — all explicit record construction |
| `\|\|>` constructor matching | `\|\|> Constructor args -> expr` | Yes |
| `&\|>` applicative cluster | `&\|> sig1 &\|> sig2 +\|> seed step` | No |

The only genuinely new feature needed is **method call syntax** (`expr.method(args)` → `method expr args`).
Everything else is already in the language.

---

## The root of snake.aivi's verbosity

`TickState` exists for one reason: the `+|>` accumulate fires only on its primary signal (`tick`),
so to access `direction` and `restartCount` during a tick, the current code injects them into the
state. This creates the entire `seenRestartCount` / `restartPending` / `restartIfRequested`
coordination chain.

The fix is to stop using `+|>` for the primary game state and use **reactive `when` updates**
instead. `when` clauses targeting the same signal can fire from different sources independently —
no coordination needed...

---

## Rewritten structure (in words)

### Types — 5 fewer types

The five helper record types (`TickState`, `CellHeadState`, `CellTakeState`, `FoodSearch`,
`BoardTextState`) all exist to carry intermediate values through reduces and the tick pipeline.
With reactive updates and pipe memos, none of them are needed.

- `TickState` is eliminated entirely because `direction` and `restart` are handled by separate
  `when` clauses, not bundled into the accumulate input.
- `CellHeadState` and `CellTakeState` are eliminated by using `#memo` bindings inside the snake
  manipulation functions instead of threading a record through a reduce.
- `FoodSearch` is eliminated the same way: the `spawnFood` reduce uses `#seed` and `#candidate`
  memos in successive pipe stages rather than carrying a search-state record.
- `BoardTextState` is eliminated by using memos to bind row text across pipe stages.

### Functions — roughly half the current count

With `<|` patches, every function that returns an updated game state becomes a one-liner — no
more explicit record literals listing all unchanged fields. `gameOverState` disappears entirely
(replaced inline with `game <| { .status: GameOver }`). `applyRunningStep` shrinks to a single
patch expression.

The `willCrash` and `willEat` functions can be inlined into `stepRunning` using `#next` and
`#ate` memos. `stepRunning` becomes a single pipe expression: compute the next head, bind it as
`#next`, then branch via `T|>` / `F|>` with the two cases (`gameOver` patch or grow/move patch)
directly in the arms.

`stepOnTick` and `stepTickState` disappear completely — they only exist to unwrap `TickState`.

`restartIfRequested`, `countRestart`, `restartPending` all disappear — handled by a `when` clause
directly (see signals section).

`nextScore`, `nextFood`, `nextSnake` can be inlined into the `T|>` arm of `stepRunning` using
`<|` patches: a single record patch updating all four fields at once, with the new seed bound
as a memo in an earlier stage.

With method call syntax, `firstCell`, `snakeContains`, `cellLength` disappear — they become
standard library calls on `List` (`snake.head`, `snake.contains(cell)`, `snake.length`).
`moveCell` and `growSnake`/`moveSnake` become methods on `Cell` and `List Cell` respectively.

### Signals — no coordination dance

The key structural change:

**Before:** `signal gameState = tick +|> initialTickState stepOnTick`
where `stepOnTick` must read `direction` and `restartCount` through closure capture and compare
`seenRestartCount` against `restartCount` to detect a restart — requiring `TickState` to carry
that comparison across ticks.

**After:**
- `direction` is a reactive signal with initial value `East`, updated by a `when keyDown` clause
  that maps the key to a new direction (unchanged if opposite).
- `gameState` is a reactive signal with initial value `initialGame`, updated by two `when` clauses:
  one on `tick` that calls `stepRunning direction gameState`, and one on `keyDown == Space`
  that resets to `initialGame` immediately. No tick synchronisation needed for restart.
- The `restartCount` and `restartPending` signals are completely gone.

The `game`, `snake`, `food`, `score`, `gameOver` projection signals remain but they now project
from a cleaner `gameState`.

### View — unchanged

---

## Remaining work after the above (existing features only)

After applying all the above — reactive `when` updates, `<|` patches, memos, `||>` inline matching
— the estimate is roughly **180–200 lines** (from 362), with **no new language features**.

The remaining length is the snake logic itself: food spawning (a reduce over 600 cells),
board rendering (a reduce over 600 cells), and `moveCell` direction dispatch.

---

## The one new feature still worth adding: method call syntax

Without `expr.method(args)`, every function call that has a clear "subject" reads backwards:
`firstCell snake`, `snakeContains snake cell`, `moveCell direction cell`, `hitsWall board cell`.
These aren't just readability preferences — they force helper functions like `firstCell` to
exist because there is no way to express `snake.head` without assuming a stdlib function
can be applied in a fluent style.

Method call syntax is the single highest-value addition. It desugars purely in the parser
(`expr.method(args)` → `Apply(method, [expr, args…])`), requires no runtime changes, and
would let users write `snake.head`, `snake.contains(cell)`, `cell.moved(direction)`,
`board.contains(cell)`, and similar expressions that are self-evidently readable.

With method calls added on top of the existing-feature rewrite, the estimate drops further to
roughly **140–160 lines**, and the helper function count halves again.

---

## Layers needed for method call syntax

- `parse.rs` — after consuming `.identifier`, peek for `(`; if found, consume argument list
  and emit `Apply { callee: Name(method), arguments: [receiver, …args] }`
- `cst.rs` — no new node; reuses existing `Apply`
- `format.rs` — when printing an `Apply` whose first argument is a projection base, optionally
  round-trip as `receiver.method(args)` form (needs a heuristic or annotation)
- `hir/lower.rs` — no change; resolves as normal function application
- `typecheck.rs` — no change
- LSP `completion.rs` — dot-triggered completion should offer all functions in scope whose
  first parameter type matches the receiver's type
