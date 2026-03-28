# aivi.text

Utilities for working with `Text` values: emptiness checks, joining lists of strings, and wrapping text with a prefix and suffix.

```aivi
use aivi.text (
    isEmpty
    nonEmpty
    join
    concat
    surround
)
```

---

## isEmpty

Returns `True` if the text is an empty string (`""`).

```
isEmpty : Text -> Bool
```

```aivi
use aivi.text (isEmpty)

fun placeholderFor:Text isBlank:Bool label:Text => isBlank
  T|> "Untitled"
  F|> label

fun showPlaceholder:Text label:Text =>
    placeholderFor (isEmpty label) label
```

---

## nonEmpty

Returns `True` if the text is not empty. The negation of `isEmpty`.

```
nonEmpty : Text -> Bool
```

```aivi
use aivi.text (nonEmpty)

fun filterLabels: List Text labels: List Text =>
    filter nonEmpty labels
```

---

## join

Joins a list of `Text` values into a single string, inserting the given separator between each element.

```
join : Text -> List Text -> Text
```

```aivi
use aivi.text (join)

fun csvLine:Text fields: List Text =>
    join "," fields
```

---

## concat

Concatenates a list of `Text` values with no separator. Equivalent to `join ""`.

```
concat : List Text -> Text
```

```aivi
use aivi.text (concat)

fun buildPath:Text segments: List Text =>
    concat segments
```

---

## surround

Wraps a `Text` value with a prefix and a suffix.

```
surround : Text -> Text -> Text -> Text
```

```aivi
use aivi.text (surround)

fun htmlParagraph:Text content:Text =>
    surround "<p>" "</p>" content
```
