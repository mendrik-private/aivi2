# aivi.env

Read environment variables from the current process.

All reads return `Task Text A`. In plain language, that means the lookup runs through the
runtime and can fail with a text error message. When a variable may be missing, you get an
`Option Text`: `Some value` when it exists, or `None` when it does not.

## Import

```aivi
use aivi.env (
    get
    list
    lookup
    listWithPrefix
    listAll
    HOME
    PATH
    USER
    SHELL
    LANG
    TERM
)
```

## Overview

| Value | Type | Description |
| --- | --- | --- |
| `get` | `Text -> Task Text (Option Text)` | Look up one variable by name |
| `list` | `Text -> Task Text (List (Text, Text))` | List variables whose names start with a prefix |
| `lookup` | `Text -> Task Text (Option Text)` | Alias for `get` |
| `listWithPrefix` | `Text -> Task Text (List (Text, Text))` | Alias for `list` |
| `listAll` | `Task Text (List (Text, Text))` | List every environment variable |

## Functions

### `get`

```aivi
get : Text -> Task Text (Option Text)
```

Look up a single environment variable.

```aivi
use aivi.env (get)

value apiToken : Task Text (Option Text) = get "ACCESS_TOKEN"
```

### `list`

```aivi
list : Text -> Task Text (List (Text, Text))
```

Return every environment variable whose name starts with the given prefix. Pass `""` to get
everything.

```aivi
use aivi.env (list)

value xdgVars : Task Text (List (Text, Text)) = list "XDG_"
```

## Convenience helpers

### `lookup`

```aivi
lookup : Text -> Task Text (Option Text)
```

This is just `get` with a friendlier name for app code.

### `listWithPrefix`

```aivi
listWithPrefix : Text -> Task Text (List (Text, Text))
```

This is just `list` with a name that makes the filtering behavior obvious.

### `listAll`

```aivi
listAll : Task Text (List (Text, Text))
```

Equivalent to `list ""`.

```aivi
use aivi.env (listAll)

value allVars : Task Text (List (Text, Text)) = listAll
```

## Common variables

These exported values are pre-wired lookups for common shell variables:

| Value | Type | Equivalent lookup |
| --- | --- | --- |
| `HOME` | `Task Text (Option Text)` | `get "HOME"` |
| `PATH` | `Task Text (Option Text)` | `get "PATH"` |
| `USER` | `Task Text (Option Text)` | `get "USER"` |
| `SHELL` | `Task Text (Option Text)` | `get "SHELL"` |
| `LANG` | `Task Text (Option Text)` | `get "LANG"` |
| `TERM` | `Task Text (Option Text)` | `get "TERM"` |

```aivi
use aivi.env (
    HOME
    SHELL
)

value homeDir : Task Text (Option Text) = HOME
value currentShell : Task Text (Option Text) = SHELL
```

## Example — inspect a prefix safely

```aivi
use aivi.env (
    get
    listWithPrefix
)

value accessToken : Task Text (Option Text) = get "ACCESS_TOKEN"
value gtkVars : Task Text (List (Text, Text)) = listWithPrefix "GTK_"
```

If you want the same information as a startup signal instead of a task, see the source form
`@source env.get "NAME"` in the source guide.
