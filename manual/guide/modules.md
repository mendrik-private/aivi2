# Modules

Each `.aivi` file is a module. Modules import names with `use` and expose names with `export`.

## Importing with `use`

```aivi
use aivi.network (
    http
    socket
    Request
    Channel
)

type PrimaryRequest = (Request Text)

type ProviderChannel = (Channel Text Text)
```

Imported names become available to the rest of the file.

## Import aliases

Use `as` when you want a local name that differs from the exported one:

```aivi
use aivi.network (
    http as primaryHttp
    Request as HttpRequest
)

type RequestPayload = (HttpRequest Text)

value selectedProvider = primaryHttp
```

## Exporting names

You can export one name:

```aivi
value greeting = "hello"

export greeting
```

Or several names together:

```aivi
type Direction =
  | Up
  | Down
  | Left
  | Right

type Direction -> Direction
func opposite = .
 ||> Up    -> Down
 ||> Down  -> Up
 ||> Left  -> Right
 ||> Right -> Left

value startDirection : Direction = Right

export (Direction, opposite, startDirection)
```

## A small complete module

```aivi
use aivi.network (
    http
    socket
)

type Text -> Text -> Text
func joinProviders = left right =>
    "{left}/{right}"

value primaryProvider = http
value fallbackProvider = socket
value providerPair = joinProviders primaryProvider fallbackProvider

export providerPair
```

## Typical module layout

A practical order is:

1. `use`
2. `type` / `domain` / `class`
3. `func` and `value`
4. `signal`
5. `export`

That ordering is not required by the language, but it keeps modules easy to scan.

## Summary

| Form | Meaning |
| --- | --- |
| `use module (names)` | Import selected names |
| `use module (name as localName)` | Import one name under a local alias |
| `export name` | Export one name |
| `export (a, b, c)` | Export several names |
