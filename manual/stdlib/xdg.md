# aivi.desktop.xdg

XDG-related error vocabulary retained for provider-backed path sources.

Public `dataHome` / `configHome` / `cacheHome` imports are no longer the preferred surface. Use
`@source path` and `PathSource` for host directory snapshots.

## Preferred usage

```aivi
use aivi.path (PathSource)

@source path
signal paths : PathSource

signal dataHome : Signal Text = paths.dataHome
signal configHome : Signal Text = paths.configHome
signal cacheHome : Signal Text = paths.cacheHome
signal tempDir : Signal Text = paths.tempDir
```

## What this module still exports

```aivi
use aivi.desktop.xdg (
    XdgError
    XdgHomeUnset
    XdgRuntimeDirUnavailable
    XdgUserDir
    XdgHome
    XdgDesktop
    XdgDocuments
    XdgDownloads
    XdgMusic
    XdgPictures
    XdgVideos
    XdgTemplates
    XdgPublicShare
    XdgTask
)
```

Those exports remain as shared type vocabulary for path/XDG-related APIs. Actual directory lookup
is now exposed through `PathSource`.
