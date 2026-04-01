# aivi.desktop.xdg

XDG Base Directory Specification helpers for Linux desktop apps.

All functions are synchronous and pure — they read environment variables with XDG-spec fallbacks.
No I/O is performed.

## Import

```aivi
use aivi.desktop.xdg (
    dataHome
    configHome
    cacheHome
    stateHome
    runtimeDir
    dataDirs
    configDirs
)
```

## Overview

| Function     | Type               | Description                                    |
|--------------|--------------------|------------------------------------------------|
| `dataHome`   | `Text`             | `$XDG_DATA_HOME` or `$HOME/.local/share`       |
| `configHome` | `Text`             | `$XDG_CONFIG_HOME` or `$HOME/.config`          |
| `cacheHome`  | `Text`             | `$XDG_CACHE_HOME` or `$HOME/.cache`            |
| `stateHome`  | `Text`             | `$XDG_STATE_HOME` or `$HOME/.local/state`      |
| `runtimeDir` | `Option Text`      | `$XDG_RUNTIME_DIR` or `None` if unset          |
| `dataDirs`   | `List Text`        | `$XDG_DATA_DIRS` or `/usr/local/share:/usr/share` |
| `configDirs` | `List Text`        | `$XDG_CONFIG_DIRS` or `/etc/xdg`               |

## Functions

### dataHome

```aivi
dataHome : Text
```

The base directory for user-specific data files.

Default: `$HOME/.local/share`

```aivi
use aivi.desktop.xdg (dataHome)

use aivi.path (join)

value appDataDir = join dataHome "myapp"
```

### configHome

```aivi
configHome : Text
```

The base directory for user-specific configuration files.

Default: `$HOME/.config`

```aivi
use aivi.desktop.xdg (configHome)

use aivi.path (join)

value appConfigDir = join configHome "myapp"
```

### cacheHome

```aivi
cacheHome : Text
```

The base directory for user-specific cache files.

Default: `$HOME/.cache`

```aivi
use aivi.desktop.xdg (cacheHome)

use aivi.path (join)

value appCacheDir = join cacheHome "myapp"
```

### stateHome

```aivi
stateHome : Text
```

The base directory for user-specific state data (logs, history, etc.).

Default: `$HOME/.local/state`

```aivi
use aivi.desktop.xdg (stateHome)

use aivi.path (join)

value appStateDir = join stateHome "myapp"
```

### runtimeDir

```aivi
runtimeDir : Option Text
```

The base directory for user-specific non-essential runtime files (sockets, pipes, etc.).

Returns `None` when `$XDG_RUNTIME_DIR` is not set. On a running GNOME session this is
always set (typically `/run/user/1000`).

```aivi
use aivi.desktop.xdg (runtimeDir)

use aivi.option (withDefault)

value socketBase = withDefault "/tmp" runtimeDir
```

### dataDirs

```aivi
dataDirs : List Text
```

Ordered search path for system-wide data directories.

Default: `["/usr/local/share", "/usr/share"]`

```aivi
use aivi.desktop.xdg (
    dataDirs
    dataHome
)

value allDataDirs
```

### configDirs

```aivi
configDirs : List Text
```

Ordered search path for system-wide configuration directories.

Default: `["/etc/xdg"]`

```aivi
use aivi.desktop.xdg (
    configDirs
    configHome
)

value allConfigDirs
```

## Error type

```aivi
type XdgError =
  | XdgHomeUnset
  | XdgRuntimeDirUnavailable
```

`XdgError` covers the two situations where an XDG path cannot be resolved:

- `XdgHomeUnset` — neither the env var nor `$HOME` is available
- `XdgRuntimeDirUnavailable` — the runtime dir intrinsic returned `None`

## Example — build standard app directories

```aivi
use aivi.desktop.xdg (
    dataHome
    configHome
    cacheHome
)

use aivi.path (join)

value dataDir = join dataHome "com.example.MyApp"
value configDir = join configHome "com.example.MyApp"
value cacheDir = join cacheHome "com.example.MyApp"
```

## Example — find a data file across the search path

```aivi
use aivi.desktop.xdg (
    dataDirs
    dataHome
)

use aivi.path (join)

use aivi.fs (exists)

use aivi.list (filter)

func findDataFile = name =>
```
