# aivi.process

Process-related shared types.

`aivi.process` exports process records, result types, error variants, and the `ProcessSource`
handle marker. Public process work now goes through `@source process`.

Current status: `ProcessSource` is the public capability-handle surface for process context and
spawning; this module remains the shared vocabulary for that work.

## Import

```aivi
use aivi.process (
    ProcessSource
    ProcessError
    SpawnFailed
    ProcessTimeout
    NonZeroExit
    ProcessProtocolError
    ProcessStatus
    Exited
    Killed
    ProcessOutput
    ProcessConfig
    ProcessTask
)
```

## Overview

| Type | Description |
| --- | --- |
| `ProcessError` | Why process work failed |
| `ProcessStatus` | How the process finished |
| `ProcessOutput` | Captured stdout, stderr, and final status |
| `ProcessConfig` | Command, arguments, working directory, environment, and timeout |
| `ProcessSource` | Handle annotation for `@source process` |
| `ProcessTask` | Alias for `Task ProcessError ProcessOutput` |

## `ProcessError`

```aivi
type ProcessError =
  | SpawnFailed Text
  | ProcessTimeout
  | NonZeroExit Int
  | ProcessProtocolError Text
```

These variants cover the main failure cases:

- `SpawnFailed message` — the child process could not be started
- `ProcessTimeout` — the process ran for too long
- `NonZeroExit code` — the process finished, but with a failing exit code
- `ProcessProtocolError message` — host/runtime process handling failed in some other way

## `ProcessStatus`

```aivi
type ProcessStatus =
  | Exited Int
  | Killed
```

This describes the final state of a process that did run.

```aivi
use aivi.process (
    ProcessStatus
    Exited
    Killed
)

type ProcessStatus -> Text
func describeStatus = status => status
 ||> Exited code -> "finished with code {code}"
 ||> Killed      -> "killed"
```

## `ProcessOutput`

```aivi
type ProcessOutput = {
    stdout: Text,
    stderr: Text,
    status: ProcessStatus
}
```

A finished process result: captured standard output, captured standard error, and the final
status.

## `ProcessConfig`

```aivi
type ProcessConfig = {
    command: Text,
    args: List Text,
    workingDir: Option Text,
    env: List (Text, Text),
    timeoutMs: Option Int
}
```

Use this when you want to describe process work in a structured way.

- `command` — executable name or path
- `args` — command-line arguments
- `workingDir` — optional working directory
- `env` — extra environment variable pairs
- `timeoutMs` — optional timeout in raw milliseconds

```aivi
use aivi.process (ProcessConfig)

value gitStatus : ProcessConfig = {
    command: "git",
    args: ["status", "--short"],
    workingDir: None,
    env: [],
    timeoutMs: Some 5000
}
```

## `ProcessTask`

```aivi
type ProcessTask = (Task ProcessError ProcessOutput)
```

This is a handy alias when you write your own wrappers around process-running logic.

## Example — typed error handling

```aivi
use aivi.process (
    ProcessError
    SpawnFailed
    ProcessTimeout
    NonZeroExit
    ProcessProtocolError
)

type ProcessError -> Text
func describeFailure = error => error
 ||> SpawnFailed message         -> "could not start process: {message}"
 ||> ProcessTimeout              -> "process timed out"
 ||> NonZeroExit code            -> "process exited with code {code}"
 ||> ProcessProtocolError detail -> "runtime process error: {detail}"
```

For public process access, prefer a handle such as:

```aivi
use aivi.process (ProcessSource)

@source process
signal runtime : ProcessSource

signal cliArgs : Signal (List Text) = runtime.args
signal cwd : Signal Text = runtime.cwd
```
