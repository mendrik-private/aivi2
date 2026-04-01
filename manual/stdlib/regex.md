# aivi.regex

Regular-expression matching and replacement as async tasks.

All functions in this module return `Task RegexError A`. A bad pattern fails the task with a text
error. A valid pattern that simply finds nothing is not an error — you get `False`, `None`, an
empty list, or the original text, depending on the function.

## Import

```aivi
use aivi.regex (
    isMatch
    find
    findText
    findAll
    replace
    replaceAll
    emailPattern
    whitespacePattern
)
```

Alias names such as `matches`, `firstIndex`, `firstMatch`, `allMatches`, `replaceFirst`,
`replaceEach`, and `hasMatch` are also exported.

## Types

### Pattern

```aivi
type Pattern = Text
```

A regex pattern written as text. Patterns are compiled when the task runs. Pattern syntax
currently follows Rust's `regex` engine, and normal AIVI string escaping still applies, so a
single backslash in the pattern is written as `\\` in source code.

## Overview

### Search and replace

| Name | Type | Description |
|------|------|-------------|
| `isMatch` / `matches` / `hasMatch` | `Pattern -> Text -> Task RegexError Bool` | Check whether the text contains a match |
| `find` / `firstIndex` | `Pattern -> Text -> Task RegexError (Option Int)` | Find the first match position |
| `findText` / `firstMatch` | `Pattern -> Text -> Task RegexError (Option Text)` | Return the first matched text |
| `findAll` / `allMatches` | `Pattern -> Text -> Task RegexError (List Text)` | Return all matched snippets |
| `replace` / `replaceFirst` | `Pattern -> Text -> Text -> Task RegexError Text` | Replace the first match |
| `replaceAll` / `replaceEach` | `Pattern -> Text -> Text -> Task RegexError Text` | Replace every match |

### Built-in patterns and validators

| Name | Type | Description |
|------|------|-------------|
| `emailPattern` | `Pattern` | Simple email-shaped text |
| `urlPattern` | `Pattern` | `http://` or `https://` URL text |
| `intPattern` | `Pattern` | Whole numbers with an optional leading `-` |
| `floatPattern` | `Pattern` | Whole or decimal numbers with an optional leading `-` |
| `whitespacePattern` | `Pattern` | One or more whitespace characters |
| `alphanumPattern` | `Pattern` | Letters and digits only |
| `isEmail` | `Text -> Task RegexError Bool` | Check `emailPattern` |
| `isUrl` | `Text -> Task RegexError Bool` | Check `urlPattern` |
| `isIntText` | `Text -> Task RegexError Bool` | Check `intPattern` |
| `isAlphaNum` | `Text -> Task RegexError Bool` | Check `alphanumPattern` |

## Functions

### isMatch / matches / hasMatch

```aivi
isMatch : Pattern -> Text -> Task RegexError Bool
matches : Pattern -> Text -> Task RegexError Bool
hasMatch : Pattern -> Text -> Task RegexError Bool
```

Return `True` when the text contains at least one match. Returns `False` when the pattern is valid
but nothing matches.

### find / firstIndex

```aivi
find : Pattern -> Text -> Task RegexError (Option Int)
firstIndex : Pattern -> Text -> Task RegexError (Option Int)
```

Return the first match position as a character index. Returns `None` when there is no match.

The index counts characters, not UTF-8 bytes, so it stays useful with non-ASCII text.

### findText / firstMatch

```aivi
findText : Pattern -> Text -> Task RegexError (Option Text)
firstMatch : Pattern -> Text -> Task RegexError (Option Text)
```

Return the first matched snippet as text. Returns `None` when there is no match.

### findAll / allMatches

```aivi
findAll : Pattern -> Text -> Task RegexError (List Text)
allMatches : Pattern -> Text -> Task RegexError (List Text)
```

Return every full match from left to right. If nothing matches, the result is an empty list.

This wrapper returns matched text only. It does not currently expose capture groups.

### replace / replaceFirst

```aivi
replace : Pattern -> Text -> Text -> Task RegexError Text
replaceFirst : Pattern -> Text -> Text -> Task RegexError Text
```

Replace the first match and return the updated text. If nothing matches, the original text is
returned unchanged.

The argument order is `pattern -> replacement -> text`.

### replaceAll / replaceEach

```aivi
replaceAll : Pattern -> Text -> Text -> Task RegexError Text
replaceEach : Pattern -> Text -> Text -> Task RegexError Text
```

Replace every match and return the updated text. If nothing matches, the original text is returned
unchanged.

### Common patterns

```aivi
emailPattern : Pattern
urlPattern : Pattern
intPattern : Pattern
floatPattern : Pattern
whitespacePattern : Pattern
alphanumPattern : Pattern
```

These ready-made patterns cover a few common cases:

- `emailPattern` — simple email-shaped text
- `urlPattern` — URLs that start with `http://` or `https://`
- `intPattern` — whole numbers like `7` or `-12`
- `floatPattern` — whole or decimal numbers like `7` or `-12.5`
- `whitespacePattern` — one or more spaces, tabs, or other whitespace
- `alphanumPattern` — letters and digits only

### Convenience validators

```aivi
isEmail : Text -> Task RegexError Bool
isUrl : Text -> Task RegexError Bool
isIntText : Text -> Task RegexError Bool
isAlphaNum : Text -> Task RegexError Bool
```

These helpers run `isMatch` with the matching built-in pattern.

There is currently no `isFloatText` helper. Use `isMatch floatPattern text` when you need that
check.

## Error type

```aivi
type RegexError = Text
```

Regex task failures carry a descriptive `Text` message. This usually happens when the pattern
cannot be compiled.

## Example — check a signup email

```aivi
use aivi.regex (isEmail)

func emailLooksValid = email =>
```

## Example — normalise repeated whitespace

```aivi
use aivi.regex (
    replaceAll
    whitespacePattern
)

func tidySentence = text =>
```
