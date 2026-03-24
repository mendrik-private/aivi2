# List Rendering

Rendering lists efficiently is one of the most common tasks in UI programming.
AIVI provides two complementary tools: `<each>` for markup and `*\|>` for signal-level
transformations.

## Basic list rendering with each

```aivi
type Task = {
    id: Int,
    text: Text,
    done: Bool
}

type Orientation =
  | Vertical
  | Horizontal

sig tasks : Signal (List Task) = []

val main =
    <Window title="Tasks">
        <Box orientation={Vertical} spacing={4}>
            <each of={tasks} as={task} key={task.id}>
                <Box orientation={Horizontal} spacing={4}>
                    <Label text={task.text} />
                </Box>
            </each>
        </Box>
    </Window>

export main
```

The `key` attribute is how the runtime tracks which widget corresponds to which item.
When the list updates, the runtime:
1. Reuses widgets for items with matching keys.
2. Creates widgets for new keys.
3. Destroys widgets for removed keys.

**Always provide a stable, unique `key`.**

## Transforming lists before rendering

Use `*\|>` (map pipe) to transform each item in a list signal before passing it to `<each>`:

```aivi
type Task = {
    id: Int,
    text: Text,
    done: Bool
}

type TaskView = {
    id: Int,
    text: Text,
    styleClass: Text
}

fun styleFor:Text #done:Bool =>
    done
     T|> "done"
     F|> "active"

fun toTaskView:TaskView #task:Task =>
    {
        id: task.id,
        text: task.text,
        styleClass: styleFor task.done
    }

sig tasks : Signal (List Task) = []

sig taskViews : Signal (List TaskView) =
    tasks
     *|> toTaskView
```

`*\|>` applies `toTaskView` to every item in the list.
The result is a new list of the same length with transformed items.

## Filtering with partition

To render only a subset of a list, derive helper functions with `aivi.list.partition` and then
select the result you want:

```aivi
use aivi.list (partition)

type Task = {
    id: Int,
    text: Text,
    done: Bool
}

type Filter =
  | All
  | Active
  | Done

fun isDone:Bool #task:Task =>
    task.done

fun isActive:Bool #task:Task =>
    task.done == False

fun doneTasks:(List Task) #allTasks:(List Task) =>
    partition isDone allTasks
     ||> { matched } => matched

fun activeTasks:(List Task) #allTasks:(List Task) =>
    partition isActive allTasks
     ||> { matched } => matched

fun applyFilter:(List Task) #filter:Filter #allTasks:(List Task) =>
    filter
     ||> All    => allTasks
     ||> Done   => doneTasks allTasks
     ||> Active => activeTasks allTasks

sig tasks : Signal (List Task) = []
sig currentFilter : Signal Filter = All

sig filteredTasks : Signal (List Task) =
  &|> currentFilter
  &|> tasks
  |> applyFilter
```

`filteredTasks` recomputes whenever `tasks` or `currentFilter` changes.

## Fan-out with *\|> and <\|*

The fan-out pattern applies a transformation to every item in a list and then reduces the results
with an explicit reducer. `*\|>` maps each item; `<\|*` immediately follows with the reducer:

```aivi
use aivi.text (join)

type User = {
    id: Int,
    name: Text,
    email: Text
}

sig users : Signal (List User) = []

sig emailList : Signal Text =
    users
     *|> .email
     <|* join ", "
```

`*\|>` is pure mapping — it does not produce nested signals.
`<\|*` is legal only immediately after `*\|>` and takes a reducer function.

## Nested lists

The snake game renders a board as a list of rows, each containing a list of cells.
This is nested `<each>`:

```aivi
type CellKind =
  | SnakeHead
  | SnakeBody
  | Food
  | Empty

type BoardCell = {
    id: Int,
    kind: CellKind
}

type BoardRow = {
    id: Int,
    cells: List BoardCell
}

type Orientation =
  | Vertical
  | Horizontal

fun cellGlyph:Text #kind:CellKind =>
    kind
     ||> SnakeHead => "@"
     ||> SnakeBody => "o"
     ||> Food      => "*"
     ||> Empty     => "."

sig boardRows : Signal (List BoardRow) = []

val main =
    <Window title="Board">
        <Box orientation={Vertical} spacing={2}>
            <each of={boardRows} as={row} key={row.id}>
                <Box orientation={Horizontal} spacing={2}>
                    <each of={row.cells} as={cell} key={cell.id}>
                        <Label text={cellGlyph cell.kind} />
                    </each>
                </Box>
            </each>
        </Box>
    </Window>

export main
```

The outer `<each>` iterates rows; the inner `<each>` iterates cells within each row.
Keys are scoped to their respective `<each>` block.

## Dynamic keys

The `key` attribute must be unique within a single `<each>` block but does not need to be
globally unique. Row ids and cell ids can both be integers starting from `0` as long as
they are unique within their own list.

## Computing list statistics

```aivi
use aivi.list (
    length
    count
)

type Task = {
    id: Int,
    text: Text,
    done: Bool
}

fun isDone:Bool #task:Task =>
    task.done

fun isActive:Bool #task:Task =>
    task.done == False

fun formatStatus:Text #n:Int =>
    "{n} items remaining"

sig tasks : Signal (List Task) = []

sig taskCount : Signal Int =
    tasks
     |> length

sig doneCount : Signal Int =
    tasks
     |> count isDone

sig activeCount : Signal Int =
    tasks
     |> count isActive

sig statusText : Signal Text =
    activeCount
     |> formatStatus
```

These are all derived signals — they update automatically when `tasks` changes.

## Summary

- `<each of={listSignal} as={item} key={item.id}>` renders a list.
- `key` is required and must be unique within the block.
- `*\|>` transforms every item in a list signal.
- Use `partition` or a derived helper to select a filtered subset.
- `*\|>` maps each item; `<\|*` immediately follows with a reducer function.
- Nest `<each>` blocks for 2D data structures.
