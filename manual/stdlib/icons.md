# aivi.gtk.icons

Typed constants for standard GNOME/freedesktop icon names. Import specific icons to use them in widget properties instead of writing magic strings.

## Import

```aivi
use aivi.gtk.icons (
    documentSaveSymbolic
    editDeleteSymbolic
)
```

## At a glance

| Category | Count | Notes |
|----------|-------|-------|
| Actions | 35 | Document, edit, list, zoom, media |
| Navigation | 13 | Go-*, bookmark, menu, overflow |
| Status | 12 | Dialog icons, emblems, network, battery |
| Objects | 20 | Files, audio, devices, users |
| Symbolic variants | 27 | Single-colour `-symbolic` companions |

## Actions

| Constant | Icon name |
|----------|-----------|
| `documentNew` | `document-new` |
| `documentOpen` | `document-open` |
| `documentSave` | `document-save` |
| `documentSaveAs` | `document-save-as` |
| `documentClose` | `document-close` |
| `documentPrint` | `document-print` |
| `editCopy` | `edit-copy` |
| `editCut` | `edit-cut` |
| `editPaste` | `edit-paste` |
| `editDelete` | `edit-delete` |
| `editUndo` | `edit-undo` |
| `editRedo` | `edit-redo` |
| `editFind` | `edit-find` |
| `editFindReplace` | `edit-find-replace` |
| `editSelectAll` | `edit-select-all` |
| `editClear` | `edit-clear` |
| `listAdd` | `list-add` |
| `listRemove` | `list-remove` |
| `viewRefresh` | `view-refresh` |
| `viewFullscreen` | `view-fullscreen` |
| `viewRestoreFullscreen` | `view-restore` |
| `zoomIn` | `zoom-in` |
| `zoomOut` | `zoom-out` |
| `zoomFitBest` | `zoom-fit-best` |
| `zoomOriginal` | `zoom-original` |
| `windowClose` | `window-close` |
| `applicationExit` | `application-exit` |
| `mailSend` | `mail-send` |
| `mailReply` | `mail-reply` |
| `mailForward` | `mail-forward` |
| `callStart` | `call-start` |
| `callStop` | `call-stop` |
| `mediaPlay` | `media-playback-start` |
| `mediaPause` | `media-playback-pause` |
| `mediaStop` | `media-playback-stop` |

## Navigation

| Constant | Icon name |
|----------|-----------|
| `goPrevious` | `go-previous` |
| `goNext` | `go-next` |
| `goUp` | `go-up` |
| `goDown` | `go-down` |
| `goHome` | `go-home` |
| `goFirst` | `go-first` |
| `goLast` | `go-last` |
| `goJump` | `go-jump` |
| `bookmarkNew` | `bookmark-new` |
| `openMenu` | `open-menu` |
| `openMenuHorizontal` | `open-menu-horizontal` |
| `viewMore` | `view-more` |
| `viewMoreHorizontal` | `view-more-horizontal` |

## Status

| Constant | Icon name |
|----------|-----------|
| `dialogError` | `dialog-error` |
| `dialogWarning` | `dialog-warning` |
| `dialogInformation` | `dialog-information` |
| `dialogQuestion` | `dialog-question` |
| `emblemOk` | `emblem-ok` |
| `emblemImportant` | `emblem-important` |
| `networkOffline` | `network-offline` |
| `networkTransmit` | `network-transmit` |
| `networkReceive` | `network-receive` |
| `batteryFull` | `battery` |
| `syncSynchronizing` | `emblem-synchronizing` |
| `processStop` | `process-stop` |

## Objects

| Constant | Icon name |
|----------|-----------|
| `addressBook` | `address-book-new` |
| `audioVolumeMute` | `audio-volume-muted` |
| `audioVolumeHigh` | `audio-volume-high` |
| `audioVolumeLow` | `audio-volume-low` |
| `audioVolumeMedium` | `audio-volume-medium` |
| `camera` | `camera-photo` |
| `cameraVideo` | `camera-video` |
| `contact` | `contact-new` |
| `folder` | `folder` |
| `folderNew` | `folder-new` |
| `folderOpen` | `folder-open` |
| `image` | `image-x-generic` |
| `preferences` | `preferences-system` |
| `systemSettings` | `preferences-other` |
| `security` | `security-medium` |
| `user` | `system-users` |
| `computer` | `computer` |
| `phone` | `phone` |
| `printer` | `printer` |
| `scanner` | `scanner` |

## Symbolic variants

Symbolic icons render as a single-colour glyph that adapts to the current foreground colour.
Prefer symbolic variants inside header bars and toolbars where the icon must respect
the theme colour and high-contrast settings.

| Constant | Icon name |
|----------|-----------|
| `documentNewSymbolic` | `document-new-symbolic` |
| `documentOpenSymbolic` | `document-open-symbolic` |
| `documentSaveSymbolic` | `document-save-symbolic` |
| `documentSaveAsSymbolic` | `document-save-as-symbolic` |
| `editCopySymbolic` | `edit-copy-symbolic` |
| `editCutSymbolic` | `edit-cut-symbolic` |
| `editPasteSymbolic` | `edit-paste-symbolic` |
| `editDeleteSymbolic` | `edit-delete-symbolic` |
| `editUndoSymbolic` | `edit-undo-symbolic` |
| `editRedoSymbolic` | `edit-redo-symbolic` |
| `listAddSymbolic` | `list-add-symbolic` |
| `listRemoveSymbolic` | `list-remove-symbolic` |
| `viewRefreshSymbolic` | `view-refresh-symbolic` |
| `goPreviousSymbolic` | `go-previous-symbolic` |
| `goNextSymbolic` | `go-next-symbolic` |
| `goUpSymbolic` | `go-up-symbolic` |
| `goDownSymbolic` | `go-down-symbolic` |
| `goHomeSymbolic` | `go-home-symbolic` |
| `openMenuSymbolic` | `open-menu-symbolic` |
| `searchSymbolic` | `system-search-symbolic` |
| `settingsSymbolic` | `emblem-system-symbolic` |
| `notificationSymbolic` | `notification-symbolic` |
| `userSymbolic` | `system-users-symbolic` |
| `checkmarkSymbolic` | `emblem-ok-symbolic` |
| `errorSymbolic` | `dialog-error-symbolic` |
| `warningSymbolic` | `dialog-warning-symbolic` |
| `infoSymbolic` | `dialog-information-symbolic` |

## Usage examples

### Header bar with icon buttons

```aivi
use aivi.gtk.icons (
    documentSaveSymbolic
    editUndoSymbolic
    editRedoSymbolic
)

value view =
    <Window title="Editor">
        <HeaderBar>
            <Button iconName={editUndoSymbolic} tooltipText="Undo" />
            <Button iconName={editRedoSymbolic} tooltipText="Redo" />
            <Button iconName={documentSaveSymbolic} tooltipText="Save" label="Save" />
        </HeaderBar>
    </Window>
```

### Status icon in a row

```aivi
use aivi.gtk.icons (
    errorSymbolic
    checkmarkSymbolic
)

type Bool -> Text
func iconFor = arg1 => arg1
 T|> checkmarkSymbolic
 F|> errorSymbolic
```

## Notes

- All constants are plain `Text` values; use them anywhere a `Text` expression is accepted.
- Icon names follow the [freedesktop Icon Naming Specification](https://specifications.freedesktop.org/icon-naming-spec/latest/) and the GNOME/Adwaita icon set.
- Symbolic variants (`-symbolic`) adapt to foreground colour and respect high-contrast themes; prefer them in header bars and toolbars.
- If you need an icon not listed here, pass the raw string directly — GTK will look it up in the current icon theme.
