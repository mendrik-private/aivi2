# aivi.text

Utilities for working with `Text` values.

The module combines low-level text intrinsics with higher-level helpers built in stdlib code. The
current surface is broader than the older docs suggested, even though it is still a helper module
rather than a richer text domain.

## Representative import

```aivi
use aivi.text (
    isEmpty
    nonEmpty
    join
    surround
    trim
    split
    replaceAll
    capitalize
    padStart
    lines
)
```

## Core operations

| Name | Type | Description |
| --- | --- | --- |
| `length` | `Text -> Int` | Length of a text value |
| `byteLen` | `Text -> Int` | Byte length of a text value |
| `slice` | `Int -> Int -> Text -> Text` | Slice text by start/end indexes |
| `find` | `Text -> Text -> Option Int` | Find one text inside another |
| `contains` | `Text -> Text -> Bool` | Test whether text contains a substring |
| `startsWith` | `Text -> Text -> Bool` | Test the start of a text value |
| `endsWith` | `Text -> Text -> Bool` | Test the end of a text value |
| `toUpper` | `Text -> Text` | Uppercase conversion |
| `toLower` | `Text -> Text` | Lowercase conversion |
| `trim` | `Text -> Text` | Trim both ends |
| `trimStart` | `Text -> Text` | Trim leading whitespace |
| `trimEnd` | `Text -> Text` | Trim trailing whitespace |
| `replace` | `Text -> Text -> Text -> Text` | Replace the first match |
| `replaceAll` | `Text -> Text -> Text -> Text` | Replace every match |
| `split` | `Text -> Text -> List Text` | Split text on a separator |
| `repeat` | `Int -> Text -> Text` | Repeat a chunk of text |
| `fromInt` | `Int -> Text` | Convert an integer to text |
| `parseInt` | `Text -> Option Int` | Parse text as an integer |
| `fromBool` | `Bool -> Text` | Convert a boolean to text |
| `parseBool` | `Text -> Option Bool` | Parse text as a boolean |
| `concat` | `List Text -> Text` | Concatenate several text values |

## Stdlib helpers

| Name | Type | Description |
| --- | --- | --- |
| `isEmpty` | `Text -> Bool` | `True` for `""` |
| `nonEmpty` | `Text -> Bool` | `True` when text is not empty |
| `join` | `Text -> List Text -> Text` | Join text values with a separator |
| `surround` | `Text -> Text -> Text -> Text` | Wrap text with prefix/suffix |
| `surroundWith` | `Text -> Text -> Text -> Text` | Alias of `surround` |
| `withDefault` | `Text -> Text -> Text` | Replace `""` with a fallback |
| `upper` | `Text -> Text` | Wrapper around `toUpper` |
| `lower` | `Text -> Text` | Wrapper around `toLower` |
| `capitalize` | `Text -> Text` | Uppercase the first character and lowercase the rest |
| `hasMinLength` | `Text -> Int -> Bool` | Check a minimum length |
| `hasMaxLength` | `Text -> Int -> Bool` | Check a maximum length |
| `includesText` | `Text -> Text -> Bool` | Helper over `contains` |
| `stripBlanks` | `Text -> Text` | Wrapper around `trim` |
| `padStart` | `Int -> Text -> Text -> Text` | Left-pad text to a target length |
| `padEnd` | `Int -> Text -> Text -> Text` | Right-pad text to a target length |
| `parseIntOrElse` | `Int -> Text -> Int` | Parse an int or use a fallback |
| `lines` | `Text -> List Text` | Split on newline characters |
| `words` | `Text -> List Text` | Split on spaces |

## Example

```aivi
use aivi.text (
    join
    trim
    capitalize
    lines
    parseIntOrElse
)

value title : Text =
    capitalize (trim "  aivi  ")

value csv : Text =
    join "," ["Ada", "Grace", "Linus"]

value count : Int =
    parseIntOrElse 0 "42"

value rows : List Text =
    lines "a\nb\nc"
```

## Current limits

`aivi.text` is still a helper-oriented module:

- no richer text domain with structured patch/algebra support
- no dedicated interpolation, formatting, or template surface here
- no explicit grapheme-aware or locale-aware text model in the public stdlib page yet
