# aivi.url

Typed URLs with explicit parsing.

`aivi.url` gives you a `Url` domain over `Text`. That means you can keep validated URLs as
their own type instead of passing around raw strings everywhere.

Construction is explicit: you parse text into `Url`, and you unwrap a `Url` back to `Text`
when you need the raw value again.

## Import

```aivi
use aivi.url (
    Url
    UrlError
    parse
    unwrap
    scheme
    host
    port
    path
    query
    fragment
    withPath
    withQuery
)
```

## Overview

| Member | Type | Description |
| --- | --- | --- |
| `parse` | `Text -> Result UrlError Url` | Parse text into a typed URL |
| `unwrap` | `Url -> Text` | Get the raw URL text back |
| `scheme` | `Url -> Option Text` | Read the scheme if present |
| `host` | `Url -> Option Text` | Read the host if present |
| `port` | `Url -> Option Int` | Read the port if present |
| `path` | `Url -> Text` | Read the path part |
| `query` | `Url -> Option Text` | Read the query text if present |
| `fragment` | `Url -> Option Text` | Read the fragment text if present |
| `withPath` | `Url -> Text -> Url` | Return a copy with a different path |
| `withQuery` | `Url -> Text -> Url` | Return a copy with a different query |

## `parse`

```aivi
parse : Text -> Result UrlError Url
```

Use this when URL text comes from config, user input, or another external source.

```aivi
use aivi.url (
    Url
    UrlError
    parse
)

value apiBase : Result UrlError Url =
    parse "https://api.example.com/v1/users?page=1"
```

## `unwrap`

```aivi
unwrap : Url -> Text
```

Convert a typed URL back to raw text.

```aivi
use aivi.url (
    Url
    unwrap
)

type Url -> Text
func rawAddress = url =>
    unwrap url
```

## Accessors

The accessors let you inspect one part at a time.

- `scheme`, `host`, `port`, `query`, and `fragment` return `Option ...` because those parts
  may be absent
- `path` always returns `Text`

```aivi
use aivi.url (
    Url
    host
    path
    query
)

type Url -> Text
func routeOnly = url =>
    path url
```

The query is returned as plain text if present. This module does not split it into separate
key/value pairs for you.

## Updating a URL

### `withPath`

```aivi
withPath : Url -> Text -> Url
```

Return a new `Url` with a different path.

### `withQuery`

```aivi
withQuery : Url -> Text -> Url
```

Return a new `Url` with a different query string.

```aivi
use aivi.url (
    Url
    withPath
    withQuery
)

type Url -> Url
func moveToSearchPath = url =>
    withPath url "/search"

type Url -> Url
func toSearchPage = url =>
    withQuery (moveToSearchPath url) "q=aivi"
```

## Error type

```aivi
type UrlError = Text
```

Failed parses report a plain text error message.

## Example — keep URLs typed until the edge

```aivi
use aivi.url (
    Url
    unwrap
    withPath
)

type Url -> Text
func avatarEndpoint = base =>
    unwrap (withPath base "/api/avatar")
```

## Current limits

`aivi.url` is still a small `Text`-backed domain:

- no path-segment composition helpers
- no query-parameter merge/split helpers
- no `withScheme`, `withHost`, `withPort`, or `withFragment`
- no record-patch style field updates over URL parts
