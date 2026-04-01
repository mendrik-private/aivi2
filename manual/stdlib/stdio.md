# aivi.stdio

Write text to standard output and standard error.

These functions are useful in CLI-style programs and quick debugging tasks. The current
runtime writes the text immediately and flushes the stream right away.

## Import

```aivi
use aivi.stdio (
    WriteError
    StdioUnavailable
    Stream
    Stdout
    Stderr
    StdioTask
    StdoutTask
    StderrTask
    stdoutWrite
    stderrWrite
    writeLine
    writeErrorLine
)
```

## Overview

| Value | Type | Description |
| --- | --- | --- |
| `stdoutWrite` | `Text -> Task Text Unit` | Write text to standard output |
| `stderrWrite` | `Text -> Task Text Unit` | Write text to standard error |
| `writeLine` | `Text -> Task Text Unit` | Write a line to stdout and append `\n` |
| `writeErrorLine` | `Text -> Task Text Unit` | Write a line to stderr and append `\n` |

## Core functions

### `stdoutWrite`

```aivi
stdoutWrite : Text -> Task Text Unit
```

Write text exactly as given. No newline is added for you.

```aivi
use aivi.stdio (stdoutWrite)

value prompt : Task Text Unit = stdoutWrite "Name: "
```

### `stderrWrite`

```aivi
stderrWrite : Text -> Task Text Unit
```

Write text to the error stream. This is a good fit for warnings and failures.

```aivi
use aivi.stdio (stderrWrite)

value warning : Task Text Unit = stderrWrite "missing config\n"
```

## Line helpers

### `writeLine`

```aivi
writeLine : Text -> Task Text Unit
```

Convenience wrapper around `stdoutWrite` that adds a trailing newline.

### `writeErrorLine`

```aivi
writeErrorLine : Text -> Task Text Unit
```

Convenience wrapper around `stderrWrite` that adds a trailing newline.

```aivi
use aivi.stdio (
    writeLine
    writeErrorLine
)

value done : Task Text Unit = writeLine "Finished"
value failed : Task Text Unit = writeErrorLine "Something went wrong"
```

## Supporting types

```aivi
type WriteError =
  | StdioUnavailable

type Stream = Stdout | Stderr

type StdioTask = (Task WriteError Unit)
type StdoutTask = (Task Text Unit)
type StderrTask = (Task Text Unit)
```

`Stream` is handy when your own code needs to remember where output should go.

**Current behavior note:** the callable functions `stdoutWrite` and `stderrWrite` currently
return `Task Text Unit`. The exported `StdioTask` alias uses `WriteError`, so treat it as a
separate vocabulary type rather than the exact function return type.

## Example — progress plus failure output

```aivi
use aivi.stdio (
    writeLine
    writeErrorLine
)

value started : Task Text Unit = writeLine "Starting sync"
value badToken : Task Text Unit = writeErrorLine "ACCESS_TOKEN is missing"
```

If you need input from standard input, pair this module with the source form
`@source stdio.read`.
