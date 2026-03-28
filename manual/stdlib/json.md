# aivi.data.json

JSON parsing, querying, and formatting as async tasks.

All JSON operations return `Task Text A` — errors are task failures with a descriptive message.
The JSON value is always represented as a `Text` fragment (a raw JSON string), so you can
compose operations without needing a dedicated `Json` type.

## Import

```aivi
use aivi.data.json (validate, get, at, keys, pretty, minify)
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

```aivi
use aivi.data.json (validate)

fun checkJson json =
  validate json
```

### get

```
get : Text -> Text -> Task Text (Option Text)
```

Retrieve an object field by key. The result is the field value serialised back to JSON text,
so nested objects and arrays are preserved as `Text`. Returns `None` when the key is absent.
Fails the task when the input is not valid JSON.

```aivi
use aivi.data.json (get)

fun getName json =
  get json "name"
// None  -- key absent
// Some "\"Alice\""  -- string values include their JSON quotes
```

### at

```
at : Text -> Int -> Task Text (Option Text)
```

Retrieve an array element by zero-based index. Returns `None` when the index is out of bounds.
Fails the task when the input is not valid JSON.

```aivi
use aivi.data.json (at)

fun firstItem json =
  at json 0
// None  -- empty array
// Some "42"  -- numeric values are plain text
```

### keys

```
keys : Text -> Task Text (List Text)
```

Return the keys of a JSON object in insertion order. Returns an empty list for non-objects.
Fails the task when the input is not valid JSON.

```aivi
use aivi.data.json (keys)

fun objectKeys json =
  keys json
// ["name", "age", "active"]
```

### pretty

```
pretty : Text -> Task Text Text
```

Re-format JSON with two-space indentation. Fails the task when the input is not valid JSON.

```aivi
use aivi.data.json (pretty)

fun format json =
  pretty json
```

### minify

```
minify : Text -> Task Text Text
```

Remove all insignificant whitespace from JSON. Fails the task when the input is not valid JSON.

```aivi
use aivi.data.json (minify)

fun compact json =
  minify json
```

## Error type

```aivi
type JsonError = InvalidJson | MissingKey | IndexOutOfBounds | WrongType
```

`JsonError` represents the four logical failure modes when working with JSON data.
Task failures carry a descriptive `Text` error message (the `Text` in `Task Text A`).

## Example — decode a simple object

```aivi
use aivi.data.json (get, keys)
use aivi.option (withDefault)

fun extractName json =
  get json "name"
  |> map (withDefault "unknown")
```

## Example — normalise before storage

```aivi
use aivi.data.json (minify, validate)

fun storeJson raw =
  validate raw
  |> andThen (result =>
      result
      ||> True -> minify raw
      ||> False -> Task.fail "invalid JSON input"
  )
```
