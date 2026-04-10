# Manual Hallucination Report

Audit performed 2026-04-10 against stdlib source, `syntax.md`, `AIVI_RFC.md`, `crates/aivi-gtk/src/schema.rs`, and `manual/guide/source-catalog.md`.

## Summary

| Severity | Count |
|----------|-------|
| Critical | 7 |
| High | 5 |
| Medium | 3 |
| Low | 2 |
| **Overall risk** | **HIGH** |

---

## Critical Findings

### `manual/guide/integrations.md` — stale source option names (5 findings)

| Lines | Claim | Truth |
|-------|-------|-------|
| 86–96 | `trigger:` option on `http.get` | Must be `refreshOn:` |
| 104–110 | `trigger:` option on `http.post` | Must be `refreshOn:` |
| 131–139 | `trigger:` option on `fs.read` | Must be `reloadOn:` |
| 177–182 | `params: [selectedTag]` option on `db.live` | No `params` option; use `refreshOn:` |
| 194–200 | `destination:` and `path:` as options on `dbus.signal` | `path` is positional; `destination` is not an option at all |
| 206–212 | `destination:` as option on `dbus.method` | `destination` is the positional arg; not valid as a named option |

### `manual/stdlib/url.md` — domain members listed as module exports

Lines 14, 84, 116, 142 show `use aivi.url ( parse  scheme  host  port  path  query  fragment  withPath  withQuery )`.  
`aivi.url` exports **only** `Url` and `UrlError`. All others are domain members accessed via dot notation.

### `manual/guide/record-patterns.md` — nonexistent stdlib name

Lines 116, 147, 157 use `|> toUpperCase`. The correct AIVI name is `toUpper` (from `aivi.text`).

---

## High Findings

### `manual/stdlib/duration.md`

Lines 13, 63, 80 import `trySeconds` and `millis` from `aivi.duration`.  
Neither is a standalone export — they are domain members. Because `duration.aivi` declares `hoist`, they are project-wide accessible **without** a use clause, but cannot appear in `use aivi.duration (...)`.

### `manual/stdlib/color.md`

- Line 86: `withAlpha` used as standalone function; it is a domain member of `Color`. Correct: `theme.accent.withAlpha 180`.
- Line 104: `blend` same issue. Correct: `theme.background.blend theme.accent 0.15`.

### `manual/guide/signals.md` — logical impossibility

Lines 394–401: `||>` arms with `None` and `Some _` patterns on a `Bool` subject. These arms are unreachable. The function cannot implement its stated semantics as written.

---

## Medium Findings

### `manual/stdlib/async.md` — invalid type syntax

Line 66: `type List User = { ... }` — `List User` is a type application and cannot appear as a type declaration name. Should be `type User = { ... }`.

### `manual/guide/modules.md` — nonexistent module

Lines 8–17, 27–34, 71–84 reference `aivi.network`. No such module exists; HTTP lives in `aivi.http`.

### `manual/guide/building-snake.md` — prose/code contradiction

Line 379 prose says `matrixIndices boardW`; code correctly says `indices boardW`. `matrixIndices` does not exist in stdlib.

---

## Low Findings

### `manual/guide/openapi-source.md`

Line 69: `use _` — unfilled placeholder. Will produce a compiler error.

---

## Patterns

1. **Domain-member-as-standalone-export** (`url.md`, `duration.md`, `color.md`) — writer treated domain members as regular exports.
2. **Stale source option names** (`integrations.md`) — written against an older/imagined API, never reconciled with `source-catalog.md`.
3. **Nonexistent or placeholder names** (`record-patterns.md`, `modules.md`, `openapi-source.md`) — filled from intuition rather than actual stdlib surface.
4. **Prose/code split** (`building-snake.md`) — stdlib renamed `matrixIndices` → `indices`; only code was updated.
