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

## Exporting names

You can export one name:

```aivi
value greeting = "hello"

export greeting
```

Or several names together:

```aivi
data Direction =
  | Up
  | Down
  | Left
  | Right

fun opposite: Direction direction:Direction =>
    direction
     ||> Up    -> Down
     ||> Down  -> Up
     ||> Left  -> Right
     ||> Right -> Left

value startDirection: Direction = Right

export (Direction, opposite, startDirection)
```

## A small complete module

```aivi
use aivi.network (
    http
    socket
)

fun joinProviders: Text left:Text right:Text =>
    "{left}/{right}"

value primaryProvider = http
value fallbackProvider = socket
value providerPair = joinProviders primaryProvider fallbackProvider

export providerPair
```

## Typical module layout

A practical order is:

1. `use`
2. `type` / `data` / `domain` / `class`
3. `fun` and `value`
4. `signal`
5. `export`

That ordering is not required by the language, but it keeps modules easy to scan.

## Summary

| Form | Meaning |
| --- | --- |
| `use module (names)` | Import selected names |
| `export name` | Export one name |
| `export (a, b, c)` | Export several names |
