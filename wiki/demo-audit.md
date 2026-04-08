# Demo Audit — Snake & Reversi

Audit of `demos/snake.aivi` and `demos/reversi.aivi` against AIVI standards.  
Performed: 2026-04-06. All issues below are fixed in the demo files.

---

## Snake (`demos/snake.aivi`)

### ✅ What's good

- **Pure functional state machine**: `step : Event -> GameState -> GameState` — no side effects in game logic.
- **Signal merge syntax correct**: `||>` with `=>` (fat arrow), not `->`.
- **Recurrence signal**: `+|> initial step` — correct fold-style accumulation.
- **Explicit type annotations** on all functions and values.
- **Domain abstraction**: `Snake over NonEmptyList Cell` — correct use of the domain system to encapsulate the snake body.
- **No if/else, no loops**: pure pattern matching and pipe algebra throughout.
- **Patch operator**: `<|` used correctly for record field updates.
- **`cellEq` comparator**: Required AIVI pattern — `==` at polymorphic type needs an explicit `Eq` comparator passed to `listContains`. Correct as-is (see [type-system.md](type-system.md)).
- **Timer source**: `@source timer.every 120ms with { immediate: False, coalesce: True }` — correct Duration literal with options.

### ❌ Issues found & fixed

#### 1. Domain abstraction leak — `toList st.snake.carrier` (line 161)

**Before**:
```aivi
func renderBoard = st => indices boardH
  |> map (renderRowAt (toList st.snake.carrier) st)
```

`st.snake.carrier` bypasses the `Snake` domain by accessing the raw `NonEmptyList Cell` carrier directly. AIVI domains are meant to be opaque.

**Fix**: Added `cells` method to the `Snake` domain, updated call site:

```aivi
// in Snake domain:
type List Cell
cells = toList self

// in renderBoard:
func renderBoard = st => indices boardH
  |> map (renderRowAt st.snake.cells st)
```

#### 2. Double computation of `nextSeed st.seed` (line 141)

**Before**:
```aivi
func resolveMoveGrow = st h =>
    st <| { snake: st.snake.grow h, food: spawnFood (nextSeed st.seed), score: st.score + 1, seed: nextSeed st.seed }
```

`nextSeed st.seed` is evaluated twice — once for `food` and once for `seed`. Redundant and fragile if the function changes.

**Fix**: Extract a helper that receives the pre-computed seed:

```aivi
type GameState -> Cell -> Int -> GameState
func resolveMoveGrowWithSeed = st h newSeed =>
    st <| { snake: st.snake.grow h, food: spawnFood newSeed, score: st.score + 1, seed: newSeed }

type GameState -> Cell -> GameState
func resolveMoveGrow = st h =>
    resolveMoveGrowWithSeed st h (nextSeed st.seed)
```

---

## Reversi (`demos/reversi.aivi`)

### ✅ What's good

- **Rich animation state machine** (`AnimState`) — properly modelled as a closed ADT.
- **AI player** implemented purely as positional-weight scoring + `listMaximum`.
- **Ray-scan algorithm** cleanly expressed as reducible steps over `boardIndices`.
- **All game logic is pure** — no side-effectful operations in the computation layer.
- **Correct signal merge syntax** and recurrence with `+|>`.
- **Markup**: Good use of `<each>`, `<with>`, `<show>`, `HeaderBar`.

### ❌ Issues found & fixed

#### 1. `setAnimState` copies all 9 fields instead of using `<|`

**Before**:
```aivi
func setAnimState = state animState =>
    {
        board: state.board,
        status: state.status,
        legalMoves: state.legalMoves,
        // ... 6 more fields manually copied
        animState: animState
    }
```

Manual full-record construction to update one field is the primary AIVI anti-pattern. The `<|` patch operator exists for this.

**Fix**:
```aivi
func setAnimState = state animState =>
    state <| { animState: animState }
```

#### 2. `applyHumanMoveWithFlips` manually constructs `GameState`

Same anti-pattern — constructs the full record rather than patching with `<|`.

**Fix**: Uses `state <| { board: ..., status: ..., legalMoves: ..., ... }`.

#### 3. `computerAnimState` manually constructs `GameState`

Same anti-pattern. The seven fields not being changed (`moveNumber`, `lastMove`, `lastFlips`) were copied verbatim.

**Fix**: Uses `state <| { board: ..., status: ..., legalMoves: ..., humanCount: ..., computerCount: ..., animState: ... }`.

#### 4. `@source timer.every 100` — missing time unit

**Before**: `@source timer.every 100`  
**After**: `@source timer.every 100ms`

The source catalog documents bare integers as legacy. Duration literals (`100ms`) are the current standard. Snake correctly uses `120ms`.

#### 5. `Candidate.flips` was always `0` — dead tiebreaker

**Before**:
```aivi
func buildCandidate = coord =>
    { coord: coord, flips: 0, score: finalCandidateScore coord }

func candidateForCoord = board coord =>
    buildCandidate coord
```

The `Candidate` type has a `flips` field and `betterCandidate` uses it as a tiebreaker — but it was always `0`. The tiebreaker was permanently inactive.

**Fix**: Compute the actual flip count and pass it into the candidate. Also renamed `betterCandidate` → `candidateLessThan` (it's a less-than comparator for `listMaximum`, not a "better" predicate):

```aivi
type Coord -> Int -> Candidate
func buildCandidateWithFlips = coord flipCount =>
    { coord: coord, flips: flipCount, score: finalCandidateScore coord }

type List (List Disc) -> Coord -> Candidate
func candidateForCoord = board coord =>
    buildCandidateWithFlips coord (listLength (flipsForMove board (discForTurn Computer) coord))

type Candidate -> Candidate -> Bool
func candidateLessThan = left right =>
    left.score < right.score or (left.score == right.score and left.flips < right.flips) or ...
```

The tiebreaker semantics (fewer flips wins when positional scores are equal) aligns with conservative positional play — the computer prefers corners and edges over greedy captures.

### 2026-04-08 latency note

- `demos/reversi.aivi` now takes the fast path on human clicks: the board and turn update immediately, while the heavier `Snapshot` recompute is deferred by a 1ms timer tick.
- The AI preview dot is computed directly from the current board so the human red stones paint before the delayed snapshot bookkeeping catches up.

---

## Summary table

| File | Issue | Severity | Fixed |
|------|-------|----------|-------|
| snake | `toList st.snake.carrier` bypasses domain | Medium | ✅ |
| snake | `nextSeed st.seed` computed twice | Low | ✅ |
| reversi | `setAnimState` copies all fields instead of `<\|` | High | ✅ |
| reversi | `applyHumanMoveWithFlips` copies all fields | Medium | ✅ |
| reversi | `computerAnimState` copies all fields | Medium | ✅ |
| reversi | `@source timer.every 100` missing unit | Low | ✅ |
| reversi | `Candidate.flips` always 0 — dead tiebreaker | Medium | ✅ |
| reversi | `betterCandidate` misleading name | Low | ✅ |

*See also: [signal-model.md](signal-model.md), [type-system.md](type-system.md), [stdlib.md](stdlib.md)*
