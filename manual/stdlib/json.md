# aivi.data.json

JSON parsing, querying, and formatting as async tasks.

All JSON operations return `Task Text A` — errors are task failures with a descriptive message.
The JSON value is always represented as a `Text` fragment (a raw JSON string), so you can
compose operations without needing a dedicated `Json` type.

## Import

```aivi
use aivi.data.json (
    validate
    get
    at
    keys
    pretty
    minify
)
```

## Overview

| Function   | Type                                      | Description                       |
|------------|-------------------------------------------|-----------------------------------|
| `validate` | `Text -> Task Text Bool`                  | Check whether text is valid JSON  |
| `get`      | `Text -> Text -> Task Text (Option Text)` | Get an object field by key        |
| `at`       | `Text -> Int -> Task Text (Option Text)`  | Get an array element by index     |
| `keys`     | `Text -> Task Text (List Text)`           | List object keys                  |
| `pretty`   | `Text -> Task Text Text`                  | Pretty-print JSON                 |
| `minify`   | `Text -> Task Text Text`                  | Minify JSON (remove whitespace)   |

## Functions

### validate

```
validate : Text -> Task Text Bool
```

Returns `True` if the text is valid JSON, `False` otherwise. Never fails — invalid text yields
`False`, not a task error.

```
use aivi.data.json (validate)

func checkJson = json=>```

### get

```
get : Text -> Text -> Task Text (Option Text)
```

Retrieve an object field by key. The result is the field value serialised back to JSON text,
so nested objects and arrays are preserved as `Text`. Returns `None` when the key is absent.
Fails the task when the input is not valid JSON.

```
use aivi.data.json (get)

func getName = json=>```

### at

```
at : Text -> Int -> Task Text (Option Text)
```

Retrieve an array element by zero-based index. Returns `None` when the index is out of bounds.
Fails the task when the input is not valid JSON.

```
use aivi.data.json (at)

func firstItem = json=>```

### keys

```
keys : Text -> Task Text (List Text)
```

Return the keys of a JSON object in insertion order. Returns an empty list for non-objects.
Fails the task when the input is not valid JSON.

```
use aivi.data.json (keys)

func objectKeys = json=>```

### pretty

```
pretty : Text -> Task Text Text
```

Re-format JSON with two-space indentation. Fails the task when the input is not valid JSON.

```
use aivi.data.json (pretty)

func format = json=>```

### minify

```
minify : Text -> Task Text Text
```

Remove all insignificant whitespace from JSON. Fails the task when the input is not valid JSON.

```
use aivi.data.json (minify)

func compact = json=>```

## Error type

```aivi
type JsonError =
  | InvalidJson
  | MissingKey
  | IndexOutOfBounds
  | WrongType
```

`JsonError` represents the four logical failure modes when working with JSON data.
Task failures carry a descriptive `Text` error message (the `Text` in `Task Text A`).

## Example — decode a simple object

```
use aivi.data.json (
    get
    keys
)

use aivi.option (withDefault)

func extractName = json=>```

## Example — normalise before storage

Extract each step into a named function so no pipes are nested.

```
use aivi.data.json (
    minify
    validate
)

use aivi.core (Task)

func minifyIfValid = raw isValid=>
func storeJson = raw=>```
