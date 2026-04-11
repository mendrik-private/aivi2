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

| Name | Type | Description |
| --- | --- | --- |
| `Url` | domain over `Text` | A validated URL value |
| `UrlError` | `Text` | Parse failure message |
| `.carrier` | `Url -> Text` | The raw URL text |

## Domain

```aivi
domain Url over Text
    parse : Text -> Result UrlError Url
    scheme : Url -> Option Text
    host : Url -> Option Text
    port : Url -> Option Int
    path : Url -> Text
    query : Url -> Option Text
    fragment : Url -> Option Text
    withPath : Url -> Text -> Url
    withQuery : Url -> Text -> Url
```

The domain members — `parse`, `scheme`, `host`, `port`, `path`, `query`, `fragment`,
`withPath`, `withQuery` — are part of the domain's internal implementation and are not
individually importable from user code. Use `Url` as an opaque validated type and access
`.carrier` when the raw text is required.

## `.carrier`

Access the raw URL text backing a `Url` value.

```aivi
use aivi.url (Url)

type Url -> Text
func rawAddress = url =>
    url.carrier
```

## Error type

```aivi
type UrlError = Text
```

When parsing fails, the module reports a plain text error message.

## Current limits

`aivi.url` is still a small `Text`-backed domain:

- no path-segment composition helpers
- no query-parameter merge/split helpers
- no `withScheme`, `withHost`, `withPort`, or `withFragment`
- no record-patch style field updates over URL parts
