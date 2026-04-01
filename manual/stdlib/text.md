# aivi.text

Utilities for working with `Text` values: emptiness checks, joining lists of strings, and wrapping text with a prefix and suffix.

```aivi
use aivi.text (
    isEmpty
    nonEmpty
    join
    surround
)
```

---

## isEmpty

Returns `True` if the text is an empty string (`""`).

```aivi
isEmpty : Text -> Bool
```

```aivi
use aivi.text (isEmpty)

type Text -> Text
func showPlaceholder = label=> isEmpty label T|> "Untitled"
 F|> label
```

---

## nonEmpty

Returns `True` if the text is not empty. The negation of `isEmpty`.

```aivi
nonEmpty : Text -> Bool
```

```aivi
use aivi.list (filter)

use aivi.text (nonEmpty)

type List Text -> List Text
func filterLabels = labels=>    filter nonEmpty labels
```

---

## join

Joins a list of `Text` values into a single string, inserting the given separator between each element. Use `join "" parts` when you want concatenation with no separator.

```aivi
join : Text -> List Text -> Text
```

```aivi
use aivi.text (join)

type List Text -> Text
func csvLine = fields=>    join "," fields
```


---

## surround

Wraps a `Text` value with a prefix and a suffix.

```aivi
surround : Text -> Text -> Text -> Text
```

```aivi
use aivi.text (surround)

type Text -> Text
func htmlParagraph = content=>    surround "<p>" "</p>" content
```
