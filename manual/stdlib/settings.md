# aivi.gnome.settings

Types for working with GNOME settings (GSettings).

GSettings is the desktop settings system used for values such as the color scheme, text
scaling, and many other GNOME preferences.

This module currently exports the schema type, key type, setting value type, and task alias
used by GSettings integrations. The stdlib comments also document the watcher source shape.

## Import

```aivi
use aivi.gnome.settings (
    SettingsError
    SettingsSchema
    SettingsKey
    SettingValue
    SettingsTask
)
```

## Overview

| Item | Type | Description |
|------|------|-------------|
| `SettingsError` | type | Things that can go wrong when resolving or decoding a setting |
| `SettingsSchema` | domain over `Text` | Checked schema identifier |
| `SettingsKey` | domain over `Text` | Wrapped key name |
| `SettingValue` | type | Generic setting value |
| `SettingsTask A` | `Task SettingsError A` | Generic settings task alias |
| `gsettings.watch` | source | Documented watcher source shape |

## Types

### SettingsError

```aivi
type SettingsError =
  | SchemaNotFound Text
  | KeyNotFound Text
  | TypeMismatch Text
  | SettingsUnavailable
```

These variants describe the usual GSettings failure cases.

- `SchemaNotFound` — the schema ID does not exist
- `KeyNotFound` — the schema exists, but the key does not
- `TypeMismatch` — the key exists, but not with the type you expected
- `SettingsUnavailable` — GSettings access is not available in the current runtime

### SettingsSchema

```aivi
domain SettingsSchema over Text
    parse : Text -> Result SettingsError SettingsSchema
```

Checked schema identifier such as `"org.gnome.desktop.interface"`.

```aivi
use aivi.gnome.settings (SettingsSchema)

value interfaceSchema : Result SettingsError SettingsSchema = parse "org.gnome.desktop.interface"
```

### SettingsKey

```aivi
domain SettingsKey over Text
    make : Text -> SettingsKey
```

Wrapped key name such as `"color-scheme"`.

`SettingsKey` uses `make` rather than `parse`, so wrapping a key name is direct.

```aivi
use aivi.gnome.settings (SettingsKey)

value colorSchemeKey : SettingsKey = make "color-scheme"
```

### SettingValue

```aivi
type SettingValue =
  | SettingBool Bool
  | SettingInt Int
  | SettingFloat Float
  | SettingText Text
  | SettingList (List Text)
```

Generic setting value for code that needs to handle several setting types in one place.

```aivi
use aivi.gnome.settings (
    SettingValue
    SettingBool
    SettingInt
    SettingFloat
    SettingText
    SettingList
)

type SettingValue -> Text
func settingKind = value => value
 ||> SettingBool b  -> "bool"
 ||> SettingInt n   -> "int"
 ||> SettingFloat x -> "float"
 ||> SettingText t  -> "text"
 ||> SettingList xs -> "list"
```

### SettingsTask

```aivi
type SettingsTask A = (Task SettingsError A)
```

Generic alias for settings-related tasks.

## Documented source shapes

The stdlib module comments document the following watcher patterns:

```aivi
@source gsettings.watch "org.gnome.desktop.interface" "color-scheme"
signal colorScheme : Signal (Result SettingsError Text)

@source gsettings.watch "org.gnome.desktop.interface" "text-scaling-factor"
signal textScale : Signal (Result SettingsError Float)
```

The concrete signal payload depends on the key you watch. This module does not currently
export direct read or write helpers.
