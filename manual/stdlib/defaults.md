# aivi.defaults

Small default values for a few common built-in types.

This module is intentionally tiny. Use it when you want a clear starting value for text, numbers,
or flags without repeating `""`, `0`, or `False` across your code. It also re-exports `Option`, so
small modules can import the type and these defaults together.

## Import

```aivi
use aivi.defaults (
    Option
    defaultText
    defaultInt
    defaultBool
)
```

## Overview

| Name | Type | Description |
|------|------|-------------|
| `defaultText` | `Text` | Empty text |
| `defaultInt` | `Int` | Zero |
| `defaultBool` | `Bool` | `False` |
| `Option` | `Option A` | Standard `Option` type, re-exported unchanged |

## Values

### defaultText

```aivi
// <unparseable item>
```

An empty `Text` value. Useful for form fields, search boxes, labels, and other text that starts
blank.

```aivi
use aivi.defaults (defaultText)

value searchQuery = defaultText
```

### defaultInt

```aivi
// <unparseable item>
```

The number `0`. Useful for counters, indexes, totals, or retry counts that should start empty.

```aivi
use aivi.defaults (defaultInt)

value retryCount = defaultInt
```

### defaultBool

```aivi
// <unparseable item>
```

The boolean value `False`. Useful for flags that should start turned off.

```aivi
use aivi.defaults (defaultBool)

value hasUnsavedChanges = defaultBool
```

## Re-export

### Option

The standard `Option` type is re-exported unchanged. This module does not add new option helpers;
if you need option functions, import them from `aivi.option`.

## Example — seed a simple draft record

```aivi
use aivi.defaults (
    defaultText
    defaultInt
    defaultBool
)

type Draft = {
    title: Text,
    retries: Int,
    dirty: Bool
}

value emptyDraft : Draft = {
    title: defaultText,
    retries: defaultInt,
    dirty: defaultBool
}
```

## Example — keep a filter optional

```aivi
use aivi.defaults (Option)

type SearchFilter = Option Text
```
