# aivi.data.json

Structural JSON types and predicates.

`aivi.data.json` no longer exports public JSON-as-text task helpers such as `validate`, `get`, or
`pretty`. External JSON is expected to decode at the provider boundary straight into the annotated
target type.

## Import

```aivi
use aivi.data.json (
    JsonError
    InvalidJson
    MissingKey
    IndexOutOfBounds
    WrongType
    Json
    JsonNull
    JsonBool
    JsonNumber
    JsonString
    JsonArray
    JsonObject
    JsonPath
    isNull
    isObject
    isArray
    isBool
    isNumber
    isString
)
```

## Preferred external usage

```aivi
@source http "https://api.example.com"
signal api : HttpSource

signal profile : Signal (HttpResponse Profile) = api.get "/profile"
```

Use `Json` and the predicate helpers when you are already holding structural JSON data inside the
language. Do not round-trip raw external payloads through JSON text helpers.
