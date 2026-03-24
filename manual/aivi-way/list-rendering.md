# List Rendering

Rendering lists efficiently is one of the most common tasks in UI programming.
AIVI provides two complementary tools: `<each>` for markup and `*\|>` for signal-level
transformations.

## Basic list rendering with each

```text
// TODO: add a verified AIVI example here
```

The `key` attribute is how the runtime tracks which widget corresponds to which item.
When the list updates, the runtime:
1. Reuses widgets for items with matching keys.
2. Creates widgets for new keys.
3. Destroys widgets for removed keys.

**Always provide a stable, unique `key`.**

## Transforming lists before rendering

Use `*\|>` (map pipe) to transform each item in a list signal before passing it to `<each>`:

```text
// TODO: add a verified AIVI example here
```

`*\|>` applies `toTaskView` to every item in the list.
The result is a new list of the same length with transformed items.

## Filtering with partition

To render only a subset of a list, derive helper functions with `aivi.list.partition` and then
select the result you want:

```text
// TODO: add a verified AIVI example here
```

`filteredTasks` recomputes whenever `tasks` or `currentFilter` changes.

## Fan-out with *\|> and <\|*

The fan-out pattern applies a transformation to every item in a list and then reduces the results
with an explicit reducer. `*\|>` maps each item; `<\|*` immediately follows with the reducer:

```text
// TODO: add a verified AIVI example here
```

`*\|>` is pure mapping — it does not produce nested signals.
`<\|*` is legal only immediately after `*\|>` and takes a reducer function.

## Nested lists

The snake game renders a board as a list of rows, each containing a list of cells.
This is nested `<each>`:

```text
// TODO: add a verified AIVI example here
```

The outer `<each>` iterates rows; the inner `<each>` iterates cells within each row.
Keys are scoped to their respective `<each>` block.

## Dynamic keys

The `key` attribute must be unique within a single `<each>` block but does not need to be
globally unique. Row ids and cell ids can both be integers starting from `0` as long as
they are unique within their own list.

## Computing list statistics

```text
// TODO: add a verified AIVI example here
```

These are all derived signals — they update automatically when `tasks` changes.

## Summary

- `<each of={listSignal} as={item} key={item.id}>` renders a list.
- `key` is required and must be unique within the block.
- `*\|>` transforms every item in a list signal.
- Use `partition` or a derived helper to select a filtered subset.
- `*\|>` maps each item; `<\|*` immediately follows with a reducer function.
- Nest `<each>` blocks for 2D data structures.
