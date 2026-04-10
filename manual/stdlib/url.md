# aivi.url

Typed URLs with explicit parsing.

`aivi.url` gives you a `Url` domain over `Text`. That means you can keep validated URLs as
their own type instead of passing around raw strings everywhere.

Construction is explicit: you parse text into `Url`, and you access `.carrier` on a `Url`
when you need the raw value again.

## Import

```aivi
use aivi.url (
    Url
    UrlError
)
```

## Overview

| Member | Type | Description |
| --- | --- | --- |
| `Url.parse` | `Text -> Result UrlError Url` | Parse text into a typed URL |
| `.scheme` | `Url -> Option Text` | Read the scheme if present |
| `.host` | `Url -> Option Text` | Read the host if present |
| `.port` | `Url -> Option Int` | Read the port if present |
| `.path` | `Url -> Text` | Read the path part |
| `.query` | `Url -> Option Text` | Read the query text if present |
| `.fragment` | `Url -> Option Text` | Read the fragment text if present |
| `.withPath` | `Url -> Text -> Url` | Return a copy with a different path |
| `.withQuery` | `Url -> Text -> Url` | Return a copy with a different query |

## `parse`

```aivi
```

Use this when URL text comes from config, user input, or another external source.

```aivi
use aivi.url (
    Url
    UrlError
)

value apiBase : Result UrlError Url = Url.parse "https://api.example.com/v1/users?page=1"
```

## `.carrier`

```aivi
```

Access the raw URL text.

```aivi
use aivi.url (Url)

type Url -> Text
func rawAddress = url =>
    url.carrier
```

## Accessors

The accessors let you inspect one part at a time.

- `.scheme`, `.host`, `.port`, `.query`, and `.fragment` return `Option ...` because those parts
  may be absent
- `.path` always returns `Text`

All accessors are domain members and are called using dot notation on a `Url` value.

```aivi
use aivi.url (Url)

type Url -> Text
func routeOnly = url =>
    url.path
```

The query is returned as plain text if present. This module does not split it into separate
key/value pairs for you.

## Updating a URL

### `withPath`

```aivi
```

Return a new `Url` with a different path.

### `withQuery`

```aivi
```

Return a new `Url` with a different query string.

```aivi
use aivi.url (Url)

type Url -> Url
func moveToSearchPath = url =>
    url.withPath "/search"

type Url -> Url
func toSearchPage = url =>
    (url.withPath "/search").withQuery "q=aivi"
```

## Error type

```aivi
type UrlError = Text
```

Failed parses report a plain text error message.

## Example — keep URLs typed until the edge

```aivi
use aivi.url (Url)

type Url -> Text
func avatarEndpoint = base =>
    (base.withPath "/api/avatar").carrier
```

## Current limits

`aivi.url` is still a small `Text`-backed domain:

- no path-segment composition helpers
- no query-parameter merge/split helpers
- no `withScheme`, `withHost`, `withPort`, or `withFragment`
- no record-patch style field updates over URL parts
