# List Rendering

Rendering lists efficiently is one of the most common tasks in UI programming.
AIVI provides two complementary tools: `<each>` for markup and `*\|>` for signal-level
transformations.

## Basic list rendering with each

```text
-- declare a product type 'Task' with integer id, text content, and a done boolean
-- declare a signal 'tasks' holding a list of Tasks
-- render a vertical Box iterating over tasks, keyed by task id
-- for each task render a horizontal Box with a CheckButton reflecting task.done and a Label showing task.text
```

The `key` attribute is how the runtime tracks which widget corresponds to which item.
When the list updates, the runtime:
1. Reuses widgets for items with matching keys (no re-render for unchanged items).
2. Creates widgets for new keys.
3. Destroys widgets for removed keys.

**Always provide a stable, unique `key`.**

## Transforming lists before rendering

Use `*\|>` (map pipe) to transform each item in a list signal before passing it to `<each>`:

```text
-- declare a product type 'Task' with integer id, text content, and a done boolean
-- declare a product type 'TaskView' with integer id, display text, and a style class name
-- declare a function 'toTaskView' converting a Task to a TaskView
--   copying id and text, and setting styleClass to "done" if task is done, "active" otherwise
-- declare a signal 'tasks' holding a list of Tasks
-- derive 'taskViews' by mapping toTaskView over every item in the tasks list
```

`*\|>` applies `toTaskView` to every item in the list.
The result is a new list of the same length with transformed items.

## Filtering with ?\|> on list elements

To render only a subset of a list, use `List.filter` combined with the gate pipe:

```text
-- declare a sum type 'Filter' with variants All, Active, Done
-- declare a signal 'currentFilter' holding the active filter selection
-- declare a predicate 'isActive' that returns True when a task is not done
-- declare a predicate 'isDone' that returns True when a task is done
-- declare a function 'applyFilter' that keeps all, active-only, or done-only tasks based on the filter
-- combine tasks and currentFilter applicatively, applying applyFilter to get 'filteredTasks'
-- filteredTasks recomputes when either tasks or currentFilter changes
```

`filteredTasks` recomputes whenever `tasks` or `currentFilter` changes.

## Fan-out with *\|> and <\|*

The fan-out pattern applies a transformation to every item in a list and then reduces the results
with an explicit reducer. `*\|>` maps each item; `<\|*` immediately follows with the reducer:

```text
-- derive 'emailList' from the users signal
-- extract the email field from every user in the list
-- join all emails into a single comma-separated text string
```

`*\|>` is pure mapping — it does not produce nested signals.
`<\|*` is legal only immediately after `*\|>` and takes a reducer function.

## Nested lists

The snake game renders a board as a list of rows, each containing a list of cells.
This is nested `<each>`:

```text
-- declare a signal 'boardRows' holding a list of rows
-- render a vertical Box iterating over rows, keyed by row id
-- for each row render a horizontal Box
-- iterate over each cell in the row, keyed by cell id
-- render a Label showing the cell's glyph based on its kind
```

The outer `<each>` iterates rows; the inner `<each>` iterates cells within each row.
Keys are scoped to their respective `<each>` block.

## Dynamic keys

The `key` attribute must be unique within a single `<each>` block but does not need to be
globally unique. Row IDs and cell IDs can both be integers starting from `0` as long as
they are unique within their own list.

## Computing list statistics

```text
-- derive 'taskCount' as the total number of tasks
-- derive 'doneCount' as the number of completed tasks
-- derive 'activeCount' as the number of incomplete tasks
-- derive 'statusText' from activeCount, formatted as "N items remaining"
-- all four signals recompute automatically when tasks changes
```

These are all derived signals — they update automatically when `tasks` changes.

## Summary

- `<each of={listSignal} as={item} key={item.id}>` renders a list.
- `key` is required and must be unique within the block.
- `*\|>` transforms every item in a list signal.
- Filter with `List.filter` applied to the list signal.
- `*\|>` maps each item; `<\|*` immediately follows with a reducer function.
- Nest `<each>` blocks for 2D data structures.
