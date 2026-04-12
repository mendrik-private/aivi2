# aivi.data.json

JSON text helpers plus structural JSON vocabulary.

This module has two layers today:

1. structural `Json` / `JsonError` types exported from the stdlib source file
2. compiler-backed helpers like `validate`, `get`, and `pretty` that operate on raw JSON text and
   return `Task` values in the current runtime

```aivi
use aivi.data.json (
    Json
    JsonNull
    isNull
    validate
    get
    pretty
)
```

---

## Structural JSON types

```aivi
use aivi.core.dict (Dict)

type Json =
  | JsonNull
  | JsonBool Bool
  | JsonNumber Float
  | JsonString Text
  | JsonArray (List Json)
  | JsonObject (Dict Text Json)
```

```aivi
use aivi.core.dict (Dict)

use aivi.data.json (
    Json
    JsonObject
    isObject
)

value payload : Json =
    JsonObject {
        entries: []
    }

value isEmptyObject : Bool = isObject payload
```

The module also exports `JsonError` and `JsonPath`:

```aivi
type JsonError =
  | InvalidJson Text
  | MissingKey Text
  | IndexOutOfBounds Int
  | WrongType Text

type JsonPath = List Text
```

Predicates exported from the module file:

- `isNull`
- `isObject`
- `isArray`
- `isBool`
- `isNumber`
- `isString`

---

## Text-level JSON helpers

These are compiler-backed helpers over raw JSON text.

| Name | Type |
| --- | --- |
| `validate` | `Text -> Task Text Bool` |
| `get` | `Text -> Text -> Task Text (Option Text)` |
| `at` | `Text -> Int -> Task Text (Option Text)` |
| `keys` | `Text -> Task Text (List Text)` |
| `pretty` | `Text -> Task Text Text` |
| `minify` | `Text -> Task Text Text` |

```aivi
use aivi.data.json (
    get
    pretty
    validate
)

value validPayload : Task Text Bool = validate "\{\"name\":\"Ada\"\}"
value userName : Task Text (Option Text) = get "\{\"name\":\"Ada\"\}" "name"
value prettyPayload : Task Text Text = pretty "\{\"name\":\"Ada\",\"role\":\"admin\"\}"
```

`get` and `at` return raw JSON text fragments today, not decoded `Json` values. That is a
compatibility surface; newer provider-based decode paths should prefer typed decoding at the source
boundary.
